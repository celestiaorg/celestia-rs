//! `celestia-rs debug-bundle`: the archive an operator attaches to a bug
//! report. It holds the build, host, config, sign state, node RPC and log
//! data that maintainers need, and never the home's private keys.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;

use celestia_rs::debug_bundle::{self, Entry, Options};
use celestia_rs::init;
use serde_json::Value;

fn init_home(home: &Path) {
    init::init(&init::InitOptions {
        home: home.to_path_buf(),
        moniker: "n0".into(),
        chain_id: "bundle-test".into(),
        network: None,
        overwrite: true,
        recover_mnemonic: None,
        download_genesis: false,
    })
    .unwrap();
}

fn options(home: &Path) -> Options {
    Options {
        home: home.to_path_buf(),
        output: None,
        node: String::new(),
        rpc: false,
        log_files: Vec::new(),
        journal_unit: None,
        max_log_bytes: debug_bundle::DEFAULT_MAX_LOG_BYTES,
    }
}

fn entry<'a>(entries: &'a [Entry], name: &str) -> &'a Entry {
    entries
        .iter()
        .find(|e| e.name == name)
        .unwrap_or_else(|| panic!("no {name} in {:?}", names(entries)))
}

fn names(entries: &[Entry]) -> Vec<&str> {
    entries.iter().map(|e| e.name.as_str()).collect()
}

fn json(e: &Entry) -> Value {
    serde_json::from_slice(&e.data).unwrap()
}

/// The base64 private key strings of the home's key files.
fn private_keys(home: &Path) -> Vec<String> {
    ["config/priv_validator_key.json", "config/node_key.json"]
        .iter()
        .map(|f| {
            let v: Value = serde_json::from_slice(&std::fs::read(home.join(f)).unwrap()).unwrap();
            v["priv_key"]["value"].as_str().unwrap().to_owned()
        })
        .collect()
}

#[test]
fn bundle_holds_diagnostics_and_never_private_keys() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    init_home(home);
    let entries = debug_bundle::collect(&options(home));

    for name in [
        "manifest.json",
        "version.json",
        "system.json",
        "home.json",
        "genesis.json",
        "config/config.toml",
        "config/app.toml",
        "config/client.toml",
        "data/priv_validator_state.json",
    ] {
        entry(&entries, name);
    }
    let version = json(entry(&entries, "version.json"));
    assert_eq!(version["version"], env!("CARGO_PKG_VERSION"));
    assert!(version["commit"].is_string());
    let system = json(entry(&entries, "system.json"));
    assert_eq!(system["os"], std::env::consts::OS);
    assert_eq!(system["arch"], std::env::consts::ARCH);
    // The genesis is a summary (mainnet's file is large), not the file.
    let genesis = json(entry(&entries, "genesis.json"));
    assert_eq!(genesis["chain_id"], "bundle-test");
    assert_eq!(genesis["sha256"].as_str().unwrap().len(), 64);
    assert!(genesis.get("app_state").is_none());

    let manifest = json(entry(&entries, "manifest.json"));
    assert_eq!(manifest["format"], 1);
    let listed: Vec<&str> = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    for e in &entries {
        if e.name != "manifest.json" {
            assert!(
                listed.contains(&e.name.as_str()),
                "{} not in manifest",
                e.name
            );
        }
    }
    // RPC was turned off: say so instead of failing.
    assert!(
        manifest["skipped"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str().unwrap().contains("rpc"))
    );

    for name in names(&entries) {
        assert!(!name.contains("priv_validator_key"), "{name}");
        assert!(!name.contains("node_key"), "{name}");
        assert!(!name.contains("keyring"), "{name}");
    }
    for key in private_keys(home) {
        for e in &entries {
            let text = String::from_utf8_lossy(&e.data);
            assert!(!text.contains(&key), "{} holds a private key", e.name);
        }
    }
}

#[test]
fn bundle_of_a_missing_home_still_has_build_and_host_data() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("absent");
    let entries = debug_bundle::collect(&options(&home));
    entry(&entries, "version.json");
    entry(&entries, "system.json");
    let manifest = json(entry(&entries, "manifest.json"));
    let errors = manifest["errors"].as_object().unwrap();
    assert!(errors.contains_key("genesis.json"), "{errors:?}");
}

