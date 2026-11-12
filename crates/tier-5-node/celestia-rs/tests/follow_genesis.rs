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

#[test]
fn genesis_follower_authenticates_first_commit_and_resumes_without_keys() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    make_chain(&source, 1);
    let server = Server::new(&source, false);
    let home = temp.path().join("follower");
    let mut opts = options(&source, &home, &server, 2);
    let first = follow::follow(&opts).unwrap();
    assert_eq!((first.first, first.last, first.blocks), (1, 2, 2));
    opts.stop_height = 3;
    let resumed = follow::follow(&opts).unwrap();
    assert_eq!((resumed.first, resumed.last, resumed.blocks), (3, 3, 1));
    assert!(!home.join("config/priv_validator_key.json").exists());
    assert!(!home.join("data/priv_validator_state.json").exists());
    let forged = Server::new(&source, true);
    let forged_home = temp.path().join("forged");
    let opts = options(&source, &forged_home, &forged, 2);
    let error = follow::follow(&opts).unwrap_err().to_string();
    assert!(error.contains("commit of block 1"), "{error}");
    let (_, store) = node::open_stores(&forged_home).unwrap();
    assert_eq!(store.load().unwrap().unwrap().last_block_height, 0);
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
