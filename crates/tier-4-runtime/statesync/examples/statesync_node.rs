//! A Rust state-syncing node (used by `conformance/devnet/statesync-rust.sh`).
//!
//! p2p `Switch` + state-sync reactor + light-client state provider against
//! celestia-core RPC + the in-process celestia-app state machine:
//!
//! 1. joins a Go devnet (persistent peer), discovers snapshots over 0x60,
//!    verifies the snapshot height with the light client (headers H, H+1,
//!    H+2 and consensus params), fetches chunks over 0x61 from Go peers and
//!    restores them into the Rust app, then checks the restored app hash
//!    against the light-client-verified header and bootstraps the
//!    (in-memory) node stores with the resulting State + Commit;
//! 2. takes a snapshot of the restored state and serves it to other nodes
//!    through the same reactor for `--serve-secs` seconds (a Go node
//!    state-syncs from it).
//!
//! Prints a JSON summary on stdout; exits 0 on success.
//!
//! ```text
//! cargo run -p statesync --example statesync_node -- --home DIR --print-id
//! cargo run -p statesync --example statesync_node -- --home DIR --chain-id C \
//!     --listen 127.0.0.1:P --peer ID@127.0.0.1:Q --rpc 127.0.0.1:R --rpc 127.0.0.1:R \
//!     --trust-height H --trust-hash HEX [--serve-secs 120]
//! ```
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use abci_server::Application;
use app::App;
use celestia_core_proto::tendermint::abci::{self, request, response};
use p2p::{
    ChannelDescriptor, Envelope, MConnConfig, NodeInfo, NodeInfoOther, NodeKey, ProtocolVersion,
    Reactor as _, Switch, SwitchConfig,
};
use statesync::syncer::Config;
use statesync::{
    AppConn, Bootstrapper as _, InProcessConn, LightClientStateProvider, NodeStores, Reactor,
};

/// The app behind a mutex, answering the ABCI calls state sync makes.
struct SharedApp(Mutex<App>);

impl Application for SharedApp {
    fn handle(&self, req: request::Value) -> response::Value {
        let mut app = self.0.lock().unwrap();
        match req {
            request::Value::Info(_) => response::Value::Info(app.info()),
            request::Value::ListSnapshots(_) => {
                response::Value::ListSnapshots(app.list_snapshots())
            }
            request::Value::LoadSnapshotChunk(r) => {
                response::Value::LoadSnapshotChunk(app.load_snapshot_chunk(&r))
            }
            request::Value::OfferSnapshot(r) => {
                response::Value::OfferSnapshot(app.offer_snapshot(r))
            }
            request::Value::ApplySnapshotChunk(r) => {
                response::Value::ApplySnapshotChunk(app.apply_snapshot_chunk(r))
            }
            other => response::Value::Exception(abci::ResponseException {
                error: format!("unsupported {other:?}"),
            }),
        }
    }
}

/// Counts what the reactor serves.
struct Counting {
    inner: InProcessConn<SharedApp>,
    lists: AtomicU64,
    chunks: AtomicU64,
    chunk_bytes: AtomicU64,
}

impl AppConn for Counting {
    fn list_snapshots(&self) -> Result<abci::ResponseListSnapshots, String> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        self.inner.list_snapshots()
    }
    fn load_snapshot_chunk(
        &self,
        req: abci::RequestLoadSnapshotChunk,
    ) -> Result<abci::ResponseLoadSnapshotChunk, String> {
        let r = self.inner.load_snapshot_chunk(req)?;
        self.chunks.fetch_add(1, Ordering::SeqCst);
        self.chunk_bytes
            .fetch_add(r.chunk.len() as u64, Ordering::SeqCst);
        Ok(r)
    }
    fn offer_snapshot(
        &self,
        req: abci::RequestOfferSnapshot,
    ) -> Result<abci::ResponseOfferSnapshot, String> {
        self.inner.offer_snapshot(req)
    }
    fn apply_snapshot_chunk(
        &self,
        req: abci::RequestApplySnapshotChunk,
    ) -> Result<abci::ResponseApplySnapshotChunk, String> {
        self.inner.apply_snapshot_chunk(req)
    }
    fn info(&self, req: abci::RequestInfo) -> Result<abci::ResponseInfo, String> {
        self.inner.info(req)
    }
}

/// Accepts (and drops) traffic on the channels celestia-core peers require
/// (`celestia_peer_filter`: propagation 0x50/0x51, CAT mempool 0x31/0x32).
struct Sink;

impl p2p::Reactor for Sink {
    fn name(&self) -> String {
        "SINK".into()
    }
    fn channels(&self) -> Vec<ChannelDescriptor> {
        [0x31u8, 0x32, 0x50, 0x51]
            .iter()
            .map(|&id| ChannelDescriptor {
                send_queue_capacity: 100,
                recv_message_capacity: 64 << 20,
                ..ChannelDescriptor::new(id, 1)
            })
            .collect()
    }
    fn receive(&self, _env: Envelope) -> Result<(), String> {
        Ok(())
    }
    fn queue_size(&self) -> usize {
        1000
    }
}

