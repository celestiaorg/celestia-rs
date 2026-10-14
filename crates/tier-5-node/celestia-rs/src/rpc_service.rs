//! RPC wiring of `celestia-rs start` (ADR 0003, crate `rpc`): the
//! CometBFT JSON-RPC/WebSocket server on `[rpc] laddr`, celestia-core's
//! gRPC services on `[rpc] grpc_laddr`, and the app's gRPC query server
//! (`app.toml [grpc] address`, with BlockAPI mounted like celestia-app's
//! multiplexer does) plus its REST API; the kv tx/block indexers
//! (`[tx_index] indexer = "kv"`) and the event bus fed by the block
//! executor and consensus.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, anyhow};
use ctypes::proto::abci;
use nodestore::{BlockStore, GenesisDoc, StateStore};
use rpc::events::{BlockEventPublisher, EventBus, EventData};
use rpc::{AppQuery, BlockIndexer, Environment, GenesisData, NodeInfo, RpcConfig, TxIndexer};

/// RPC-related settings from `config/config.toml` and `config/app.toml`.
#[derive(Debug, Clone, Default)]
pub struct RpcSettings {
    /// `[rpc]`.
    pub rpc: RpcConfig,
    /// `moniker`.
    pub moniker: String,
    /// `[p2p] laddr`.
    pub p2p_laddr: String,
    /// `[tx_index] indexer == "kv"`.
    pub tx_index: bool,
    /// `app.toml [grpc] address` (empty = disabled).
    pub app_grpc: String,
    /// `app.toml [api] address` (empty = disabled).
    pub app_api: String,
}

fn toml_file(path: &Path) -> anyhow::Result<toml::Table> {
    match std::fs::read_to_string(path) {
        Ok(s) => s
            .parse::<toml::Table>()
            .with_context(|| format!("parse {}", path.display())),
        Err(_) => Ok(toml::Table::new()),
    }
}

fn get<'a>(t: &'a toml::Table, section: &str, key: &str) -> Option<&'a toml::Value> {
    t.get(section)?.as_table()?.get(key)
}

fn get_str(t: &toml::Table, section: &str, key: &str) -> Option<String> {
    get(t, section, key)?.as_str().map(str::to_owned)
}

fn get_int(t: &toml::Table, section: &str, key: &str) -> Option<i64> {
    get(t, section, key)?.as_integer()
}

fn get_list(t: &toml::Table, section: &str, key: &str) -> Option<Vec<String>> {
    Some(
        get(t, section, key)?
            .as_array()?
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
    )
}

/// Go `time.ParseDuration` for the simple forms config files use
/// (`10s`, `500ms`, `1m30s`); milliseconds.
fn parse_duration_ms(s: &str) -> Option<u64> {
    let mut total = 0f64;
    let mut num = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
            continue;
        }
        let mut unit = c.to_string();
        while let Some(&n) = chars.peek() {
            if n.is_ascii_alphabetic() || n == 'µ' {
                unit.push(n);
                chars.next();
            } else {
                break;
            }
        }
        let v: f64 = num.parse().ok()?;
        num.clear();
        total += v * match unit.as_str() {
            "ns" => 1e-6,
            "us" | "µs" => 1e-3,
            "ms" => 1.0,
            "s" => 1e3,
            "m" => 6e4,
            "h" => 3.6e6,
            _ => return None,
        };
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some(total as u64)
}

