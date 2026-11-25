//! `celestia-rs migrate`: prepares a stopped Go `celestia-appd` home for
//! celestia-rs (operator guide, section 5). It backs up `config/` and the
//! sign state, moves the Go databases from `data/` to `data.go/` (never
//! deletes them) and creates a fresh `data/` holding a byte-identical
//! `priv_validator_state.json`, the double-sign guard. Key files and
//! keyrings stay where they are; the node is not started.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use light::RpcClient;

use crate::config::{ConfigFiles, File};

/// Go databases in a celestia-appd `data/` (celestia-rs uses `*.rocksdb`).
const GO_DBS: [&str; 3] = ["application.db", "blockstore.db", "state.db"];
/// The double-sign guard.
const STATE_FILE: &str = "priv_validator_state.json";
/// State sync trusts the header this many heights below the tip.
const TRUST_DEPTH: i64 = 2000;

/// `migrate` options (the CLI flags).
#[derive(Debug, Clone, Default, clap::Args)]
pub struct MigrateOptions {
    /// The Go node home (`--home`).
    #[arg(skip)]
    pub home: PathBuf,
    /// Print the plan and change nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Confirm that the Go node is stopped and cannot restart (systemd
    /// unit / launchd job / cosmovisor disabled).
    #[arg(long)]
    pub go_node_stopped: bool,
    /// Allow a home with a validator key but no
    /// data/priv_validator_state.json (a key that never signs).
    #[arg(long)]
    pub not_a_validator: bool,
    /// Enable state sync in config.toml from these RPC servers
    /// (comma-separated); trust height and hash are read from them.
    #[arg(long, value_delimiter = ',')]
    pub statesync_rpc: Vec<String>,
}

/// The RPC reads behind `--statesync-rpc` (a trait so tests need no network).
pub trait TrustSource {
    /// `(chain ID, latest block height)` of `server`.
    fn status(&self, server: &str) -> Result<(String, i64)>;
    /// The header hash at `height` on `server`.
    fn header_hash(&self, server: &str, height: i64) -> Result<Vec<u8>>;
}

/// [`TrustSource`] over celestia-core RPC.
#[derive(Debug, Clone, Copy)]
pub struct RpcTrust;

impl TrustSource for RpcTrust {
    fn status(&self, server: &str) -> Result<(String, i64)> {
        let s = RpcClient::new(server, Duration::from_secs(30))
            .status()
            .map_err(|e| anyhow!("{server}: /status: {e}"))?;
        Ok((s.network, s.latest_block_height))
    }

    fn header_hash(&self, server: &str, height: i64) -> Result<Vec<u8>> {
        let (sh, _) = RpcClient::new(server, Duration::from_secs(30))
            .commit(Some(height))
            .map_err(|e| anyhow!("{server}: /commit at {height}: {e}"))?;
        sh.header
            .as_ref()
            .and_then(ctypes::Header::hash)
            .ok_or_else(|| anyhow!("{server}: no header at height {height}"))
    }
}

/// `[statesync]` values: `rpc_servers`, `trust_height`, `trust_hash`.
#[derive(Debug)]
struct Trust(Vec<String>, i64, String);

/// Trusts the first server's header [`TRUST_DEPTH`] below its tip, and
/// requires every server to serve `chain_id` and the same hash there.
fn statesync_trust(src: &dyn TrustSource, servers: &[String], chain_id: &str) -> Result<Trust> {
    let Some(primary) = servers.first() else {
        bail!("--statesync-rpc: no RPC server given");
    };
    let height = (src.status(primary)?.1 - TRUST_DEPTH).max(1);
    let hash = src.header_hash(primary, height)?;
    for s in servers {
        let (network, _) = src.status(s)?;
        if network != chain_id {
            bail!("{s} serves chain {network:?}, but config/genesis.json is {chain_id:?}");
        }
        if src.header_hash(s, height)? != hash {
            bail!("{primary} and {s} disagree on the header hash at height {height}");
        }
    }
    let mut rpc = servers.to_vec();
    if rpc.len() == 1 {
        // CometBFT requires two entries; one server twice is the usual setup.
        rpc.push(primary.clone());
    }
    Ok(Trust(rpc, height, hex::encode_upper(hash)))
}

