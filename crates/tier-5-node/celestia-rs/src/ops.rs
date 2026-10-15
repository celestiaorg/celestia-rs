//! Operational plumbing of `celestia-rs start`: graceful shutdown
//! (SIGINT / SIGTERM), logging configuration (`log_level`, `log_format`)
//! and the open-files limit check.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

type Hook = Box<dyn Fn() + Send>;

/// A shutdown request shared by the signal handler and the node's loops:
/// hooks run once when it triggers (stop consensus, stop block sync), and
/// [`Shutdown::wait`] blocks until then.
#[derive(Clone, Default)]
pub struct Shutdown {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    flag: AtomicBool,
    hooks: Mutex<Vec<Hook>>,
    lock: Mutex<()>,
    cv: Condvar,
}

impl std::fmt::Debug for Shutdown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shutdown")
            .field("triggered", &self.is_triggered())
            .finish()
    }
}

impl Shutdown {
    /// A fresh, untriggered handle.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests shutdown: runs the hooks (once) and wakes waiters.
    pub fn trigger(&self) {
        if self.inner.flag.swap(true, Ordering::SeqCst) {
            return;
        }
        let hooks = std::mem::take(
            &mut *self
                .inner
                .hooks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for h in hooks {
            h();
        }
        let _g = self
            .inner
            .lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.inner.cv.notify_all();
    }

    /// Whether shutdown was requested.
    #[must_use]
    pub fn is_triggered(&self) -> bool {
        self.inner.flag.load(Ordering::SeqCst)
    }

    /// Runs `f` on shutdown (immediately when already triggered).
    pub fn on_trigger(&self, f: impl Fn() + Send + 'static) {
        if self.is_triggered() {
            f();
            return;
        }
        self.inner
            .hooks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(Box::new(f));
        // Raced with `trigger`: run the hooks that it may have missed.
        if self.is_triggered() {
            let hooks = std::mem::take(
                &mut *self
                    .inner
                    .hooks
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            for h in hooks {
                h();
            }
        }
    }

    /// Blocks until shutdown is requested.
    pub fn wait(&self) {
        let mut g = self
            .inner
            .lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !self.is_triggered() {
            g = self
                .inner
                .cv
                .wait_timeout(g, std::time::Duration::from_millis(200))
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }

    /// Sleeps up to `d`, returning early (true) on shutdown.
    #[must_use]
    pub fn sleep(&self, d: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + d;
        let mut g = self
            .inner
            .lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !self.is_triggered() {
            let now = std::time::Instant::now();
            if now >= deadline {
                return false;
            }
            g = self
                .inner
                .cv
                .wait_timeout(g, deadline - now)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        true
    }
}

/// Installs SIGINT / SIGTERM handlers on the current tokio runtime that
/// trigger `shutdown` (a second signal exits immediately).
pub fn install_signal_handlers(shutdown: &Shutdown) {
    let s = shutdown.clone();
    tokio::spawn(async move {
        #[cfg(unix)]
        let term = async {
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(mut t) => {
                    t.recv().await;
                }
                Err(_) => std::future::pending::<()>().await,
            }
        };
        #[cfg(not(unix))]
        let term = std::future::pending::<()>();
        let sig = tokio::select! {
            _ = tokio::signal::ctrl_c() => "SIGINT",
            () = term => "SIGTERM",
        };
        tracing::info!(
            signal = sig,
            "shutting down (flushing state, WAL and stores)"
        );
        s.trigger();
        let _ = tokio::signal::ctrl_c().await;
        tracing::warn!("second signal: exiting immediately");
        std::process::exit(130);
    });
}

/// Maps Go log settings to a `tracing` filter: `info`, `debug`, `error`,
/// `warn`, `none`, or module lists like `consensus:debug,*:error`.
#[must_use]
pub fn log_filter(level: &str) -> String {
    let level = level.trim();
    if level.is_empty() {
        return "info".into();
    }
    level
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| {
            let map = |l: &str| match l {
                "none" => "off".to_owned(),
                other => other.to_owned(),
            };
            match p.split_once(':') {
                Some(("*", l)) => map(l),
                Some((m, l)) => format!("{m}={}", map(l)),
                None => map(p),
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Whether log lines get ANSI colors: only when stdout is a terminal, so a
/// redirected log (a file, launchd's `StandardOutPath`, journald) stays
/// plain text, and never when `NO_COLOR` is set to a non-empty value.
fn use_ansi(stdout_is_terminal: bool, no_color: Option<String>) -> bool {
    stdout_is_terminal && no_color.is_none_or(|v| v.is_empty())
}

/// Initializes logging: `RUST_LOG` wins over `level`; `format` is `plain`
/// (default) or `json`.
pub fn init_logging(level: &str, format: &str) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_filter(level)));
    let ansi = use_ansi(
        std::io::IsTerminal::is_terminal(&std::io::stdout()),
        std::env::var("NO_COLOR").ok(),
    );
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(ansi);
    let _ = if format == "json" {
        builder.json().try_init()
    } else {
        builder.try_init()
    };
}

/// The soft `RLIMIT_NOFILE` of this process, if it can be determined.
#[must_use]
pub fn open_files_limit() -> Option<u64> {
    if let Ok(s) = std::fs::read_to_string("/proc/self/limits") {
        for line in s.lines() {
            if let Some(rest) = line.strip_prefix("Max open files") {
                let soft = rest.split_whitespace().next()?;
                return if soft == "unlimited" {
                    Some(u64::MAX)
                } else {
                    soft.parse().ok()
                };
            }
        }
    }
    // macOS (and other Unixes without procfs): the shell builtin reports
    // the limit inherited from this process.
    let out = std::process::Command::new("sh")
        .args(["-c", "ulimit -n"])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if s == "unlimited" {
        Some(u64::MAX)
    } else {
        s.parse().ok()
    }
}

/// Minimum open-files limit the node wants (RocksDB keeps up to 512 SST
/// files open per database, plus P2P / RPC sockets).
pub const MIN_OPEN_FILES: u64 = 4096;

/// Warns when the open-files limit is below [`MIN_OPEN_FILES`].
pub fn check_open_files() -> Option<u64> {
    let n = open_files_limit()?;
    if n < MIN_OPEN_FILES {
        tracing::warn!(
            limit = n,
            recommended = MIN_OPEN_FILES,
            "the open-files limit (ulimit -n) is low; raise it before starting the node \
             (e.g. `ulimit -n {MIN_OPEN_FILES}`, or NumberOfFiles in the launchd plist) — \
             RocksDB and P2P fail with 'Too many open files' otherwise"
        );
    }
    Some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters() {
        assert_eq!(log_filter("info"), "info");
        assert_eq!(log_filter("none"), "off");
        assert_eq!(
            log_filter("consensus:debug,*:error"),
            "consensus=debug,error"
        );
        assert_eq!(log_filter(""), "info");
    }

    #[test]
    fn ansi_only_on_a_terminal() {
        // A file or launchd/systemd log must not get escape codes.
        assert!(use_ansi(true, None));
        assert!(!use_ansi(false, None));
        // https://no-color.org: any non-empty NO_COLOR disables colors.
        assert!(!use_ansi(true, Some("1".into())));
        assert!(use_ansi(true, Some(String::new())));
    }

    #[test]
    fn shutdown_hooks_run_once() {
        let s = Shutdown::new();
        let n = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let n2 = n.clone();
        s.on_trigger(move || {
            n2.fetch_add(1, Ordering::SeqCst);
        });
        assert!(!s.sleep(std::time::Duration::from_millis(1)));
        s.trigger();
        s.trigger();
        s.wait();
        assert_eq!(n.load(Ordering::SeqCst), 1);
        assert!(s.sleep(std::time::Duration::from_secs(10)));
    }

    #[test]
    fn nofile_limit_is_readable() {
        assert!(open_files_limit().is_some());
    }
}