impl RpcSettings {
    /// Reads a node home's configuration (missing files: defaults).
    pub fn load(home: &Path) -> anyhow::Result<Self> {
        let cfg_path = home.join("config").join("config.toml");
        let cfg = toml_file(&cfg_path)?;
        let app = toml_file(&home.join("config").join("app.toml"))?;
        let mut rpc = RpcConfig::default();
        // A home without config.toml (test fixtures, bare homes) serves
        // nothing; `celestia-appd init` homes carry the Go defaults.
        let configured = cfg_path.exists();
        if !configured {
            rpc.laddr = String::new();
            rpc.grpc_laddr = String::new();
        }
        if let Some(v) = get_str(&cfg, "rpc", "laddr") {
            rpc.laddr = v;
        }
        if let Some(v) = get_str(&cfg, "rpc", "grpc_laddr") {
            rpc.grpc_laddr = v;
        }
        if let Some(v) = get_list(&cfg, "rpc", "cors_allowed_origins") {
            rpc.cors_allowed_origins = v;
        }
        if let Some(v) = get_list(&cfg, "rpc", "cors_allowed_methods") {
            rpc.cors_allowed_methods = v;
        }
        if let Some(v) = get_list(&cfg, "rpc", "cors_allowed_headers") {
            rpc.cors_allowed_headers = v;
        }
        let n = |v: i64| usize::try_from(v).unwrap_or(0);
        if let Some(v) = get_int(&cfg, "rpc", "max_open_connections") {
            rpc.max_open_connections = n(v);
        }
        if let Some(v) = get_int(&cfg, "rpc", "grpc_max_open_connections") {
            rpc.grpc_max_open_connections = n(v);
        }
        if let Some(v) = get(&cfg, "rpc", "unsafe").and_then(toml::Value::as_bool) {
            rpc.unsafe_routes = v;
        }
        if let Some(v) = get_int(&cfg, "rpc", "max_subscription_clients") {
            rpc.max_subscription_clients = n(v);
        }
        if let Some(v) = get_int(&cfg, "rpc", "max_subscriptions_per_client") {
            rpc.max_subscriptions_per_client = n(v);
        }
        if let Some(v) = get_int(&cfg, "rpc", "experimental_subscription_buffer_size") {
            rpc.subscription_buffer_size = n(v);
        }
        if let Some(v) = get_int(&cfg, "rpc", "experimental_websocket_write_buffer_size") {
            rpc.websocket_write_buffer_size = n(v);
        }
        if let Some(v) =
            get(&cfg, "rpc", "experimental_close_on_slow_client").and_then(toml::Value::as_bool)
        {
            rpc.close_on_slow_client = v;
        }
        if let Some(v) = get_str(&cfg, "rpc", "timeout_broadcast_tx_commit")
            .as_deref()
            .and_then(parse_duration_ms)
        {
            rpc.timeout_broadcast_tx_commit_ms = v;
        }
        if let Some(v) = get_int(&cfg, "rpc", "max_request_batch_size") {
            rpc.max_request_batch_size = n(v);
        }
        if let Some(v) = get_int(&cfg, "rpc", "max_body_bytes") {
            rpc.max_body_bytes = n(v);
        }
        if let Some(v) = get_int(&cfg, "rpc", "max_concurrent_heavy_requests") {
            rpc.max_concurrent_heavy_requests = v;
        }
        let enabled = |section: &str| {
            get(&app, section, "enable")
                .and_then(toml::Value::as_bool)
                .unwrap_or(section == "grpc")
        };
        Ok(Self {
            rpc,
            moniker: cfg
                .get("moniker")
                .and_then(toml::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            p2p_laddr: get_str(&cfg, "p2p", "laddr").unwrap_or_default(),
            // celestia-app's default config indexes txs ("kv" unless set to
            // "null").
            tx_index: get_str(&cfg, "tx_index", "indexer").is_none_or(|v| v == "kv"),
            app_grpc: if configured && enabled("grpc") {
                get_str(&app, "grpc", "address").unwrap_or_else(|| "localhost:9090".into())
            } else {
                String::new()
            },
            app_api: if configured && enabled("api") {
                get_str(&app, "api", "address").unwrap_or_default()
            } else {
                String::new()
            },
        })
    }
}

/// Binds a listener (on the entered runtime). A listener that cannot be
/// bound (address in use, bad address) is logged and skipped so the node
/// still runs: several nodes of a test topology on one machine often share
/// `celestia-appd init`'s default ports.
fn bind(what: &str, laddr: &str) -> Option<tokio::net::TcpListener> {
    let res = (|| {
        let addr = socket_addr(laddr)?;
        let listener = std::net::TcpListener::bind(addr).with_context(|| format!("bind {addr}"))?;
        listener.set_nonblocking(true)?;
        let listener = tokio::net::TcpListener::from_std(listener)?;
        Ok::<_, anyhow::Error>((addr, listener))
    })();
    match res {
        Ok((addr, listener)) => {
            tracing::info!(%addr, "{what} listening");
            Some(listener)
        }
        Err(e) => {
            tracing::error!(laddr, error = format!("{e:#}"), "{what} not started");
            None
        }
    }
}

/// `tcp://host:port` / `host:port` → socket address (`localhost` resolved).
pub fn socket_addr(laddr: &str) -> anyhow::Result<SocketAddr> {
    let a = laddr
        .strip_prefix("tcp://")
        .unwrap_or(laddr)
        .replace("localhost", "127.0.0.1");
    a.parse()
        .with_context(|| format!("invalid listen address {laddr:?}"))
}

/// The node id (`p2p.PubKeyToID`) of `config/node_key.json`.
fn node_id(home: &Path) -> String {
    use base64::Engine as _;
    let Ok(bz) = std::fs::read(home.join("config").join("node_key.json")) else {
        return String::new();
    };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bz) else {
        return String::new();
    };
    let Some(Ok(key)) = v["priv_key"]["value"]
        .as_str()
        .map(|s| base64::engine::general_purpose::STANDARD.decode(s))
    else {
        return String::new();
    };
    if key.len() != 64 {
        return String::new();
    }
    hex::encode(crypto::ed25519::address(
        &key[32..].try_into().unwrap_or([0; 32]),
    ))
}

