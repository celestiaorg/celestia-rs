//! Server metrics (the instruments of Go `server_metrics.go`, plus the
//! status-code, rejection, connection and health series an operator needs
//! to debug an embedded server).
//!
//! The node owns the Prometheus endpoint: it appends
//! [`ServerMetrics::render`] to its own exposition. Names follow the
//! OpenTelemetry-to-Prometheus mapping of Go's instrument names
//! (`fibre.server.upload_shard.duration` with unit `s` becomes
//! `fibre_server_upload_shard_duration_seconds`), so dashboards built on
//! Go's exported metrics carry over; histogram buckets are Go's.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tonic::Code;

use crate::occupancy::Occupancy;

/// Go's buckets of the RPC duration histograms.
const RPC_BUCKETS: &[f64] = &[
    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
];
/// Go's buckets of the store operation histograms.
const STORE_BUCKETS: &[f64] = &[0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0];
/// Go's buckets of the signing histogram.
const SIGN_BUCKETS: &[f64] = &[0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0];
/// Go's buckets of the prune histogram.
const PRUNE_BUCKETS: &[f64] = &[0.001, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0];

/// The 17 gRPC status codes, by number.
const CODE_NAMES: [&str; 17] = [
    "OK",
    "Canceled",
    "Unknown",
    "InvalidArgument",
    "DeadlineExceeded",
    "NotFound",
    "AlreadyExists",
    "PermissionDenied",
    "ResourceExhausted",
    "FailedPrecondition",
    "Aborted",
    "OutOfRange",
    "Unimplemented",
    "Internal",
    "Unavailable",
    "DataLoss",
    "Unauthenticated",
];

#[derive(Debug)]
struct HistogramState {
    counts: Vec<u64>,
    sum: f64,
    count: u64,
}

/// A fixed-bucket histogram with a `success` label.
#[derive(Debug)]
struct Histogram {
    buckets: &'static [f64],
    /// Index 0: `success="false"`, 1: `success="true"`.
    series: [Mutex<HistogramState>; 2],
}

impl Histogram {
    fn new(buckets: &'static [f64]) -> Self {
        let state = || {
            Mutex::new(HistogramState {
                counts: vec![0; buckets.len()],
                sum: 0.0,
                count: 0,
            })
        };
        Self {
            buckets,
            series: [state(), state()],
        }
    }

    fn observe(&self, success: bool, d: Duration) {
        let v = d.as_secs_f64();
        let mut s = self.series[usize::from(success)]
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for (i, b) in self.buckets.iter().enumerate() {
            if v <= *b {
                s.counts[i] += 1;
            }
        }
        s.sum += v;
        s.count += 1;
    }

    fn count(&self, success: bool) -> u64 {
        self.series[usize::from(success)]
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .count
    }

    fn render(&self, out: &mut String, name: &str, help: &str, labels: &str) {
        let _ = writeln!(out, "# HELP {name} {help}");
        let _ = writeln!(out, "# TYPE {name} histogram");
        for (idx, success) in [(1usize, "true"), (0, "false")] {
            let s = self.series[idx]
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let l = join(labels, &format!("success=\"{success}\""));
            for (i, b) in self.buckets.iter().enumerate() {
                let _ = writeln!(
                    out,
                    "{name}_bucket{{{}}} {}",
                    join(&l, &format!("le=\"{b}\"")),
                    s.counts[i]
                );
            }
            let _ = writeln!(
                out,
                "{name}_bucket{{{}}} {}",
                join(&l, "le=\"+Inf\""),
                s.count
            );
            let _ = writeln!(out, "{name}_sum{{{l}}} {}", s.sum);
            let _ = writeln!(out, "{name}_count{{{l}}} {}", s.count);
        }
    }
}

fn join(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_owned(),
        (_, true) => a.to_owned(),
        _ => format!("{a},{b}"),
    }
}

fn codes() -> [AtomicU64; 17] {
    std::array::from_fn(|_| AtomicU64::new(0))
}

