//! `celestia-rs debug-bundle`: collects what maintainers need to diagnose
//! a node problem into one `.tar.gz` that an operator attaches to a bug
//! report. Each part is best-effort: a part that cannot be read becomes an
//! entry in the manifest's `errors`, and the bundle is still written.
//!
//! The bundle holds:
//!
//! - `manifest.json`: bundle format, creation time, the files in the
//!   bundle, the parts that failed (`errors`) and were not collected
//!   (`skipped`);
//! - `version.json`: build identity (version, commit, toolchain, targets);
//! - `system.json`: OS, CPU, memory, disk and open-files limit;
//! - `home.json`: the sizes of the home's `config/` and `data/` entries;
//! - `genesis.json`: a summary of the genesis (chain ID, SHA-256), not the
//!   file;
//! - `config/{config,app,client}.toml`: with secret values and URL
//!   credentials redacted ([`redact_toml`]);
//! - `data/priv_validator_state.json`, `data/upgrade-info.json`;
//! - `rpc/<method>.json`: `status`, `net_info`, `num_unconfirmed_txs`,
//!   `abci_info` and `dump_consensus_state` of the running node;
//! - `metrics.txt`: the Prometheus metrics, when they are enabled;
//! - `logs/`: the tails of log files, the systemd journal and RocksDB
//!   `LOG` files.
//!
//! It never reads the private key files (`priv_validator_key.json`,
//! `node_key.json`) or the keyrings: it reads only the paths named above.

use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::net::{TcpStream, ToSocketAddrs as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};
use serde_json::{Map, Value, json};

use crate::config::{ConfigFiles, File};
use crate::util;

/// The bundle layout version in `manifest.json`.
pub const FORMAT: u64 = 1;

/// The default size limit of each log tail (16 MiB; logs compress well).
pub const DEFAULT_MAX_LOG_BYTES: u64 = 16 << 20;

/// The size limit of each RocksDB `LOG` tail.
const ROCKSDB_LOG_BYTES: u64 = 1 << 20;

/// The node RPC methods recorded under `rpc/`.
const RPC_METHODS: [&str; 5] = [
    "status",
    "net_info",
    "num_unconfirmed_txs",
    "abci_info",
    "dump_consensus_state",
];

/// Connect and read timeout of the metrics request.
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);

/// The notice printed after the bundle is written.
pub const NOTICE: &str = "The bundle holds your node config (secrets redacted), peer addresses, \
    node state and logs, but no private keys. Review it before you share it publicly.";

/// What to collect and where to write it.
#[derive(Debug, Clone)]
pub struct Options {
    /// Node home.
    pub home: PathBuf,
    /// Archive path (default: `celestia-rs-debug-<time>.tar.gz` in the
    /// working directory).
    pub output: Option<PathBuf>,
    /// Node RPC address (empty: client.toml `node`, else localhost).
    pub node: String,
    /// Query the running node over RPC and its metrics endpoint.
    pub rpc: bool,
    /// Log files to include the tail of.
    pub log_files: Vec<PathBuf>,
    /// systemd unit whose journal to include (Linux).
    pub journal_unit: Option<String>,
    /// Size limit of each log tail.
    pub max_log_bytes: u64,
}

/// One file of the bundle.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Path inside the bundle directory.
    pub name: String,
    /// Contents.
    pub data: Vec<u8>,
}

/// Collects the entries and the manifest that records them.
#[derive(Default)]
struct Collector {
    entries: Vec<Entry>,
    errors: Map<String, Value>,
    skipped: Vec<Value>,
}

impl Collector {
    fn add(&mut self, name: &str, data: impl Into<Vec<u8>>) {
        self.entries.push(Entry {
            name: name.to_owned(),
            data: data.into(),
        });
    }

    fn add_json(&mut self, name: &str, v: &Value) {
        self.add(name, format!("{}\n", util::json_indent(v, "  ")));
    }