/// Node status shared between the consensus loop and the RPC.
#[derive(Default)]
pub struct NodeShared {
    /// Own node info.
    pub info: NodeInfo,
    /// Round state JSON (`dump_consensus_state`, `consensus_state`).
    pub round_state: Mutex<(String, String)>,
    /// Block / state sync running.
    pub syncing: AtomicBool,
    /// The P2P switch (`net_info`, `dial_seeds`, `dial_peers`).
    pub switch: std::sync::OnceLock<p2p::Switch>,
    /// The consensus reactor (`WaitSync`, peer round states).
    pub consensus: std::sync::OnceLock<Arc<consensus::reactor::Reactor>>,
    /// The block propagation reactor (`IsCatchingUp`).
    pub propagation: std::sync::OnceLock<Arc<propagation::Reactor>>,
}

impl std::fmt::Debug for NodeShared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeShared")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

/// Go's `p2p.ConnectionStatus` JSON.
fn connection_status_json(st: &p2p::mconnection::ConnectionStatus) -> String {
    let mon = |m: &p2p::flowrate::MonitorStatus| {
        serde_json::json!({
            "Start": "0001-01-01T00:00:00Z",
            "Bytes": m.bytes.to_string(),
            "Samples": "0",
            "InstRate": "0",
            "CurRate": "0",
            "AvgRate": format!("{}", m.avg_rate.round()),
            "PeakRate": "0",
            "BytesRem": "0",
            "Duration": m.duration.as_nanos().to_string(),
            "Idle": "0",
            "TimeRem": "0",
            "Progress": 0,
            "Active": true,
        })
    };
    serde_json::json!({
        "Duration": st.duration.as_nanos().to_string(),
        "SendMonitor": mon(&st.send_monitor),
        "RecvMonitor": mon(&st.recv_monitor),
        "Channels": st.channels.iter().map(|c| serde_json::json!({
            "ID": c.id,
            "SendQueueCapacity": c.send_queue_capacity.to_string(),
            "SendQueueSize": c.send_queue_size.to_string(),
            "Priority": c.priority.to_string(),
            "RecentlySent": c.recently_sent.to_string(),
        })).collect::<Vec<_>>(),
    })
    .to_string()
}

/// `dial_seeds` / `dial_peers` take a list (JSON array or comma
/// separated).
fn split_addrs(s: &str) -> Vec<String> {
    s.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|a| a.trim().trim_matches('"').to_owned())
        .filter(|a| !a.is_empty())
        .collect()
}

