//! celestia-app v10 state machine.
//!
//! [`App`] mirrors `baseapp.BaseApp` + `app.App`: it owns the multistore,
//! drives InitChain → (FinalizeBlock → Commit)* and dispatches modules in
//! Go's exact order lists ([`modules`]). [`AbciApp`] adapts it to the ABCI
//! socket server.

pub mod check_tx;
pub mod comet;
pub mod export;
pub mod fibre_tx;
pub mod genutil;
pub mod hyperlane;
pub mod ibc;
pub mod inclusion_proof;
pub mod modules;
pub mod msg_count;
pub mod node_services;
pub mod proposal;
pub mod query;
pub use multistore::PruningOptions;
pub mod query_services;
pub mod router;
pub mod stores;
pub mod tx;
pub mod txpipe;
pub mod upgrades;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use abci_server::Application;
use celestia_proto::celestia_core_proto::abci::{
    self, ResponseCommit, ResponseException, ResponseFinalizeBlock, ResponseInfo,
    ResponseInitChain, TimeoutInfo, request, response,
};
use celestia_proto::celestia_core_proto::tendermint::types::{ConsensusParams, VersionParams};
use multistore::kvdb::{self, DbOptions, KvBackend};
use pbjson_types::Duration;
use sdk::{Context, ExecMode, Header, MultiStore, Result, SdkError};

/// `baseapp.InitialAppVersion` is 0; celestia genesis files carry the app
/// version explicitly in consensus params.
pub const INITIAL_APP_VERSION: u64 = 0;

/// The application.
#[derive(Debug)]
pub struct App {
    ms: MultiStore,
    chain_id: String,
    initial_height: i64,
    /// Header of the block being finalized; `Some` between InitChain (or
    /// FinalizeBlock) and Commit.
    finalize_header: Option<Header>,
    last_commit_hash: Vec<u8>,
    last_height: i64,
    /// Node-local `x/upgrade` keeper state.
    upgrade: x_upgrade::NodeState,
    /// State-sync snapshots (storage-backed apps only).
    snapshots: Option<snapshots::Manager>,
    /// Header of the check state (`setState(execModeCheck, ...)` at
    /// InitChain and Commit).
    check_header: Header,
    /// The block being finalized changed the app version: its `Commit` is
    /// where the multiplexer switches to the next binary, a fresh process
    /// (`BaseApp.Init`: check state with an empty header).
    switch_on_commit: bool,
    /// Node-local `minimum-gas-prices` (utia) for CheckTx; `None` (the v10
    /// default `""`) means `appconsts.DefaultMinGasPrice`.
    node_min_gas_price: Option<math::LegacyDec>,
    /// Node-local block retention, halt and upgrade-info settings.
    node: NodeOptions,
    /// The square of this node's last PrepareProposal and its data root,
    /// reused by ProcessProposal of the same square.
    proposed: proposal::ProposedSquare,
}

/// Node-local settings of `app.toml` that do not affect consensus state.
#[derive(Debug, Clone, Default)]
pub struct NodeOptions {
    /// `min-retain-blocks`: `ResponseCommit.retain_height` keeps at least
    /// this many blocks (0 = no block pruning).
    pub min_retain_blocks: u64,
    /// `halt-height`: FinalizeBlock of this height (and above) fails with
    /// Go's "halt per configuration" error.
    pub halt_height: u64,
    /// `halt-time` (Unix seconds): same for blocks at or after this time.
    pub halt_time: u64,
    /// Node home: `x/upgrade` writes `data/upgrade-info.json` there when an
    /// upgrade without a handler is due (cosmovisor's trigger).
    pub home: Option<PathBuf>,
}

/// Go's `checkHalt` error prefix.
pub const HALT_ERROR_PREFIX: &str = "halt per configuration";

/// Whether a FinalizeBlock error is the planned stop of an upgrade
/// without a handler in this binary (`UPGRADE "x" NEEDED at height: …`).
#[must_use]
pub fn is_upgrade_needed(log: &str) -> bool {
    log.contains("UPGRADE \"") && log.contains("NEEDED at height")
}

/// `Keeper.DumpUpgradeInfoToDisk`: `<home>/data/upgrade-info.json` with
/// Go's `encoding/json` of the plan (`name`, `time`, `height`, `info`).
pub fn dump_upgrade_info(
    home: &Path,
    height: i64,
    plan: &celestia_proto::cosmos::upgrade::v1beta1::Plan,
) -> std::io::Result<()> {
    let dir = home.join("data");
    std::fs::create_dir_all(&dir)?;
    let mut v = serde_json::Map::new();
    if !plan.name.is_empty() {
        v.insert("name".into(), serde_json::Value::String(plan.name.clone()));
    }
    v.insert(
        "time".into(),
        serde_json::Value::String("0001-01-01T00:00:00Z".into()),
    );
    if height != 0 {
        v.insert("height".into(), serde_json::Value::from(height));
    }
    if !plan.info.is_empty() {
        v.insert("info".into(), serde_json::Value::String(plan.info.clone()));
    }
    // Go marshals struct fields in declaration order.
    let order = ["name", "time", "height", "info"];
    let body = order
        .iter()
        .filter_map(|k| {
            v.get(*k)
                .map(|x| format!("{}:{x}", serde_json::Value::String((*k).to_owned())))
        })
        .collect::<Vec<_>>()
        .join(",");
    std::fs::write(dir.join("upgrade-info.json"), format!("{{{body}}}"))
}

/// Metadata key of the chain ID in the state database.
const CHAIN_ID_KEY: &[u8] = b"chain_id";

/// Options of a storage-backed [`App`] ([`App::open_with`]).
#[derive(Debug, Clone, Default)]
pub struct AppOptions {
    /// Multistore pruning and node-cache size.
    pub store: multistore::StoreOptions,
    /// Database backend, cache size and durability.
    pub db: DbOptions,
    /// Snapshot interval / retention (`interval == 0` disables snapshots).
    pub snapshots: snapshots::SnapshotOptions,
    /// Chain ID to use when the database does not know it yet (a node
    /// bootstrapped by state sync never sees InitChain).
    pub chain_id: Option<String>,
    /// Node-local settings (block retention, halt, upgrade info).
    pub node: NodeOptions,
}

