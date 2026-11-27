//! Block sync of the synthetic chain from genesis through every upgrade,
//! offline: an in-process "archive peer" (a block sync reactor over a block
//! store holding the chain's blocks with their commits) serves a fresh
//! node that starts from the synthetic genesis and runs every block through
//! the real block executor, across app versions 1, 2, 3 (cosmos-sdk v0.46
//! driver), the v3 to v4 hand-off and app versions 4, 5, 6, 8, 9, 10 and
//! 11.
//!
//! The chain is the oracles' `-consensus` run
//! (`conformance/fixtures/synthetic/consensus`): each block holds the txs,
//! square size and data root of the app's PrepareProposal, and each commit
//! is signed by the validators' consensus keys (with absent validators, and
//! a nil vote in the commit of block 80), so the node checks every commit
//! (with the validator set changes the chain makes), every header (time,
//! hashes, app hash), the data root of each block of app versions 1 to 8,
//! and ProcessProposal from app version 9. The node stops and restarts
//! around the upgrades, crosses the v11 upgrade in one process (it switches
//! to the v11 release in-process, as `celestia-rs start` does), and must
//! reproduce every app hash the oracles committed. A commit with one bad signature, and a block whose
//! txs no longer give its data root (its commit signed again for the new
//! part set), must stop the sync before that block.
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
const STAGES: [&str; 8] = [
    "v1-v3",
    "v4-to-v5",
    "v5-to-v6",
    "v6-to-v8",
    "v8-to-v9",
    "v9-to-v10",
    "v10-to-v11",
    "v11",
];
const LAST: i64 = 81;
/// The first block of each app version after 1.
const UPGRADES: [i64; 9] = [6, 11, 16, 26, 37, 47, 57, 67, 77];
/// The first v11 block. The node runs the block before it with the v10
/// release and, once that block commits consensus version 11 with plan
/// `v11` due, executes this one with the v11 release in the same process.
const V11_UPGRADE: i64 = 77;
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