impl rpc::NodeStatus for NodeShared {
    fn node_info(&self) -> NodeInfo {
        self.info.clone()
    }
    fn is_listening(&self) -> bool {
        true
    }
    fn listeners(&self) -> Vec<String> {
        vec![format!("Listener(@{})", self.info.listen_addr)]
    }
    fn peers(&self) -> Vec<rpc::PeerInfo> {
        let Some(sw) = self.switch.get() else {
            return Vec::new();
        };
        sw.peers()
            .iter()
            .map(|p| {
                let ni = p.node_info();
                rpc::PeerInfo {
                    node_info: NodeInfo {
                        p2p_version: ni.protocol_version.p2p,
                        block_version: ni.protocol_version.block,
                        app_version: ni.protocol_version.app,
                        id: ni.id.clone(),
                        listen_addr: ni.listen_addr.clone(),
                        network: ni.network.clone(),
                        version: ni.version.clone(),
                        channels: ni.channels.clone(),
                        moniker: ni.moniker.clone(),
                        tx_index: ni.other.tx_index.clone(),
                        rpc_address: ni.other.rpc_address.clone(),
                    },
                    is_outbound: p.is_outbound(),
                    connection_status: connection_status_json(&p.status()),
                    remote_ip: p.remote_ip().to_string(),
                    socket_addr: p.remote_addr().to_string(),
                    peer_state: None,
                }
            })
            .collect()
    }
    fn wait_sync(&self) -> bool {
        self.syncing.load(Ordering::SeqCst) || self.consensus.get().is_some_and(|r| r.wait_sync())
    }
    fn is_catching_up(&self) -> bool {
        self.wait_sync()
            || self
                .propagation
                .get()
                .is_some_and(|p| propagation::Propagator::is_catching_up(&**p))
    }
    fn dial_seeds(&self, seeds: &str) -> Result<String, String> {
        let seeds = split_addrs(seeds);
        if seeds.is_empty() {
            return Err("no seeds provided".into());
        }
        let sw = self.switch.get().ok_or("p2p is disabled")?;
        sw.dial_peers_async(&seeds).map_err(|e| e.to_string())?;
        Ok("Dialing seeds in progress. See /net_info for details".into())
    }
    fn dial_peers(
        &self,
        peers: &str,
        persistent: bool,
        unconditional: bool,
        private: bool,
    ) -> Result<String, String> {
        let peers = split_addrs(peers);
        if peers.is_empty() {
            return Err("no peers provided".into());
        }
        let sw = self.switch.get().ok_or("p2p is disabled")?;
        let ids: Vec<String> = peers
            .iter()
            .filter_map(|a| a.split('@').next().map(str::to_owned))
            .collect();
        if persistent {
            sw.add_persistent_peers(&peers).map_err(|e| e.to_string())?;
        }
        if unconditional {
            sw.add_unconditional_peer_ids(&ids)
                .map_err(|e| e.to_string())?;
        }
        if private {
            sw.add_private_peer_ids(&ids).map_err(|e| e.to_string())?;
        }
        sw.dial_peers_async(&peers).map_err(|e| e.to_string())?;
        Ok("Dialing peers in progress. See /net_info for details".into())
    }
    fn round_state_json(&self) -> String {
        self.lock().0.clone()
    }
    fn round_state_simple_json(&self) -> String {
        self.lock().1.clone()
    }
}

impl NodeShared {
    fn lock(&self) -> std::sync::MutexGuard<'_, (String, String)> {
        self.round_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Refreshes the round state from consensus.
    pub fn update_round_state(&self, rs: &consensus::RoundState) {
        let simple = rpc::json::J::obj([
            (
                "height/round/step",
                rpc::json::J::str(format!("{}/{}/{}", rs.height, rs.round, rs.step.as_u32())),
            ),
            ("start_time", rpc::json::J::time(&rs.start_time)),
            (
                "proposal_block_hash",
                rpc::json::J::hex(
                    &rs.proposal_block
                        .as_ref()
                        .map_or_else(Vec::new, |b| b.clone().hash().unwrap_or_default()),
                ),
            ),
            (
                "locked_block_hash",
                rpc::json::J::hex(
                    &rs.locked_block
                        .as_ref()
                        .map_or_else(Vec::new, |b| b.clone().hash().unwrap_or_default()),
                ),
            ),
            (
                "valid_block_hash",
                rpc::json::J::hex(
                    &rs.valid_block
                        .as_ref()
                        .map_or_else(Vec::new, |b| b.clone().hash().unwrap_or_default()),
                ),
            ),
            ("height_vote_set", rpc::json::J::Arr(Vec::new())),
            (
                "proposer",
                rs.validators.proposer().map_or(rpc::json::J::Null, |p| {
                    rpc::json::J::obj([
                        ("address", rpc::json::J::hex(&p.address)),
                        (
                            "index",
                            rpc::json::J::num(
                                rs.validators
                                    .get_by_address(&p.address)
                                    .map_or(-1, |(i, _)| i),
                            ),
                        ),
                    ])
                }),
            ),
        ]);
        let full = rpc::json::J::obj([
            ("height", rpc::json::J::i64s(rs.height)),
            ("round", rpc::json::J::num(rs.round)),
            ("step", rpc::json::J::num(rs.step.as_u32())),
            ("start_time", rpc::json::J::time(&rs.start_time)),
            ("commit_time", rpc::json::J::time(&rs.commit_time)),
            ("validators", rpc::encode::validator_set(&rs.validators)),
            ("locked_round", rpc::json::J::num(rs.locked_round)),
            ("valid_round", rpc::json::J::num(rs.valid_round)),
            ("commit_round", rpc::json::J::num(rs.commit_round)),
            (
                "last_validators",
                rpc::encode::validator_set(&rs.last_validators),
            ),
            (
                "triggered_timeout_precommit",
                rpc::json::J::Bool(rs.triggered_timeout_precommit),
            ),
        ]);
        *self.lock() = (full.encode(), simple.encode());
    }
}