/// Directory holding the state database of a node home directory.
#[must_use]
pub fn data_dir(home: &Path) -> PathBuf {
    home.join("data")
}

/// Name of the application state database inside [`data_dir`].
pub const STATE_DB_NAME: &str = "application";

/// State database of a node home directory for `backend`
/// (`<home>/data/application.rocksdb/` or `<home>/data/application.redb`).
#[must_use]
pub fn state_db_path(home: &Path, backend: kvdb::BackendKind) -> PathBuf {
    backend.db_path(&data_dir(home), STATE_DB_NAME)
}

/// Snapshot directory of a node home directory.
#[must_use]
pub fn snapshot_dir(home: &Path) -> PathBuf {
    home.join("data").join("snapshots")
}

fn store_err(e: impl std::fmt::Display) -> SdkError {
    sdk::error::sdk::logic(format!("store: {e}"))
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

fn dur(seconds: i64, nanos: i32) -> Option<Duration> {
    Some(Duration { seconds, nanos })
}

/// `app.TimeoutInfo()` with default config values.
#[must_use]
pub fn timeout_info() -> TimeoutInfo {
    TimeoutInfo {
        timeout_propose: dur(3, 0),
        timeout_propose_delta: dur(0, 500_000_000),
        timeout_prevote: dur(2, 0),
        timeout_prevote_delta: dur(0, 500_000_000),
        timeout_precommit: dur(3, 0),
        timeout_precommit_delta: dur(0, 500_000_000),
        timeout_commit: dur(0, 500_000_000),
        delayed_precommit_timeout: dur(2, 100_000_000),
    }
}

impl App {
    /// Creates an app with all v10 stores mounted.
    ///
    /// # Panics
    /// Never: store names are static and unique.
    #[must_use]
    pub fn new() -> Self {
        let mut ms = MultiStore::new();
        for (name, kind) in stores::all() {
            #[allow(clippy::expect_used)]
            ms.mount(name, kind).expect("static store list is valid");
        }
        Self::from_multistore(ms)
    }

    fn from_multistore(ms: MultiStore) -> Self {
        modules::install_staking_hooks();
        Self {
            ms,
            chain_id: String::new(),
            initial_height: 1,
            finalize_header: None,
            last_commit_hash: sdk_empty_hash(),
            last_height: 0,
            upgrade: x_upgrade::NodeState::default(),
            snapshots: None,
            check_header: Header::default(),
            switch_on_commit: false,
            node_min_gas_price: None,
            node: NodeOptions::default(),
            proposed: proposal::ProposedSquare::default(),
        }
    }

    /// Sets the node-local options (`AppOptions::node`).
    pub fn set_node_options(&mut self, node: NodeOptions) {
        self.node = node;
    }

    /// Go `CommitMultiStore().RollbackToVersion(height)` (`rollback`): the
    /// committed state at `height` becomes the latest.
    pub fn rollback_to(&mut self, height: i64) -> Result<()> {
        self.ms
            .root_mut()
            .rollback_to_version(height)
            .map_err(store_err)?;
        self.finalize_header = None;
        self.load_committed_state()
    }

    /// `BaseApp.GetBlockRetentionHeight(commitHeight)`.
    #[must_use]
    pub fn block_retention_height(&self, commit_height: i64) -> i64 {
        let min_retain = i64::try_from(self.node.min_retain_blocks).unwrap_or(i64::MAX);
        if min_retain == 0 {
            return 0;
        }
        let min_non_zero = |x: i64, y: i64| match (x, y) {
            (0, y) => y,
            (x, 0) => x,
            (x, y) => x.min(y),
        };
        let mut retention = 0;
        if let Some(m) = &self.snapshots {
            let o = m.options();
            let snapshot_heights =
                i64::try_from(o.interval.saturating_mul(u64::from(o.keep_recent)))
                    .unwrap_or(i64::MAX);
            if snapshot_heights > 0 {
                retention = min_non_zero(retention, commit_height - snapshot_heights);
            }
        }
        retention = min_non_zero(retention, commit_height - min_retain);
        retention.max(0)
    }

    /// Sets the node's minimum gas price (utia) used by CheckTx.
    pub fn set_node_min_gas_price(&mut self, price: Option<math::LegacyDec>) {
        self.node_min_gas_price = price;
    }

    /// The node's configured minimum gas price (utia), if any.
    #[must_use]
    pub fn node_min_gas_price(&self) -> Option<&math::LegacyDec> {
        self.node_min_gas_price.as_ref()
    }

    /// Opens (or creates) the persistent state of node home `home`
    /// ([`state_db_path`]) with default options and resumes
    /// from the last committed height.
    pub fn open(home: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(home, AppOptions::default())
    }

    /// [`Self::open`] with explicit options.
    pub fn open_with(home: impl AsRef<Path>, options: AppOptions) -> Result<Self> {
        let home = home.as_ref();
        let db =
            kvdb::open_backend(&data_dir(home), STATE_DB_NAME, &options.db).map_err(store_err)?;
        let mut app = Self::open_backend(db, &options)?;
        app.node = options.node.clone();
        app.snapshots = Some(
            snapshots::Manager::new(snapshot_dir(home), options.snapshots).map_err(store_err)?,
        );
        Ok(app)
    }

    /// Opens an app over an arbitrary storage backend (tests).
    pub fn open_backend(db: Arc<dyn KvBackend>, options: &AppOptions) -> Result<Self> {
        let mut ms = MultiStore::open(db, options.store);
        for (name, kind) in stores::all() {
            ms.mount(name, kind).map_err(store_err)?;
        }
        ms.load_latest_version().map_err(store_err)?;
        // A state committed at app version 9 has no fibre / valaddr stores
        // (they are added by the v10 upgrade, ADR 0004).
        let missing: Vec<&'static str> = ms
            .root()
            .last_commit_info()
            .map(|info| {
                stores::ADDED_IN_V10
                    .iter()
                    .copied()
                    .filter(|s| info.store_info(s).is_none())
                    .collect()
            })
            .unwrap_or_default();
        for name in missing {
            ms.unmount_iavl(name).map_err(store_err)?;
        }
        let mut app = Self::from_multistore(ms);
        app.load_committed_state()?;
        app.mount_upgrade_stores()?;
        if app.chain_id.is_empty()
            && let Some(chain_id) = &options.chain_id
        {
            app.chain_id.clone_from(chain_id);
            app.ms
                .root_mut()
                .set_metadata(CHAIN_ID_KEY, chain_id.clone().into_bytes());
        }
        Ok(app)
    }

    /// Refreshes height, app hash and chain ID from the committed store.
    fn load_committed_state(&mut self) -> Result<()> {
        let id = self.ms.root().last_commit_id().map_err(store_err)?;
        self.last_height = id.version;
        self.last_commit_hash = if id.version == 0 {
            sdk_empty_hash()
        } else {
            id.hash.to_vec()
        };
        if let Some(chain_id) = self
            .ms
            .root()
            .get_metadata(CHAIN_ID_KEY)
            .map_err(store_err)?
        {
            self.chain_id = String::from_utf8_lossy(&chain_id).into_owned();
        }
        self.ms.reset_check();
        self.check_header = self.init_check_header();
        Ok(())
    }

    /// `BaseApp.Init`: `setState(execModeCheck, Header{ChainID})`, height 0
    /// and Go's zero `time.Time` (0001-01-01, security.md 70).
    fn init_check_header(&self) -> Header {
        Header {
            chain_id: self.chain_id.clone(),
            time: pbjson_types::Timestamp {
                seconds: -62_135_596_800,
                nanos: 0,
            },
            ..Header::default()
        }
    }

    /// The snapshot manager of a storage-backed app.
    #[must_use]
    pub fn snapshot_manager(&self) -> Option<&snapshots::Manager> {
        self.snapshots.as_ref()
    }

    /// Mutable access to the snapshot manager (tests).
    pub fn snapshot_manager_mut(&mut self) -> Option<&mut snapshots::Manager> {
        self.snapshots.as_mut()
    }

    /// Chain ID (from InitChain, the database or [`AppOptions::chain_id`]).
    #[must_use]
    pub fn chain_id(&self) -> &str {
        &self.chain_id
    }

    /// Read access to the multistore (tests, diagnostics).
    #[must_use]
    pub fn multistore(&self) -> &MultiStore {
        &self.ms
    }

    /// `Info`.
    #[must_use]
    pub fn info(&self) -> ResponseInfo {
        ResponseInfo {
            data: "celestia-app".into(),
            version: String::new(),
            app_version: self.app_version(),
            last_block_height: self.last_height,
            last_block_app_hash: self.last_commit_hash.clone(),
            timeout_info: Some(timeout_info()),
        }
    }

    fn app_version(&self) -> u64 {
        if self.last_height == 0 {
            return INITIAL_APP_VERSION;
        }
        let mut ms_view = self
            .ms
            .root()
            .get_store(x_consensus::STORE_KEY)
            .ok()
            .and_then(|s| s.get(x_consensus::PARAMS_KEY).ok().flatten());
        ms_view
            .take()
            .and_then(|b| <ConsensusParams as prost::Message>::decode(b.as_slice()).ok())
            .and_then(|cp| cp.version)
            .map_or(INITIAL_APP_VERSION, |v| v.app)
    }

    /// `InitChain`.
    pub fn init_chain(&mut self, req: abci::RequestInitChain) -> Result<ResponseInitChain> {
        if req.initial_height > 1 {
            return Err(sdk::error::sdk::not_supported("initial_height > 1"));
        }
        self.chain_id.clone_from(&req.chain_id);
        self.ms
            .root_mut()
            .set_metadata(CHAIN_ID_KEY, req.chain_id.clone().into_bytes());
        self.initial_height = req.initial_height.max(1);
        let header = Header {
            chain_id: req.chain_id.clone(),
            height: 0,
            time: req.time.unwrap_or_default(),
            ..Header::default()
        };
        self.check_header = header.clone();
        let genesis_app_version = req
            .consensus_params
            .as_ref()
            .and_then(|cp| cp.version.as_ref())
            .map_or(INITIAL_APP_VERSION, |v| v.app);
        sdk::version::set_app_version(genesis_app_version);
        // celestia-app v9.0.8 has no fibre / valaddr stores (ADR 0004).
        if stores::is_v9(genesis_app_version) {
            for name in stores::ADDED_IN_V10 {
                if self.ms.is_mounted(name) {
                    self.ms.unmount_iavl(name).map_err(store_err)?;
                }
            }
        }
        let mut ctx = Context::new(&mut self.ms, header.clone(), ExecMode::Finalize);

        if let Some(mut cp) = req.consensus_params.clone() {
            if cp.version.as_ref().is_none_or(|v| v.app == 0) {
                cp.version = Some(VersionParams {
                    app: INITIAL_APP_VERSION,
                });
            }
            x_consensus::set_params(&mut ctx, &cp)?;
            ctx.consensus_params = cp;
        }

        // InitChainer.
        let genesis: serde_json::Value = serde_json::from_slice(&req.app_state_bytes)
            .map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))?;
        let app_state = &genesis;
        x_upgrade::set_module_version_map(
            &mut ctx,
            modules::VERSION_MAP
                .iter()
                .copied()
                .filter(|(m, _)| stores::exists_at(m, genesis_app_version)),
        )?;
        let mut validators = Vec::new();
        for module in modules::INIT_GENESIS {
            if !stores::exists_at(module, genesis_app_version) {
                continue;
            }
            let updates = modules::init_genesis(&mut ctx, module, &app_state[*module])?;
            if !updates.is_empty() {
                if !validators.is_empty() {
                    return Err(sdk::error::sdk::logic(
                        "validator InitGenesis updates already set by a previous module",
                    ));
                }
                validators = updates;
            }
        }
        drop(ctx);

        self.finalize_header = Some(Header {
            height: self.initial_height,
            ..header
        });
        Ok(ResponseInitChain {
            consensus_params: None,
            validators,
            app_hash: self.last_commit_hash.clone(),
            timeout_info: Some(timeout_info()),
        })
    }

    /// `FinalizeBlock` (no optimistic execution).
    pub fn finalize_block(
        &mut self,
        req: abci::RequestFinalizeBlock,
    ) -> Result<ResponseFinalizeBlock> {
        self.check_halt(&req)?;
        let header = Header {
            chain_id: self.chain_id.clone(),
            height: req.height,
            time: req.time.unwrap_or_default(),
            proposer_address: req.proposer_address.clone(),
            hash: req.hash.clone(),
            app_hash: self.last_commit_hash.clone(),
            next_validators_hash: req.next_validators_hash.clone(),
        };
        let mut ctx = Context::new(&mut self.ms, header.clone(), ExecMode::Finalize);
        ctx.consensus_params = x_consensus::get_params(&mut ctx)?.unwrap_or_default();
        sdk::version::set_app_version(ctx.consensus_params.version.as_ref().map_or(0, |v| v.app));
        ctx.vote_infos = req
            .decided_last_commit
            .as_ref()
            .map(|c| c.votes.clone())
            .unwrap_or_default();
        ctx.misbehavior.clone_from(&req.misbehavior);

        let mut params_changed = false;
        for module in modules::PRE_BLOCKERS {
            match modules::pre_block(&mut ctx, module, &mut self.upgrade) {
                Ok(changed) => params_changed |= changed,
                Err(e) => {
                    if is_upgrade_needed(&e.log) {
                        if let (Some(home), Ok(Some(plan))) =
                            (&self.node.home, x_upgrade::get_upgrade_plan(&mut ctx))
                            && let Err(io) = dump_upgrade_info(home, req.height, &plan)
                        {
                            return Err(sdk::error::sdk::logic(format!(
                                "unable to write upgrade info to filesystem: {io}"
                            )));
                        }
                        tracing::error!("{}", e.log);
                    }
                    return Err(e);
                }
            }
        }
        if params_changed {
            ctx.consensus_params = x_consensus::get_params(&mut ctx)?.unwrap_or_default();
        }
        // PreBlocker events are reported as emitted (own event manager, not
        // marked for indexing); BeginBlock / EndBlock events get Go's `mode`
        // attribute (`baseapp.beginBlock` / `endBlock`).
        let mut events = ctx.events.take();
        for e in &mut events {
            for a in &mut e.attributes {
                a.index = false;
            }
        }
        let app_version = ctx.consensus_params.version.as_ref().map_or(0, |v| v.app);
        for module in modules::BEGIN_BLOCKERS {
            if stores::exists_at(module, app_version) {
                modules::begin_block(&mut ctx, module)?;
            }
        }
        let mut begin = ctx.events.take();
        with_mode(&mut begin, "BeginBlock");
        events.extend(begin);

        // cosmos-sdk v0.52.8 (celestia-app v9) delivers each tx in turn;
        // v0.52.12 (v10) runs every ante handler before any message.
        let tx_results = if stores::is_v9(app_version) {
            execute_txs_sequential(&mut ctx, &req.txs)
        } else {
            execute_txs_phased(&mut ctx, &req.txs)
        };

        let mut validator_updates = Vec::new();
        for module in modules::END_BLOCKERS {
            if !stores::exists_at(module, app_version) {
                continue;
            }
            let updates = modules::end_block(&mut ctx, module)?;
            if !updates.is_empty() {
                validator_updates = updates;
            }
        }
        modules::app_end_block(&mut ctx)?;
        let mut end = ctx.events.take();
        with_mode(&mut end, "EndBlock");
        events.extend(end);
        let cp = x_consensus::get_params(&mut ctx)?;
        drop(ctx);
        self.switch_on_commit = cp
            .as_ref()
            .and_then(|c| c.version.as_ref())
            .is_some_and(|v| v.app != app_version);

        self.ms.flush()?;
        let app_hash = self.ms.working_hash()?.to_vec();
        self.finalize_header = Some(header);
        Ok(ResponseFinalizeBlock {
            events,
            tx_results,
            validator_updates,
            consensus_param_updates: cp,
            app_hash,
            timeout_info: Some(timeout_info()),
        })
    }

    /// `QuerySequence` (celestia-core mempool): the signer's sequence in
    /// check state, 0 for unknown accounts.
    pub fn query_sequence(&mut self, signer: &[u8]) -> abci::ResponseQuerySequence {
        let header = self.finalize_header.clone().unwrap_or_default();
        let mut ctx = Context::new(&mut self.ms, header, ExecMode::Check);
        let sequence = x_auth::get_account(&mut ctx, signer)
            .ok()
            .flatten()
            .map_or(0, |a| a.sequence());
        abci::ResponseQuerySequence { sequence }
    }

    /// `Commit`.
    pub fn commit(&mut self) -> Result<ResponseCommit> {
        let header = self
            .finalize_header
            .take()
            .ok_or_else(|| sdk::error::sdk::logic("commit without finalized block"))?;
        // The height's proposals are decided: free the cached square.
        self.proposed = proposal::ProposedSquare::default();
        // InitChain writes are flushed with the first block; flushing again
        // here is a no-op for already-flushed state.
        self.ms.flush()?;
        self.ms
            .root_mut()
            .set_commit_timestamp(multistore_timestamp(&header.time));
        let id = self.ms.commit()?;
        // setState(execModeCheck): mempool validation restarts from the new state.
        self.ms.reset_check();
        self.last_commit_hash = id.hash.to_vec();
        self.last_height = id.version;
        self.check_header = header;
        if std::mem::take(&mut self.switch_on_commit) {
            // The multiplexer stops this binary after the commit and opens
            // the next app version's (`multiplexer/abci` `Commit`): its
            // `LoadLatestVersion` / `Init` sets the check state (CheckTx,
            // queries) to `Header{ChainID}`, height 0 and zero time, until
            // the next commit (ADR 0004; security.md 70).
            self.check_header = self.init_check_header();
            // The next binary also starts with empty memory stores: until
            // its first BeginBlock (`x/capability` `InitMemStore`) CheckTx
            // finds no capability (IBC packet txs fail with `capability
            // not found`), so comet's recheck drops them.
            self.ms.clear_memory_stores();
        }
        self.mount_upgrade_stores()?;
        self.snapshot_if_applicable();
        Ok(ResponseCommit {
            retain_height: self.block_retention_height(id.version),
        })
    }

    /// The v10 store loader (`UpgradeStoreLoader(H+1, Added: fibre,
    /// valaddr)`): once the committed app version is 10 or more, the stores
    /// v9 lacks are mounted empty with their first version at the next
    /// height. In Go the multiplexer opens the v10 app right after the
    /// commit that raised the app version, and that app's loader adds them.
    fn mount_upgrade_stores(&mut self) -> Result<()> {
        let version = self.app_version();
        if stores::is_v9(version) || version == INITIAL_APP_VERSION {
            return Ok(());
        }
        for name in stores::ADDED_IN_V10 {
            if !self.ms.is_mounted(name) {
                tracing::info!(store = name, height = self.last_height + 1, "adding store");
                self.ms
                    .mount_iavl_at(name, self.last_height + 1)
                    .map_err(store_err)?;
            }
        }
        Ok(())
    }

    /// Go `checkHalt`.
    fn check_halt(&self, req: &abci::RequestFinalizeBlock) -> Result<()> {
        let NodeOptions {
            halt_height,
            halt_time,
            ..
        } = self.node;
        let height_hit = halt_height > 0 && u64::try_from(req.height).unwrap_or(0) >= halt_height;
        let time_hit = halt_time > 0
            && req
                .time
                .as_ref()
                .is_some_and(|t| t.seconds >= i64::try_from(halt_time).unwrap_or(i64::MAX));
        if height_hit || time_hit {
            return Err(sdk::error::sdk::logic(format!(
                "{HALT_ERROR_PREFIX} height {halt_height} time {halt_time}"
            )));
        }
        Ok(())
    }

    /// `SnapshotIfApplicable` (cosmos-sdk `baseapp/abci.go` Commit →
    /// `store/snapshots/manager.go`): the snapshot is written on its own
    /// thread from a pinned view of the committed height, so Commit returns
    /// at once; a failure is logged and never halts the chain.
    fn snapshot_if_applicable(&mut self) {
        let Some(manager) = &mut self.snapshots else {
            return;
        };
        let Ok(height) = u64::try_from(self.last_height) else {
            return;
        };
        if !manager.should_snapshot(height) {
            return;
        }
        if let Err(e) = manager.snapshot_in_background(self.ms.root(), height) {
            tracing::error!(height, error = %e, "failed to create state snapshot");
        }
    }

    /// Waits for the background snapshot, if any (tests, tools).
    pub fn wait_for_snapshot(&mut self) {
        if let Some(manager) = &mut self.snapshots {
            let _ = manager.wait_for_snapshot();
        }
    }

    /// `ListSnapshots`.
    #[must_use]
    pub fn list_snapshots(&self) -> abci::ResponseListSnapshots {
        let snapshots = self
            .snapshots
            .as_ref()
            .and_then(|m| m.list().ok())
            .unwrap_or_default()
            .into_iter()
            .map(|s| abci::Snapshot {
                metadata: s.metadata_bytes(),
                height: s.height,
                format: s.format,
                chunks: s.chunks,
                hash: s.hash,
            })
            .collect();
        abci::ResponseListSnapshots { snapshots }
    }

    /// `LoadSnapshotChunk`.
    #[must_use]
    pub fn load_snapshot_chunk(
        &self,
        req: &abci::RequestLoadSnapshotChunk,
    ) -> abci::ResponseLoadSnapshotChunk {
        let chunk = self
            .snapshots
            .as_ref()
            .and_then(|m| {
                m.load_chunk(req.height, req.format, req.chunk)
                    .ok()
                    .flatten()
            })
            .unwrap_or_default();
        abci::ResponseLoadSnapshotChunk { chunk }
    }

    /// `OfferSnapshot`.
    pub fn offer_snapshot(
        &mut self,
        req: abci::RequestOfferSnapshot,
    ) -> abci::ResponseOfferSnapshot {
        use abci::response_offer_snapshot::Result as R;
        let result = match (&mut self.snapshots, req.snapshot) {
            (None, _) => R::Abort,
            (Some(_), None) => R::Reject,
            (Some(manager), Some(s)) => {
                match snapshots::Snapshot::from_abci(
                    s.height,
                    s.format,
                    s.chunks,
                    s.hash,
                    &s.metadata,
                )
                .and_then(|snapshot| manager.begin_restore(snapshot))
                {
                    Ok(()) => {
                        // Only accepted offers select stores. A later offer
                        // may replace an abandoned restore of another version.
                        if self.last_height == 0 {
                            for name in stores::ADDED_IN_V10 {
                                let mounted = self.ms.is_mounted(name);
                                let result = if stores::is_v9(req.app_version) && mounted {
                                    self.ms.unmount_iavl(name)
                                } else if !stores::is_v9(req.app_version) && !mounted {
                                    self.ms.mount(name, sdk::Kind::Iavl)
                                } else {
                                    Ok(())
                                };
                                if let Err(e) = result {
                                    tracing::error!(store = name, error = %e, "failed to select snapshot stores");
                                    return abci::ResponseOfferSnapshot {
                                        result: R::Abort as i32,
                                    };
                                }
                            }
                        }
                        R::Accept
                    }
                    Err(snapshots::Error::UnknownFormat(_)) => R::RejectFormat,
                    Err(snapshots::Error::InvalidMetadata(e)) => {
                        tracing::error!(error = %e, "rejecting invalid snapshot");
                        R::Reject
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "failed to restore snapshot");
                        R::Abort
                    }
                }
            }
        };
        abci::ResponseOfferSnapshot {
            result: result as i32,
        }
    }

    /// `ApplySnapshotChunk`. After the last chunk the app resumes from the
    /// restored height.
    pub fn apply_snapshot_chunk(
        &mut self,
        req: abci::RequestApplySnapshotChunk,
    ) -> abci::ResponseApplySnapshotChunk {
        use abci::response_apply_snapshot_chunk::Result as R;
        let abort = || abci::ResponseApplySnapshotChunk {
            result: R::Abort as i32,
            ..Default::default()
        };
        let Some(manager) = &mut self.snapshots else {
            return abort();
        };
        match manager.restore_chunk(self.ms.root_mut(), &req.chunk) {
            Ok(done) => {
                if let Some(id) = done {
                    tracing::info!(height = id.version, "restored state-sync snapshot");
                    if let Err(e) = self.load_committed_state() {
                        tracing::error!(error = %e, "failed to load restored state");
                        return abort();
                    }
                }
                abci::ResponseApplySnapshotChunk {
                    result: R::Accept as i32,
                    ..Default::default()
                }
            }
            Err(snapshots::Error::ChunkHashMismatch(_)) => abci::ResponseApplySnapshotChunk {
                result: R::Retry as i32,
                refetch_chunks: vec![req.index],
                reject_senders: vec![req.sender],
            },
            Err(e) => {
                tracing::error!(error = %e, "failed to restore snapshot");
                abort()
            }
        }
    }
}

