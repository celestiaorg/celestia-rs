//! `celestia-rs start`: a single-process node running the Rust consensus
//! engine on top of the in-process Rust state machine (ADR 0003), and
//! `celestia-rs export-blocks` for the Go verifier.
//!
//! Home layout (compatible with `celestia-appd init` / `collect-gentxs`):
//!
//! | path | content |
//! |---|---|
//! | `config/genesis.json` | genesis (SDK `AppGenesis` or CometBFT `GenesisDoc`) |
//! | `config/priv_validator_key.json` | validator key (Go format) |
//! | `data/priv_validator_state.json` | last-sign state (Go format) |
//! | `data/application.redb` | app state (ADR 0002) |
//! | `data/blockstore.redb`, `data/state.redb` | block / state stores |
//! | `data/cs.wal/wal` | consensus WAL (Go format) |
//! | `data/tx-inbox/*.tx` | raw txs to include (local mempool until P2P/RPC exist) |
//!
//! There is no P2P yet: the node is the only validator (or a validator
//! that never reaches quorum when others exist).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, anyhow};
use consensus::{ConsensusConfig, ConsensusState, Event, FileWal, SystemClock};
use ctypes::proto::abci;
use kvdb::{KvBackend, RedbBackend, RedbOptions};
use nodestore::{
    AppConn, BlockExecutor, BlockStore, GenesisDoc, Handshaker, MemEvidencePool, Mempool,
    StateStore,
};
use privval::FilePv;
use prost::Message as _;

/// The Rust app behind [`AppConn`].
struct AppAdapter(app::App);

fn app_err(e: &sdk::SdkError) -> nodestore::Error {
    nodestore::Error::new(e.log.clone())
}

impl AppConn for AppAdapter {
    fn info(&mut self) -> nodestore::Result<abci::ResponseInfo> {
        Ok(self.0.info())
    }
    fn init_chain(
        &mut self,
        req: abci::RequestInitChain,
    ) -> nodestore::Result<abci::ResponseInitChain> {
        self.0.init_chain(req).map_err(|e| app_err(&e))
    }
    fn prepare_proposal(
        &mut self,
        req: abci::RequestPrepareProposal,
    ) -> nodestore::Result<abci::ResponsePrepareProposal> {
        self.0.prepare_proposal(&req).map_err(|e| app_err(&e))
    }
    fn process_proposal(
        &mut self,
        req: abci::RequestProcessProposal,
    ) -> nodestore::Result<abci::ResponseProcessProposal> {
        self.0.process_proposal(&req).map_err(|e| app_err(&e))
    }
    fn finalize_block(
        &mut self,
        req: abci::RequestFinalizeBlock,
    ) -> nodestore::Result<abci::ResponseFinalizeBlock> {
        self.0.finalize_block(req).map_err(|e| app_err(&e))
    }
    fn commit(&mut self) -> nodestore::Result<abci::ResponseCommit> {
        self.0.commit().map_err(|e| app_err(&e))
    }
}

/// A local mempool: raw tx files in `<home>/data/tx-inbox` (sorted by
/// name), a few per block. Reaped files are removed once their block is
/// committed (included or rejected by `PrepareProposal`).
struct DirMempool {
    dir: PathBuf,
    per_block: usize,
    reaped: Vec<PathBuf>,
}

impl DirMempool {
    fn files(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(&self.dir)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "tx"))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }
}

impl Mempool for DirMempool {
    fn reap_max_bytes_max_gas(&mut self, max_bytes: i64, _: i64) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut size = 0i64;
        self.reaped.clear();
        for p in self.files().into_iter().take(self.per_block) {
            let Ok(bz) = std::fs::read(&p) else { continue };
            let n = i64::try_from(bz.len()).unwrap_or(i64::MAX);
            if max_bytes >= 0 && size + n > max_bytes {
                break;
            }
            size += n;
            out.push(bz);
            self.reaped.push(p);
        }
        out
    }

    fn update(&mut self, height: i64, txs: &[ctypes::Tx], results: &[abci::ExecTxResult]) {
        for p in self.reaped.drain(..) {
            let _ = std::fs::remove_file(&p);
        }
        for (tx, r) in txs.iter().zip(results) {
            tracing::info!(
                height,
                hash = %hex::encode_upper(tx.hash()),
                code = r.code,
                gas_used = r.gas_used,
                log = %r.log,
                "tx committed"
            );
        }
    }

    fn has_txs(&self) -> bool {
        !self.files().is_empty()
    }
}

