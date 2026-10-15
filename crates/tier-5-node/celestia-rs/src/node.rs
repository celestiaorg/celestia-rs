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
//! | `data/application.rocksdb/` | app state (ADR 0002; `.redb` files with the redb backend) |
//! | `data/blockstore.rocksdb/`, `data/state.rocksdb/` | block / state stores |
//! | `data/cs.wal/wal` | consensus WAL (Go format) |
//! | `data/tx-inbox/*.tx` | raw txs submitted to the CAT mempool via CheckTx (until RPC exists) |
//!
//! Shutdown (SIGINT / SIGTERM through [`crate::ops::Shutdown`]) stops
//! consensus between steps, so the WAL, the privval state and the stores
//! are consistent; `halt-height` / `halt-time` (app.toml) stop the node
//! like Go's `checkHalt`, and a due upgrade without a handler stops
//! consensus with `data/upgrade-info.json` written (cosmovisor's trigger)
//! while RPC keeps serving, exactly like a Go node.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, anyhow};
use consensus::{ConsensusConfig, ConsensusState, Event, FileWal, SystemClock};
use ctypes::proto::abci;
use kvdb::{DbOptions, KvBackend};
use nodestore::{AppConn, BlockExecutor, BlockStore, GenesisDoc, Handshaker, StateStore};
use privval::FilePv;
use prost::Message as _;

/// The Rust app behind [`AppConn`].
pub(crate) struct AppAdapter(pub(crate) Arc<Mutex<app::App>>);

fn app_err(e: &sdk::SdkError) -> nodestore::Error {
    nodestore::Error::new(e.log.clone())
}

impl AppAdapter {
    fn app(&self) -> std::sync::MutexGuard<'_, app::App> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The mempool connection to the same app (CheckTx runs on its check
/// state; `QuerySequence` for the CAT mempool).
struct MempoolApp(Arc<Mutex<app::App>>);

impl mempool::AppConnMempool for MempoolApp {
    fn check_tx(&self, req: abci::RequestCheckTx) -> Result<abci::ResponseCheckTx, String> {
        let mut app = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        app.check_tx(&req)
    }