/// `executeTxsPhased`: every tx's ante handler first, then every surviving
/// tx's messages. Ante writes persist even when the messages fail.
/// Appends the `mode` attribute (indexed) to every event.
fn with_mode(events: &mut [abci::Event], mode: &str) {
    for e in events {
        e.attributes.push(abci::EventAttribute {
            key: "mode".into(),
            value: mode.into(),
            index: true,
        });
    }
}

/// `BaseApp.extractSigners`: the bech32 addresses of the signer infos'
/// public keys (nil on any error).
fn extract_signers(decode: tx::DecodeFn, tx_bytes: &[u8]) -> Vec<String> {
    let Ok(tx) = decode(tx_bytes) else {
        return Vec::new();
    };
    let Ok(sigs) = x_ante::txview::signatures_v2(&tx) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(sigs.len());
    for s in sigs {
        let Some(pk) = s.pub_key else {
            return Vec::new();
        };
        match pk.address() {
            Ok(a) => out.push(x_ante::txview::bytes_to_bech32(&a)),
            Err(_) => return Vec::new(),
        }
    }
    out
}

fn execute_txs_phased(ctx: &mut Context<'_>, txs: &[Vec<u8>]) -> Vec<abci::ExecTxResult> {
    let mut results: Vec<Option<abci::ExecTxResult>> = vec![None; txs.len()];
    let mut prepared: Vec<Option<tx::Prepared>> = Vec::with_capacity(txs.len());
    for (i, raw) in txs.iter().enumerate() {
        if txpipe::decode_tx(raw).is_err() {
            // comet expects a result for every tx, even undecodable ones.
            results[i] = Some(tx::error_result(
                &sdk::error::sdk::tx_decode(""),
                0,
                0,
                Vec::new(),
            ));
            prepared.push(None);
            continue;
        }
        let (p, res) = tx::run_ante(ctx, raw, txpipe::decode_tx, txpipe::ante, false);
        match res {
            Ok(()) => prepared.push(Some(p)),
            Err(e) => {
                let mut r = tx::exec_result(&p, Err(e));
                r.signers = extract_signers(txpipe::decode_tx, raw);
                results[i] = Some(r);
                prepared.push(None);
            }
        }
    }
    for (i, p) in prepared.iter_mut().enumerate() {
        if let Some(p) = p {
            let res = tx::run_msgs(ctx, p, true);
            let mut r = tx::exec_result(p, res);
            r.signers = extract_signers(txpipe::decode_tx, &p.tx_bytes);
            results[i] = Some(r);
        }
    }
    // Go runs every tx on its own copy of the finalize-block context, so
    // the end blockers see the block context's infinite gas meter and no
    // tx bytes. Without this reset they inherited the last tx's meter: a
    // passed gov proposal executed in EndBlock after a nearly exhausted
    // tx failed with "out of gas" and halted the Rust nodes (soak).
    ctx.gas = kvstore::GasMeter::infinite();
    ctx.tx_bytes.clear();
    results.into_iter().map(Option::unwrap_or_default).collect()
}