#[derive(Default)]
struct Args {
    home: PathBuf,
    print_id: bool,
    chain_id: String,
    listen: String,
    peers: Vec<String>,
    rpcs: Vec<String>,
    trust_height: i64,
    trust_hash: String,
    serve_secs: u64,
    snapshot_interval: u64,
}

fn parse_args() -> Args {
    let mut a = Args {
        listen: "127.0.0.1:0".into(),
        snapshot_interval: 10,
        ..Args::default()
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| panic!("missing value for {k}"));
        match k.as_str() {
            "--home" => a.home = PathBuf::from(v()),
            "--print-id" => a.print_id = true,
            "--chain-id" => a.chain_id = v(),
            "--listen" => a.listen = v(),
            "--peer" => a.peers.push(v()),
            "--rpc" => a.rpcs.push(v()),
            "--trust-height" => a.trust_height = v().parse().expect("trust height"),
            "--trust-hash" => a.trust_hash = v(),
            "--serve-secs" => a.serve_secs = v().parse().expect("serve secs"),
            "--snapshot-interval" => a.snapshot_interval = v().parse().expect("interval"),
            other => panic!("unknown argument {other}"),
        }
    }
    a
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let args = parse_args();
    std::fs::create_dir_all(&args.home).unwrap();
    let key = NodeKey::load_or_generate(&args.home.join("node_key.json")).unwrap();
    if args.print_id {
        println!("{}", key.id());
        return;
    }
    let my_id = key.id();

    // the in-process app (fresh redb state; chain ID known up front since a
    // state-synced node never sees InitChain)
    let app = App::open_with(
        args.home.join("app"),
        app::AppOptions {
            chain_id: Some(args.chain_id.clone()),
            snapshots: snapshots_options(args.snapshot_interval),
            ..app::AppOptions::default()
        },
    )
    .unwrap();
    assert_eq!(app.info().last_block_height, 0, "app home must be empty");
    let shared = Arc::new(SharedApp(Mutex::new(app)));
    let conn = Arc::new(Counting {
        inner: InProcessConn(Arc::clone(&shared)),
        lists: AtomicU64::new(0),
        chunks: AtomicU64::new(0),
        chunk_bytes: AtomicU64::new(0),
    });

    let info = NodeInfo {
        protocol_version: ProtocolVersion {
            p2p: p2p::node_info::P2P_PROTOCOL,
            block: p2p::node_info::BLOCK_PROTOCOL,
            app: 0,
        },
        network: args.chain_id.clone(),
        version: p2p::node_info::TM_CORE_SEMVER.into(),
        moniker: "rust-statesync".into(),
        listen_addr: format!("tcp://{}", args.listen),
        other: NodeInfoOther {
            tx_index: "off".into(),
            rpc_address: String::new(),
        },
        ..NodeInfo::default()
    };
    let cfg = SwitchConfig {
        listen_address: Some(format!("tcp://{}", args.listen)),
        allow_duplicate_ip: true,
        persistent_peers: args.peers.clone(),
        mconn: MConnConfig {
            send_rate: 20_480_000,
            recv_rate: 20_480_000,
            flush_throttle: Duration::from_millis(10),
            ..MConnConfig::default()
        },
        ..SwitchConfig::default()
    };
    let sw = Switch::new(cfg, key, info).unwrap();
    let ss_cfg = Config {
        temp_dir: args.home.join("tmp"),
        ..Config::default()
    };
    let reactor = Arc::new(Reactor::new(ss_cfg, Arc::clone(&conn) as Arc<dyn AppConn>));
    sw.add_reactor(Arc::clone(&reactor) as Arc<dyn p2p::Reactor>)
        .unwrap();
    sw.add_reactor(Arc::new(Sink)).unwrap();
    sw.start().await.unwrap();
    eprintln!(
        "rust node {my_id} listening on {:?}, channels {:x?}",
        sw.net_address(),
        reactor.channels().iter().map(|c| c.id).collect::<Vec<_>>()
    );

    // --- state sync ---------------------------------------------------------
    let trust = light::TrustOptions {
        period: 168 * 3600 * 1_000_000_000,
        height: args.trust_height,
        hash: hex::decode(&args.trust_hash).expect("trust hash"),
    };
    let chain_id = args.chain_id.clone();
    let rpcs = args.rpcs.clone();
    let r2 = Arc::clone(&reactor);
    let started = std::time::Instant::now();
    let res = tokio::task::spawn_blocking(move || {
        let sp = LightClientStateProvider::new(
            &chain_id,
            ctypes::Consensus {
                block: ctypes::header::BLOCK_PROTOCOL,
                app: 0,
            },
            1,
            &rpcs,
            &trust,
        )
        .map_err(|e| format!("failed to set up light client state provider: {e}"))?;
        r2.sync(Arc::new(sp), Duration::from_secs(5))
            .map_err(|e| format!("State sync failed: {e}"))
    })
    .await
    .unwrap();
    let (state, commit) = match res {
        Ok(r) => r,
        Err(e) => {
            println!("{}", serde_json::json!({ "ok": false, "error": e }));
            sw.stop();
            std::process::exit(1);
        }
    };
    let sync_secs = started.elapsed().as_secs_f64();
    // bootstrap the node stores (redb, like the app) and read them back
    let node_db: Arc<dyn kvdb::KvBackend> = Arc::new(
        kvdb::RedbBackend::open(args.home.join("node.redb"), kvdb::RedbOptions::default()).unwrap(),
    );
    let stores = NodeStores {
        state_store: nodestore::StateStore::new(Arc::clone(&node_db)),
        block_store: nodestore::BlockStore::new(Arc::clone(&node_db)).unwrap(),
    };
    stores.bootstrap(&state, &commit).unwrap();
    let stored_state = stores.state_store.load().unwrap();
    let stored_commit = stores
        .block_store
        .load_seen_commit(state.last_block_height)
        .unwrap();
    let h1 = state.last_block_height + 1;
    let stored_vals = stores.state_store.load_validators(h1).unwrap();

    // independent checks of what the syncer verified
    let app_info = shared.0.lock().unwrap().info();
    let mut failures = Vec::new();
    if app_info.last_block_app_hash != state.app_hash {
        failures.push("restored app hash != light-client-verified app hash".to_string());
    }
    if app_info.last_block_height != state.last_block_height {
        failures.push("restored height != snapshot height".into());
    }
    if commit.height != state.last_block_height || commit.block_id != state.last_block_id {
        failures.push("commit does not match the state's last block".into());
    }
    // compare encodings (a decoded validator set keeps its proposer
    // detached, so the structs differ in representation only)
    if stored_state.as_ref().map(|s| s.bytes().unwrap()) != Some(state.bytes().unwrap()) {
        failures.push("state store does not hold the bootstrapped state".into());
    }
    if stored_commit.as_ref() != Some(&commit) {
        failures.push("block store does not hold the seen commit".into());
    }
    if stored_vals.hash() != state.validators.hash() {
        failures.push("state store validators differ".into());
    }
    if state.validators.validators.is_empty() || state.consensus_params.block.max_bytes == 0 {
        failures.push("incomplete bootstrap state".into());
    }
    let height = u64::try_from(state.last_block_height).unwrap();

    // --- serve the restored state to others --------------------------------
    let served_snapshot = {
        let app = shared.0.lock().unwrap();
        let mgr = app.snapshot_manager().expect("snapshot manager");
        mgr.create(app.multistore().root(), height)
            .map(|s| (s.height, s.chunks, hex::encode(&s.hash)))
    };
    eprintln!(
        "serving snapshot {served_snapshot:?} for {}s",
        args.serve_secs
    );
    let fetched_by_peers_before = conn.chunks.load(Ordering::SeqCst);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(args.serve_secs);
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let peers: Vec<String> = sw
        .peers()
        .iter()
        .map(|p| format!("{}:{}", p.id(), p.node_info().moniker))
        .collect();
    let summary = serde_json::json!({
        "ok": failures.is_empty(),
        "node_id": my_id.to_string(),
        "snapshot_height": height,
        "sync_seconds": sync_secs,
        "trusted_app_hash": hex::encode_upper(&state.app_hash),
        "restored_app_hash": hex::encode_upper(&app_info.last_block_app_hash),
        "restored_height": app_info.last_block_height,
        "app_version": app_info.app_version,
        "state": {
            "chain_id": state.chain_id,
            "last_block_height": state.last_block_height,
            "last_block_id": hex::encode_upper(&state.last_block_id.hash),
            "validators": state.validators.validators.len(),
            "last_height_validators_changed": state.last_height_validators_changed,
            "consensus_params_hash": hex::encode_upper(state.consensus_params.hash()),
            "version": state.version.app,
            "timeout_commit_ms": state.timeouts.commit / 1_000_000,
        },
        "commit_signatures": commit.signatures.len(),
        "served_snapshot": served_snapshot.as_ref().ok(),
        "snapshot_list_requests": conn.lists.load(Ordering::SeqCst),
        "chunks_served": conn.chunks.load(Ordering::SeqCst) - fetched_by_peers_before,
        "chunk_bytes_served": conn.chunk_bytes.load(Ordering::SeqCst),
        "peers": peers,
        "failures": failures,
    });
    println!("{summary}");
    sw.stop();
    if !failures.is_empty() {
        std::process::exit(1);
    }
}

fn snapshots_options(interval: u64) -> snapshots::SnapshotOptions {
    snapshots::SnapshotOptions {
        interval,
        keep_recent: 2,
    }
}