/// The app's ABCI query / mempool connections.
struct AppQueries(Arc<Mutex<app::App>>);

impl AppQuery for AppQueries {
    fn info(&self) -> Result<abci::ResponseInfo, String> {
        Ok(self
            .0
            .lock()
            .map_err(|_| "app lock poisoned".to_owned())?
            .info())
    }
    fn query(&self, req: abci::RequestQuery) -> Result<abci::ResponseQuery, String> {
        Ok(self
            .0
            .lock()
            .map_err(|_| "app lock poisoned".to_owned())?
            .query(&req))
    }
    fn check_tx(&self, tx: &[u8]) -> Result<abci::ResponseCheckTx, String> {
        self.0
            .lock()
            .map_err(|_| "app lock poisoned".to_owned())?
            .check_tx(&abci::RequestCheckTx {
                tx: tx.to_vec(),
                ..Default::default()
            })
    }
}

/// The node's evidence pool as seen by `broadcast_evidence`.
struct Evidence(evidence::Pool);

impl rpc::EvidenceSink for Evidence {
    fn add_evidence(&self, ev: ctypes::Evidence) -> Result<(), String> {
        self.0.add_evidence(ev).map_err(|e| e.to_string())
    }
}

/// Everything the node needs to serve RPC.
pub struct Rpc {
    /// The environment.
    pub env: Arc<Environment>,
    /// Block event sink for the executor.
    pub publisher: Arc<BlockEventPublisher>,
    /// Node status (updated by the consensus loop).
    pub shared: Arc<NodeShared>,
    /// Settings.
    pub settings: RpcSettings,
    app: Arc<Mutex<app::App>>,
    /// Owned runtime when `start` runs outside of one (tests).
    runtime: Option<tokio::runtime::Runtime>,
}

impl std::fmt::Debug for Rpc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rpc").finish_non_exhaustive()
    }
}

/// Inputs of [`Rpc::new`].
#[derive(Debug)]
pub struct RpcInputs<'a> {
    /// Node home.
    pub home: &'a Path,
    /// Genesis.
    pub genesis: &'a GenesisDoc,
    /// Block store.
    pub block_store: BlockStore,
    /// State store.
    pub state_store: StateStore,
    /// The app.
    pub app: Arc<Mutex<app::App>>,
    /// The mempool.
    pub mempool: Arc<mempool::TxPool>,
    /// Evidence pool shared with consensus and the evidence reactor.
    pub evidence: evidence::Pool,
    /// Validator key.
    pub pub_key: ctypes::PubKey,
    /// Current app version (node info).
    pub app_version: u64,
}

impl Rpc {
    /// Builds the environment: indexers on `data/tx_index`, event bus,
    /// genesis document.
    pub fn new(i: RpcInputs<'_>) -> anyhow::Result<Self> {
        let settings = RpcSettings::load(i.home)?;
        let (txi, bi) = if settings.tx_index {
            let db = kvdb::open_backend(
                &i.home.join("data"),
                "tx_index",
                &kvdb::DbOptions::default(),
            )
            .map_err(|e| anyhow!("open tx_index: {e}"))?;
            (
                Some(Arc::new(TxIndexer::new(db.clone()))),
                Some(Arc::new(BlockIndexer::new(db))),
            )
        } else {
            (None, None)
        };
        let genesis_bytes = std::fs::read(i.home.join("config").join("genesis.json"))?;
        let genesis = GenesisData::from_file(&genesis_bytes, i.genesis).map_err(|e| anyhow!(e))?;
        let bus = Arc::new(EventBus::new());
        let shared = Arc::new(NodeShared {
            info: NodeInfo {
                p2p_version: 8,
                block_version: 11,
                app_version: i.app_version,
                id: node_id(i.home),
                listen_addr: settings.p2p_laddr.clone(),
                network: i.genesis.chain_id.clone(),
                version: "0.38.17".into(),
                channels: hex::decode("40202223303860613231505100").unwrap_or_default(),
                moniker: settings.moniker.clone(),
                tx_index: if settings.tx_index { "on" } else { "off" }.into(),
                rpc_address: settings.rpc.laddr.clone(),
            },
            ..NodeShared::default()
        });
        let publisher = Arc::new(BlockEventPublisher {
            bus: bus.clone(),
            tx_indexer: txi.clone(),
            block_indexer: bi.clone(),
        });
        let env = Environment::new(
            i.block_store,
            i.state_store.clone(),
            Arc::new(AppQueries(i.app.clone())),
            i.mempool,
            shared.clone(),
            Arc::new(Evidence(i.evidence)),
            i.pub_key,
            genesis,
            txi,
            bi,
            bus,
            settings.rpc.clone(),
        );
        Ok(Self {
            env: Arc::new(env),
            publisher,
            shared,
            settings,
            app: i.app,
            runtime: None,
        })
    }