/// cosmos-sdk v0.52.8 (celestia-app v9) `FinalizeBlock`: `deliverTx` per
/// tx (`runTx`: ante, then messages), with the block gas meter of a finite
/// `block.max_gas` (`getBlockGasMeter`; `-1`/`0` means infinite).
fn execute_txs_sequential(ctx: &mut Context<'_>, txs: &[Vec<u8>]) -> Vec<abci::ExecTxResult> {
    let max_gas = ctx
        .consensus_params
        .block
        .as_ref()
        .map_or(-1, |b| b.max_gas);
    let block_limit = u64::try_from(max_gas).ok().filter(|l| *l > 0);
    let mut block_used: u64 = 0;
    let mut results = Vec::with_capacity(txs.len());
    for raw in txs {
        if txpipe::decode_tx_v9(raw).is_err() {
            results.push(tx::error_result(
                &sdk::error::sdk::tx_decode(""),
                0,
                0,
                Vec::new(),
            ));
            continue;
        }
        let signers = extract_signers(txpipe::decode_tx_v9, raw);
        // `runTx`: "only run the tx if there is block gas remaining".
        if block_limit.is_some_and(|l| block_used >= l) {
            let mut r = tx::error_result(
                &sdk::error::sdk::out_of_gas("no block gas left to run tx"),
                0,
                0,
                Vec::new(),
            );
            r.signers = signers;
            results.push(r);
            continue;
        }
        // `consumeBlockGas`: the tx's gas (capped at its limit) is added to
        // the block meter after the tx; exceeding the block limit panics
        // and the recovered error replaces the tx's result.
        let block_overflow = |used: u64, gas: &kvstore::GasMeter, gas_wanted: u64| {
            let total = used.saturating_add(gas.consumed_to_limit());
            let over = block_limit.is_some_and(|l| total > l);
            let err = over.then(|| {
                sdk::error::sdk::out_of_gas(format!(
                    "out of gas in location: block gas meter; gasWanted: {gas_wanted}, gasUsed: {}",
                    gas.consumed()
                ))
            });
            (total, err)
        };
        let (mut p, res) = tx::run_ante(ctx, raw, txpipe::decode_tx_v9, txpipe::ante, false);
        let mut r = if let Err(e) = res {
            let (total, over) = block_overflow(block_used, &p.gas, p.gas_wanted);
            block_used = total;
            let e = over.unwrap_or(e);
            // A failed ante handler returns no events.
            tx::error_result(
                &e,
                as_i64(p.gas_wanted),
                as_i64(p.gas.consumed()),
                Vec::new(),
            )
        } else {
            let gas_wanted = p.gas_wanted;
            let mut consumed = None;
            let res = tx::run_msgs_gated(ctx, &mut p, true, |gas| {
                let (total, over) = block_overflow(block_used, gas, gas_wanted);
                consumed = Some(total);
                over.map_or(Ok(()), Err)
            });
            if let Some(total) = consumed {
                block_used = total;
            } else {
                let (total, over) = block_overflow(block_used, &p.gas, gas_wanted);
                block_used = total;
                if let (Some(e), Err(_)) = (over, &res) {
                    let mut r = tx::exec_result(&p, Err(e));
                    r.signers = signers;
                    results.push(r);
                    continue;
                }
            }
            tx::exec_result(&p, res)
        };
        r.signers = signers;
        results.push(r);
    }
    ctx.gas = kvstore::GasMeter::infinite();
    ctx.tx_bytes.clear();
    results
}

