//! Block sync of Celestia mainnet from genesis, offline after a one-time
//! download: an in-process "archive peer" (a block sync reactor over a
//! block store holding real mainnet blocks 1..=N+1) serves a fresh node
//! that starts from the canonical mainnet genesis, runs InitChain and every
//! block through the app version 1 legacy driver, restarts midway and must
//! reproduce every app hash the network committed.
//!
//! Gated (`--ignored`): it downloads the 88 MB mainnet genesis and the
//! blocks over HTTPS once and caches them.
//!
//! - `CELESTIA_MAINNET_CACHE`: cache directory (default
//!   `$CARGO_TARGET_TMPDIR/mainnet-cache`).
//! - `CELESTIA_BLOCKSYNC_HEIGHT`: last height to sync (default 300).
//! - `CELESTIA_ARCHIVE_RPC`: archive RPC (default polkachu archive).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr
)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use blocksync::reactor::Network;
use blocksync::{Reactor, ReactorOptions};
use celestia_rs::{networks, node, ops::Shutdown, sync};
use ctypes::{BLOCK_PART_SIZE_BYTES, Block};
use nodestore::{BlockExecutor, BlockStore, Handshaker, StateStore};
use prost::Message as _;

const ARCHIVE: &str = "archive";
const CLIENT: &str = "client";

fn cache_dir() -> PathBuf {
    std::env::var_os("CELESTIA_MAINNET_CACHE").map_or_else(
        || Path::new(env!("CARGO_TARGET_TMPDIR")).join("mainnet-cache"),
        PathBuf::from,
    )
}

fn genesis_file(cache: &Path) -> PathBuf {
    let path = cache.join("genesis.json");
    if !path.exists() {
        std::fs::create_dir_all(cache).unwrap();
        networks::download_genesis(&networks::MAINNET, &path).unwrap();
    }
    networks::verify_genesis(&networks::MAINNET, &std::fs::read(&path).unwrap()).unwrap();
    path
}

/// Mainnet block `height`, from the cache or the archive RPC (with back-off
/// on errors such as HTTP 429).
fn mainnet_block(cache: &Path, rpc: &light::RpcClient, height: i64) -> Block {
    let dir = cache.join("blocks");
    let path = dir.join(format!("{height}.pb"));
    if let Ok(bytes) = std::fs::read(&path) {
        return Block::from_proto(&ctypes::proto::types::Block::decode(bytes.as_slice()).unwrap())
            .unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    let mut wait = Duration::from_millis(500);
    let block = loop {
        match rpc.block(height) {
            Ok((_, block)) => break block,
            Err(e) if wait < Duration::from_secs(120) => {
                eprintln!("block {height}: {e}; retrying in {wait:?}");
                std::thread::sleep(wait);
                wait *= 2;
            }
            Err(e) => panic!("block {height}: {e}"),
        }
    };
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, block.to_proto().unwrap().encode_to_vec()).unwrap();
    std::fs::rename(&tmp, &path).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    block
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

/// The archive peer: blocks `1..=blocks.len()-1` with their canonical
/// commits (the next block's `LastCommit`).
fn archive_peer(genesis: &nodestore::GenesisDoc, blocks: &[Block]) -> Arc<Reactor> {
    let bs = BlockStore::new(Arc::new(kvdb::MemBackend::new())).unwrap();
    let ss = StateStore::new(Arc::new(kvdb::MemBackend::new()));
    for pair in blocks.windows(2) {
        let mut block = pair[0].clone();
        let parts = block.make_part_set(BLOCK_PART_SIZE_BYTES).unwrap();
        bs.save_block(&mut block, &parts, pair[1].last_commit.as_ref().unwrap())
            .unwrap();
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
    let app = app::App::open_with(home, opts).unwrap_or_else(|e| panic!("open app: {}", e.log));
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
    let out = sync::block_sync(
        &client,
        state,
        &mut exec,
        false,
        false,
        stop_height,
        &shutdown,
    )
    .unwrap();
    let t = client.timings();
    eprintln!(
        "synced {} blocks: {:?} total, per phase {t:?}",
        t.blocks,
        t.total()
    );
    // Break the reactor <-> network cycle so the stores close.
    net.reactors.write().unwrap().clear();
    let stopped = net.stopped.lock().unwrap().clone();
    assert!(stopped.is_empty(), "peers stopped: {stopped:?}");
    out.state
}

#[test]
#[ignore = "downloads the mainnet genesis (88 MB) and blocks over HTTPS once"]
fn block_syncs_mainnet_from_genesis_with_restart() {
    let cache = cache_dir();
    let last: i64 = std::env::var("CELESTIA_BLOCKSYNC_HEIGHT")
        .ok()
        .map_or(300, |v| v.parse().unwrap());
    assert!(last >= 4, "CELESTIA_BLOCKSYNC_HEIGHT must be at least 4");
    let rpc = light::RpcClient::new(
        &std::env::var("CELESTIA_ARCHIVE_RPC")
            .unwrap_or_else(|_| "https://celestia-archive-rpc.polkachu.com".into()),
        Duration::from_secs(30),
    );
    let genesis_path = genesis_file(&cache);
    // Syncing block H needs H+1 (its commit); the archive stores H+1 with
    // the commit carried by H+2.
    let blocks: Vec<Block> = (1..=last + 2)
        .map(|h| mainnet_block(&cache, &rpc, h))
        .collect();
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("config")).unwrap();
    std::fs::create_dir_all(home.path().join("data")).unwrap();
    std::fs::copy(&genesis_path, home.path().join("config/genesis.json")).unwrap();
    let genesis = node::load_genesis(home.path()).unwrap();
    let archive = archive_peer(&genesis, &blocks);

    // Crash/restart in the middle of the v0.46 era: the second run reloads
    // the legacy app state and the consensus state from disk.
    let middle = last / 2;
    let state = sync_node(home.path(), &archive, middle);
    assert_eq!(state.last_block_height, middle);
    let state = sync_node(home.path(), &archive, last);
    assert_eq!(state.last_block_height, last);

    let (bs, ss) = node::open_stores(home.path()).unwrap();
    for h in 1..=last {
        let block = &blocks[usize::try_from(h - 1).unwrap()];
        let next = &blocks[usize::try_from(h).unwrap()];
        assert_eq!(
            bs.load_block_meta(h).unwrap().unwrap().block_id,
            next.header.last_block_id,
            "block id at {h}"
        );
        assert_eq!(block.header.version.app, 1, "app version at {h}");
        let got = ss
            .load_finalize_block_response(h)
            .unwrap()
            .unwrap()
            .app_hash;
        assert_eq!(
            hex::encode_upper(got),
            hex::encode_upper(&next.header.app_hash),
            "app hash after block {h}"
        );
    }
}