    /// Adds the result of `f` as `name`, or records its error.
    fn try_add(&mut self, name: &str, f: impl FnOnce() -> anyhow::Result<Vec<u8>>) {
        match f() {
            Ok(data) => self.add(name, data),
            Err(e) => self.error(name, &e),
        }
    }

    fn error(&mut self, name: &str, e: &anyhow::Error) {
        self.errors.insert(name.to_owned(), json!(format!("{e:#}")));
    }

    fn skip(&mut self, what: &str) {
        self.skipped.push(json!(what));
    }

    fn finish(mut self) -> Vec<Entry> {
        let files: Vec<Value> = self.entries.iter().map(|e| json!(e.name)).collect();
        let manifest = json!({
            "format": FORMAT,
            "created": util::now_rfc3339(),
            "notice": NOTICE,
            "files": files,
            "errors": self.errors,
            "skipped": self.skipped,
        });
        let mut entries = vec![Entry {
            name: "manifest.json".into(),
            data: format!("{}\n", util::json_indent(&manifest, "  ")).into_bytes(),
        }];
        entries.append(&mut self.entries);
        entries
    }
}

/// Collects the bundle's entries (`manifest.json` first). Never fails:
/// parts that cannot be read are listed in the manifest's `errors`.
#[must_use]
pub fn collect(opts: &Options) -> Vec<Entry> {
    let mut c = Collector::default();
    let home = &opts.home;
    c.add_json("version.json", &version());
    c.add_json("system.json", &system(home));
    c.add_json("home.json", &home_summary(home));

    let files = ConfigFiles::load(home);
    let home_path = |key: &str, default: &str| {
        let rel = files
            .as_ref()
            .map(|f| f.string(File::Config, key))
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| default.to_owned());
        home.join(rel)
    };

    let genesis = home_path("genesis_file", "config/genesis.json");
    c.try_add("genesis.json", || {
        let v = genesis_summary(&genesis)?;
        Ok(format!("{}\n", util::json_indent(&v, "  ")).into_bytes())
    });
    for f in [File::Config, File::App, File::Client] {
        let path = home.join("config").join(f.name());
        c.try_add(&format!("config/{}", f.name()), || {
            let s = std::fs::read_to_string(&path)
                .with_context(|| format!("read {}", path.display()))?;
            Ok(redact_toml(&s).into_bytes())
        });
    }
    let state = home_path(
        "priv_validator_state_file",
        "data/priv_validator_state.json",
    );
    c.try_add("data/priv_validator_state.json", || read(&state));
    let upgrade = home.join("data").join("upgrade-info.json");
    if upgrade.exists() {
        c.try_add("data/upgrade-info.json", || read(&upgrade));
    }

    if opts.rpc {
        let node = crate::txs::Node::new(&crate::client::node_addr(home, &opts.node));
        for method in RPC_METHODS {
            let name = format!("rpc/{method}.json");
            match node.call(method, json!({})) {
                Ok(v) => c.add_json(&name, &v),
                Err(e) => {
                    c.error(&name, &e);
                    // The node is not reachable: do not wait on each call.
                    if method == "status" {
                        c.skip("rpc: the node did not answer /status");
                        break;
                    }
                }
            }
        }
        match files.as_ref().ok().and_then(metrics_addr) {
            Some(addr) => c.try_add("metrics.txt", || http_get(&addr, "/metrics")),
            None => c.skip("metrics: [instrumentation] prometheus is off"),
        }
    } else {
        c.skip("rpc: --no-rpc");
    }

    for path in &opts.log_files {
        let base = path
            .file_name()
            .map_or_else(|| "log".into(), |n| n.to_string_lossy().into_owned());
        c.try_add(&format!("logs/{base}"), || tail(path, opts.max_log_bytes));
    }
    match &opts.journal_unit {
        Some(unit) if cfg!(target_os = "linux") => {
            c.try_add(&format!("logs/journal-{unit}.log"), || {
                journal(unit, opts.max_log_bytes)
            });
        }
        _ => c.skip("journal: no systemd unit"),
    }
    for (rel, path) in rocksdb_logs(&home.join("data")) {
        c.try_add(&format!("logs/rocksdb/{rel}/LOG"), || {
            tail(&path, ROCKSDB_LOG_BYTES)
        });
    }
    c.finish()
}