#[allow(clippy::cast_possible_wrap)]
const fn as_i64(v: u64) -> i64 {
    v as i64
}

fn multistore_timestamp(t: &pbjson_types::Timestamp) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: t.seconds,
        nanos: t.nanos,
    }
}

fn sdk_empty_hash() -> Vec<u8> {
    // sha256 of the empty commit info (version 0).
    vec![
        0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f, 0xb9,
        0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b, 0x78, 0x52,
        0xb8, 0x55,
    ]
}

/// Thread-safe ABCI adapter.
#[derive(Debug, Default)]
pub struct AbciApp {
    inner: Arc<Mutex<App>>,
}

impl AbciApp {
    /// Wraps an app.
    #[must_use]
    pub fn new(app: App) -> Self {
        Self {
            inner: Arc::new(Mutex::new(app)),
        }
    }

    /// Wraps an app shared with other in-process users (the consensus
    /// node's block executor and mempool connections).
    #[must_use]
    pub fn from_shared(app: Arc<Mutex<App>>) -> Self {
        Self { inner: app }
    }

    /// The shared app.
    #[must_use]
    pub fn shared(&self) -> Arc<Mutex<App>> {
        Arc::clone(&self.inner)
    }
}

fn exception(e: &SdkError) -> response::Value {
    response::Value::Exception(ResponseException {
        error: e.log.clone(),
    })
}