    fn query_sequence(&self, signer: &[u8]) -> Result<u64, String> {
        let mut app = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(app.query_sequence(signer).sequence)
    }
}

impl AppConn for AppAdapter {
    fn info(&mut self) -> nodestore::Result<abci::ResponseInfo> {
        Ok(self.app().info())
    }
    fn init_chain(
        &mut self,
        req: abci::RequestInitChain,
    ) -> nodestore::Result<abci::ResponseInitChain> {
        self.app().init_chain(req).map_err(|e| app_err(&e))
    }
    fn prepare_proposal(
        &mut self,
        req: abci::RequestPrepareProposal,
    ) -> nodestore::Result<abci::ResponsePrepareProposal> {
        self.app().prepare_proposal(&req).map_err(|e| app_err(&e))
    }
    fn process_proposal(
        &mut self,
        req: abci::RequestProcessProposal,
    ) -> nodestore::Result<abci::ResponseProcessProposal> {
        self.app().process_proposal(&req).map_err(|e| app_err(&e))
    }
    fn finalize_block(
        &mut self,
        req: abci::RequestFinalizeBlock,
    ) -> nodestore::Result<abci::ResponseFinalizeBlock> {
        self.app().finalize_block(req).map_err(|e| app_err(&e))
    }
    fn commit(&mut self) -> nodestore::Result<abci::ResponseCommit> {
        self.app().commit().map_err(|e| app_err(&e))
    }
}

/// Submits the raw tx files in `<home>/data/tx-inbox` (name order) to the
/// mempool through CheckTx and removes them (a local stand-in for RPC
/// `broadcast_tx` until the RPC server exists).
fn feed_inbox(dir: &Path, pool: &mempool::TxPool) {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "tx"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    for p in files {
        let Ok(bz) = std::fs::read(&p) else { continue };
        let _ = std::fs::remove_file(&p);
        let info = mempool::TxInfo {
            from_broadcast: true,
            ..mempool::TxInfo::default()
        };
        match pool.check_tx(&bz, info) {
            Ok(r) => tracing::info!(
                file = %p.display(),
                code = r.code,
                log = %r.log,
                "CheckTx"
            ),
            Err(e) => tracing::warn!(file = %p.display(), err = %e, "mempool rejected tx"),
        }
    }
}

/// Prometheus endpoint options.
#[derive(Debug, Clone)]
pub struct MetricsOptions {
    /// Listen address (`:26660`, `127.0.0.1:26660`).
    pub laddr: String,
    /// Metric namespace (`cometbft`).
    pub namespace: String,
}

/// Options of `celestia-rs start`.
#[derive(Debug, Clone)]
pub struct StartOptions {
    /// Node home.
    pub home: PathBuf,
    /// Stop after committing this height (0 = run forever); unlike Go's
    /// `halt-height` ([`app::NodeOptions::halt_height`]) the block at this
    /// height is committed.
    pub stop_height: i64,
    /// Consensus config overrides.
    pub config: ConsensusConfig,
    /// P2P (None: no networking, a lone validator).
    pub p2p: Option<crate::net::P2pOptions>,
    /// `block_sync`: catch up from peers before consensus (Go default).
    pub block_sync: bool,
    /// `[statesync]` (fresh nodes only).
    pub state_sync: Option<crate::sync::StateSyncOptions>,
    /// App state options (pruning, caches, snapshots, block retention,
    /// halt height / time).
    pub app: app::AppOptions,
    /// `minimum-gas-prices` (utia) for CheckTx.
    pub min_gas_price: Option<math::LegacyDec>,
    /// Prometheus endpoint.
    pub metrics: Option<MetricsOptions>,
    /// Shutdown requests (signals, tests).
    pub shutdown: crate::ops::Shutdown,
}

impl StartOptions {
    /// A node without networking.
    #[must_use]
    pub fn new(home: PathBuf, stop_height: i64, config: ConsensusConfig) -> Self {
        Self {
            home,
            stop_height,
            config,
            p2p: None,
            block_sync: true,
            state_sync: None,
            app: app::AppOptions::default(),
            min_gas_price: None,
            metrics: None,
            shutdown: crate::ops::Shutdown::new(),
        }
    }
}

/// Whether consensus messages are gossiped over P2P (the consensus and
/// block propagation reactors are wired), so every node hands over from
/// block sync to consensus once caught up, as in Go.
pub const CONSENSUS_GOSSIP: bool = true;

/// Opens node database `name` in `data` on the default backend (ADR 0002)
/// with a 64 MiB cache: block and state store reads are mostly recent.
pub(crate) fn open_db(data: &Path, name: &str) -> anyhow::Result<Arc<dyn KvBackend>> {
    let options = DbOptions {
        cache_size: 64 << 20,
        ..DbOptions::default()
    };
    let db = kvdb::open_backend(data, name, &options)
        .map_err(|e| anyhow!("open {name} in {}: {e}", data.display()))?;
    crate::memstats::register_db(name, &db);
    register_for_flush(&db);
    Ok(db)
}

/// Databases flushed by [`flush_databases`] (weak: closing one is fine).
static OPEN_DBS: std::sync::Mutex<Vec<std::sync::Weak<dyn KvBackend>>> =
    std::sync::Mutex::new(Vec::new());

/// Adds `db` to the databases written out at shutdown.
pub(crate) fn register_for_flush(db: &Arc<dyn KvBackend>) {
    if let Ok(mut dbs) = OPEN_DBS.lock() {
        dbs.retain(|d| d.strong_count() > 0);
        dbs.push(Arc::downgrade(db));
    }
}

/// Writes the in-memory buffers of every open database to its files
/// (clean shutdown): RocksDB otherwise keeps them only in its WAL and the
/// next start replays it before the node can serve (B1 phase C: ~2 s of
/// a 2.1 s restart).
pub(crate) fn flush_databases() {
    let dbs: Vec<_> = OPEN_DBS
        .lock()
        .map(|dbs| dbs.iter().filter_map(std::sync::Weak::upgrade).collect())
        .unwrap_or_default();
    for db in dbs {
        if let Err(e) = db.flush() {
            tracing::warn!(error = %e, "flushing a database at shutdown");
        }
    }
}

/// Calls [`flush_databases`] when dropped (every return of `start`).
struct FlushDatabasesOnExit;

impl Drop for FlushDatabasesOnExit {
    fn drop(&mut self) {
        flush_databases();
    }
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
    let bs =
        BlockStore::new(open_db(&data, "blockstore")?).map_err(|e| anyhow!("block store: {e}"))?;
    let ss = StateStore::new(open_db(&data, "state")?);
    Ok((bs, ss))
}

/// `celestia-rs start`.
#[allow(clippy::too_many_lines)]
pub fn start(opts: &StartOptions) -> anyhow::Result<()> {
    let home = &opts.home;
    let genesis = load_genesis(home)?;
    // Like Go's node: wait for the genesis time.
    let now = consensus::ticker::system_now();
    if genesis.genesis_time > now {
        let wait = nodestore::state::sub_times(genesis.genesis_time, now);
        tracing::info!(seconds = wait / 1_000_000_000, "waiting for genesis time");
        if opts.shutdown.sleep(std::time::Duration::from_nanos(
            u64::try_from(wait).unwrap_or(0),
        )) {
            return Ok(());
        }
    }
    let mut app_opts = opts.app.clone();
    app_opts.chain_id = Some(genesis.chain_id.clone());
    app_opts.node.home = Some(home.clone());
    let mut app =
        app::App::open_with(home, app_opts).map_err(|e| anyhow!("open app state: {}", e.log))?;
    app.set_node_min_gas_price(opts.min_gas_price.clone());
    if let Some(db) = app.multistore().root().backend() {
        crate::memstats::register_db("application", db);
        register_for_flush(db);
    }
    let _flush_on_exit = FlushDatabasesOnExit;
    {
        let nc = app.multistore().root().node_cache().clone();
        #[allow(clippy::cast_precision_loss)]
        crate::memstats::register_gauge("iavl_node_cache_nodes", move || nc.stats().len as f64);
    }
    let (bs, ss) = open_stores(home)?;
    let state = ss
        .load_from_db_or_genesis(&genesis)
        .map_err(|e| anyhow!("load state: {e}"))?;
    let inbox = home.join("data").join("tx-inbox");
    std::fs::create_dir_all(&inbox)?;
    let app = Arc::new(Mutex::new(app));
    let pool = Arc::new(
        mempool::TxPool::new(
            mempool::MempoolConfig::celestia_app(),
            Arc::new(MempoolApp(app.clone())),
            state.last_block_height,
        )
        .with_pre_check(nodestore::execution::tx_pre_check(&state))
        .with_post_check(nodestore::execution::tx_post_check(&state)),
    );
    let evpool = crate::sync::open_evidence_pool(home, &ss, &bs, &state)?;
    let mut exec = BlockExecutor::new(
        ss.clone(),
        bs.clone(),
        Box::new(AppAdapter(app.clone())),
        pool.clone(),
        evpool.executor_handle(),
    );
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
    // RPC (ADR 0003 `rpc`): JSON-RPC/WebSocket, celestia-core gRPC, the
    // app's gRPC (+BlockAPI) and REST servers, indexers and the event bus.
    let mut rpc = crate::rpc_service::Rpc::new(crate::rpc_service::RpcInputs {
        home,
        genesis: &genesis,
        block_store: bs.clone(),
        state_store: ss.clone(),
        app: app.clone(),
        mempool: pool.clone(),
        evidence: evpool.clone(),
        pub_key: ctypes::PubKey::Ed25519(pv.key.pub_key),
        app_version: state.consensus_params.version.app,
    })?;
    exec.set_event_sink(rpc.publisher.clone());
    rpc.spawn()?;
    let local_addr = pv.address().to_vec();
    // Go: a fresh node with state sync enabled skips the handshake; the
    // state comes from the snapshot.
    let state_sync_mode = opts.state_sync.is_some() && state.last_block_height == 0;
    if opts.state_sync.is_some() && !state_sync_mode {
        tracing::info!("found local state with non-zero height, skipping state sync");
    }
    let mut state = if state_sync_mode {
        state
    } else {
        let mut hs = Handshaker::new(
            ss.clone(),
            state,
            bs.clone(),
            &genesis,
            consensus::ticker::system_now(),
        );
        let state = hs
            .handshake(&mut exec)
            .map_err(|e| anyhow!("handshake: {e}"))?;
        tracing::info!(
            height = state.last_block_height,
            app_hash = %hex::encode_upper(&state.app_hash),
            replayed = hs.n_blocks(),
            "handshake complete"
        );
        state
    };
    if opts.stop_height > 0 && state.last_block_height >= opts.stop_height {
        tracing::info!(height = state.last_block_height, "stop height reached");
        return Ok(());
    }
    let metrics = match &opts.metrics {
        Some(m) => Some(start_metrics(m, &genesis.chain_id, &pool)?),
        None => None,
    };
    // The consensus state machine and its reactors exist from the start
    // (Go's `NewState` + `NewReactor`); block sync executes blocks through
    // the same block executor and then hands over (`SwitchToConsensus`).
    let wal = FileWal::open(
        home.join("data").join("cs.wal").join("wal"),
        consensus::ticker::system_now(),
    )
    .map_err(|e| anyhow!("wal: {e}"))?;
    // Set at shutdown: ends a delayed precommit wait without signing.
    let cs_stopping = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut cs = ConsensusState::new(
        opts.config.clone(),
        state.clone(),
        exec,
        Box::new(wal),
        Box::new(SystemClock::with_stop(cs_stopping.clone())),
        Some(Box::new(pv)),
    )
    .map_err(|e| anyhow!("consensus: {e}"))?;
    let only_validator_is_us =
        state.validators.size() == 1 && state.validators.has_address(&local_addr);
    let wait_sync =
        opts.p2p.is_some() && (state_sync_mode || (opts.block_sync && !only_validator_is_us));
    let self_id = crate::net::node_key(home)?.id();
    let creactors = consensus_reactors(&mut cs, &bs, &pool, wait_sync, self_id);
    let _ = rpc.shared.consensus.set(creactors.reactor.clone());
    let _ = rpc.shared.propagation.set(creactors.propagation.clone());
    let net = match &opts.p2p {
        None => None,
        Some(p2p_opts) => Some(start_network(
            home,
            &genesis.chain_id,
            p2p_opts,
            &state,
            &bs,
            &ss,
            &evpool,
            &app,
            &pool,
            &local_addr,
            &creactors,
        )?),
    };
    if let Some((sw, _)) = &net {
        let _ = rpc.shared.switch.set(sw.clone());
        if let Some(m) = &metrics {
            let sw = sw.clone();
            #[allow(clippy::cast_precision_loss)]
            m.probe("p2p_peers", move || sw.peers().len() as f64);
        }
    }
    {
        // Shutdown: stop consensus between steps (Go `OnStop`).
        let stop = creactors.stop.clone();
        opts.shutdown.on_trigger(move || {
            cs_stopping.store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = stop.send(consensus::service::Input::Stop);
        });
    }
    let result = (|| {
        let mut state_synced = false;
        let mut blocks_synced = 0;
        if let (true, Some((_, reactors)), Some(ss_opts)) =
            (state_sync_mode, &net, &opts.state_sync)
        {
            state = crate::sync::state_sync(&reactors.statesync, ss_opts, &state, &ss, &bs)?;
            state_synced = true;
        }
        if let Some((_, reactors)) = &net
            && ((opts.block_sync && !only_validator_is_us) || state_synced)
        {
            let is_validator = state.validators.has_address(&local_addr);
            let out = crate::sync::block_sync(
                &reactors.blocksync,
                state.clone(),
                &mut cs.block_exec,
                state_synced,
                CONSENSUS_GOSSIP || is_validator,
                opts.stop_height,
                &opts.shutdown,
            )?;
            state = out.state;
            blocks_synced = out.blocks_synced;
            tracing::info!(
                height = state.last_block_height,
                blocks_synced = out.blocks_synced,
                reason = ?out.reason,
                "block sync finished"
            );
            if out.reason == blocksync::SwitchToConsensus::Stopped {
                return Ok(None);
            }
        }
        Ok::<Option<bool>, anyhow::Error>(Some(state_synced || blocks_synced > 0))
    })();
    let stop = |net: &Option<(p2p::Switch, crate::sync::SyncReactors)>| {
        if let Some((sw, _)) = net {
            sw.stop();
        }
    };
    let skip_wal = match result {
        Ok(None) => {
            tracing::info!(height = state.last_block_height, "block sync stopped");
            stop(&net);
            return Ok(());
        }
        Ok(Some(synced)) => synced,
        Err(e) => {
            stop(&net);
            return planned_stop(&format!("{e:#}"), &opts.shutdown);
        }
    };
    if opts.shutdown.is_triggered() {
        stop(&net);
        return Ok(());
    }
    let started = if wait_sync {
        // `SwitchToConsensus`.
        cs.switch_to_consensus(state, skip_wal).map(|()| {
            creactors.reactor.set_wait_sync(false);
            propagation::Propagator::start_processing(&*creactors.propagation);
            creactors
                .reactor
                .set_snapshot(consensus::service::snapshot_of(&cs));
            creactors.reactor.broadcast_current_round_step();
        })
    } else {
        cs.start()
    };
    if let Err(e) = started {
        stop(&net);
        return Err(anyhow!("consensus start: {e}"));
    }
    rpc.shared.update_round_state(cs.round_state());
    let local = (!local_addr.is_empty()).then(|| local_addr.clone());
    let res = run(
        &mut cs,
        &creactors,
        &pool,
        &inbox,
        opts.stop_height,
        &rpc,
        metrics.as_deref(),
        local.as_deref(),
        &bs,
    );
    stop(&net);
    match res {
        Ok(()) => Ok(()),
        Err(e) => planned_stop(&format!("{e:#}"), &opts.shutdown),
    }
}

/// Go stops consensus on a FinalizeBlock error ("CONSENSUS FAILURE"). Two
/// such errors are planned: `halt-height` / `halt-time` (the node exits
/// cleanly here) and a due upgrade without a handler (`x/upgrade` wrote
/// `data/upgrade-info.json`; like Go the process stays up with RPC
/// serving until it is stopped, so cosmovisor or the operator can swap the
/// binary). Anything else is an error.
fn planned_stop(err: &str, shutdown: &crate::ops::Shutdown) -> anyhow::Result<()> {
    if err.contains(app::HALT_ERROR_PREFIX) {
        tracing::info!(reason = err, "halting node per configuration");
        return Ok(());
    }
    if app::is_upgrade_needed(err) {
        tracing::error!(
            reason = err,
            "consensus halted for a software upgrade (data/upgrade-info.json written); \
             RPC keeps serving — stop the node and start the upgraded binary"
        );
        shutdown.wait();
        return Ok(());
    }
    Err(anyhow!("{err}"))
}

/// Starts the Prometheus endpoint (`[instrumentation]`).
fn start_metrics(
    m: &MetricsOptions,
    chain_id: &str,
    pool: &Arc<mempool::TxPool>,
) -> anyhow::Result<Arc<crate::metrics::Metrics>> {
    let metrics = crate::metrics::Metrics::new(&m.namespace, chain_id);
    crate::memstats::register_probes(&metrics);
    {
        let pool1 = pool.clone();
        #[allow(clippy::cast_precision_loss)]
        metrics.probe("mempool_size", move || pool1.size() as f64);
        let pool2 = pool.clone();
        #[allow(clippy::cast_precision_loss)]
        metrics.probe("mempool_size_bytes", move || pool2.size_bytes() as f64);
    }
    let laddr = if m.laddr.starts_with(':') {
        format!("0.0.0.0{}", m.laddr)
    } else {
        m.laddr.clone()
    };
    let addr = crate::rpc_service::socket_addr(&laddr)?;
    let rt = tokio::runtime::Handle::try_current()
        .map_err(|_| anyhow!("metrics need a tokio runtime"))?;
    let _g = rt.enter();
    let listener = std::net::TcpListener::bind(addr)
        .with_context(|| format!("bind prometheus_listen_addr {addr}"))?;
    listener.set_nonblocking(true)?;
    let listener = tokio::net::TcpListener::from_std(listener)?;
    tracing::info!(%addr, "Prometheus metrics listening");
    crate::metrics::serve(listener, metrics.clone());
    Ok(metrics)
}

/// What the metrics need about a committed block.
fn block_report(
    bs: &BlockStore,
    state: &nodestore::State,
    height: i64,
    round: i32,
    local: Option<&[u8]>,
) -> Option<crate::metrics::BlockReport> {
    let block = bs.load_block(height).ok()??;
    let vals = &state.last_validators;
    let mut missing = (0usize, 0i64);
    let mut signed = None;
    if let Some(c) = &block.last_commit {
        for (i, sig) in c.signatures.iter().enumerate() {
            let absent = sig.block_id_flag == ctypes::commit::BLOCK_ID_FLAG_ABSENT;
            if let Some(v) = vals.validators.get(i) {
                if absent {
                    missing.0 += 1;
                    missing.1 += v.voting_power;
                }
                if local.is_some_and(|a| a == v.address.as_slice()) {
                    signed = Some(!absent);
                }
            }
        }
    }
    let own = local.and_then(|a| {
        let (_, v) = vals.get_by_address(a)?;
        Some((hex::encode_upper(a), v.voting_power, signed.unwrap_or(true)))
    });
    Some(crate::metrics::BlockReport {
        height,
        round,
        time_ns: i64::try_from(block.header.time.unix_nanos()).unwrap_or(0),
        num_txs: block.data.txs.len(),
        size_bytes: block.to_proto().map_or(0, |p| p.encode_to_vec().len()),
        validators: (vals.size(), vals.total_voting_power()),
        missing,
        byzantine: (block.evidence.evidence.len(), 0),
        own,
    })
}

/// The consensus and block propagation reactors around a state machine.
pub(crate) struct ConsensusReactors {
    reactor: Arc<consensus::reactor::Reactor>,
    propagation: Arc<propagation::Reactor>,
    inputs: std::sync::mpsc::Receiver<consensus::service::Input>,
    /// Sender for [`consensus::service::Input::Stop`] (shutdown).
    stop: std::sync::mpsc::Sender<consensus::service::Input>,
}

/// Creates the consensus reactor and the propagation reactor (`RECOVERY`)
/// for `cs` and connects them to its input channel. Propagation processes
/// right away unless the node first syncs (Go: `StartProcessing` on
/// `SwitchToConsensus`).
fn consensus_reactors(
    cs: &mut ConsensusState,
    bs: &BlockStore,
    pool: &Arc<mempool::TxPool>,
    wait_sync: bool,
    self_id: p2p::NodeId,
) -> ConsensusReactors {
    let prop = propagation::Reactor::new(
        self_id,
        propagation::Config {
            store: bs.clone(),
            mempool: pool.clone(),
            chain_id: cs.state().chain_id.clone(),
            block_max_bytes: cs.state().consensus_params.block.max_bytes,
        },
    );
    if !wait_sync {
        propagation::Propagator::start_processing(&*prop);
    }
    cs.set_propagator(prop.clone());
    let (tx, rx) = std::sync::mpsc::channel::<consensus::service::Input>();
    let reactor = consensus::reactor::Reactor::new(
        consensus::reactor::ReactorConfig {
            gossip_data_enabled: cs.gossip_data_enabled,
            ..consensus::reactor::ReactorConfig::default()
        },
        bs.clone(),
        consensus::service::snapshot_of(cs),
        wait_sync,
    );
    reactor.set_inputs(tx.clone());
    let stop = tx.clone();
    prop.set_output(Arc::new(move |o| {
        let _ = tx.send(consensus::service::Input::Propagation(o));
    }));
    ConsensusReactors {
        reactor,
        propagation: prop,
        inputs: rx,
        stop,
    }
}

/// Creates and starts the P2P switch with the mempool, block sync,
/// evidence and state sync reactors (`createSwitch` + reactor setup).
#[allow(clippy::too_many_arguments)]
fn start_network(
    home: &Path,
    chain_id: &str,
    p2p_opts: &crate::net::P2pOptions,
    state: &nodestore::State,
    bs: &BlockStore,
    ss: &StateStore,
    evpool: &evidence::Pool,
    app: &Arc<Mutex<app::App>>,
    pool: &Arc<mempool::TxPool>,
    local_addr: &[u8],
    consensus: &ConsensusReactors,
) -> anyhow::Result<(p2p::Switch, crate::sync::SyncReactors)> {
    let sw = crate::net::new_switch(home, chain_id, p2p_opts)?;
    sw.add_reactor(consensus.reactor.clone())
        .map_err(|e| anyhow!("{e}"))?;
    sw.add_reactor(consensus.propagation.clone())
        .map_err(|e| anyhow!("{e}"))?;
    let cfg = mempool::MempoolConfig::celestia_app();
    let mreactor = mempool::cat::Reactor::new(
        pool.clone(),
        mempool::cat::ReactorOptions {
            listen_only: !cfg.broadcast,
            max_tx_size: i64::try_from(cfg.max_tx_bytes).unwrap_or(i64::MAX),
            max_gossip_delay: cfg.max_gossip_delay,
            max_persistent_sticky_peers: cfg.max_persistent_sticky_peers,
            ..mempool::cat::ReactorOptions::default()
        },
    )
    .map_err(|e| anyhow!("mempool reactor: {e}"))?;
    sw.add_reactor(mempool::cat::P2pReactor::new(mreactor))
        .map_err(|e| anyhow!("{e}"))?;
    let reactors = crate::sync::add_sync_reactors(
        &sw,
        home,
        state,
        bs,
        ss,
        evpool,
        app,
        Some(local_addr.to_vec()),
    )?;
    crate::net::start_switch(&tokio::runtime::Handle::current(), &sw)?;
    Ok((sw, reactors))
}

/// The node loop: the consensus thread (`consensus::service::run`) with
/// the reactors, the local tx inbox fed from a helper thread; stops at the
/// halt height or on a consensus failure.
#[allow(clippy::too_many_arguments)]
fn run(
    cs: &mut ConsensusState,
    reactors: &ConsensusReactors,
    pool: &Arc<mempool::TxPool>,
    inbox: &Path,
    stop_height: i64,
    rpc: &crate::rpc_service::Rpc,
    metrics: Option<&crate::metrics::Metrics>,
    local: Option<&[u8]>,
    bs: &BlockStore,
) -> anyhow::Result<()> {
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let feeder = {
        let (pool, inbox, done) = (pool.clone(), inbox.to_path_buf(), done.clone());
        std::thread::spawn(move || {
            while !done.load(std::sync::atomic::Ordering::SeqCst) {
                // The app's check state only has genesis state after the
                // first commit (as in the SDK), so txs wait for block 1.
                if pool.height() > 0 {
                    feed_inbox(&inbox, &pool);
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        })
    };
    let res = consensus::service::run(cs, &reactors.reactor, &reactors.inputs, |cs, ev| {
        rpc.consensus_event(ev, cs.round_state());
        if let Event::NewBlock {
            height,
            block_id,
            app_hash,
        } = ev
        {
            tracing::info!(
                height,
                hash = %hex::encode_upper(&block_id.hash),
                app_hash = %hex::encode_upper(app_hash),
                "committed block"
            );
            if let Some(m) = metrics {
                let round = cs.round_state().commit_round;
                if let Some(r) = block_report(bs, cs.state(), *height, round, local) {
                    m.block(&r);
                }
            }
            if stop_height > 0 && *height >= stop_height {
                tracing::info!(height, "stop height reached");
                return consensus::service::Flow::Stop;
            }
        }
        consensus::service::Flow::Continue
    });
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = feeder.join();
    res.map_err(|e| anyhow!(e))
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