/// Where the Go databases are: `data/` (`Go`), `data.go/` without `data/`
/// (`Interrupted` between the two renames) or `data.go/` (`Migrated`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    Go,
    Interrupted,
    Migrated,
}

fn layout(home: &Path) -> Result<Layout> {
    let not_go = |why: String| anyhow!("{} is not a Go celestia-appd home: {why}", home.display());
    for f in ["config/config.toml", "config/genesis.json"] {
        if !home.join(f).is_file() {
            return Err(not_go(format!("{f} is missing")));
        }
    }
    let go_dbs = |d: &str| GO_DBS.iter().any(|db| home.join(d).join(db).is_dir());
    let (data, go) = (home.join("data").is_dir(), go_dbs("data.go"));
    match (data, go_dbs("data"), go) {
        (true, true, false) => Ok(Layout::Go),
        (true, false, true) => Ok(Layout::Migrated),
        (false, _, true) => Ok(Layout::Interrupted),
        (true, true, true) => {
            bail!("both data/ and data.go/ hold Go databases; move one away by hand")
        }
        _ => Err(not_go(format!("data/ holds none of {}", GO_DBS.join(", ")))),
    }
}

/// Refuses config.toml paths celestia-rs does not read (it uses the
/// defaults) and listeners that are still bound.
fn check_config(home: &Path) -> Result<()> {
    let files = ConfigFiles::load(home)?;
    let key_file = "config/priv_validator_key.json";
    let state_file = format!("data/{STATE_FILE}");
    for (key, default) in [
        ("db_dir", "data"),
        ("priv_validator_key_file", key_file),
        ("priv_validator_state_file", &state_file),
    ] {
        let v = files.string(File::Config, key);
        anyhow::ensure!(
            v == default,
            "config.toml {key} = {v:?}: celestia-rs reads only {default:?}; move the file first"
        );
    }
    for key in ["rpc.laddr", "p2p.laddr"] {
        let laddr = files.string(File::Config, key);
        if let Some(addr) = laddr.strip_prefix("tcp://")
            && let Err(e) = std::net::TcpListener::bind(addr)
            && e.kind() == std::io::ErrorKind::AddrInUse
        {
            bail!("{addr} (config.toml {key}) is in use: a node still runs on this home; stop it");
        }
    }
    Ok(())
}

/// The sign state to carry over (`None`: not a validator) and its last
/// signed height (Go writes it as a JSON string).
fn sign_state(home: &Path, data: &Path, no_val: bool) -> Result<Option<(PathBuf, String)>> {
    let path = data.join(STATE_FILE);
    if !path.exists() {
        if home.join("config/priv_validator_key.json").exists() && !no_val {
            bail!(
                "{} is missing but config/priv_validator_key.json exists: refusing to migrate a \
                 validator without its double-sign guard. Restore the file from the node that \
                 signed last, or pass --not-a-validator if this key never signs",
                path.display()
            );
        }
        return Ok(None);
    }
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)
        .with_context(|| format!("{} is not valid JSON; refusing", path.display()))?;
    let Some(height) = v["height"].as_str().map(str::to_owned) else {
        bail!("{} has no height; refusing", path.display());
    };
    Ok(Some((path, height)))
}

fn private_dir(p: &Path) -> std::io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(p)
}

/// Copies `src` to `dst`, fsyncs it and checks the bytes.
fn copy_verified(src: &Path, dst: &Path) -> Result<()> {
    std::fs::copy(src, dst).with_context(|| format!("copy {}", src.display()))?;
    std::fs::File::open(dst)?.sync_all()?;
    if std::fs::read(src)? != std::fs::read(dst)? {
        bail!("{} differs from its source after copying", dst.display());
    }
    Ok(())
}

fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    private_dir(dst).with_context(|| format!("create {}", dst.display()))?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &dst.join(e.file_name()))?;
        } else {
            copy_verified(&e.path(), &dst.join(e.file_name()))?;
        }
    }
    Ok(())
}

fn sync_dir(p: &Path) {
    if let Ok(d) = std::fs::File::open(p) {
        let _ = d.sync_all();
    }
}

/// Runs `celestia-rs migrate` and returns the plan (dry run) or summary.
/// Every check (and the state-sync RPC reads) runs before the first
/// change; then it backs up, edits `[statesync]`, stages a new `data/`
/// with the sign state and swaps it in with two renames.
#[allow(clippy::too_many_lines)]
pub fn migrate(opts: &MigrateOptions, src: &dyn TrustSource) -> Result<String> {
    let home = opts.home.as_path();
    let h = home.display();
    let (data, go) = (home.join("data"), home.join("data.go"));
    let staging = home.join("data.migrate-tmp");
    let start = format!("celestia-rs start --home {h}");
    let layout = layout(home)?;
    if layout == Layout::Migrated {
        return Ok(format!(
            "{h} is already migrated; nothing changed.\nNext: {start}"
        ));
    }
    check_config(home)?;
    let src_data = if layout == Layout::Go { &data } else { &go };
    let state = sign_state(home, src_data, opts.not_a_validator)?;
    if !opts.dry_run && !opts.go_node_stopped {
        bail!(
            "stop the Go node and make sure it cannot restart (systemd unit, launchd job, \
             cosmovisor), then re-run with --go-node-stopped: two nodes signing with one \
             validator key double-sign"
        );
    }
    let trust = if opts.statesync_rpc.is_empty() {
        None
    } else {
        let g: serde_json::Value =
            serde_json::from_slice(&std::fs::read(home.join("config/genesis.json"))?)
                .context("parse config/genesis.json")?;
        let chain_id = g["chain_id"].as_str().unwrap_or_default();
        Some(statesync_trust(src, &opts.statesync_rpc, chain_id)?)
    };

    let stamp = crate::util::rfc3339_nano(crate::util::unix_now(), 0).replace(['-', ':'], "");
    let backup = home.join(format!("migrate-backup-{stamp}"));
    let mut plan = vec![format!(
        "back up config/ and data/{STATE_FILE} to {}",
        backup.display()
    )];
    plan.push(match &trust {
        Some(Trust(rpc, height, hash)) => format!(
            "set [statesync] in config/config.toml: enable = true, rpc_servers = \"{}\", \
             trust_height = {height}, trust_hash = \"{hash}\"",
            rpc.join(",")
        ),
        None => "leave config/config.toml unchanged".to_owned(),
    });
    if layout == Layout::Go {
        plan.push("move data/ to data.go/ (the Go databases, kept)".to_owned());
    }
    plan.push(match &state {
        Some((_, height)) => format!(
            "create data/ holding {STATE_FILE} copied byte for byte (last signed height {height})"
        ),
        None => "create an empty data/ (no sign state: not a validator)".to_owned(),
    });
    let next = if trust.is_some() {
        format!("Next: {start}")
    } else {
        format!(
            "Next: to state-sync, set [statesync] enable, rpc_servers, trust_height and trust_hash in \
             config/config.toml (otherwise the node block-syncs from genesis), then: {start}"
        )
    };
    let plan = format!("  - {}\n{next}", plan.join("\n  - "));
    if opts.dry_run {
        return Ok(format!("Dry run, nothing changed. Plan:\n{plan}"));
    }

    private_dir(&backup).with_context(|| format!("create {}", backup.display()))?;
    let (d, g, b) = (data.display(), go.display(), backup.display());
    let restore = format!("rm -rf {d} && mv {g} {d}, and restore config/ from {b}/config");
    let steps = || -> Result<()> {
        copy_dir(&home.join("config"), &backup.join("config"))?;
        if let Some((path, _)) = &state {
            copy_verified(path, &backup.join(STATE_FILE))?;
        }
        sync_dir(&backup);
        if let Some(Trust(rpc, height, hash)) = &trust {
            crate::config::set_many(
                home,
                File::Config,
                &[
                    ("statesync.enable", "true"),
                    ("statesync.rpc_servers", &rpc.join(",")),
                    ("statesync.trust_height", &height.to_string()),
                    ("statesync.trust_hash", hash),
                ],
            )?;
        }
        if staging.exists() {
            // Left by an earlier run; it only ever holds a copy.
            std::fs::remove_dir_all(&staging)?;
        }
        private_dir(&staging)?;
        if let Some((path, _)) = &state {
            copy_verified(path, &staging.join(STATE_FILE))?;
        }
        sync_dir(&staging);
        if layout == Layout::Go {
            std::fs::rename(&data, &go)?;
        }
        if let Err(e) = std::fs::rename(&staging, &data) {
            if layout == Layout::Go {
                let _ = std::fs::rename(&go, &data);
            }
            return Err(e.into());
        }
        sync_dir(home);
        Ok(())
    };
    steps().with_context(|| format!("migrate stopped; to run the Go node again: {restore}"))?;
    Ok(format!(
        "Migrated {h} for celestia-rs:\n{plan}\nTo go back to Go: {restore}"
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const STATE: &[u8] = b"{\n  \"height\": \"123456\",\n  \"round\": 0,\n  \"step\": 3\n}";

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    /// A Go validator home: config, keys, keyring, Go databases.
    fn go_home(rpc_laddr: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::config::CONFIG_TEMPLATE
            .replace("tcp://127.0.0.1:26657", rpc_laddr)
            .replace("tcp://0.0.0.0:26656", "tcp://127.0.0.1:0");
        for (f, bytes) in [
            ("config/config.toml", config.as_bytes()),
            ("config/genesis.json", br#"{"chain_id":"mocha-4"}"#),
            ("config/priv_validator_key.json", b"SECRET"),
            ("keyring-file/operator.info", b"KEYRING"),
            ("data/priv_validator_state.json", STATE),
            ("data/blockstore.db/LOCK", b""),
            ("data/state.db/000001.log", b"state"),
            ("data/cs.wal/wal", b"wal"),
        ] {
            write(&dir.path().join(f), bytes);
        }
        dir
    }

    fn home() -> tempfile::TempDir {
        go_home("tcp://127.0.0.1:0")
    }

    /// Every path under `dir` with its bytes (directories: `None`).
    fn tree(dir: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
        fn walk(root: &Path, d: &Path, out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
            for e in std::fs::read_dir(d).unwrap() {
                let p = e.unwrap().path();
                let bytes = (!p.is_dir()).then(|| std::fs::read(&p).unwrap());
                out.insert(p.strip_prefix(root).unwrap().to_path_buf(), bytes);
                if p.is_dir() {
                    walk(root, &p, out);
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(dir, dir, &mut out);
        out
    }

    /// A fake RPC: `chain` empty fails every call; `liar` serves another hash.
    struct Fake(&'static str, i64, Option<&'static str>);
    const NO_NET: Fake = Fake("", 0, None);
    impl TrustSource for Fake {
        fn status(&self, _: &str) -> Result<(String, i64)> {
            anyhow::ensure!(!self.0.is_empty(), "no network");
            Ok((self.0.into(), self.1))
        }
        fn header_hash(&self, server: &str, height: i64) -> Result<Vec<u8>> {
            let mut h = vec![0xab; 32];
            h[0] = u8::try_from(height % 256).unwrap();
            h[1] = if self.2 == Some(server) { 0 } else { 0xab };
            Ok(h)
        }
    }

    fn opts(h: &Path) -> MigrateOptions {
        MigrateOptions {
            home: h.to_path_buf(),
            go_node_stopped: true,
            ..MigrateOptions::default()
        }
    }

    fn read(p: PathBuf) -> Vec<u8> {
        std::fs::read(p).unwrap()
    }

    #[test]
    fn happy_path_moves_go_data_and_keeps_the_sign_state() {
        let dir = home();
        let h = dir.path();
        let config = read(h.join("config/config.toml"));
        let text = migrate(&opts(h), &NO_NET).unwrap();
        assert!(text.starts_with("Migrated"), "{text}");
        let t = tree(h);
        for f in ["blockstore.db/LOCK", "state.db/000001.log", "cs.wal/wal"] {
            assert!(t.contains_key(&Path::new("data.go").join(f)), "{f} kept");
        }
        // Fresh data/ with only the sign state, byte for byte.
        let mut data = t.keys().filter(|p| p.parent() == Some(Path::new("data")));
        assert_eq!(
            data.next().unwrap(),
            Path::new("data/priv_validator_state.json")
        );
        assert_eq!(data.next(), None);
        assert_eq!(read(h.join("data/priv_validator_state.json")), STATE);
        assert_eq!(read(h.join("data.go/priv_validator_state.json")), STATE);
        let mut keys = t.keys().map(|p| p.to_string_lossy().into_owned());
        let backup = h.join(keys.find(|p| p.starts_with("migrate-backup-")).unwrap());
        assert_eq!(read(backup.join("priv_validator_state.json")), STATE);
        assert_eq!(read(backup.join("config/config.toml")), config);
        // Keys and config.toml stay as they were (no --statesync-rpc).
        assert_eq!(read(h.join("config/priv_validator_key.json")), b"SECRET");
        assert_eq!(read(h.join("keyring-file/operator.info")), b"KEYRING");
        assert_eq!(read(h.join("config/config.toml")), config);
        assert!(text.contains(&format!("celestia-rs start --home {}", h.display())));
        assert!(
            text.contains("123456") && !text.contains("SECRET"),
            "{text}"
        );
    }

    /// `migrate` fails with `msg` and changes nothing.
    fn refuse(h: &Path, o: &MigrateOptions, src: &Fake, msg: &str) {
        let before = tree(h);
        let err = migrate(o, src).unwrap_err().to_string();
        assert!(err.contains(msg), "{err}");
        assert_eq!(tree(h), before);
    }

    #[test]
    fn dry_run_changes_nothing() {
        let dir = home();
        let before = tree(dir.path());
        let mut o = opts(dir.path());
        (o.dry_run, o.go_node_stopped) = (true, false);
        let text = migrate(&o, &NO_NET).unwrap();
        assert!(
            text.starts_with("Dry run") && text.contains("data.go"),
            "{text}"
        );
        assert_eq!(tree(dir.path()), before);
    }

    #[test]
    fn missing_sign_state_on_a_validator_home_fails_closed() {
        let dir = home();
        let h = dir.path();
        std::fs::remove_file(h.join("data/priv_validator_state.json")).unwrap();
        refuse(h, &opts(h), &NO_NET, "priv_validator_state.json is missing");
        let mut o = opts(h);
        o.not_a_validator = true;
        assert!(migrate(&o, &NO_NET).unwrap().starts_with("Migrated"));
        assert!(!h.join("data/priv_validator_state.json").exists());
    }

    #[test]
    fn rerun_is_a_no_op() {
        let dir = home();
        migrate(&opts(dir.path()), &NO_NET).unwrap();
        let after_first = tree(dir.path());
        let again = migrate(&opts(dir.path()), &NO_NET).unwrap();
        assert!(
            again.contains("already migrated; nothing changed"),
            "{again}"
        );
        assert_eq!(tree(dir.path()), after_first);
    }

    #[test]
    fn resumes_after_an_interruption_between_the_renames() {
        let dir = home();
        let h = dir.path();
        std::fs::rename(h.join("data"), h.join("data.go")).unwrap();
        write(&h.join("data.migrate-tmp/partial"), b"");
        assert!(migrate(&opts(h), &NO_NET).unwrap().starts_with("Migrated"));
        assert_eq!(read(h.join("data/priv_validator_state.json")), STATE);
        assert!(!h.join("data/partial").exists() && h.join("data.go/blockstore.db/LOCK").exists());
    }

    #[test]
    fn refuses_unsafe_homes_without_changes() {
        let empty = tempfile::tempdir().unwrap();
        let e = empty.path();
        refuse(e, &opts(e), &NO_NET, "config.toml is missing");
        // A celestia-rs home (RocksDB stores) is not a Go home.
        let dir = home();
        let h = dir.path();
        for db in ["blockstore.db", "state.db"] {
            std::fs::remove_dir_all(h.join("data").join(db)).unwrap();
        }
        refuse(h, &opts(h), &NO_NET, "not a Go celestia-appd home");
        // No confirmation that the Go node is stopped.
        let dir = home();
        let h = dir.path();
        let mut o = opts(h);
        o.go_node_stopped = false;
        refuse(h, &o, &NO_NET, "--go-node-stopped");
        // State sync: RPC errors and a different chain.
        o = opts(h);
        o.statesync_rpc = vec!["https://a".into()];
        refuse(h, &o, &NO_NET, "no network");
        refuse(h, &o, &Fake("celestia", 10_000, None), "mocha-4");
        // A sign state path celestia-rs would not read.
        crate::config::set(h, File::Config, "priv_validator_state_file", "/x").unwrap();
        refuse(h, &opts(h), &NO_NET, "priv_validator_state_file");
        // The home's RPC port is still in use.
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let busy = go_home(&format!("tcp://{}", l.local_addr().unwrap()));
        refuse(busy.path(), &opts(busy.path()), &NO_NET, "in use");
    }

    #[test]
    fn statesync_fills_config_and_keeps_the_rest() {
        let dir = home();
        let h = dir.path();
        let before = std::fs::read_to_string(h.join("config/config.toml")).unwrap();
        let mut o = opts(h);
        o.statesync_rpc = vec!["https://a:443".into(), "https://b".into()];
        let text = migrate(&o, &Fake("mocha-4", 10_500, None)).unwrap();
        let hash = format!("{:02X}{}", 8_500 % 256, "AB".repeat(31));
        assert!(text.contains(&hash), "{text}");
        let after = std::fs::read_to_string(h.join("config/config.toml")).unwrap();
        let pairs = before.lines().zip(after.lines());
        let changed: Vec<_> = pairs.filter(|(a, b)| a != b).map(|(_, b)| b).collect();
        let want = [
            "enable = true".to_owned(),
            "rpc_servers = \"https://a:443,https://b\"".to_owned(),
            "trust_height = 8500".to_owned(),
            format!("trust_hash = \"{hash}\""),
        ];
        // Only these four lines changed; comments and the rest are kept.
        assert_eq!(changed, want);
        assert_eq!(before.lines().count(), after.lines().count());
    }

    #[test]
    fn statesync_trust_checks_every_server() {
        let servers = ["a".to_owned(), "b".to_owned()];
        let t = statesync_trust(&Fake("mocha-4", 100, None), &servers, "mocha-4").unwrap();
        assert_eq!(
            (t.0.len(), t.1),
            (2, 1),
            "both servers; a young chain trusts height 1"
        );
        let liar = Fake("mocha-4", 100_000, Some("b"));
        let err = statesync_trust(&liar, &servers, "mocha-4").unwrap_err();
        assert!(err.to_string().contains("disagree"), "{err}");
        // One server is listed twice, as CometBFT requires two entries.
        let one = statesync_trust(&Fake("mocha-4", 100, None), &servers[..1], "mocha-4").unwrap();
        assert_eq!(one.0, ["a", "a"]);
    }
}