impl Application for AbciApp {
    fn handle_envelope(&self, request: request::Value) -> Option<response::Value> {
        let request::Value::Query(req) = request else {
            return Some(self.handle(request));
        };
        if let Some(resp) = node_services::abci_query(self, &req) {
            return Some(response::Value::Query(resp));
        }
        match self.inner.lock() {
            Ok(mut app) => app.query_response(&req).map(response::Value::Query),
            Err(_) => Some(response::Value::Exception(ResponseException {
                error: "poisoned".into(),
            })),
        }
    }

    fn handle(&self, request: request::Value) -> response::Value {
        // celestia-core-backed queries run outside the lock (see
        // `node_services`).
        if let request::Value::Query(r) = &request
            && let Some(resp) = node_services::abci_query(self, r)
        {
            return response::Value::Query(resp);
        }
        let Ok(mut app) = self.inner.lock() else {
            return response::Value::Exception(ResponseException {
                error: "poisoned".into(),
            });
        };
        match request {
            request::Value::Echo(e) => {
                response::Value::Echo(abci::ResponseEcho { message: e.message })
            }
            request::Value::Info(_) => response::Value::Info(app.info()),
            request::Value::InitChain(r) => app
                .init_chain(r)
                .map_or_else(|e| exception(&e), response::Value::InitChain),
            request::Value::FinalizeBlock(r) => app
                .finalize_block(r)
                .map_or_else(|e| exception(&e), response::Value::FinalizeBlock),
            request::Value::Commit(_) => app
                .commit()
                .map_or_else(|e| exception(&e), response::Value::Commit),
            request::Value::PrepareProposal(r) => app
                .prepare_proposal(&r)
                .map_or_else(|e| exception(&e), response::Value::PrepareProposal),
            request::Value::ProcessProposal(r) => app
                .process_proposal(&r)
                .map_or_else(|e| exception(&e), response::Value::ProcessProposal),
            request::Value::CheckTx(r) => app.check_tx(&r).map_or_else(
                |error| response::Value::Exception(ResponseException { error }),
                response::Value::CheckTx,
            ),
            request::Value::Query(r) => response::Value::Query(app.query(&r)),
            request::Value::QuerySequence(r) => {
                response::Value::QuerySequence(app.query_sequence(&r.signer))
            }
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
            other => response::Value::Exception(ResponseException {
                error: format!("unsupported request {other:?}"),
            }),
        }
    }
}