/// Collects the bundle and writes it; returns the archive path.
pub fn create(opts: &Options) -> anyhow::Result<PathBuf> {
    let path = opts.output.clone().unwrap_or_else(default_output);
    let entries = collect(opts);
    let root = bundle_root(&path);
    let bytes = tar_gz(&root, &entries)?;
    // The bundle holds the node config: readable by its owner only.
    util::write_private(&path, &bytes).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

/// `celestia-rs-debug-<UTC time>.tar.gz` in the working directory.
fn default_output() -> PathBuf {
    let stamp: String = util::now_rfc3339()
        .chars()
        .take(19)
        .filter(|c| c.is_ascii_digit() || *c == 'T')
        .collect();
    PathBuf::from(format!("celestia-rs-debug-{stamp}Z.tar.gz"))
}

/// The archive's top-level directory: its file name without `.tar.gz`.
fn bundle_root(path: &Path) -> String {
    let name = path.file_name().map_or_else(
        || "celestia-rs-debug".into(),
        |n| n.to_string_lossy().into_owned(),
    );
    let root = [".tar.gz", ".tgz", ".gz", ".tar"]
        .iter()
        .find_map(|ext| name.strip_suffix(ext))
        .unwrap_or(&name);
    if root.is_empty() {
        "celestia-rs-debug".into()
    } else {
        root.to_owned()
    }
}

fn read(path: &Path) -> anyhow::Result<Vec<u8>> {
    std::fs::read(path).with_context(|| format!("read {}", path.display()))
}

fn version() -> Value {
    json!({
        "name": "celestia-rs",
        "version": crate::version::SOFTWARE_VERSION,
        "node_version": crate::version::node_version(),
        "commit": env!("CELESTIA_RS_GIT_COMMIT"),
        "build_profile": env!("CELESTIA_RS_BUILD_PROFILE"),
        "toolchain": env!("CELESTIA_RS_TOOLCHAIN"),
        "app_version": app::upgrades::APP_VERSION,
        "compatibility_target": {
            "celestia_app": app::COMPATIBILITY_VERSION,
            "celestia_core": crate::version::CORE_COMPATIBILITY_VERSION,
        },
        "features": features(),
    })
}

fn features() -> Vec<&'static str> {
    let mut f = Vec::new();
    if cfg!(feature = "rocksdb") {
        f.push("rocksdb");
    }
    if cfg!(feature = "jemalloc") {
        f.push("jemalloc");
    }
    if cfg!(feature = "memstats") {
        f.push("memstats");
    }
    if cfg!(feature = "bridge") {
        f.push("bridge");
    }
    if cfg!(feature = "da-light") {
        f.push("da-light");
    }
    if cfg!(feature = "da-fibre") {
        f.push("da-fibre");
    }
    if cfg!(feature = "da-webrtc") {
        f.push("da-webrtc");
    }
    if cfg!(feature = "da-webtransport") {
        f.push("da-webtransport");
    }
    f
}