/// Options of `celestia-rs start`.
#[derive(Debug, Clone)]
pub struct StartOptions {
    /// Node home.
    pub home: PathBuf,
    /// Stop after committing this height (0 = run forever).
    pub halt_height: i64,
    /// Txs taken from the inbox per block.
    pub txs_per_block: usize,
    /// Consensus config overrides.
    pub config: ConsensusConfig,
}

fn open_db(path: &Path) -> anyhow::Result<Arc<dyn KvBackend>> {
    Ok(Arc::new(
        RedbBackend::open(path, RedbOptions::default())
            .map_err(|e| anyhow!("open {}: {e}", path.display()))?,
    ))
}

/// Loads the genesis document of a home directory.
pub fn load_genesis(home: &Path) -> anyhow::Result<GenesisDoc> {
    let path = home.join("config").join("genesis.json");
    let bz = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    GenesisDoc::from_json(&bz, consensus::ticker::system_now())
        .map_err(|e| anyhow!("genesis {}: {e}", path.display()))
}

/// Opens the block and state stores of a home directory.
pub fn open_stores(home: &Path) -> anyhow::Result<(BlockStore, StateStore)> {
    let data = home.join("data");
    let bs = BlockStore::new(open_db(&data.join("blockstore.redb"))?)
        .map_err(|e| anyhow!("block store: {e}"))?;
    let ss = StateStore::new(open_db(&data.join("state.redb"))?);
    Ok((bs, ss))
}

/// `celestia-rs start`.
pub fn start(opts: &StartOptions) -> anyhow::Result<()> {
    let home = &opts.home;
    let genesis = load_genesis(home)?;
    // Like Go's node: wait for the genesis time.
    let now = consensus::ticker::system_now();
    if genesis.genesis_time > now {
        let wait = nodestore::state::sub_times(genesis.genesis_time, now);
        tracing::info!(seconds = wait / 1_000_000_000, "waiting for genesis time");
        std::thread::sleep(std::time::Duration::from_nanos(
            u64::try_from(wait).unwrap_or(0),
        ));
    }
    let app = app::App::open_with(
        home,
        app::AppOptions {
            chain_id: Some(genesis.chain_id.clone()),
            ..app::AppOptions::default()
        },
    )
    .map_err(|e| anyhow!("open app state: {}", e.log))?;
    let (bs, ss) = open_stores(home)?;
    let state = ss
        .load_from_db_or_genesis(&genesis)
        .map_err(|e| anyhow!("load state: {e}"))?;
    let inbox = home.join("data").join("tx-inbox");
    std::fs::create_dir_all(&inbox)?;
    let mut exec = BlockExecutor::new(
        ss.clone(),
        bs.clone(),
        Box::new(AppAdapter(app)),
        Box::new(DirMempool {
            dir: inbox,
            per_block: opts.txs_per_block,
            reaped: Vec::new(),
        }),
        Box::new(MemEvidencePool::new(ss.clone(), bs.clone())),
    );
    let mut hs = Handshaker::new(ss, state, bs, &genesis, consensus::ticker::system_now());
    let state = hs
        .handshake(&mut exec)
        .map_err(|e| anyhow!("handshake: {e}"))?;
    tracing::info!(
        height = state.last_block_height,
        app_hash = %hex::encode_upper(&state.app_hash),
        replayed = hs.n_blocks(),
        "handshake complete"
    );
    if opts.halt_height > 0 && state.last_block_height >= opts.halt_height {
        tracing::info!(height = state.last_block_height, "halt height reached");
        return Ok(());
    }
    let key_path = home.join("config").join("priv_validator_key.json");
    let state_path = home.join("data").join("priv_validator_state.json");
    let pv = if state_path.exists() {
        FilePv::load(&key_path, &state_path)
    } else {
        // `celestia-appd init` writes the initial state; create it otherwise.
        FilePv::load_empty_state(&key_path, &state_path).and_then(|pv| {
            pv.last_sign_state.save()?;
            Ok(pv)
        })
    }
    .map_err(|e| anyhow!("privval: {e}"))?;
    let wal = FileWal::open(
        home.join("data").join("cs.wal").join("wal"),
        consensus::ticker::system_now(),
    )
    .map_err(|e| anyhow!("wal: {e}"))?;
    let mut cs = ConsensusState::new(
        opts.config.clone(),
        state,
        exec,
        Box::new(wal),
        Box::new(SystemClock),
        Some(Box::new(pv)),
    )
    .map_err(|e| anyhow!("consensus: {e}"))?;
    cs.start().map_err(|e| anyhow!("consensus start: {e}"))?;
    run(&mut cs, opts.halt_height)
}