    /// Starts the listeners on the current tokio runtime (or on an owned
    /// one when called outside of a runtime).
    pub fn spawn(&mut self) -> anyhow::Result<()> {
        let s = &self.settings;
        if s.rpc.laddr.trim().is_empty()
            && s.rpc.grpc_laddr.is_empty()
            && s.app_grpc.is_empty()
            && s.app_api.is_empty()
        {
            return Ok(());
        }
        let rt = if let Ok(h) = tokio::runtime::Handle::try_current() {
            h
        } else {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()?;
            let h = runtime.handle().clone();
            self.runtime = Some(runtime);
            h
        };
        let rt = &rt;
        let _guard = rt.enter();
        for laddr in self
            .settings
            .rpc
            .laddr
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let Some(listener) = bind("RPC server", laddr) else {
                continue;
            };
            let env = self.env.clone();
            rt.spawn(async move {
                if let Err(e) = rpc::server::serve(listener, env).await {
                    tracing::error!(error = %e, "RPC server failed");
                }
            });
        }
        if !self.settings.rpc.grpc_laddr.is_empty() {
            if let Some(listener) = bind("celestia-core gRPC", &self.settings.rpc.grpc_laddr) {
                let env = self.env.clone();
                rt.spawn(async move {
                    if let Err(e) = rpc::grpc::serve(listener, env).await {
                        tracing::error!(error = %e, "gRPC server failed");
                    }
                });
            }
        }
        let abci = Arc::new(app::AbciApp::from_shared(self.app.clone()));
        if !self.settings.app_grpc.is_empty() {
            if let Some(listener) =
                bind("gRPC query server (with BlockAPI)", &self.settings.app_grpc)
            {
                let router = rpc::grpc::router(self.env.clone())
                    .merge(api_server::grpc_router(abci.clone()));
                rt.spawn(async move {
                    if let Err(e) = axum::serve(listener, router).await {
                        tracing::error!(error = %e, "app gRPC server failed");
                    }
                });
            }
        }
        if !self.settings.app_api.is_empty() {
            if let Some(listener) = bind("REST API", &self.settings.app_api) {
                rt.spawn(async move {
                    if let Err(e) = api_server::serve_rest_on(listener, abci).await {
                        tracing::error!(error = %e, "REST API failed");
                    }
                });
            }
        }
        Ok(())
    }

    /// Publishes a consensus event on the bus and refreshes the round state.
    pub fn consensus_event(&self, ev: &consensus::Event, rs: &consensus::RoundState) {
        let bus = &self.env.event_bus;
        match ev {
            consensus::Event::NewRoundStep {
                height,
                round,
                step,
                ..
            } => {
                bus.publish_type(
                    "NewRoundStep",
                    EventData::RoundState {
                        height: *height,
                        round: *round,
                        step: step.as_str().into(),
                    },
                );
                self.shared.update_round_state(rs);
            }
            consensus::Event::Vote(v) => bus.publish_type(
                "Vote",
                EventData::Vote {
                    vote: Box::new(v.clone()),
                },
            ),
            consensus::Event::ValidBlock { height, round, .. } => bus.publish_type(
                "ValidBlock",
                EventData::RoundState {
                    height: *height,
                    round: *round,
                    step: rs.step.as_str().into(),
                },
            ),
            _ => {}
        }
    }
}