#[cfg(test)]
mod finalize_gas_tests {
    use super::*;
    use celestia_proto::Any;
    use celestia_proto::cosmos::auth::v1beta1::{BaseAccount, Params as AuthParams};
    use celestia_proto::cosmos::bank::v1beta1::MsgSend;
    use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;
    use celestia_proto::cosmos::tx::v1beta1::{
        AuthInfo, Fee, ModeInfo, SignDoc, SignerInfo, TxBody, TxRaw, mode_info,
    };
    use prost::Message as _;

    /// A DIRECT-signed MsgSend from a funded account with a 100,000 gas
    /// limit, chain "test", sequence 0.
    fn signed_send(ctx: &mut Context<'_>) -> Vec<u8> {
        let sk = crypto::secp256k1::secret_key_from_secret(b"end-block-gas");
        let pk = crypto::secp256k1::public_key_from_secret_key(&sk).unwrap_or([0; 33]);
        let addr = crypto::secp256k1::address(&pk).to_vec();
        let ok = |r: Result<()>| assert!(r.is_ok(), "{r:?}");
        ok(x_auth::set_params(
            ctx,
            &AuthParams {
                max_memo_characters: 256,
                tx_sig_limit: 7,
                tx_size_cost_per_byte: 10,
                sig_verify_cost_ed25519: 590,
                sig_verify_cost_secp256k1: 1000,
            },
        ));
        ok(x_auth::get_module_account(ctx, x_auth::FEE_COLLECTOR).map(|_| ()));
        let account_number = x_auth::next_account_number(ctx).unwrap_or_default();
        ok(x_auth::set_account(
            ctx,
            &x_auth::Account::Base(BaseAccount {
                address: x_auth::bech32(&addr).unwrap_or_default(),
                pub_key: None,
                account_number,
                sequence: 0,
            }),
        ));
        ok(x_bank::set_params(
            ctx,
            celestia_proto::cosmos::bank::v1beta1::Params {
                default_send_enabled: true,
                ..Default::default()
            },
        ));
        ok(x_bank::set_balance(
            ctx,
            &addr,
            &sdk::coins::Coin::new("utia", 1_000_000),
        ));
        ok(ctx.set(
            "minfee",
            b"params".to_vec(),
            celestia_proto::celestia::minfee::v1::Params {
                network_min_gas_price: "1000000000000".into(),
            }
            .encode_to_vec(),
        ));
        let body = TxBody {
            messages: vec![Any::pack(&MsgSend {
                from_address: x_auth::bech32(&addr).unwrap_or_default(),
                to_address: x_auth::bech32(&[7; 20]).unwrap_or_default(),
                amount: vec![PbCoin {
                    denom: "utia".into(),
                    amount: "10".into(),
                }],
            })],
            ..Default::default()
        };
        let mut key = vec![0x0a, 33];
        key.extend_from_slice(&pk);
        let auth = AuthInfo {
            signer_infos: vec![SignerInfo {
                public_key: Some(Any {
                    type_url: "/cosmos.crypto.secp256k1.PubKey".into(),
                    value: key,
                }),
                mode_info: Some(ModeInfo {
                    sum: Some(mode_info::Sum::Single(mode_info::Single { mode: 1 })),
                }),
                sequence: 0,
            }],
            fee: Some(Fee {
                amount: vec![PbCoin {
                    denom: "utia".into(),
                    amount: "2000".into(),
                }],
                gas_limit: 100_000,
                ..Default::default()
            }),
            ..Default::default()
        };
        let (body_bytes, auth_info_bytes) = (body.encode_to_vec(), auth.encode_to_vec());
        let doc = SignDoc {
            body_bytes: body_bytes.clone(),
            auth_info_bytes: auth_info_bytes.clone(),
            chain_id: "test".into(),
            account_number,
        };
        let sig = crypto::secp256k1::sign(&sk, &doc.encode_to_vec()).unwrap_or([0; 64]);
        TxRaw {
            body_bytes,
            auth_info_bytes,
            signatures: vec![sig.to_vec()],
        }
        .encode_to_vec()
    }

