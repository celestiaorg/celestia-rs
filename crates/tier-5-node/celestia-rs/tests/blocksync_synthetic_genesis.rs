//! Block sync of the synthetic chain from genesis through every upgrade,
//! offline: an in-process "archive peer" (a block sync reactor over a block
//! store holding the chain's blocks with their commits) serves a fresh
//! node that starts from the synthetic genesis and runs every block through
//! the real block executor, across app versions 1, 2, 3 (cosmos-sdk v0.46
//! driver), the v3 to v4 hand-off and app versions 4, 5, 6, 8 and 9.
//!
//! The chain is the oracles' `-consensus` run
//! (`conformance/fixtures/synthetic/consensus`): each block holds the txs,
//! square size and data root of the app's PrepareProposal, and each commit
//! is signed by the validators' consensus keys, so the node checks every
//! commit (with the validator set changes the chain makes), every header
//! (time, hashes, app hash), the data root of each block of app versions 1
//! to 8, and ProcessProposal at app version 9. The node stops and restarts
//! around the upgrades, and must reproduce every app hash the oracles
//! committed.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr
)]

use std::collections::HashMap;
use std::io::BufRead as _;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

use blocksync::reactor::Network;
use blocksync::{Reactor, ReactorOptions};
use celestia_rs::{node, ops::Shutdown, sync};
use ctypes::{BLOCK_PART_SIZE_BYTES, Block, Commit};
use nodestore::{BlockExecutor, BlockStore, Handshaker, StateStore};

const ARCHIVE: &str = "archive";
const CLIENT: &str = "client";
const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/consensus"
);
const STAGES: [&str; 6] = [
    "v1-v3", "v4-to-v5", "v5-to-v6", "v6-to-v8", "v8-to-v9", "v9",
];
const LAST: i64 = 61;
/// The last block the node can sync: block sync applies block h once it
/// holds block h+1, whose last commit commits h.
const SYNCED: i64 = LAST - 1;

fn gz_lines(path: &str) -> Vec<serde_json::Value> {
    let f = std::fs::File::open(path).unwrap();
    std::io::BufReader::new(flate2::read::GzDecoder::new(f))
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
        .collect()
}

fn json(path: &str) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// The chain: blocks 1 to 61, the commit of block 61, and the app hash the
/// oracles committed after each block.
struct Chain {
    blocks: Vec<Block>,
    last_commit: Commit,
    app_hash: HashMap<i64, String>,
}

fn chain() -> Chain {
    let mut blocks = Vec::new();
    let mut app_hash = HashMap::new();
    for stage in STAGES {
        let dir = format!("{FIXTURES}/{stage}");
        let roots = json(&format!("{dir}/store-roots.json"));
        let first = roots["handoff_height"].as_i64().unwrap_or(0) + 1;
        for row in gz_lines(&format!("{dir}/blocks.jsonl.gz")) {
            let h = row["height"].as_i64().unwrap();
            if h >= first {
                blocks.push(light::http::block_from_json(&row["result"]["block"]).unwrap());
            }
        }
        for r in roots["heights"].as_array().unwrap() {
            let h = r["height"].as_i64().unwrap();
            if h >= first {
                app_hash.insert(h, r["app_hash"].as_str().unwrap().to_owned());
            }
        }
    }
    assert_eq!(
        blocks.iter().map(|b| b.header.height).collect::<Vec<_>>(),
        (1..=LAST).collect::<Vec<_>>()
    );
    let last_commit =
        light::http::commit_from_json(&json(&format!("{FIXTURES}/v9/last-commit.json"))).unwrap();
    assert_eq!(last_commit.height, LAST);
    Chain {
        blocks,
        last_commit,
        app_hash,
    }
}

/// The synthetic chain's v1 to v2 upgrade height (the oracle's
/// `--v2-upgrade-height`).
fn v2_upgrade_height() -> i64 {
    json(&format!("{FIXTURES}/v1-v3/store-roots.json"))["upgrade_height_v2"]
        .as_i64()
        .unwrap()
}

/// In-memory block sync network (synchronous delivery).
#[derive(Default)]
struct Net {
    reactors: RwLock<HashMap<String, Arc<Reactor>>>,
    stopped: Mutex<Vec<(String, String)>>,
}

struct Endpoint {
    me: String,
    net: Arc<Net>,
}

impl Network for Endpoint {
    fn try_send(&self, peer: &str, msg: Vec<u8>) -> bool {
        let Some(r) = self.net.reactors.read().unwrap().get(peer).cloned() else {
            return false;
        };
        if let Err(e) = r.receive_bytes(&self.me, &msg) {
            self.net.stopped.lock().unwrap().push((self.me.clone(), e));
        }
        true
    }
    fn broadcast(&self, msg: Vec<u8>) {
        let peers: Vec<String> = self.net.reactors.read().unwrap().keys().cloned().collect();
        for p in peers.iter().filter(|p| **p != self.me) {
            self.try_send(p, msg.clone());
        }
    }
    fn stop_peer_for_error(&self, peer: &str, reason: &str) {
        self.net
            .stopped
            .lock()
            .unwrap()
            .push((peer.to_owned(), reason.to_owned()));
        if let Some(r) = self.net.reactors.read().unwrap().get(&self.me) {
            r.pool().remove_peer(peer);
        }
    }
}

fn register(net: &Arc<Net>, name: &str, reactor: Arc<Reactor>) {
    reactor.set_network(Arc::new(Endpoint {
        me: name.into(),
        net: Arc::clone(net),
    }));
    net.reactors.write().unwrap().insert(name.into(), reactor);
}