/// Metrics of one [`crate::Server`]. Cheap to update from any task; the
/// node renders them with [`Self::render`].
#[derive(Debug)]
pub struct ServerMetrics {
    /// The current server's limiter; replaced when a restarted server
    /// takes these metrics over ([`crate::Server::with_metrics`]).
    occupancy: Mutex<Arc<Occupancy>>,

    upload_in_flight: AtomicI64,
    upload_memory_reserved: AtomicI64,
    upload_codes: [AtomicU64; 17],
    upload_duration: Histogram,
    upload_bytes: AtomicU64,
    upload_rejected: Mutex<BTreeMap<&'static str, u64>>,
    upload_last_success: AtomicI64,

    download_in_flight: AtomicI64,
    download_codes: [AtomicU64; 17],
    download_duration: Histogram,
    download_bytes: AtomicU64,

    store_put_duration: Histogram,
    store_get_duration: Histogram,
    sign_duration: Histogram,

    prune_entries: AtomicU64,
    prune_freed_bytes: AtomicU64,
    prune_duration: Histogram,
    budget_recompute_failures: AtomicU64,

    connections_active: AtomicI64,
    connections_accepted: AtomicU64,
    handshake_failures: AtomicU64,

    health_checks: Mutex<BTreeMap<&'static str, bool>>,
    ready: AtomicBool,
    live: AtomicBool,
}

/// Why an `UploadShard` was rejected (the `reason` label of
/// `fibre_server_upload_shard_rejected_total`). Go counts only
/// `budget_exceeded`; the others are additions for debugging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    /// The storage limiter refused the shard (Go's only reason).
    BudgetExceeded,
    /// The payment promise failed verification.
    PromiseInvalid,
    /// The request carried no shard.
    ShardMissing,
    /// The rows are not this validator's assignment.
    AssignmentInvalid,
    /// The shard failed verification.
    ShardInvalid,
    /// The request exceeded the codec's row or proof-segment limits.
    CodecLimit,
    /// The in-flight upload memory budget (`max_inflight_upload_bytes`)
    /// had no room for the declared message.
    InflightMemory,
}

impl RejectReason {
    /// The label value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BudgetExceeded => "budget_exceeded",
            Self::PromiseInvalid => "promise_invalid",
            Self::ShardMissing => "shard_missing",
            Self::AssignmentInvalid => "assignment_invalid",
            Self::ShardInvalid => "shard_invalid",
            Self::CodecLimit => "codec_limit",
            Self::InflightMemory => "inflight_memory",
        }
    }
}

/// Which RPC a [`RpcTimer`] measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rpc {
    Upload,
    Download,
}

/// Counts an RPC in flight; [`RpcTimer::finish`] records its outcome.
pub(crate) struct RpcTimer<'a> {
    metrics: &'a ServerMetrics,
    rpc: Rpc,
    start: std::time::Instant,
    done: bool,
}

impl RpcTimer<'_> {
    /// Records the duration and status code and leaves the in-flight count.
    pub(crate) fn finish(mut self, code: Code) {
        self.done = true;
        let m = self.metrics;
        let success = code == Code::Ok;
        let (in_flight, codes, hist) = match self.rpc {
            Rpc::Upload => (&m.upload_in_flight, &m.upload_codes, &m.upload_duration),
            Rpc::Download => (
                &m.download_in_flight,
                &m.download_codes,
                &m.download_duration,
            ),
        };
        in_flight.fetch_sub(1, Ordering::Relaxed);
        codes[code as usize].fetch_add(1, Ordering::Relaxed);
        hist.observe(success, self.start.elapsed());
        if success && self.rpc == Rpc::Upload {
            m.upload_last_success.store(unix_secs(), Ordering::Relaxed);
        }
    }
}

