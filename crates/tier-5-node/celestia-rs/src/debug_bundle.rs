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
        // The limit of this shell; the node inherits it when started from
        // the same environment.
        "open_files_limit": crate::ops::open_files_limit(),
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
    v
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
    for line in src.split_inclusive('\n') {
        let (body, eol) = match line.strip_suffix('\n') {
            Some(b) => (b, "\n"),
            None => (line, ""),
        };
        let trimmed = body.trim_start();
        if !trimmed.starts_with('#')
            && let Some((key, _)) = body.split_once('=')
        {
            let k = key.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
            let k = k.replace('-', "_");
            if SECRET_WORDS.iter().any(|w| k.contains(w)) {
                out.push_str(key.trim_end());
                out.push_str(" = \"<redacted>\"");
                out.push_str(eol);
                continue;
            }
        }
        out.push_str(&redact_url_credentials(body));
        out.push_str(eol);
    }
    out
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

/// The tail of a systemd unit's journal.
fn journal(unit: &str, max: u64) -> anyhow::Result<Vec<u8>> {
    let out = std::process::Command::new("journalctl")
        .args([
            "--no-pager",
            "-o",
            "short-iso-precise",
            "-n",
            "200000",
            "-u",
            unit,
        ])
        .output()
        .context("run journalctl")?;
    if !out.status.success() {
        bail!(
            "journalctl: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let data = out.stdout;
    let max = usize::try_from(max).unwrap_or(usize::MAX);
    Ok(data[data.len().saturating_sub(max)..].to_vec())
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
