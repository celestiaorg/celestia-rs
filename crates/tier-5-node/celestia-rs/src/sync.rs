//! Catch-up wiring of `celestia-rs start` (Go `node.go`: `createEvidenceReactor`,
//! `createBlocksyncReactor`, `createStateSyncReactor`, `startStateSync`):
//! the persistent evidence pool and its reactor, the block sync reactor,
//! and the state sync → block sync → consensus switch-over.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::anyhow;
use ctypes::proto::abci;
use nodestore::{BlockExecutor, BlockStore, State, StateStore};
use p2p::Switch;
use statesync::Bootstrapper as _;

/// `[statesync]` options.
#[derive(Debug, Clone)]
pub struct StateSyncOptions {
    /// RPC servers for the light client (at least two; the first is the
    /// primary).
    pub rpc_servers: Vec<String>,
    /// Trusted height.
    pub trust_height: i64,
    /// Trusted header hash.
    pub trust_hash: Vec<u8>,
    /// Trusting period (ns).
    pub trust_period: i64,
    /// How long to discover snapshots before choosing one.
    pub discovery_time: Duration,
}

/// The catch-up reactors registered on the switch.
pub struct SyncReactors {
    /// Block sync (0x40).
    pub blocksync: Arc<blocksync::Reactor>,
    /// Evidence (0x38).
    pub evidence: Arc<evidence::Reactor>,
    /// State sync (0x60/0x61).
    pub statesync: Arc<statesync::Reactor>,
}

/// Opens the evidence pool on `data/evidence` (Go `evidence.db`) on the
/// node's database backend.
pub fn open_evidence_pool(
    home: &Path,
    ss: &StateStore,
    bs: &BlockStore,
    state: &State,
) -> anyhow::Result<evidence::Pool> {
    let db = crate::node::open_db(&home.join("data"), "evidence")?;
    let st = match ss.load().map_err(|e| anyhow!("{e}"))? {
        Some(s) => s,
        None => state.clone(),
    };
    evidence::Pool::with_state(db, ss.clone(), bs.clone(), st)
        .map_err(|e| anyhow!("evidence pool: {e}"))
}

/// The app's snapshot connection for state sync.
pub struct SnapshotConn(pub Arc<Mutex<app::App>>);

impl SnapshotConn {
    fn app(&self) -> std::sync::MutexGuard<'_, app::App> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl statesync::AppConn for SnapshotConn {
    fn list_snapshots(&self) -> Result<abci::ResponseListSnapshots, String> {
        Ok(self.app().list_snapshots())
    }
    fn load_snapshot_chunk(
        &self,
        req: abci::RequestLoadSnapshotChunk,
    ) -> Result<abci::ResponseLoadSnapshotChunk, String> {
        Ok(self.app().load_snapshot_chunk(&req))
    }
    fn offer_snapshot(
        &self,
        req: abci::RequestOfferSnapshot,
    ) -> Result<abci::ResponseOfferSnapshot, String> {
        Ok(self.app().offer_snapshot(req))
    }
    fn apply_snapshot_chunk(
        &self,
        req: abci::RequestApplySnapshotChunk,
    ) -> Result<abci::ResponseApplySnapshotChunk, String> {
        Ok(self.app().apply_snapshot_chunk(req))
    }
    fn info(&self, _req: abci::RequestInfo) -> Result<abci::ResponseInfo, String> {
        Ok(self.app().info())
    }
}