/// The stdout of `cmd args`, trimmed; `None` when it fails.
fn command_output(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn system(home: &Path) -> Value {
    let mut v = json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "cpus": std::thread::available_parallelism().map_or(0, std::num::NonZero::get),
        // The limit of this command's shell; `node_processes` has the
        // running node's own limit (Linux).
        "shell_open_files_limit": crate::ops::open_files_limit(),
        "time": util::now_rfc3339(),
    });
    let mut set = |k: &str, val: Option<String>| {
        if let Some(val) = val {
            v[k] = json!(val);
        }
    };
    set("kernel", command_output("uname", &["-srm"]));
    // `df` of the home (or the nearest existing parent): free disk space.
    let mut dir = home;
    while !dir.exists() {
        match dir.parent() {
            Some(p) => dir = p,
            None => break,
        }
    }
    set(
        "disk",
        command_output("df", &["-Pk", &dir.to_string_lossy()]),
    );
    if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
        let keep = ["MemTotal:", "MemAvailable:", "SwapTotal:", "SwapFree:"];
        let lines: Vec<&str> = meminfo
            .lines()
            .filter(|l| keep.iter().any(|k| l.starts_with(k)))
            .collect();
        set("memory", Some(lines.join("\n")));
    } else {
        set(
            "memory",
            command_output("sysctl", &["-n", "hw.memsize"]).map(|b| format!("{b} bytes")),
        );
    }
    set(
        "load_average",
        std::fs::read_to_string("/proc/loadavg")
            .ok()
            .map(|s| s.trim().to_owned())
            .or_else(|| command_output("sysctl", &["-n", "vm.loadavg"])),
    );
    set(
        "uptime_seconds",
        std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|s| s.split_whitespace().next().map(str::to_owned)),
    );
    if Path::new("/proc/self/limits").exists() {
        v["node_processes"] = node_open_files_limits(Path::new("/proc"), std::process::id());
    }
    v
}

/// The other `celestia-rs` processes under `proc` (procfs) with their
/// soft open-files limit: the limit the running node really has.
fn node_open_files_limits(proc: &Path, own_pid: u32) -> Value {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(proc) else {
        return Value::Array(out);
    };
    let mut pids: Vec<u32> = rd
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse().ok())
        .filter(|&pid| pid != own_pid)
        .collect();
    pids.sort_unstable();
    for pid in pids {
        let dir = proc.join(pid.to_string());
        let Ok(cmdline) = std::fs::read(dir.join("cmdline")) else {
            continue;
        };
        let args: Vec<String> = cmdline
            .split(|&b| b == 0)
            .filter(|a| !a.is_empty())
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect();
        let Some(program) = args.first() else {
            continue;
        };
        if Path::new(program).file_name().and_then(|n| n.to_str()) != Some("celestia-rs") {
            continue;
        }
        let limit = std::fs::read_to_string(dir.join("limits"))
            .ok()
            .and_then(|l| {
                l.lines()
                    .find_map(|line| line.strip_prefix("Max open files"))
                    .and_then(|rest| rest.split_whitespace().next().map(str::to_owned))
            })
            .map(|soft| {
                soft.parse::<u64>()
                    .map_or_else(|_| json!(soft), |n| json!(n))
            });
        // The node's flags help diagnosis; URL credentials are removed.
        let command = redact_url_credentials(&args.join(" "));
        out.push(json!({
            "pid": pid,
            "command": command,
            "open_files_soft_limit": limit,
        }));
    }
    Value::Array(out)
}

/// Total size and file count of `path` (recursive; symlinks not followed).
fn du(path: &Path) -> (u64, u64) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return (0, 0);
    };
    if !meta.is_dir() {
        return (meta.len(), 1);
    }
    let mut total = (0, 0);
    if let Ok(rd) = std::fs::read_dir(path) {
        for e in rd.flatten() {
            let (b, n) = du(&e.path());
            total.0 += b;
            total.1 += n;
        }
    }
    total
}

/// Names, sizes and file counts of the entries of `config/` and `data/`.
fn home_summary(home: &Path) -> Value {
    let mut v = json!({ "home": home.display().to_string(), "exists": home.is_dir() });
    for dir in ["config", "data"] {
        let mut entries = Map::new();
        if let Ok(rd) = std::fs::read_dir(home.join(dir)) {
            let mut list: Vec<_> = rd.flatten().collect();
            list.sort_by_key(std::fs::DirEntry::file_name);
            for e in list {
                let (bytes, files) = du(&e.path());
                entries.insert(
                    e.file_name().to_string_lossy().into_owned(),
                    json!({ "bytes": bytes, "files": files }),
                );
            }
        }
        v[dir] = Value::Object(entries);
    }
    v
}