/// The node loop: fire due timeouts, report commits, stop at the halt
/// height or on a consensus failure.
fn run(cs: &mut ConsensusState, halt_height: i64) -> anyhow::Result<()> {
    loop {
        cs.tick();
        for ev in cs.take_events() {
            if let Event::NewBlock {
                height,
                block_id,
                app_hash,
            } = ev
            {
                tracing::info!(
                    height,
                    hash = %hex::encode_upper(&block_id.hash),
                    app_hash = %hex::encode_upper(&app_hash),
                    "committed block"
                );
                if halt_height > 0 && height >= halt_height {
                    tracing::info!(height, "halt height reached");
                    return Ok(());
                }
            }
        }
        if let Some(f) = cs.failure() {
            return Err(anyhow!("CONSENSUS FAILURE: {f}"));
        }
        let now = consensus::ticker::system_now();
        let wait = cs.next_timeout().map_or(100_000_000, |d| {
            nodestore::state::sub_times(d, now).clamp(0, 100_000_000)
        });
        if wait > 0 {
            std::thread::sleep(std::time::Duration::from_nanos(
                u64::try_from(wait).unwrap_or(0),
            ));
        }
    }
}

/// `celestia-rs export-blocks`: blocks, commits, validator sets, params and
/// FinalizeBlock responses of the store as JSON (hex protobufs) for
/// `conformance/consensusvectors/verify`.
pub fn export_blocks(home: &Path, out: &Path) -> anyhow::Result<()> {
    let genesis = load_genesis(home)?;
    let (bs, ss) = open_stores(home)?;
    let state = ss
        .load()
        .map_err(|e| anyhow!("{e}"))?
        .ok_or_else(|| anyhow!("no state in {}", home.display()))?;
    let mut blocks = Vec::new();
    for h in bs.base()..=bs.height() {
        let e = |x: nodestore::Error| anyhow!("height {h}: {x}");
        let meta = bs
            .load_block_meta(h)
            .map_err(e)?
            .ok_or_else(|| anyhow!("missing meta {h}"))?;
        let block = bs
            .load_block(h)
            .map_err(e)?
            .ok_or_else(|| anyhow!("missing block {h}"))?;
        let seen = bs
            .load_seen_commit(h)
            .map_err(e)?
            .map(|c| hex::encode(c.to_proto().encode_to_vec()));
        let vals = ss.load_validators(h).map_err(e)?;
        let next_vals = ss.load_validators(h + 1).map_err(e)?;
        let params = ss.load_consensus_params(h).map_err(e)?;
        let resp = ss.load_finalize_block_response(h).map_err(e)?;
        blocks.push(serde_json::json!({
            "height": h,
            "block": hex::encode(block.to_proto().map_err(|x| anyhow!("{x}"))?.encode_to_vec()),
            "block_id_hash": hex::encode(&meta.block_id.hash),
            "parts_total": meta.block_id.part_set_header.total,
            "parts_hash": hex::encode(&meta.block_id.part_set_header.hash),
            "seen_commit": seen,
            "vals": hex::encode(vals.to_proto().map_err(|x| anyhow!("{x}"))?.encode_to_vec()),
            "next_vals": hex::encode(next_vals.to_proto().map_err(|x| anyhow!("{x}"))?.encode_to_vec()),
            "params": hex::encode(params.to_proto().encode_to_vec()),
            "finalize_response": resp.map(|r| hex::encode(r.encode_to_vec())),
        }));
    }
    let doc = serde_json::json!({
        "chain_id": genesis.chain_id,
        "initial_height": genesis.initial_height,
        "last_height": state.last_block_height,
        "last_app_hash": hex::encode(&state.app_hash),
        "last_results_hash": hex::encode(&state.last_results_hash),
        "blocks": blocks,
    });
    std::fs::write(out, serde_json::to_vec_pretty(&doc)?)?;
    tracing::info!(n = bs.height() - bs.base() + 1, out = %out.display(), "exported blocks");
    Ok(())
}