/// Creates the block sync, evidence and state sync reactors and adds them
/// to the switch.
#[allow(clippy::too_many_arguments)]
pub fn add_sync_reactors(
    sw: &Switch,
    home: &Path,
    state: &State,
    bs: &BlockStore,
    ss: &StateStore,
    evpool: &evidence::Pool,
    app: &Arc<Mutex<app::App>>,
    local_addr: Option<Vec<u8>>,
) -> anyhow::Result<SyncReactors> {
    let bcr = Arc::new(
        blocksync::Reactor::new(
            state,
            bs.clone(),
            ss.clone(),
            0,
            blocksync::ReactorOptions {
                local_addr,
                ..blocksync::ReactorOptions::default()
            },
        )
        .map_err(|e| anyhow!("{e}"))?,
    );
    let evr = Arc::new(evidence::Reactor::new(evpool.clone()));
    let ssr = Arc::new(statesync::Reactor::new(
        statesync::Config {
            temp_dir: home.join("data").join("statesync-tmp"),
            ..statesync::Config::default()
        },
        Arc::new(SnapshotConn(Arc::clone(app))),
    ));
    sw.add_reactor(Arc::clone(&bcr) as Arc<dyn p2p::Reactor>)
        .map_err(|e| anyhow!("{e}"))?;
    sw.add_reactor(Arc::clone(&evr) as Arc<dyn p2p::Reactor>)
        .map_err(|e| anyhow!("{e}"))?;
    sw.add_reactor(Arc::clone(&ssr) as Arc<dyn p2p::Reactor>)
        .map_err(|e| anyhow!("{e}"))?;
    Ok(SyncReactors {
        blocksync: bcr,
        evidence: evr,
        statesync: ssr,
    })
}

/// `startStateSync`: restores a snapshot from peers (verified with the
/// light client against `opts.rpc_servers`), bootstraps the state and
/// block stores and returns the state to block sync from.
pub fn state_sync(
    reactor: &statesync::Reactor,
    opts: &StateSyncOptions,
    genesis_state: &State,
    ss: &StateStore,
    bs: &BlockStore,
) -> anyhow::Result<State> {
    let trust = light::TrustOptions {
        period: opts.trust_period,
        height: opts.trust_height,
        hash: opts.trust_hash.clone(),
    };
    let sp = statesync::LightClientStateProvider::new(
        &genesis_state.chain_id,
        genesis_state.version,
        genesis_state.initial_height,
        &opts.rpc_servers,
        &trust,
    )
    .map_err(|e| anyhow!("failed to set up light client state provider: {e}"))?;
    tracing::info!(
        trust_height = opts.trust_height,
        servers = ?opts.rpc_servers,
        "starting state sync"
    );
    let (state, commit) = reactor
        .sync(Arc::new(sp), opts.discovery_time)
        .map_err(|e| anyhow!("State sync failed: {e}"))?;
    statesync::NodeStores {
        state_store: ss.clone(),
        block_store: bs.clone(),
    }
    .bootstrap(&state, &commit)
    .map_err(|e| anyhow!("{e}"))?;
    tracing::info!(
        height = state.last_block_height,
        app_hash = %hex::encode_upper(&state.app_hash),
        "state sync complete; switching to block sync"
    );
    Ok(state)
}

/// `poolRoutine`: block sync until caught up (`switch_to_consensus`) or,
/// without consensus networking, forever (following the chain by block
/// sync) until `halt_height`. Logs every applied block.
pub fn block_sync(
    reactor: &blocksync::Reactor,
    state: State,
    exec: &mut BlockExecutor,
    state_synced: bool,
    switch_to_consensus: bool,
    halt_height: i64,
) -> anyhow::Result<blocksync::SyncOutcome> {
    let mut last = state.last_block_height;
    tracing::info!(height = last, switch_to_consensus, "starting block sync");
    let mut stop = |s: &State| {
        if s.last_block_height != last {
            last = s.last_block_height;
            tracing::info!(
                height = s.last_block_height,
                hash = %hex::encode_upper(&s.last_block_id.hash),
                app_hash = %hex::encode_upper(&s.app_hash),
                "synced block"
            );
        }
        halt_height > 0 && s.last_block_height >= halt_height
    };
    reactor
        .run_sync(state, exec, state_synced, switch_to_consensus, &mut stop)
        .map_err(|e| anyhow!("block sync: {e}"))
}
