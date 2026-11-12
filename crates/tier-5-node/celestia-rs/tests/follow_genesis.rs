//! Genesis bootstrapping, resume and first-commit authentication over offline RPC.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use celestia_rs::{
    follow::{self, FollowOptions},
    node,
    ops::Shutdown,
};
use serde_json::{Value, json};

struct Server {
    address: String,
    stopped: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    fn new(source: &Path, forged: bool) -> Self {
        let (blocks, states) = node::open_stores(source).unwrap();
        let state = states.load().unwrap().unwrap();
        let mut replies = BTreeMap::new();
        replies.insert("/status".to_string(), json!({
            "node_info":{"network":state.chain_id,"id":"offline-fixture"},
            "sync_info":{"latest_block_height":blocks.height().to_string(),"latest_block_hash":hex::encode(&state.last_block_id.hash),"earliest_block_height":blocks.base().to_string(),"catching_up":false}
        }));
        for height in blocks.base()..=blocks.height() {
            let mut block = blocks.load_block(height).unwrap().unwrap();
            let meta = blocks.load_block_meta(height).unwrap().unwrap();
            if forged && height == blocks.base() + 1 {
                block.last_commit.as_mut().unwrap().signatures[0].signature[0] ^= 1;
            }
            let mut encoded = String::new();
            rpc::encode::block(&block).write(&mut encoded);
            let mut block_id = String::new();
            rpc::encode::block_id(&meta.block_id).write(&mut block_id);
            replies.insert(
                format!("/block?height={height}"),
                json!({
                    "block_id":serde_json::from_str::<Value>(&block_id).unwrap(),
                    "block":serde_json::from_str::<Value>(&encoded).unwrap(),
                }),
            );
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let done = stopped.clone();
        let worker = std::thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                let Ok((mut connection, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                connection
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while request.len() < 8192 && !request.windows(4).any(|b| b == b"\r\n\r\n") {
                    let Ok(n) = connection.read(&mut buffer) else {
                        break;
                    };
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..n]);
                }
                let line = String::from_utf8_lossy(&request);
                let path = line.split_whitespace().nth(1).unwrap_or("");
                let body = replies.get(path).map_or_else(
                    || json!({"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"height unavailable","data":path}}),
                    |result| json!({"jsonrpc":"2.0","id":1,"result":result})
                ).to_string();
                let _ = write!(
                    connection,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
            }
        });
        Self {
            address,
            stopped,
            worker: Some(worker),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn make_chain(home: &Path, first: i64) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/home/config");
    std::fs::create_dir_all(home.join("config")).unwrap();
    std::fs::create_dir_all(home.join("data")).unwrap();
    let mut genesis: Value =
        serde_json::from_slice(&std::fs::read(fixture.join("genesis.json")).unwrap()).unwrap();
    genesis["initial_height"] = json!(first.to_string());
    std::fs::write(
        home.join("config/genesis.json"),
        serde_json::to_vec(&genesis).unwrap(),
    )
    .unwrap();
    // Public, deterministic devnet fixture key; no production home is read.
    std::fs::copy(
        fixture.join("priv_validator_key.json"),
        home.join("config/priv_validator_key.json"),
    )
    .unwrap();
    std::fs::write(
        home.join("data/priv_validator_state.json"),
        r#"{"height":"0","round":0,"step":0}"#,
    )
    .unwrap();
    let options = node::StartOptions::new(
        home.to_path_buf(),
        first + 5,
        consensus::ConsensusConfig::default(),
    );
    let inbox = home.join("data/tx-inbox");
    std::fs::create_dir_all(&inbox).unwrap();
    for entry in std::fs::read_dir(fixture.parent().unwrap().join("txs")).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), inbox.join(entry.file_name())).unwrap();
    }
    node::start(&options).unwrap();
}

fn options(source: &Path, home: &Path, server: &Server, stop: i64) -> FollowOptions {
    std::fs::create_dir_all(home.join("config")).unwrap();
    std::fs::copy(
        source.join("config/genesis.json"),
        home.join("config/genesis.json"),
    )
    .unwrap();
    FollowOptions {
        home: home.to_path_buf(),
        rpc: vec![server.address.clone()],
        chain_id: None,
        snapshot: None,
        snapshot_height: None,
        trust_hash: None,
        trust_period: 10000 * 3600 * 1_000_000_000,
        stop_height: stop,
        prefetch: 2,
        verify_data: true,
        witness_interval: 0,
        app: app::AppOptions::default(),
        shutdown: Shutdown::new(),
    }
}

