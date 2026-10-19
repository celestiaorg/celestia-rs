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

/// Stops `reactor`'s state sync when `shutdown` triggers (SIGTERM /
/// SIGINT), so snapshot discovery does not outlive the signal.
pub fn stop_state_sync_on_shutdown(
    reactor: &Arc<statesync::Reactor>,
    shutdown: &crate::ops::Shutdown,
) {
    let r = Arc::clone(reactor);
    shutdown.on_trigger(move || r.stop_sync());
}

/// `startStateSync`: restores a snapshot from peers (verified with the
/// light client against `opts.rpc_servers`), bootstraps the state and
/// block stores and returns the state to block sync from (`None` when
/// shutdown stopped it).
pub fn state_sync(
    reactor: &Arc<statesync::Reactor>,
    opts: &StateSyncOptions,
    genesis_state: &State,
    ss: &StateStore,
    bs: &BlockStore,
    shutdown: &crate::ops::Shutdown,
) -> anyhow::Result<Option<State>> {
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
    stop_state_sync_on_shutdown(reactor, shutdown);
    let (state, commit) = match reactor.sync(Arc::new(sp), opts.discovery_time) {
        Ok(r) => r,
        Err(statesync::SyncError::Stopped) => {
            tracing::info!("state sync stopped");
            return Ok(None);
        }
        Err(e) => return Err(anyhow!("State sync failed: {e}")),
    };
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
    Ok(Some(state))
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
    stop_height: i64,
    shutdown: &crate::ops::Shutdown,
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
        shutdown.is_triggered() || (stop_height > 0 && s.last_block_height >= stop_height)
    };
    reactor
        .run_sync(state, exec, state_synced, switch_to_consensus, &mut stop)
        .map_err(|e| anyhow!("block sync: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoApp;

    impl statesync::AppConn for NoApp {
        fn list_snapshots(&self) -> Result<abci::ResponseListSnapshots, String> {
            Ok(abci::ResponseListSnapshots::default())
        }
        fn load_snapshot_chunk(
            &self,
            _: abci::RequestLoadSnapshotChunk,
        ) -> Result<abci::ResponseLoadSnapshotChunk, String> {
            Err("unused".into())
        }
        fn offer_snapshot(
            &self,
            _: abci::RequestOfferSnapshot,
        ) -> Result<abci::ResponseOfferSnapshot, String> {
            Err("unused".into())
        }
        fn apply_snapshot_chunk(
            &self,
            _: abci::RequestApplySnapshotChunk,
        ) -> Result<abci::ResponseApplySnapshotChunk, String> {
            Err("unused".into())
        }
        fn info(&self, _: abci::RequestInfo) -> Result<abci::ResponseInfo, String> {
            Err("unused".into())
        }
    }

    struct NoLight;

    impl statesync::StateProvider for NoLight {
        fn app_hash(&self, _: u64) -> light::Result<Vec<u8>> {
            Err(light::Error::NoWitnesses)
        }
        fn commit(&self, _: u64) -> light::Result<ctypes::Commit> {
            Err(light::Error::NoWitnesses)
        }
        fn state(&self, _: u64) -> light::Result<statesync::State> {
            Err(light::Error::NoWitnesses)
        }
    }

    /// SIGTERM / SIGINT (a triggered [`crate::ops::Shutdown`]) ends a state
    /// sync that is still discovering snapshots.
    #[test]
    fn shutdown_stops_state_sync_discovery() {
        let tmp = tempfile::tempdir().unwrap();
        let reactor = Arc::new(statesync::Reactor::new(
            statesync::Config {
                temp_dir: tmp.path().to_path_buf(),
                ..statesync::Config::default()
            },
            Arc::new(NoApp),
        ));
        let shutdown = crate::ops::Shutdown::new();
        stop_state_sync_on_shutdown(&reactor, &shutdown);
        let r = Arc::clone(&reactor);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(r.sync(Arc::new(NoLight), Duration::from_secs(60)));
        });
        std::thread::sleep(Duration::from_millis(300));
        shutdown.trigger();
        let got = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("state sync ignored shutdown");
        assert_eq!(got.err(), Some(statesync::SyncError::Stopped));
    }
}