    /// Regression (soak, 2026-10-14): after the tx phase the block context
    /// must hold an infinite gas meter again, not the last tx's limited one.
    /// EndBlock work (here: executing a passed gov proposal) ran out of gas
    /// on Rust nodes only and halted them.
    #[test]
    fn end_block_context_gets_an_infinite_gas_meter() {
        let mut app = App::new();
        let header = Header {
            chain_id: "test".into(),
            height: 5,
            ..Default::default()
        };
        let mut ctx = Context::new(&mut app.ms, header, ExecMode::Finalize);
        let tx = signed_send(&mut ctx);
        let results = execute_txs_phased(&mut ctx, std::slice::from_ref(&tx));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].code, 0, "{}", results[0].log);
        assert_eq!(results[0].gas_wanted, 100_000);
        assert_eq!(
            ctx.gas.limit(),
            None,
            "block context meter must be infinite"
        );
        assert_eq!(ctx.gas.consumed(), 0);
        assert!(ctx.tx_bytes.is_empty());
    }
}

#[cfg(test)]
mod abci_dispatch_tests {
    use super::*;

    /// Guards against the dispatch arms being dropped in a merge: every
    /// request celestia-core sends during normal block production must be
    /// answered with its own response type, never an exception.
    #[test]
    fn proposal_requests_are_dispatched() {
        let app = AbciApp::new(App::new());
        let r = app.handle(request::Value::PrepareProposal(
            abci::RequestPrepareProposal {
                height: 1,
                ..Default::default()
            },
        ));
        assert!(matches!(r, response::Value::PrepareProposal(_)), "{r:?}");
        let r = app.handle(request::Value::ProcessProposal(
            abci::RequestProcessProposal {
                height: 1,
                ..Default::default()
            },
        ));
        assert!(matches!(r, response::Value::ProcessProposal(_)), "{r:?}");
        // A non-canonical BlobTx (type id + unknown field): rejected with a
        // CheckTx response (app/11147), not an exception.
        let r = app.handle(request::Value::CheckTx(abci::RequestCheckTx {
            tx: b"\x1a\x04BLOB\x78\x01".to_vec(),
            ..Default::default()
        }));
        assert!(matches!(r, response::Value::CheckTx(_)), "{r:?}");
        let r = app.handle(request::Value::Query(abci::RequestQuery::default()));
        assert!(matches!(r, response::Value::Query(_)), "{r:?}");
    }
}