#[test]
fn config_secrets_and_url_credentials_are_redacted() {
    let src = r#"# a comment stays
moniker = "n0"
persistent_peers = "id@1.2.3.4:26656"

[statesync]
rpc_servers = "https://alice:hunter2@rpc.example.com:443,https://rpc2.example.com"

[da-bridge]
auth-token = "s3cr3t"
api_key = "k3y"
password = "pw"
keyring-backend = "test"
"#;
    let out = debug_bundle::redact_toml(src);
    assert!(out.contains("# a comment stays"));
    assert!(out.contains(r#"moniker = "n0""#));
    assert!(out.contains("id@1.2.3.4:26656"), "{out}");
    assert!(out.contains("https://rpc2.example.com"));
    assert!(out.contains("rpc.example.com:443"));
    assert!(out.contains(r#"keyring-backend = "test""#));
    for secret in ["hunter2", "alice", "s3cr3t", "k3y", "\"pw\""] {
        assert!(!out.contains(secret), "{secret} left in {out}");
    }
    assert!(out.contains("<redacted>"));
}

#[test]
fn multiline_secret_values_are_redacted_whole() {
    let src = r#"password = """
line-one-secret
line-two-secret"""
api_key = '''
literal-secret
'''
tokens = [
  "array-secret-1",
  ["nested-secret"],
]
after = "kept"
inline_password = """one-line-secret"""
still = "kept too"
"#;
    let out = debug_bundle::redact_toml(src);
    for secret in [
        "line-one-secret",
        "line-two-secret",
        "literal-secret",
        "array-secret-1",
        "nested-secret",
        "one-line-secret",
    ] {
        assert!(!out.contains(secret), "{secret} left in {out}");
    }
    assert!(out.contains(r#"after = "kept""#), "{out}");
    assert!(out.contains(r#"still = "kept too""#), "{out}");
    // The redacted text is still valid TOML.
    out.parse::<toml::Table>().unwrap();
}

#[test]
fn bundled_config_files_are_redacted() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    init_home(home);
    let app = home.join("config/app.toml");
    let mut s = std::fs::read_to_string(&app).unwrap();
    s.push_str("\n[extra]\nauth-token = \"s3cr3t\"\n");
    std::fs::write(&app, s).unwrap();
    let entries = debug_bundle::collect(&options(home));
    let text = String::from_utf8_lossy(&entry(&entries, "config/app.toml").data).into_owned();
    assert!(!text.contains("s3cr3t"));
    assert!(text.contains("auth-token = \"<redacted>\""));
}

#[test]
fn log_files_are_tailed_to_the_size_limit() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    init_home(&home);
    let log = dir.path().join("node.log");
    std::fs::write(&log, "old line\nnew line\n").unwrap();
    let mut opts = options(&home);
    opts.log_files = vec![log, dir.path().join("missing.log")];
    opts.max_log_bytes = 9;
    let entries = debug_bundle::collect(&opts);
    assert_eq!(entry(&entries, "logs/node.log").data, b"new line\n");
    let manifest = json(entry(&entries, "manifest.json"));
    assert!(
        manifest["errors"]
            .as_object()
            .unwrap()
            .contains_key("logs/missing.log")
    );
}

/// A one-shot-per-connection JSON-RPC server that answers every method
/// with `{"method": <name>}`.
fn fake_rpc() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut s = stream.unwrap();
            let mut buf = vec![0u8; 64 * 1024];
            let mut req = Vec::new();
            let body = loop {
                let n = s.read(&mut buf).unwrap();
                assert!(n > 0, "request ended early");
                req.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&req).into_owned();
                let Some(head_end) = text.find("\r\n\r\n") else {
                    continue;
                };
                let len: usize = text[..head_end]
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse().unwrap())
                    })
                    .unwrap();
                if req.len() >= head_end + 4 + len {
                    break req[head_end + 4..head_end + 4 + len].to_vec();
                }
            };
            let v: Value = serde_json::from_slice(&body).unwrap();
            let body = serde_json::json!({
                "jsonrpc": "2.0", "id": 0, "result": {"method": v["method"]}
            })
            .to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            s.write_all(resp.as_bytes()).unwrap();
        }
    });
    format!("tcp://{addr}")
}

#[test]
fn bundle_records_node_rpc_state() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    init_home(home);
    let mut opts = options(home);
    opts.rpc = true;
    opts.node = fake_rpc();
    let entries = debug_bundle::collect(&opts);
    for method in [
        "status",
        "net_info",
        "num_unconfirmed_txs",
        "abci_info",
        "dump_consensus_state",
    ] {
        let v = json(entry(&entries, &format!("rpc/{method}.json")));
        assert_eq!(v["method"], method);
    }
}

#[test]
fn unreachable_node_is_an_error_entry_not_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    init_home(home);
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut opts = options(home);
    opts.rpc = true;
    opts.node = format!("tcp://127.0.0.1:{port}");
    let entries = debug_bundle::collect(&opts);
    let manifest = json(entry(&entries, "manifest.json"));
    assert!(
        manifest["errors"]
            .as_object()
            .unwrap()
            .contains_key("rpc/status.json")
    );
    entry(&entries, "config/config.toml");
}

fn untar(archive: &Path, into: &Path) {
    let out = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(into)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn cli_writes_a_tar_gz_that_tar_extracts() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    init_home(&home);
    let archive = dir.path().join("bundle.tar.gz");
    let out = Command::new(env!("CARGO_BIN_EXE_celestia-rs"))
        .arg("debug-bundle")
        .arg("--home")
        .arg(&home)
        .arg("--output")
        .arg(&archive)
        .arg("--no-rpc")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The path goes to stdout so scripts can pick it up.
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        archive.display().to_string()
    );
    let x = dir.path().join("x");
    std::fs::create_dir(&x).unwrap();
    untar(&archive, &x);
    let root: Vec<PathBuf> = std::fs::read_dir(&x)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    // One top-level directory, named like the archive.
    assert_eq!(root.len(), 1, "{root:?}");
    assert_eq!(root[0].file_name().unwrap(), "bundle");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root[0].join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["format"], 1);
    let config = std::fs::read_to_string(root[0].join("config/config.toml")).unwrap();
    assert!(config.contains("moniker"));
}

#[test]
fn default_output_is_a_timestamped_file_in_the_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    init_home(&home);
    let out = Command::new(env!("CARGO_BIN_EXE_celestia-rs"))
        .current_dir(dir.path())
        .args(["debug-bundle", "--no-rpc", "--home"])
        .arg(&home)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let name = path.file_name().unwrap().to_str().unwrap();
    assert!(
        name.starts_with("celestia-rs-debug-") && name.ends_with(".tar.gz"),
        "{name}"
    );
    assert!(dir.path().join(name).is_file());
}