fn receipt_rows(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn check_receipts(path: &Path, heights: &[i64], status: &str) {
    use base64::Engine as _;
    let rows = receipt_rows(path);
    assert_eq!(rows[0]["event"], "run_start");
    assert_eq!(rows[0]["data"]["candidate_commit"], "1".repeat(40));
    assert_eq!(rows[0]["data"]["binary_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(rows.last().unwrap()["event"], "run_end");
    assert_eq!(rows.last().unwrap()["data"]["status"], status);
    let records: Vec<_> = rows
        .iter()
        .filter(|r| r["event"] == "height")
        .map(|r| &r["data"])
        .collect();
    assert_eq!(
        records
            .iter()
            .map(|r| r["executed_height"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        heights
    );
    for record in records {
        assert_eq!(
            record["reference_header_height"].as_i64().unwrap(),
            record["executed_height"].as_i64().unwrap() + 1
        );
        assert_eq!(record["local_app_hash"], record["reference_app_hash"]);
        assert_eq!(
            record["local_results_hash"],
            record["reference_results_hash"]
        );
        let results: Vec<_> = record["tx_results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|result| ctypes::proto::abci::ExecTxResult {
                code: u32::try_from(result["code"].as_u64().unwrap()).unwrap(),
                data: base64::engine::general_purpose::STANDARD
                    .decode(result["data"].as_str().unwrap())
                    .unwrap(),
                gas_wanted: result["gas_wanted"].as_i64().unwrap(),
                gas_used: result["gas_used"].as_i64().unwrap(),
                ..Default::default()
            })
            .collect();
        assert_eq!(
            results.len(),
            usize::try_from(record["tx_count"].as_u64().unwrap()).unwrap()
        );
        assert_eq!(
            hex::encode(ctypes::results_hash(&results)),
            record["local_results_hash"]
        );
    }
}

#[test]
fn genesis_follower_authenticates_first_commit_and_resumes_without_keys() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    make_chain(&source, 1);
    let server = Server::new(&source, false);
    let home = temp.path().join("follower");
    let mut opts = options(&source, &home, &server, 2);
    let receipt_config = follow::FollowReceiptOptions {
        path: temp.path().join("first.jsonl"),
        candidate_commit: "1".repeat(40),
    };
    let first = follow::follow_with_receipts(&opts, &receipt_config).unwrap();
    assert_eq!((first.first, first.last, first.blocks), (1, 2, 2));
    assert!(first.txs > 0, "receipt test needs finalized transactions");
    check_receipts(&receipt_config.path, &[1, 2], "complete");
    let original = std::fs::read(&receipt_config.path).unwrap();
    assert!(follow::follow_with_receipts(&opts, &receipt_config).is_err());
    assert_eq!(std::fs::read(&receipt_config.path).unwrap(), original);
    opts.stop_height = 3;
    let resume_config = follow::FollowReceiptOptions {
        path: temp.path().join("resume.jsonl"),
        candidate_commit: "1".repeat(40),
    };
    let resumed = follow::follow_with_receipts(&opts, &resume_config).unwrap();
    check_receipts(&resume_config.path, &[3], "complete");
    let resumed_rows = receipt_rows(&resume_config.path);
    assert_eq!(
        resumed_rows
            .iter()
            .find(|r| r["event"] == "anchor_verified")
            .unwrap()["data"]["height"],
        2
    );
    assert_eq!((resumed.first, resumed.last, resumed.blocks), (3, 3, 1));
    assert!(!home.join("config/priv_validator_key.json").exists());
    assert!(!home.join("data/priv_validator_state.json").exists());
    let forged = Server::new(&source, true);
    let forged_home = temp.path().join("forged");
    let opts = options(&source, &forged_home, &forged, 2);
    let forged_config = follow::FollowReceiptOptions {
        path: temp.path().join("forged.jsonl"),
        candidate_commit: "1".repeat(40),
    };
    let error = follow::follow_with_receipts(&opts, &forged_config)
        .unwrap_err()
        .to_string();
    assert!(error.contains("commit of block 1"), "{error}");
    check_receipts(&forged_config.path, &[], "failed");
    let (_, store) = node::open_stores(&forged_home).unwrap();
    assert_eq!(store.load().unwrap().unwrap().last_block_height, 0);
    let interrupted_home = temp.path().join("interrupted");
    let opts = options(&source, &interrupted_home, &server, 2);
    opts.shutdown.trigger();
    let interrupted_config = follow::FollowReceiptOptions {
        path: temp.path().join("interrupted.jsonl"),
        candidate_commit: "1".repeat(40),
    };
    follow::follow_with_receipts(&opts, &interrupted_config).unwrap();
    check_receipts(&interrupted_config.path, &[], "interrupted");
    let wrong_home = temp.path().join("wrong-chain");
    let opts = options(&source, &wrong_home, &server, 2);
    let path = wrong_home.join("config/genesis.json");
    let mut genesis: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    genesis["chain_id"] = json!("another-network");
    std::fs::write(path, serde_json::to_vec(&genesis).unwrap()).unwrap();
    assert!(
        follow::follow(&opts)
            .unwrap_err()
            .to_string()
            .contains("genesis follows")
    );
    for (height, hash) in [(Some(1), None), (None, Some(vec![0; 32]))] {
        let flags_home = temp
            .path()
            .join(format!("unused-flags-{}", height.is_some()));
        let mut opts = options(&source, &flags_home, &server, 2);
        opts.snapshot_height = height;
        opts.trust_hash = hash;
        assert!(
            follow::follow(&opts)
                .unwrap_err()
                .to_string()
                .contains("require --snapshot")
        );
    }
}

#[test]
fn genesis_follower_preserves_unsupported_initial_height_rejection() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    make_chain(&source, 1);
    let server = Server::new(&source, false);
    let home = temp.path().join("follower");
    let opts = options(&source, &home, &server, 8);
    let path = home.join("config/genesis.json");
    let mut genesis: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    genesis["initial_height"] = json!("7");
    std::fs::write(path, serde_json::to_vec(&genesis).unwrap()).unwrap();
    let error = follow::follow(&opts).unwrap_err().to_string();
    assert!(
        error.contains("initial_height > 1: feature not supported"),
        "{error}"
    );
}