#[derive(Clone)]
/// The chain: blocks 1 to 81, the commit of block 81, and the app hash the
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
        light::http::commit_from_json(&json(&format!("{FIXTURES}/v11/last-commit.json"))).unwrap();
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
/// commit; block 81's from `last-commit.json`).
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
/// from `archive` until `stop_height`. Returns the block sync error (the
/// planned halt of an upgrade without a handler in the executing release),
/// or the state and the peers the node stopped.
fn try_sync(
    home: &Path,
    archive: &Arc<Reactor>,
    stop_height: i64,
) -> Result<(nodestore::State, Vec<(String, String)>), String> {
    let genesis = node::load_genesis(home).unwrap();
    let mut opts = app::AppOptions {
        chain_id: Some(genesis.chain_id.clone()),
        genesis_app_version: Some(genesis.consensus_params.version.app),
        ..app::AppOptions::default()
    };
    opts.node.home = Some(home.to_path_buf());
    // `celestia-rs start` activates the bundled releases.
    opts.node.activate_bundled_upgrades = true;
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
    // would wait for peers forever: stop once the peer is stopped, or after
    // two minutes.
    let watchdog = shutdown.clone();
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let finished = Arc::clone(&done);
    // The only peer stopped: nothing more can arrive, so stop syncing.
    let peers = Arc::clone(&net);
    std::thread::spawn(move || {
        for _ in 0..1200 {
            if finished.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            if !peers.stopped.lock().unwrap().is_empty() {
                break;
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
    );
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    net.reactors.write().unwrap().clear();
    let stopped = net.stopped.lock().unwrap().clone();
    let out = out.map_err(|e| format!("{e:#}"))?;
    Ok((out.state, stopped))
}

/// Block syncs the node at `home` to `stop_height`; it never halts.
fn sync_node(home: &Path, archive: &Arc<Reactor>, stop_height: i64) -> nodestore::State {
    let (state, stopped) = try_sync(home, archive, stop_height)
        .unwrap_or_else(|e| panic!("block sync to {stop_height}: {e}"));
    assert!(
        !home.join("data/upgrade-info.json").exists(),
        "block sync to {stop_height} halted for an upgrade"
    );
    assert!(
        stopped.is_empty() && state.last_block_height >= stop_height,
        "sync to {stop_height} stopped at {}; peers stopped: {stopped:?}",
        state.last_block_height
    );
    state
}

/// Copies the node home `from` (closed) to `to`.
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// The consensus key of the validator at `address`: the oracles derive
/// each validator's key from its name (`GenPrivKeyFromSecret("consensus " +
/// name)`, or `GenPrivKeyFromSecret` of SHA-256("celestia-rs synthetic
/// consensus key " + name) for validators created later).
fn consensus_key(address: &[u8]) -> crypto::ed25519::PrivateKeyBytes {
    ["val0", "val1", "val2", "val3", "alice", "bob", "carol"]
        .iter()
        .flat_map(|name| {
            [
                crypto::ed25519::private_key_from_secret(format!("consensus {name}").as_bytes()),
                crypto::ed25519::private_key_from_secret(&hash::sha256(
                    format!("celestia-rs synthetic consensus key {name}").as_bytes(),
                )),
            ]
        })
        .find(|key| {
            let public = crypto::ed25519::public_key_from_private_key(key).unwrap();
            crypto::ed25519::address(&public).as_slice() == address
        })
        .unwrap_or_else(|| panic!("no consensus key for {}", hex::encode_upper(address)))
}

/// Replaces the last commit of block `h + 1` (the commit of block `h`) with
/// `commit`, and the header fields that commit to it.
fn set_commit_of(chain: &mut Chain, h: i64, commit: Commit, block_id: ctypes::BlockId) {
    let i = usize::try_from(h).unwrap();
    let next = chain.blocks[i].clone();
    let mut header = next.header;
    header.last_block_id = block_id;
    header.last_commit_hash = commit.hash();
    chain.blocks[i] = Block::new(header, next.data, next.evidence, Some(commit));
}

/// Syncs a copy of the home at 79 to 80 from an archive of `chain`, and
/// returns the height it reached and the reasons the node stopped the peer.
fn sync_tampered(
    at_79: &Path,
    genesis: &nodestore::GenesisDoc,
    chain: &Chain,
) -> (i64, Vec<String>) {
    let home = tempfile::tempdir().unwrap();
    copy_dir(at_79, home.path());
    let archive = archive_peer(genesis, chain);
    let (state, stopped) =
        try_sync(home.path(), &archive, SYNCED).unwrap_or_else(|e| panic!("tampered sync: {e}"));
    (
        state.last_block_height,
        stopped.into_iter().map(|(_, reason)| reason).collect(),
    )
}

/// The node block syncs genesis to 80 (block 81 commits it), stopping and
/// restarting after the block before and the block of each upgrade (except
/// the v11 switch, which one process crosses), and stores Go's block IDs
/// and app hashes.
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

    // No stop after block 76: one process executes 76 with the v10 release
    // and 77 with the v11 release.
    let mut stops: Vec<i64> = UPGRADES
        .iter()
        .flat_map(|u| [u - 1, *u])
        .filter(|h| *h != V11_UPGRADE - 1)
        .collect();
    stops.extend([SYNCED - 1, SYNCED]);
    let at_79 = tempfile::tempdir().unwrap();
    for stop in stops {
        let state = sync_node(home.path(), &archive, stop);
        assert_eq!(state.last_block_height, stop);
        if stop == SYNCED - 1 {
            copy_dir(home.path(), at_79.path());
        }
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
    drop((bs, ss));

    refuses_tampered_block_80(at_79.path(), &genesis, &chain);
}

/// From the node home at 79: the archive syncs it to 80, and block 80 is
/// refused when its commit holds one bad signature, and when its tx is
/// removed and its commit signed again for the new part set (the commit
/// verifies, the data root does not).
fn refuses_tampered_block_80(at_79: &Path, genesis: &nodestore::GenesisDoc, chain: &Chain) {
    // The commit of block 80 holds absent validators and a nil vote.
    let flags: Vec<u8> = chain.blocks[80]
        .last_commit
        .as_ref()
        .unwrap()
        .signatures
        .iter()
        .map(|s| s.block_id_flag)
        .collect();
    assert!(
        flags.contains(&3),
        "no nil vote in the commit of 80: {flags:?}"
    );

    // The untampered archive syncs the copy at 79 to 80.
    let (reached, stopped) = sync_tampered(at_79, genesis, chain);
    assert_eq!((reached, stopped.len()), (SYNCED, 0), "{stopped:?}");

    // One bad signature in the commit of block 80 (the first validator's,
    // which every verifier counts).
    let mut bad_sig = chain.clone();
    let mut commit = bad_sig.blocks[80].last_commit.clone().unwrap();
    commit.signatures[0].signature[0] ^= 1;
    let id = commit.block_id.clone();
    set_commit_of(&mut bad_sig, SYNCED, commit, id);
    let (reached, stopped) = sync_tampered(at_79, genesis, &bad_sig);
    assert_eq!(reached, SYNCED - 1, "a bad signature: {stopped:?}");
    assert!(
        stopped.iter().any(|r| r.contains("wrong signature (#0)")),
        "a bad signature: {stopped:?}"
    );

    // Block 80 without its tx (header, square size and data root as
    // committed), with its commit signed again for the new part set: the
    // commit verifies, the data root does not.
    let mut no_tx = chain.clone();
    let i = usize::try_from(SYNCED - 1).unwrap();
    let mut block = no_tx.blocks[i].clone();
    assert_eq!(block.data.txs.len(), 1);
    block.data.txs.clear();
    let block = Block::new(block.header, block.data, block.evidence, block.last_commit);
    let mut hashed = block.clone();
    let id = ctypes::BlockId {
        hash: hashed.hash().unwrap(),
        part_set_header: block.make_part_set(BLOCK_PART_SIZE_BYTES).unwrap().header(),
    };
    assert_eq!(id.hash, chain.blocks[80].header.last_block_id.hash);
    no_tx.blocks[i] = block;
    let mut commit = no_tx.blocks[80].last_commit.clone().unwrap();
    commit.block_id = id.clone();
    for idx in 0..commit.signatures.len() {
        if commit.signatures[idx].block_id_flag != 2 {
            continue;
        }
        let key = consensus_key(&commit.signatures[idx].validator_address);
        let bytes = commit
            .vote_sign_bytes(
                &chain.blocks[0].header.chain_id,
                i32::try_from(idx).unwrap(),
            )
            .unwrap();
        commit.signatures[idx].signature = crypto::ed25519::sign(&key, &bytes).unwrap().to_vec();
    }
    set_commit_of(&mut no_tx, SYNCED, commit, id);
    let (reached, stopped) = sync_tampered(at_79, genesis, &no_tx);
    // At app version 11 the data root is checked by ProcessProposal.
    assert_eq!(reached, SYNCED - 1, "a tampered tx: {stopped:?}");
    assert!(
        stopped
            .iter()
            .any(|r| r.contains("application has rejected syncing block")),
        "a tampered tx: {stopped:?}"
    );
}