impl Drop for RpcTimer<'_> {
    /// A handler dropped mid-flight: a client cancel is counted as
    /// `Canceled`; a panic unwinding the handler as `Internal`, the status
    /// the recovery interceptor returns to the client.
    fn drop(&mut self) {
        if !self.done {
            let m = self.metrics;
            let (in_flight, codes) = match self.rpc {
                Rpc::Upload => (&m.upload_in_flight, &m.upload_codes),
                Rpc::Download => (&m.download_in_flight, &m.download_codes),
            };
            let code = if std::thread::panicking() {
                Code::Internal
            } else {
                Code::Cancelled
            };
            in_flight.fetch_sub(1, Ordering::Relaxed);
            codes[code as usize].fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

impl ServerMetrics {
    /// Metrics reading occupancy and budget from `occupancy`.
    #[must_use]
    pub fn new(occupancy: Arc<Occupancy>) -> Self {
        Self {
            occupancy: Mutex::new(occupancy),
            upload_in_flight: AtomicI64::new(0),
            upload_memory_reserved: AtomicI64::new(0),
            upload_codes: codes(),
            upload_duration: Histogram::new(RPC_BUCKETS),
            upload_bytes: AtomicU64::new(0),
            upload_rejected: Mutex::new(BTreeMap::new()),
            upload_last_success: AtomicI64::new(0),
            download_in_flight: AtomicI64::new(0),
            download_codes: codes(),
            download_duration: Histogram::new(RPC_BUCKETS),
            download_bytes: AtomicU64::new(0),
            store_put_duration: Histogram::new(STORE_BUCKETS),
            store_get_duration: Histogram::new(STORE_BUCKETS),
            sign_duration: Histogram::new(SIGN_BUCKETS),
            prune_entries: AtomicU64::new(0),
            prune_freed_bytes: AtomicU64::new(0),
            prune_duration: Histogram::new(PRUNE_BUCKETS),
            budget_recompute_failures: AtomicU64::new(0),
            connections_active: AtomicI64::new(0),
            connections_accepted: AtomicU64::new(0),
            handshake_failures: AtomicU64::new(0),
            health_checks: Mutex::new(BTreeMap::new()),
            ready: AtomicBool::new(false),
            live: AtomicBool::new(false),
        }
    }

    pub(crate) fn start_rpc(&self, rpc: Rpc) -> RpcTimer<'_> {
        match rpc {
            Rpc::Upload => &self.upload_in_flight,
            Rpc::Download => &self.download_in_flight,
        }
        .fetch_add(1, Ordering::Relaxed);
        RpcTimer {
            metrics: self,
            rpc,
            start: std::time::Instant::now(),
            done: false,
        }
    }

    pub(crate) fn reject(&self, reason: RejectReason) {
        *self
            .upload_rejected
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(reason.as_str())
            .or_default() += 1;
    }

    pub(crate) fn add_upload_bytes(&self, n: u64) {
        self.upload_bytes.fetch_add(n, Ordering::Relaxed);
    }

    pub(crate) fn add_download_bytes(&self, n: u64) {
        self.download_bytes.fetch_add(n, Ordering::Relaxed);
    }

    pub(crate) fn observe_store_put(&self, success: bool, d: Duration) {
        self.store_put_duration.observe(success, d);
    }

    pub(crate) fn observe_store_get(&self, success: bool, d: Duration) {
        self.store_get_duration.observe(success, d);
    }

    pub(crate) fn observe_sign(&self, success: bool, d: Duration) {
        self.sign_duration.observe(success, d);
    }

    pub(crate) fn observe_prune(&self, success: bool, d: Duration, pruned: usize, freed: i64) {
        self.prune_duration.observe(success, d);
        self.prune_entries
            .fetch_add(pruned as u64, Ordering::Relaxed);
        self.prune_freed_bytes
            .fetch_add(freed.max(0).unsigned_abs(), Ordering::Relaxed);
    }

    /// Reads occupancy and budget from `occupancy` from now on.
    pub(crate) fn attach_occupancy(&self, occupancy: Arc<Occupancy>) {
        *self
            .occupancy
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = occupancy;
    }

    fn current_occupancy(&self) -> Arc<Occupancy> {
        Arc::clone(
            &self
                .occupancy
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }

    pub(crate) fn budget_recompute_failed(&self) {
        self.budget_recompute_failures
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn connection_opened(&self) {
        self.connections_active.fetch_add(1, Ordering::Relaxed);
        self.connections_accepted.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn connection_closed(&self) {
        self.connections_active.fetch_sub(1, Ordering::Relaxed);
    }

    pub(crate) fn handshake_failed(&self) {
        self.handshake_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn set_health_check(&self, check: &'static str, passed: bool) {
        self.health_checks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(check, passed);
    }

    pub(crate) fn set_ready(&self, ready: bool) {
        self.ready.store(ready, Ordering::Relaxed);
    }

    pub(crate) fn set_live(&self, live: bool) {
        self.live.store(live, Ordering::Relaxed);
    }

    /// `UploadShard` RPCs that finished with `code`.
    #[must_use]
    pub fn upload_count(&self, code: Code) -> u64 {
        self.upload_codes[code as usize].load(Ordering::Relaxed)
    }

    /// `DownloadShard` RPCs that finished with `code`.
    #[must_use]
    pub fn download_count(&self, code: Code) -> u64 {
        self.download_codes[code as usize].load(Ordering::Relaxed)
    }

    /// `UploadShard` RPCs being handled.
    #[must_use]
    pub fn uploads_in_flight(&self) -> i64 {
        self.upload_in_flight.load(Ordering::Relaxed)
    }

    /// Bytes reserved by in-flight uploads under
    /// `max_inflight_upload_bytes`.
    #[must_use]
    pub fn upload_memory_reserved(&self) -> i64 {
        self.upload_memory_reserved.load(Ordering::Relaxed)
    }

    pub(crate) fn add_upload_memory(&self, bytes: i64) {
        self.upload_memory_reserved
            .fetch_add(bytes, Ordering::Relaxed);
    }

    /// Counts an `UploadShard` call refused before its handler ran.
    pub(crate) fn count_upload_code(&self, code: Code) {
        self.upload_codes[code as usize].fetch_add(1, Ordering::Relaxed);
    }

    /// Rejections with `reason`.
    #[must_use]
    pub fn rejected(&self, reason: RejectReason) -> u64 {
        self.upload_rejected
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(reason.as_str())
            .copied()
            .unwrap_or(0)
    }

    /// Unix time of the last successful upload; `None` before the first.
    #[must_use]
    pub fn last_upload_success(&self) -> Option<i64> {
        Some(self.upload_last_success.load(Ordering::Relaxed)).filter(|&t| t > 0)
    }

    /// Entries removed by pruning.
    #[must_use]
    pub fn pruned_entries(&self) -> u64 {
        self.prune_entries.load(Ordering::Relaxed)
    }

    /// Successful store puts recorded.
    #[must_use]
    pub fn store_puts(&self) -> u64 {
        self.store_put_duration.count(true)
    }

    /// Open connections.
    #[must_use]
    pub fn connections_active(&self) -> i64 {
        self.connections_active.load(Ordering::Relaxed)
    }

    /// Whether the server reports ready.
    #[must_use]
    pub fn ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    /// Prometheus text exposition of every series. `labels` are added to
    /// each sample (e.g. `chain_id="mocha-4"`, empty for none).
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn render(&self, labels: &str) -> String {
        let mut out = String::new();
        let lb = |extra: &str| join(labels, extra);
        let gauge = |out: &mut String, name: &str, help: &str, v: i64| {
            let _ = writeln!(out, "# HELP {name} {help}");
            let _ = writeln!(out, "# TYPE {name} gauge");
            let _ = writeln!(out, "{name}{{{labels}}} {v}");
        };
        let counter = |out: &mut String, name: &str, help: &str, v: u64| {
            let _ = writeln!(out, "# HELP {name} {help}");
            let _ = writeln!(out, "# TYPE {name} counter");
            let _ = writeln!(out, "{name}{{{labels}}} {v}");
        };
        let by_code = |out: &mut String, name: &str, help: &str, codes: &[AtomicU64; 17]| {
            let _ = writeln!(out, "# HELP {name} {help}");
            let _ = writeln!(out, "# TYPE {name} counter");
            for (i, c) in codes.iter().enumerate() {
                let v = c.load(Ordering::Relaxed);
                if v > 0 {
                    let _ = writeln!(
                        out,
                        "{name}{{{}}} {v}",
                        lb(&format!("grpc_code=\"{}\"", CODE_NAMES[i]))
                    );
                }
            }
        };

        gauge(
            &mut out,
            "fibre_server_upload_shard_in_flight",
            "Number of UploadShard RPCs currently being handled",
            self.upload_in_flight.load(Ordering::Relaxed),
        );
        by_code(
            &mut out,
            "fibre_server_upload_shard_total",
            "UploadShard RPCs by gRPC status code",
            &self.upload_codes,
        );
        self.upload_duration.render(
            &mut out,
            "fibre_server_upload_shard_duration_seconds",
            "Duration of UploadShard RPCs in seconds",
            labels,
        );
        counter(
            &mut out,
            "fibre_server_upload_shard_bytes_total",
            "Total bytes received via UploadShard RPCs",
            self.upload_bytes.load(Ordering::Relaxed),
        );
        let _ = writeln!(
            out,
            "# HELP fibre_server_upload_shard_rejected_total UploadShard RPCs rejected, by reason"
        );
        let _ = writeln!(
            out,
            "# TYPE fibre_server_upload_shard_rejected_total counter"
        );
        for (reason, v) in self
            .upload_rejected
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
        {
            let _ = writeln!(
                out,
                "fibre_server_upload_shard_rejected_total{{{}}} {v}",
                lb(&format!("reason=\"{reason}\""))
            );
        }
        if let Some(ts) = self.last_upload_success() {
            gauge(
                &mut out,
                "fibre_server_upload_shard_last_success_timestamp_seconds",
                "Unix time of the last successful UploadShard RPC; not reported before the first one",
                ts,
            );
        }
        gauge(
            &mut out,
            "fibre_server_upload_shard_occupancy_bytes",
            "Shard bytes tracked by the storage limiter: on-disk plus in-flight reserved",
            self.current_occupancy().usage(),
        );
        gauge(
            &mut out,
            "fibre_server_upload_memory_reserved_bytes",
            "Bytes reserved by in-flight uploads under max_inflight_upload_bytes",
            self.upload_memory_reserved.load(Ordering::Relaxed),
        );
        gauge(
            &mut out,
            "fibre_server_upload_shard_budget_bytes",
            "Current per-node storage budget of the limiter (0: unlimited)",
            self.current_occupancy().budget_bytes(),
        );
        gauge(
            &mut out,
            "fibre_server_download_shard_in_flight",
            "Number of DownloadShard RPCs currently being handled",
            self.download_in_flight.load(Ordering::Relaxed),
        );
        by_code(
            &mut out,
            "fibre_server_download_shard_total",
            "DownloadShard RPCs by gRPC status code",
            &self.download_codes,
        );
        self.download_duration.render(
            &mut out,
            "fibre_server_download_shard_duration_seconds",
            "Duration of DownloadShard RPCs in seconds",
            labels,
        );
        counter(
            &mut out,
            "fibre_server_download_shard_bytes_total",
            "Total bytes sent via DownloadShard RPCs",
            self.download_bytes.load(Ordering::Relaxed),
        );
        self.store_put_duration.render(
            &mut out,
            "fibre_server_store_put_duration_seconds",
            "Duration of store Put operations in seconds",
            labels,
        );
        self.store_get_duration.render(
            &mut out,
            "fibre_server_store_get_duration_seconds",
            "Duration of store Get operations in seconds",
            labels,
        );
        self.sign_duration.render(
            &mut out,
            "fibre_server_sign_duration_seconds",
            "Duration of payment promise signing in seconds",
            labels,
        );
        counter(
            &mut out,
            "fibre_server_prune_entries_total",
            "Total entries pruned",
            self.prune_entries.load(Ordering::Relaxed),
        );
        counter(
            &mut out,
            "fibre_server_prune_freed_bytes_total",
            "Total shard bytes released by pruning",
            self.prune_freed_bytes.load(Ordering::Relaxed),
        );
        self.prune_duration.render(
            &mut out,
            "fibre_server_prune_duration_seconds",
            "Duration of prune cycles in seconds",
            labels,
        );
        counter(
            &mut out,
            "fibre_server_budget_recompute_failures_total",
            "Periodic storage budget recomputations that failed (previous budget kept)",
            self.budget_recompute_failures.load(Ordering::Relaxed),
        );
        gauge(
            &mut out,
            "fibre_server_connections_active",
            "Open client connections",
            self.connections_active.load(Ordering::Relaxed),
        );
        counter(
            &mut out,
            "fibre_server_connections_accepted_total",
            "Accepted TCP connections",
            self.connections_accepted.load(Ordering::Relaxed),
        );
        counter(
            &mut out,
            "fibre_server_tls_handshake_failures_total",
            "TLS handshakes that failed or timed out",
            self.handshake_failures.load(Ordering::Relaxed),
        );
        let _ = writeln!(
            out,
            "# HELP fibre_server_health_check_passing Last result of each health check (1 passed, 0 failed)"
        );
        let _ = writeln!(out, "# TYPE fibre_server_health_check_passing gauge");
        for (check, passed) in self
            .health_checks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
        {
            let _ = writeln!(
                out,
                "fibre_server_health_check_passing{{{}}} {}",
                lb(&format!("check=\"{check}\"")),
                u8::from(*passed)
            );
        }
        gauge(
            &mut out,
            "fibre_server_ready",
            "Readiness published on the gRPC health service (1 SERVING)",
            i64::from(self.ready.load(Ordering::Relaxed)),
        );
        gauge(
            &mut out,
            "fibre_server_live",
            "Liveness published on the gRPC health service (1 SERVING)",
            i64::from(self.live.load(Ordering::Relaxed)),
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_exposition() {
        let occ = Arc::new(Occupancy::new(1000));
        occ.seed(10);
        let m = ServerMetrics::new(occ);
        assert_eq!(m.last_upload_success(), None);
        let t = m.start_rpc(Rpc::Upload);
        assert_eq!(m.uploads_in_flight(), 1);
        t.finish(Code::InvalidArgument);
        m.start_rpc(Rpc::Upload).finish(Code::Ok);
        drop(m.start_rpc(Rpc::Download));
        m.reject(RejectReason::BudgetExceeded);
        m.set_health_check("app", true);
        m.set_health_check("store", false);
        let s = m.render("chain_id=\"c\"");
        assert!(s.contains("fibre_server_upload_shard_in_flight{chain_id=\"c\"} 0"));
        assert!(s.contains(
            "fibre_server_upload_shard_total{chain_id=\"c\",grpc_code=\"InvalidArgument\"} 1"
        ));
        assert!(s.contains(
            "fibre_server_download_shard_total{chain_id=\"c\",grpc_code=\"Canceled\"} 1"
        ));
        assert!(s.contains(
            "fibre_server_upload_shard_rejected_total{chain_id=\"c\",reason=\"budget_exceeded\"} 1"
        ));
        assert!(s.contains("fibre_server_upload_shard_occupancy_bytes{chain_id=\"c\"} 10"));
        assert!(s.contains("fibre_server_upload_shard_budget_bytes{chain_id=\"c\"} 1000"));
        assert!(s.contains(
            "fibre_server_upload_shard_duration_seconds_count{chain_id=\"c\",success=\"true\"} 1"
        ));
        assert!(s.contains("fibre_server_health_check_passing{chain_id=\"c\",check=\"store\"} 0"));
        assert!(s.contains("fibre_server_upload_shard_last_success_timestamp_seconds"));
        assert!(m.last_upload_success().is_some());
        let plain = m.render("");
        assert!(plain.contains("fibre_server_ready{} 0"));
    }
}