/// The genesis' identity: chain ID, time, initial height, versions,
/// file size and SHA-256 (compare with the network's published hash).
fn genesis_summary(path: &Path) -> anyhow::Result<Value> {
    let bytes = read(path)?;
    let g: Value = serde_json::from_slice(&bytes).context("parse genesis")?;
    Ok(json!({
        "path": path.display().to_string(),
        "bytes": bytes.len(),
        "sha256": hex::encode(hash::sha256(&bytes)),
        "chain_id": g["chain_id"],
        "genesis_time": g["genesis_time"],
        "initial_height": g["initial_height"],
        "app_version": g["app_version"],
        "consensus_app_version": g["consensus"]["params"]["version"]["app"],
    }))
}

/// Words that mark a config key's value as secret.
const SECRET_WORDS: [&str; 9] = [
    "token",
    "secret",
    "password",
    "passwd",
    "passphrase",
    "mnemonic",
    "apikey",
    "api_key",
    "credential",
];

/// The config text with the values of secret-looking keys
/// ([`SECRET_WORDS`]) and the credentials of URLs (`user:pass@`)
/// replaced by `<redacted>`. Comments and layout stay.
#[must_use]
pub fn redact_toml(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    // The open value of a redacted key that continues on later lines.
    let mut open: Option<Open> = None;
    for line in src.split_inclusive('\n') {
        let (body, eol) = match line.strip_suffix('\n') {
            Some(b) => (b, "\n"),
            None => (line, ""),
        };
        if let Some(o) = open.take() {
            // A continuation line of a redacted value: drop it.
            open = o.feed(body);
            continue;
        }
        let trimmed = body.trim_start();
        if !trimmed.starts_with('#')
            && let Some((key, value)) = body.split_once('=')
        {
            let k = key.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
            let k = k.replace('-', "_");
            if SECRET_WORDS.iter().any(|w| k.contains(w)) {
                out.push_str(key.trim_end());
                out.push_str(" = \"<redacted>\"");
                out.push_str(eol);
                open = Open::start(value.trim_start());
                continue;
            }
        }
        out.push_str(&redact_url_credentials(body));
        out.push_str(eol);
    }
    out
}

/// A TOML value that does not end on its first line.
#[derive(Debug, Clone, Copy)]
enum Open {
    /// A `"""` or `'''` string, until the delimiter.
    String(&'static str),
    /// An array or inline table, until its brackets balance.
    Brackets(i64),
}

impl Open {
    /// The open state after the first line of `value`, if any.
    fn start(value: &str) -> Option<Self> {
        for delim in ["\"\"\"", "'''"] {
            if let Some(rest) = value.strip_prefix(delim) {
                return (!rest.contains(delim)).then_some(Self::String(delim));
            }
        }
        if value.starts_with(['[', '{']) {
            return Self::Brackets(0).feed(value);
        }
        None
    }