/// The archive peer: every block with its commit (the next block's last
/// commit; block 61's from `last-commit.json`).
fn archive_peer(genesis: &nodestore::GenesisDoc, chain: &Chain) -> Arc<Reactor> {
    let bs = BlockStore::new(Arc::new(kvdb::MemBackend::new())).unwrap();
    let ss = StateStore::new(Arc::new(kvdb::MemBackend::new()));
    for (i, block) in chain.blocks.iter().enumerate() {
        let commit = chain.blocks.get(i + 1).map_or(&chain.last_commit, |next| {
            next.last_commit.as_ref().unwrap()
        });
        let mut block = block.clone();
        let parts = block.make_part_set(BLOCK_PART_SIZE_BYTES).unwrap();
        bs.save_block(&mut block, &parts, commit).unwrap();
    }
    let mut state = ss.load_from_db_or_genesis(genesis).unwrap();
    state.last_block_height = bs.height();
    ss.save(&state).unwrap();
    Arc::new(Reactor::new(&state, bs, ss, 0, ReactorOptions::default()).unwrap())
}

/// Opens the node at `home` (as `celestia-rs start` does) and block syncs
/// from `archive` until `stop_height`.
fn sync_node(home: &Path, archive: &Arc<Reactor>, stop_height: i64) -> nodestore::State {
    let genesis = node::load_genesis(home).unwrap();
    let mut opts = app::AppOptions {
        chain_id: Some(genesis.chain_id.clone()),
        genesis_app_version: Some(genesis.consensus_params.version.app),
        ..app::AppOptions::default()
    };
    opts.node.home = Some(home.to_path_buf());
    let mut app = app::App::open_with(home, opts).unwrap_or_else(|e| panic!("open app: {}", e.log));
    // celestia-app's `--v2-upgrade-height` for this chain (chain ID `test`
    // has no built-in default).
    app.set_v2_upgrade_height(v2_upgrade_height());
    let app = Arc::new(Mutex::new(app));
    let (bs, ss) = node::open_stores(home).unwrap();
    let state = ss.load_from_db_or_genesis(&genesis).unwrap();
    let evpool = sync::open_evidence_pool(home, &ss, &bs, &state).unwrap();
    let mut exec = BlockExecutor::new(
        ss.clone(),
        bs.clone(),
        Box::new(node::AppAdapter(app)),
        Arc::new(nodestore::NopMempool),
        evpool.executor_handle(),
    );
    let state = Handshaker::new(
        ss.clone(),
        state,
        bs.clone(),
        &genesis,
        consensus::ticker::system_now(),
    )
    .handshake(&mut exec)
    .unwrap();
    let net = Arc::new(Net::default());
    let client = Arc::new(Reactor::new(&state, bs, ss, 0, sync::blocksync_options(None)).unwrap());
    register(&net, ARCHIVE, Arc::clone(archive));
    register(&net, CLIENT, Arc::clone(&client));
    client.add_peer_id(ARCHIVE);
    archive.add_peer_id(CLIENT);
    let shutdown = Shutdown::new();
    // A sync that stops making progress (a rejected block, a stopped peer)
    // would wait for peers forever: give up after two minutes.
    let watchdog = shutdown.clone();
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let finished = Arc::clone(&done);
    std::thread::spawn(move || {
        for _ in 0..1200 {
            if finished.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        watchdog.trigger();
    });
    let out = sync::block_sync(
        &client,
        state,
        &mut exec,
        false,
        false,
        stop_height,
        &shutdown,
    )
    .unwrap_or_else(|e| panic!("block sync to {stop_height}: {e}"));
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    net.reactors.write().unwrap().clear();
    let stopped = net.stopped.lock().unwrap().clone();
    assert!(
        stopped.is_empty() && out.state.last_block_height >= stop_height,
        "sync to {stop_height} stopped at {}; peers stopped: {stopped:?}",
        out.state.last_block_height
    );
    out.state
}

/// The node block syncs genesis to 60 (block 61 commits it), stopping and
/// restarting after the block before and the block of each upgrade, and
/// stores Go's block IDs and app hashes.
#[test]
fn block_syncs_the_synthetic_chain_from_genesis_through_every_upgrade() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .try_init();
    let chain = chain();
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("config")).unwrap();
    std::fs::create_dir_all(home.path().join("data")).unwrap();
    std::fs::copy(
        format!("{FIXTURES}/v1-v3/genesis.json"),
        home.path().join("config/genesis.json"),
    )
    .unwrap();
    let genesis = node::load_genesis(home.path()).unwrap();
    let archive = archive_peer(&genesis, &chain);

    let mut stops: Vec<i64> = [6, 11, 16, 26, 37, 47, 57]
        .iter()
        .flat_map(|u| [u - 1, *u])
        .collect();
    stops.push(SYNCED);
    for stop in stops {
        let state = sync_node(home.path(), &archive, stop);
        assert_eq!(state.last_block_height, stop);
    }

    let (bs, ss) = node::open_stores(home.path()).unwrap();
    for h in 1..=SYNCED {
        let i = usize::try_from(h - 1).unwrap();
        let block = &chain.blocks[i];
        let id = chain
            .blocks
            .get(i + 1)
            .map_or(&chain.last_commit.block_id, |next| {
                &next.header.last_block_id
            });
        assert_eq!(
            &bs.load_block_meta(h).unwrap().unwrap().block_id,
            id,
            "block id at {h}"
        );
        let got = ss
            .load_finalize_block_response(h)
            .unwrap()
            .unwrap()
            .app_hash;
        assert_eq!(
            hex::encode_upper(got),
            chain.app_hash[&h],
            "app hash after block {h} (v{})",
            block.header.version.app
        );
    }
}