    /// The state after `line`; `None` once the value has ended.
    fn feed(self, line: &str) -> Option<Self> {
        match self {
            Self::String(delim) => (!line.contains(delim)).then_some(self),
            Self::Brackets(mut depth) => {
                let mut quote: Option<char> = None;
                let mut escaped = false;
                for c in line.chars() {
                    match quote {
                        Some(q) => {
                            if escaped {
                                escaped = false;
                            } else if c == '\\' && q == '"' {
                                escaped = true;
                            } else if c == q {
                                quote = None;
                            }
                        }
                        None => match c {
                            '"' | '\'' => quote = Some(c),
                            '[' | '{' => depth += 1,
                            ']' | '}' => depth -= 1,
                            '#' => break,
                            _ => {}
                        },
                    }
                }
                (depth > 0).then_some(Self::Brackets(depth))
            }
        }
    }
}

/// Replaces `user:pass@` after each `://` with `<redacted>@`.
fn redact_url_credentials(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        let end = tail
            .find(['/', '"', '\'', ',', ' ', '\t'])
            .unwrap_or(tail.len());
        let authority = &tail[..end];
        if let Some(at) = authority.rfind('@') {
            out.push_str("<redacted>");
            out.push_str(&authority[at..]);
        } else {
            out.push_str(authority);
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// The last `max` bytes of `path`.
fn tail(path: &Path, max: u64) -> anyhow::Result<Vec<u8>> {
    let mut f = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let len = f.metadata()?.len();
    if len > max {
        f.seek(SeekFrom::Start(len - max))?;
    }
    let mut buf = Vec::new();
    f.take(max).read_to_end(&mut buf)?;
    Ok(buf)
}

/// The tail of a systemd unit's journal, read as a stream so that
/// `max` also bounds memory use.
fn journal(unit: &str, max: u64) -> anyhow::Result<Vec<u8>> {
    use std::process::{Command, Stdio};
    let mut child = Command::new("journalctl")
        .args([
            "--no-pager",
            "-o",
            "short-iso-precise",
            "-n",
            "200000",
            "-u",
            unit,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("run journalctl")?;
    let stderr = child.stderr.take();
    // Drain stderr on its own thread so that neither pipe fills up.
    let errors = std::thread::spawn(move || {
        stderr
            .and_then(|e| tail_reader(e, 64 << 10).ok())
            .map(|(b, _)| b)
            .unwrap_or_default()
    });
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("journalctl: no stdout"))?;
    let read = tail_reader(stdout, max);
    let status = child.wait().context("wait for journalctl")?;
    let errors = errors.join().unwrap_or_default();
    if !status.success() {
        bail!("journalctl: {}", String::from_utf8_lossy(&errors).trim());
    }
    Ok(read?.0)
}

/// Reads `r` to its end and keeps its last `max` bytes, holding at most
/// about `2 * max` bytes at once. Returns the tail and the largest
/// buffer size reached.
fn tail_reader(mut r: impl std::io::Read, max: u64) -> std::io::Result<(Vec<u8>, usize)> {
    let max = usize::try_from(max).unwrap_or(usize::MAX);
    let mut buf = Vec::new();
    let mut chunk = vec![0u8; 64 << 10];
    let mut peak = 0;
    loop {
        let n = match r.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        buf.extend_from_slice(&chunk[..n]);
        peak = peak.max(buf.len());
        if buf.len() > max.saturating_mul(2) {
            buf.drain(..buf.len() - max);
        }
    }
    if buf.len() > max {
        buf.drain(..buf.len() - max);
    }
    Ok((buf, peak))
}

/// RocksDB `LOG` files of the databases under `data/` (two levels deep),
/// as (path relative to `data/`, absolute path).
fn rocksdb_logs(data: &Path) -> Vec<(String, PathBuf)> {
    fn walk(dir: &Path, rel: &str, depth: u32, out: &mut Vec<(String, PathBuf)>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut list: Vec<_> = rd.flatten().collect();
        list.sort_by_key(std::fs::DirEntry::file_name);
        for e in list {
            let Ok(ft) = e.file_type() else { continue };
            if !ft.is_dir() {
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            let sub = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            let log = e.path().join("LOG");
            if log.is_file() {
                out.push((sub.clone(), log));
            }
            if depth > 1 {
                walk(&e.path(), &sub, depth - 1, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(data, "", 2, &mut out);
    out
}

/// The Prometheus endpoint (`host:port`) when `[instrumentation]
/// prometheus` is on; an empty host is localhost.
fn metrics_addr(files: &ConfigFiles) -> Option<String> {
    if !files.bool(File::Config, "instrumentation.prometheus") {
        return None;
    }
    let addr = files.string(File::Config, "instrumentation.prometheus_listen_addr");
    let addr = addr.split_once("://").map_or(addr.as_str(), |(_, r)| r);
    let addr = match addr.strip_prefix(':') {
        Some(port) => format!("127.0.0.1:{port}"),
        None => addr.replace("0.0.0.0:", "127.0.0.1:"),
    };
    Some(addr)
}

/// A plain HTTP/1.0 GET (no chunked encoding): the body of a 200 reply.
fn http_get(host_port: &str, path: &str) -> anyhow::Result<Vec<u8>> {
    let addr = host_port
        .to_socket_addrs()
        .with_context(|| format!("resolve {host_port}"))?
        .next()
        .ok_or_else(|| anyhow!("resolve {host_port}"))?;
    let mut s = TcpStream::connect_timeout(&addr, HTTP_TIMEOUT)
        .with_context(|| format!("connect {addr}"))?;
    s.set_read_timeout(Some(HTTP_TIMEOUT))?;
    s.set_write_timeout(Some(HTTP_TIMEOUT))?;
    write!(s, "GET {path} HTTP/1.0\r\nHost: {host_port}\r\n\r\n")?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw)?;
    let sep = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| anyhow!("malformed HTTP response from {addr}"))?;
    let status = String::from_utf8_lossy(&raw[..sep])
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    if status.split_whitespace().nth(1) != Some("200") {
        bail!("GET http://{host_port}{path}: {status}");
    }
    Ok(raw[sep + 4..].to_vec())
}

/// Writes `n` as a NUL-terminated octal field of `field.len()` bytes.
fn octal(field: &mut [u8], n: u64) -> anyhow::Result<()> {
    let digits = field.len() - 1;
    let s = format!("{n:0digits$o}");
    if s.len() > digits {
        bail!("{n} does not fit a tar header field");
    }
    field[..digits].copy_from_slice(s.as_bytes());
    field[digits] = 0;
    Ok(())
}

/// A POSIX ustar header for a regular file.
fn tar_header(path: &str, size: u64, mtime: u64) -> anyhow::Result<[u8; 512]> {
    let mut h = [0u8; 512];
    let bytes = path.as_bytes();
    if bytes.len() <= 100 {
        h[..bytes.len()].copy_from_slice(bytes);
    } else {
        // Split at a '/' into prefix (<= 155) and name (<= 100).
        let split = path
            .char_indices()
            .filter(|&(i, c)| c == '/' && i <= 155 && bytes.len() - i - 1 <= 100)
            .map(|(i, _)| i)
            .next_back()
            .ok_or_else(|| anyhow!("path too long for tar: {path}"))?;
        let (prefix, name) = (&bytes[..split], &bytes[split + 1..]);
        h[..name.len()].copy_from_slice(name);
        h[345..345 + prefix.len()].copy_from_slice(prefix);
    }
    octal(&mut h[100..108], 0o600)?;
    octal(&mut h[108..116], 0)?;
    octal(&mut h[116..124], 0)?;
    octal(&mut h[124..136], size)?;
    octal(&mut h[136..148], mtime)?;
    h[156] = b'0';
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    // The checksum is computed with its own field as spaces.
    h[148..156].copy_from_slice(b"        ");
    let sum: u64 = h.iter().map(|&b| u64::from(b)).sum();
    octal(&mut h[148..155], sum)?;
    h[155] = b' ';
    Ok(h)
}

/// The entries as a gzip-compressed tar under the directory `root`.
fn tar_gz(root: &str, entries: &[Entry]) -> anyhow::Result<Vec<u8>> {
    let mtime = u64::try_from(util::unix_now()).unwrap_or(0);
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    for e in entries {
        let size = e.data.len() as u64;
        gz.write_all(&tar_header(&format!("{root}/{}", e.name), size, mtime)?)?;
        gz.write_all(&e.data)?;
        let pad = (512 - e.data.len() % 512) % 512;
        gz.write_all(&vec![0u8; pad])?;
    }
    gz.write_all(&[0u8; 1024])?;
    Ok(gz.finish()?)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn root_is_the_archive_name_without_extension() {
        assert_eq!(bundle_root(Path::new("/x/a.tar.gz")), "a");
        assert_eq!(bundle_root(Path::new("b.tgz")), "b");
        assert_eq!(bundle_root(Path::new("c")), "c");
        assert_eq!(bundle_root(Path::new(".tar.gz")), "celestia-rs-debug");
    }

    #[test]
    fn default_output_has_no_colons() {
        let p = default_output();
        let name = p.to_str().unwrap_or_default();
        assert!(name.starts_with("celestia-rs-debug-2"), "{name}");
        assert!(!name.contains(':'), "{name}");
        // celestia-rs-debug-YYYYMMDDTHHMMSSZ.tar.gz
        assert_eq!(
            name.len(),
            "celestia-rs-debug-".len() + 16 + ".tar.gz".len()
        );
    }

    #[test]
    fn long_paths_use_the_ustar_prefix() {
        let path = format!("{}/{}", "d".repeat(120), "f".repeat(90));
        let h = tar_header(&path, 1, 0).unwrap();
        assert_eq!(&h[..90], "f".repeat(90).as_bytes());
        assert_eq!(&h[345..465], "d".repeat(120).as_bytes());
        assert!(tar_header(&"x".repeat(300), 1, 0).is_err());
    }

    #[test]
    fn tail_reader_keeps_only_the_last_bytes() {
        /// 500 reads of 1000 bytes; the last read is all `a`.
        struct Chunks(u8, u32);
        impl std::io::Read for Chunks {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.1 == 0 {
                    return Ok(0);
                }
                self.1 -= 1;
                let n = buf.len().min(1000);
                buf[..n].fill(if self.1 == 0 { b'a' } else { self.0 });
                Ok(n)
            }
        }
        let (tail, peak) = tail_reader(Chunks(b'z', 500), 2500).unwrap();
        assert_eq!(tail.len(), 2500);
        assert!(tail.ends_with(&[b'a'; 1000]));
        // The buffer stays bounded, not the 500 kB read.
        assert!(peak <= 2 * 2500 + 64 * 1024, "{peak}");
        let (short, _) = tail_reader(&b"abc"[..], 10).unwrap();
        assert_eq!(short, b"abc");
    }

    #[test]
    fn node_open_files_limits_come_from_proc() {
        let proc = tempfile::tempdir().unwrap();
        let limits = "Limit                     Soft Limit           Hard Limit           Units\n\
            Max open files            65536                524288               files\n";
        for (pid, cmdline) in [
            ("101", "/usr/local/bin/celestia-rs\0start\0--home\0/x\0"),
            ("102", "celestia-rs\0debug-bundle\0"),
            ("103", "/usr/bin/other\0start\0"),
        ] {
            let d = proc.path().join(pid);
            std::fs::create_dir(&d).unwrap();
            std::fs::write(d.join("cmdline"), cmdline).unwrap();
            std::fs::write(d.join("limits"), limits).unwrap();
        }
        std::fs::create_dir(proc.path().join("self")).unwrap();
        // Only the node process, not this command or other programs.
        assert_eq!(
            node_open_files_limits(proc.path(), 102),
            json!([{
                "pid": 101,
                "command": "/usr/local/bin/celestia-rs start --home /x",
                "open_files_soft_limit": 65536,
            }])
        );
    }

    #[test]
    fn metrics_address_defaults_to_localhost() {
        let dir = tempfile::tempdir().unwrap();
        let files = ConfigFiles::load(dir.path()).unwrap();
        assert_eq!(metrics_addr(&files), None);
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        std::fs::write(
            dir.path().join("config/config.toml"),
            "[instrumentation]\nprometheus = true\nprometheus_listen_addr = \":26660\"\n",
        )
        .unwrap();
        let files = ConfigFiles::load(dir.path()).unwrap();
        assert_eq!(metrics_addr(&files).as_deref(), Some("127.0.0.1:26660"));
    }
}
