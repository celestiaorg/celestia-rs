//! `rpc/core`: the RPC environment and the route handlers (`routes.go`).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ctypes::proto::abci;
use ctypes::proto::types as ptypes;
use ctypes::{Evidence, PubKey};
use nodestore::{BlockStore, StateStore};
use prost::Message as _;

use crate::encode;
use crate::events::EventBus;
use crate::indexer::{BlockIndexer, TxIndexer};
use crate::json::J;
use crate::params::{Arg, ArgType};
use crate::query::Query;

const DEFAULT_PER_PAGE: i128 = 30;
const MAX_PER_PAGE: i128 = 100;
/// `maxQueryLength`.
pub const MAX_QUERY_LENGTH: usize = 512;
const GENESIS_CHUNK_SIZE: usize = 16 * 1024 * 1024;
const DATA_COMMITMENT_BLOCKS_LIMIT: u64 = 10_000;
/// `MaxTxStatusBatchSize`.
pub const MAX_TX_STATUS_BATCH_SIZE: usize = 20;

/// The ABCI query / mempool connections to the application.
pub trait AppQuery: Send + Sync {
    /// `Info` (with `proxy.RequestInfo`).
    fn info(&self) -> Result<abci::ResponseInfo, String>;
    /// `Query`.
    fn query(&self, req: abci::RequestQuery) -> Result<abci::ResponseQuery, String>;
    /// Query envelope, preserving a nil response independently from an error or
    /// a present default Query. Adapters over Go-compatible apps override this.
    fn query_response(
        &self,
        req: abci::RequestQuery,
    ) -> Result<Option<abci::ResponseQuery>, String> {
        self.query(req).map(Some)
    }
    /// `CheckTx` on the mempool connection (without adding to the pool).
    fn check_tx(&self, tx: &[u8]) -> Result<abci::ResponseCheckTx, String>;
}

/// `p2p.DefaultNodeInfo`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeInfo {
    /// P2P protocol version.
    pub p2p_version: u64,
    /// Block protocol version.
    pub block_version: u64,
    /// App version (overridden by the latest consensus params).
    pub app_version: u64,
    /// Node id.
    pub id: String,
    /// Listen address.
    pub listen_addr: String,
    /// Network (chain id).
    pub network: String,
    /// Software version.
    pub version: String,
    /// Channels.
    pub channels: Vec<u8>,
    /// Moniker.
    pub moniker: String,
    /// `other.tx_index` (`on`/`off`).
    pub tx_index: String,
    /// `other.rpc_address`.
    pub rpc_address: String,
}

impl NodeInfo {
    /// JSON.
    #[must_use]
    pub fn to_json(&self) -> J {
        J::obj([
            (
                "protocol_version",
                J::obj([
                    ("p2p", J::u64s(self.p2p_version)),
                    ("block", J::u64s(self.block_version)),
                    ("app", J::u64s(self.app_version)),
                ]),
            ),
            ("id", J::str(&self.id)),
            ("listen_addr", J::str(&self.listen_addr)),
            ("network", J::str(&self.network)),
            ("version", J::str(&self.version)),
            ("channels", J::hex(&self.channels)),
            ("moniker", J::str(&self.moniker)),
            (
                "other",
                J::obj([
                    ("tx_index", J::str(&self.tx_index)),
                    ("rpc_address", J::str(&self.rpc_address)),
                ]),
            ),
        ])
    }
}

/// A connected peer (`net_info`).
#[derive(Debug, Clone, Default)]
pub struct PeerInfo {
    /// Its node info.
    pub node_info: NodeInfo,
    /// Outbound connection.
    pub is_outbound: bool,
    /// Connection status (Go `p2p.ConnectionStatus` JSON, pre-encoded).
    pub connection_status: String,
    /// Remote IP.
    pub remote_ip: String,
    /// `host:port` (dump_consensus_state).
    pub socket_addr: String,
    /// Peer round state JSON (dump_consensus_state), if known.
    pub peer_state: Option<String>,
}

/// Node-level state the RPC reads: P2P, consensus, sync status.
pub trait NodeStatus: Send + Sync {
    /// Own node info.
    fn node_info(&self) -> NodeInfo;
    /// `IsListening`.
    fn is_listening(&self) -> bool;
    /// `Listeners`.
    fn listeners(&self) -> Vec<String>;
    /// Connected peers.
    fn peers(&self) -> Vec<PeerInfo>;
    /// `ConsensusReactor.WaitSync` (block / state sync running).
    fn wait_sync(&self) -> bool;
    /// `ConsensusReactor.IsCatchingUp`.
    fn is_catching_up(&self) -> bool;
    /// `DialSeeds` (unsafe): the result log line.
    fn dial_seeds(&self, seeds: &str) -> Result<String, String> {
        let _ = seeds;
        Err("dialing is not supported by this node".into())
    }
    /// `DialPeers` (unsafe): the result log line.
    fn dial_peers(
        &self,
        peers: &str,
        persistent: bool,
        unconditional: bool,
        private: bool,
    ) -> Result<String, String> {
        let _ = (peers, persistent, unconditional, private);
        Err("dialing is not supported by this node".into())
    }
    /// `GetRoundStateJSON` (dump_consensus_state).
    fn round_state_json(&self) -> String;
    /// `GetRoundStateSimpleJSON` (consensus_state).
    fn round_state_simple_json(&self) -> String;
}

/// The evidence pool (`broadcast_evidence`).
pub trait EvidenceSink: Send + Sync {
    /// `AddEvidence`.
    fn add_evidence(&self, ev: Evidence) -> Result<(), String>;
}

/// `config.RPCConfig` (the fields the server uses).
#[derive(Debug, Clone)]
pub struct RpcConfig {
    /// `laddr` (`tcp://host:port`, comma separated).
    pub laddr: String,
    /// `grpc_laddr` (celestia-core gRPC services; empty disables).
    pub grpc_laddr: String,
    /// `cors_allowed_origins`.
    pub cors_allowed_origins: Vec<String>,
    /// `cors_allowed_methods`.
    pub cors_allowed_methods: Vec<String>,
    /// `cors_allowed_headers`.
    pub cors_allowed_headers: Vec<String>,
    /// `max_open_connections` (0 = unlimited).
    pub max_open_connections: usize,
    /// `grpc_max_open_connections`.
    pub grpc_max_open_connections: usize,
    /// `unsafe`.
    pub unsafe_routes: bool,
    /// `max_subscription_clients`.
    pub max_subscription_clients: usize,
    /// `max_subscriptions_per_client`.
    pub max_subscriptions_per_client: usize,
    /// `experimental_subscription_buffer_size`.
    pub subscription_buffer_size: usize,
    /// `experimental_websocket_write_buffer_size`.
    pub websocket_write_buffer_size: usize,
    /// `experimental_close_on_slow_client`.
    pub close_on_slow_client: bool,
    /// `timeout_broadcast_tx_commit` (ms).
    pub timeout_broadcast_tx_commit_ms: u64,
    /// `max_request_batch_size`.
    pub max_request_batch_size: usize,
    /// `max_body_bytes`.
    pub max_body_bytes: usize,
    /// `max_header_bytes` (0 uses Go `net/http`'s 1 MiB default).
    pub max_header_bytes: usize,
    /// Resolved `tls_cert_file` (empty unless configured).
    pub tls_cert_file: String,
    /// Resolved `tls_key_file` (empty unless configured).
    pub tls_key_file: String,
    /// `max_concurrent_heavy_requests` (0 = default 20, <0 = unlimited).
    pub max_concurrent_heavy_requests: i64,
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            laddr: "tcp://127.0.0.1:26657".into(),
            grpc_laddr: String::new(),
            cors_allowed_origins: Vec::new(),
            cors_allowed_methods: vec!["HEAD".into(), "GET".into(), "POST".into()],
            cors_allowed_headers: vec![
                "Origin".into(),
                "Accept".into(),
                "Content-Type".into(),
                "X-Requested-With".into(),
                "X-Server-Time".into(),
            ],
            max_open_connections: 900,
            grpc_max_open_connections: 900,
            unsafe_routes: false,
            max_subscription_clients: 100,
            max_subscriptions_per_client: 5,
            subscription_buffer_size: 200,
            websocket_write_buffer_size: 200,
            close_on_slow_client: false,
            timeout_broadcast_tx_commit_ms: 10_000,
            max_request_batch_size: 10,
            max_body_bytes: 1_000_000,
            max_header_bytes: 1 << 20,
            tls_cert_file: String::new(),
            tls_key_file: String::new(),
            max_concurrent_heavy_requests: 20,
        }
    }
}

impl RpcConfig {
    /// Go enables RPC TLS only when both file fields are nonempty.
    #[must_use]
    pub fn tls_enabled(&self) -> bool {
        !self.tls_cert_file.is_empty() && !self.tls_key_file.is_empty()
    }
}

/// The genesis document as served (`cmtjson.Marshal(GenesisDoc)` and its
/// base64 chunks).
#[derive(Debug, Clone, Default)]
pub struct GenesisData {
    /// The JSON document.
    pub json: String,
    /// `genChunks`.
    pub chunks: Vec<String>,
}

impl GenesisData {
    /// From the JSON document (`InitGenesisChunks`).
    #[must_use]
    pub fn new(json: String) -> Self {
        use base64::Engine as _;
        let chunks = json
            .as_bytes()
            .chunks(GENESIS_CHUNK_SIZE)
            .map(|c| base64::engine::general_purpose::STANDARD.encode(c))
            .collect();
        Self { json, chunks }
    }

    /// Builds the served document from a genesis file (SDK `AppGenesis` or
    /// CometBFT `GenesisDoc`): the validated fields plus `app_state`
    /// compacted from the file's raw bytes (key order preserved).
    pub fn from_file(bytes: &[u8], doc: &nodestore::GenesisDoc) -> Result<Self, String> {
        let text = String::from_utf8_lossy(bytes);
        let root = crate::json::parse(&text)?;
        let mut o = J::obj([
            ("genesis_time", J::time(&doc.genesis_time)),
            ("chain_id", J::str(&doc.chain_id)),
            ("initial_height", J::i64s(doc.initial_height)),
            (
                "consensus_params",
                encode::consensus_params(&doc.consensus_params),
            ),
        ]);
        if !doc.validators.is_empty() {
            o.push(
                "validators",
                J::Arr(
                    doc.validators
                        .iter()
                        .map(|v| {
                            J::obj([
                                ("address", J::hex(&v.address)),
                                ("pub_key", encode::pub_key(&v.pub_key)),
                                ("power", J::i64s(v.power)),
                                ("name", J::str(&v.name)),
                            ])
                        })
                        .collect(),
                ),
            );
        }
        o.push("app_hash", J::hex(&doc.app_hash));
        if let Some(app_state) = root.get("app_state")
            && !app_state.raw.is_empty()
        {
            o.push("app_state", J::Raw(crate::json::compact(app_state.raw)));
        }
        Ok(Self::new(o.encode()))
    }
}

/// `rpc/core.Environment`.
pub struct Environment {
    /// Block store.
    pub block_store: BlockStore,
    /// State store.
    pub state_store: StateStore,
    /// Application query/mempool connections.
    pub app: Arc<dyn AppQuery>,
    /// Mempool.
    pub mempool: Arc<dyn mempool::Mempool>,
    /// Node status (P2P, consensus).
    pub node: Arc<dyn NodeStatus>,
    /// Evidence pool.
    pub evidence: Arc<dyn EvidenceSink>,
    /// The node's validator key (`status`).
    pub pub_key: PubKey,
    /// Genesis.
    pub genesis: GenesisData,
    /// Tx indexer (`None` = `indexer = "null"`).
    pub tx_indexer: Option<Arc<TxIndexer>>,
    /// Block indexer.
    pub block_indexer: Option<Arc<BlockIndexer>>,
    /// Event bus.
    pub event_bus: Arc<EventBus>,
    /// Config.
    pub config: RpcConfig,
    heavy_in_flight: Arc<AtomicUsize>,
}

impl std::fmt::Debug for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Environment").finish_non_exhaustive()
    }
}

/// Observed Go RPC recovery for dereferencing an absent ABCI Query response.
pub(crate) const ERR_EMPTY_QUERY_RESPONSE: &str =
    "runtime error: invalid memory address or nil pointer dereference";

/// `errHeavyRequestLimit`.
pub const ERR_HEAVY_LIMIT: &str =
    "server busy: too many concurrent heavy RPC requests, retry later";

/// A route definition (`RPCFunc`).
#[derive(Debug, Clone, Copy)]
pub struct Route {
    /// Name.
    pub name: &'static str,
    /// Argument names and types.
    pub args: &'static [(&'static str, ArgType)],
    /// WebSocket only.
    pub ws: bool,
    /// `Cacheable(noCacheDefArgs...)`.
    pub cacheable: Option<&'static [&'static str]>,
    /// Bounded by the heavy-request semaphore.
    pub heavy: bool,
}

use ArgType as T;

const fn route(name: &'static str, args: &'static [(&'static str, ArgType)]) -> Route {
    Route {
        name,
        args,
        ws: false,
        cacheable: None,
        heavy: false,
    }
}

const fn cached(mut r: Route, no_cache: &'static [&'static str]) -> Route {
    r.cacheable = Some(no_cache);
    r
}

const fn heavy(mut r: Route) -> Route {
    r.heavy = true;
    r
}

const fn ws(mut r: Route) -> Route {
    r.ws = true;
    r
}

/// `GetRoutes` (celestia-core v0.42.3).
pub const ROUTES: &[Route] = &[
    ws(route("subscribe", &[("query", T::Str)])),
    ws(route("unsubscribe", &[("query", T::Str)])),
    ws(route("unsubscribe_all", &[])),
    route("health", &[]),
    route("status", &[]),
    route("net_info", &[]),
    cached(
        route(
            "blockchain",
            &[("minHeight", T::I64), ("maxHeight", T::I64)],
        ),
        &[],
    ),
    heavy(cached(route("genesis", &[]), &[])),
    heavy(cached(route("genesis_chunked", &[("chunk", T::Uint)]), &[])),
    heavy(cached(
        route("block", &[("height", T::I64Ptr)]),
        &["height"],
    )),
    heavy(cached(route("block_by_hash", &[("hash", T::Bytes)]), &[])),
    heavy(cached(
        route("block_results", &[("height", T::I64Ptr)]),
        &["height"],
    )),
    cached(route("commit", &[("height", T::I64Ptr)]), &["height"]),
    cached(route("header", &[("height", T::I64Ptr)]), &["height"]),
    cached(route("header_by_hash", &[("hash", T::HexBytes)]), &[]),
    route("check_tx", &[("tx", T::Bytes)]),
    heavy(cached(
        route("tx", &[("hash", T::Bytes), ("prove", T::Bool)]),
        &[],
    )),
    heavy(route(
        "tx_search",
        &[
            ("query", T::Str),
            ("prove", T::Bool),
            ("page", T::IntPtr),
            ("per_page", T::IntPtr),
            ("order_by", T::Str),
        ],
    )),
    heavy(route(
        "block_search",
        &[
            ("query", T::Str),
            ("page", T::IntPtr),
            ("per_page", T::IntPtr),
            ("order_by", T::Str),
        ],
    )),
    cached(
        route(
            "validators",
            &[
                ("height", T::I64Ptr),
                ("page", T::IntPtr),
                ("per_page", T::IntPtr),
            ],
        ),
        &["height"],
    ),
    route("dump_consensus_state", &[]),
    route("consensus_state", &[]),
    cached(
        route("consensus_params", &[("height", T::I64Ptr)]),
        &["height"],
    ),
    heavy(route("unconfirmed_txs", &[("limit", T::IntPtr)])),
    route("num_unconfirmed_txs", &[]),
    route("broadcast_tx_commit", &[("tx", T::Bytes)]),
    route("broadcast_tx_sync", &[("tx", T::Bytes)]),
    route("broadcast_tx_async", &[("tx", T::Bytes)]),
    route(
        "abci_query",
        &[
            ("path", T::Str),
            ("data", T::HexBytes),
            ("height", T::I64),
            ("prove", T::Bool),
        ],
    ),
    cached(route("abci_info", &[]), &[]),
    route("broadcast_evidence", &[("evidence", T::Evidence)]),
    heavy(route(
        "prove_shares",
        &[
            ("height", T::I64),
            ("startShare", T::U64),
            ("endShare", T::U64),
        ],
    )),
    heavy(route(
        "prove_shares_v2",
        &[
            ("height", T::I64),
            ("startShare", T::U64),
            ("endShare", T::U64),
        ],
    )),
    heavy(route(
        "data_root_inclusion_proof",
        &[("height", T::I64), ("start", T::U64), ("end", T::U64)],
    )),
    heavy(cached(
        route("signed_block", &[("height", T::I64Ptr)]),
        &["height"],
    )),
    heavy(route(
        "data_commitment",
        &[("start", T::U64), ("end", T::U64)],
    )),
    route("tx_status", &[("hash", T::Bytes)]),
    route("tx_status_batch", &[("hashes", T::BytesList)]),
];

/// `AddUnsafeRoutes`.
pub const UNSAFE_ROUTES: &[Route] = &[
    route("dial_seeds", &[("seeds", T::Str)]),
    route(
        "dial_peers",
        &[
            ("peers", T::Str),
            ("persistent", T::Bool),
            ("unconditional", T::Bool),
            ("private", T::Bool),
        ],
    ),
    route("unsafe_flush_mempool", &[]),
];

impl Route {
    /// `cacheableWithArgs`: `args` are the decoded arguments (fewer than
    /// declared when a JSON request had no params).
    #[must_use]
    pub fn cacheable_with(&self, args: &[Arg]) -> bool {
        let Some(no_cache) = self.cacheable else {
            return false;
        };
        for (i, (name, _)) in self.args.iter().enumerate() {
            if no_cache.contains(name) {
                match args.get(i) {
                    None => return false,
                    Some(a) if a.is_zero() => return false,
                    _ => {}
                }
            }
        }
        true
    }
}

/// Finds a route.
#[must_use]
pub fn find_route(name: &str, unsafe_routes: bool) -> Option<&'static Route> {
    ROUTES
        .iter()
        .chain(if unsafe_routes { UNSAFE_ROUTES } else { &[] })
        .find(|r| r.name == name)
}

/// RAII release of a heavy-request slot.
#[derive(Debug)]
pub struct HeavyGuard(Option<Arc<AtomicUsize>>);

impl Drop for HeavyGuard {
    fn drop(&mut self) {
        if let Some(c) = &self.0 {
            c.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

fn s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn validate_page(page: Option<i128>, per_page: i128, total: i128) -> Result<i128, String> {
    let Some(page) = page else {
        return Ok(1);
    };
    let mut pages = ((total - 1) / per_page) + 1;
    if pages == 0 {
        pages = 1;
    }
    if page <= 0 || page > pages {
        return Err(format!(
            "page should be within [1, {pages}] range, given {page}"
        ));
    }
    Ok(page)
}

fn validate_per_page(per_page: Option<i128>) -> i128 {
    match per_page {
        None => DEFAULT_PER_PAGE,
        Some(p) if p < 1 => DEFAULT_PER_PAGE,
        Some(p) if p > MAX_PER_PAGE => MAX_PER_PAGE,
        Some(p) => p,
    }
}

fn validate_skip_count(page: i128, per_page: i128) -> i128 {
    ((page - 1) * per_page).max(0)
}

fn usize_of(v: i128) -> usize {
    usize::try_from(v.max(0)).unwrap_or(usize::MAX)
}

/// Go `%X` of bytes.
fn hx(b: &[u8]) -> String {
    hex::encode_upper(b)
}

/// `types.EncodeDataRootTuple`.
fn encode_data_root_tuple(height: u64, data_root: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; 32];
    out[24..].copy_from_slice(&height.to_be_bytes());
    out.extend_from_slice(data_root);
    out
}

impl Environment {
    /// A new environment.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        block_store: BlockStore,
        state_store: StateStore,
        app: Arc<dyn AppQuery>,
        mempool: Arc<dyn mempool::Mempool>,
        node: Arc<dyn NodeStatus>,
        evidence: Arc<dyn EvidenceSink>,
        pub_key: PubKey,
        genesis: GenesisData,
        tx_indexer: Option<Arc<TxIndexer>>,
        block_indexer: Option<Arc<BlockIndexer>>,
        event_bus: Arc<EventBus>,
        config: RpcConfig,
    ) -> Self {
        Self {
            block_store,
            state_store,
            app,
            mempool,
            node,
            evidence,
            pub_key,
            genesis,
            tx_indexer,
            block_indexer,
            event_bus,
            config,
            heavy_in_flight: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// `tryAcquire` for heavy routes: `None` when the limit is reached.
    #[must_use]
    pub fn try_acquire(&self, route: &Route) -> Option<HeavyGuard> {
        if !route.heavy || self.config.max_concurrent_heavy_requests < 0 {
            return Some(HeavyGuard(None));
        }
        let limit = if self.config.max_concurrent_heavy_requests == 0 {
            20
        } else {
            usize::try_from(self.config.max_concurrent_heavy_requests).unwrap_or(usize::MAX)
        };
        let prev = self.heavy_in_flight.fetch_add(1, Ordering::SeqCst);
        if prev >= limit {
            self.heavy_in_flight.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(HeavyGuard(Some(Arc::clone(&self.heavy_in_flight))))
    }

    fn latest_uncommitted_height(&self) -> i64 {
        if self.node.wait_sync() {
            self.block_store.height()
        } else {
            self.block_store.height() + 1
        }
    }

    fn get_height(&self, latest: i64, h: Option<i128>) -> Result<i64, String> {
        let Some(h) = h else {
            return Ok(latest);
        };
        if h <= 0 {
            return Err(format!("height must be greater than 0, but got {h}"));
        }
        if h > i128::from(latest) {
            return Err(format!(
                "height {h} must be less than or equal to the current blockchain height {latest}"
            ));
        }
        let base = self.block_store.base();
        if h < i128::from(base) {
            return Err(format!(
                "height {h} is not available, lowest height is {base}"
            ));
        }
        Ok(i64::try_from(h).unwrap_or(i64::MAX))
    }

    /// Dispatches a non-WebSocket, synchronous route. `subscribe`,
    /// `unsubscribe*` and `broadcast_tx_commit` are handled by the server.
    pub fn call(&self, name: &str, a: &[Arg]) -> Result<J, String> {
        let arg = |i: usize| a.get(i).cloned().unwrap_or(Arg::Null);
        match name {
            "health" => Ok(J::Obj(Vec::new())),
            "status" => self.status(),
            "net_info" => Ok(self.net_info()),
            "blockchain" => self.blockchain_info(arg(0).int(), arg(1).int()),
            "genesis" => self.genesis(),
            "genesis_chunked" => self.genesis_chunked(arg(0).int()),
            "block" => self.block(arg(0).opt_int()),
            "block_by_hash" => self.block_by_hash(arg(0).bytes()),
            "block_results" => self.block_results(arg(0).opt_int()),
            "commit" => self.commit(arg(0).opt_int()),
            "header" => self.header(arg(0).opt_int()),
            "header_by_hash" => self.header_by_hash(arg(0).bytes()),
            "check_tx" => self.check_tx(arg(0).bytes()),
            "tx" => self.tx(arg(0).bytes(), arg(1).bool()),
            "tx_search" => self.tx_search(
                arg(0).str(),
                arg(1).bool(),
                arg(2).opt_int(),
                arg(3).opt_int(),
                arg(4).str(),
            ),
            "block_search" => self.block_search(
                arg(0).str(),
                arg(1).opt_int(),
                arg(2).opt_int(),
                arg(3).str(),
            ),
            "validators" => self.validators(arg(0).opt_int(), arg(1).opt_int(), arg(2).opt_int()),
            "dump_consensus_state" => Ok(self.dump_consensus_state()),
            "consensus_state" => Ok(J::obj([(
                "round_state",
                J::Raw(crate::json::compact(&self.node.round_state_simple_json())),
            )])),
            "consensus_params" => self.consensus_params(arg(0).opt_int()),
            "unconfirmed_txs" => Ok(self.unconfirmed_txs(arg(0).opt_int())),
            "num_unconfirmed_txs" => Ok(J::obj([
                (
                    "n_txs",
                    J::i64s(i64::try_from(self.mempool.size()).unwrap_or(0)),
                ),
                (
                    "total",
                    J::i64s(i64::try_from(self.mempool.size()).unwrap_or(0)),
                ),
                ("total_bytes", J::i64s(self.mempool.size_bytes())),
                ("txs", J::Null),
            ])),
            "broadcast_tx_sync" => self.broadcast_tx_sync(arg(0).bytes()),
            "broadcast_tx_async" => self.broadcast_tx_async(arg(0).bytes()),
            "abci_query" => self
                .abci_query_response(arg(0).str(), arg(1).bytes(), arg(2).int(), arg(3).bool())?
                .ok_or_else(|| ERR_EMPTY_QUERY_RESPONSE.to_owned()),
            "abci_info" => {
                let r = self.app.info()?;
                Ok(J::obj([("response", encode::response_info(&r))]))
            }
            "broadcast_evidence" => self.broadcast_evidence(&arg(0)),
            "prove_shares" => self.prove_shares(arg(0).int(), arg(1).int(), arg(2).int()),
            "prove_shares_v2" => self
                .prove_shares(arg(0).int(), arg(1).int(), arg(2).int())
                .map(|p| J::obj([("share_proof", p)])),
            "data_root_inclusion_proof" => {
                self.data_root_inclusion_proof(arg(0).int(), arg(1).int(), arg(2).int())
            }
            "signed_block" => self.signed_block(arg(0).opt_int()),
            "data_commitment" => self.data_commitment(arg(0).int(), arg(1).int()),
            "tx_status" => self.tx_status(arg(0).bytes()).map(|(j, _)| j),
            "tx_status_batch" => self.tx_status_batch(&arg(0)),
            "unsafe_flush_mempool" => {
                self.mempool.flush();
                Ok(J::Obj(Vec::new()))
            }
            "dial_seeds" => self
                .node
                .dial_seeds(arg(0).str())
                .map(|log| J::obj([("log", J::str(log))])),
            "dial_peers" => self
                .node
                .dial_peers(arg(0).str(), arg(1).bool(), arg(2).bool(), arg(3).bool())
                .map(|log| J::obj([("log", J::str(log))])),
            _ => Err(format!("unknown route {name}")),
        }
    }

    // --- info --------------------------------------------------------------

    fn validator_at_height(&self, h: i64) -> Option<ctypes::Validator> {
        let vals = self.state_store.load_validators(h).ok()?;
        vals.get_by_address(&self.pub_key.address()).map(|(_, v)| v)
    }

    /// `GetNodeInfo`: the app version from the latest consensus params.
    #[must_use]
    pub fn node_info(&self, latest_height: i64) -> NodeInfo {
        let mut ni = self.node.node_info();
        if let Ok(p) = self.state_store.load_consensus_params(latest_height) {
            ni.app_version = p.version.app;
        }
        ni
    }

    /// The `status` fields (shared with the gRPC `Status`).
    pub fn status_parts(&self) -> Result<StatusParts, String> {
        let mut st = StatusParts::default();
        if let Some(m) = self.block_store.load_base_meta().map_err(s)? {
            st.earliest_height = m.header.height;
            st.earliest_app_hash = m.header.app_hash.clone();
            st.earliest_hash = m.block_id.hash.clone();
            st.earliest_time = m.header.time;
        } else {
            st.earliest_time = ctypes::Time::from_unix(0, 0);
        }
        let latest = self.block_store.height();
        st.latest_time = ctypes::Time::from_unix(0, 0);
        if latest != 0
            && let Some(m) = self.block_store.load_block_meta(latest).map_err(s)?
        {
            st.latest_hash = m.block_id.hash.clone();
            st.latest_app_hash = m.header.app_hash.clone();
            st.latest_time = m.header.time;
        }
        st.latest_height = latest;
        st.voting_power = self
            .validator_at_height(self.latest_uncommitted_height())
            .map_or(0, |v| v.voting_power);
        st.node_info = self.node_info(latest);
        st.catching_up = self.node.wait_sync();
        Ok(st)
    }

    fn status(&self) -> Result<J, String> {
        let st = self.status_parts()?;
        Ok(J::obj([
            ("node_info", st.node_info.to_json()),
            (
                "sync_info",
                J::obj([
                    ("latest_block_hash", J::hex(&st.latest_hash)),
                    ("latest_app_hash", J::hex(&st.latest_app_hash)),
                    ("latest_block_height", J::i64s(st.latest_height)),
                    ("latest_block_time", J::time(&st.latest_time)),
                    ("earliest_block_hash", J::hex(&st.earliest_hash)),
                    ("earliest_app_hash", J::hex(&st.earliest_app_hash)),
                    ("earliest_block_height", J::i64s(st.earliest_height)),
                    ("earliest_block_time", J::time(&st.earliest_time)),
                    ("catching_up", J::Bool(st.catching_up)),
                ]),
            ),
            (
                "validator_info",
                J::obj([
                    ("address", J::hex(&self.pub_key.address())),
                    ("pub_key", encode::pub_key(&self.pub_key)),
                    ("voting_power", J::i64s(st.voting_power)),
                ]),
            ),
        ]))
    }

    fn net_info(&self) -> J {
        let peers = self.node.peers();
        J::obj([
            ("listening", J::Bool(self.node.is_listening())),
            (
                "listeners",
                J::Arr(self.node.listeners().into_iter().map(J::Str).collect()),
            ),
            ("n_peers", J::i64s(i64::try_from(peers.len()).unwrap_or(0))),
            (
                "peers",
                J::Arr(
                    peers
                        .iter()
                        .map(|p| {
                            J::obj([
                                ("node_info", p.node_info.to_json()),
                                ("is_outbound", J::Bool(p.is_outbound)),
                                (
                                    "connection_status",
                                    J::Raw(crate::json::compact(&p.connection_status)),
                                ),
                                ("remote_ip", J::str(&p.remote_ip)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }

    fn dump_consensus_state(&self) -> J {
        let peers = self.node.peers();
        J::obj([
            (
                "round_state",
                J::Raw(crate::json::compact(&self.node.round_state_json())),
            ),
            (
                "peers",
                J::Arr(
                    peers
                        .iter()
                        .map(|p| match &p.peer_state {
                            Some(st) => J::obj([
                                ("node_address", J::str(&p.socket_addr)),
                                ("peer_state", J::Raw(crate::json::compact(st))),
                            ]),
                            None => J::obj([("node_address", J::str("")), ("peer_state", J::Null)]),
                        })
                        .collect(),
                ),
            ),
        ])
    }

    fn genesis(&self) -> Result<J, String> {
        if self.genesis.chunks.len() > 1 {
            return Err(
                "genesis response is large, please use the genesis_chunked API instead".into(),
            );
        }
        Ok(J::obj([("genesis", J::Raw(self.genesis.json.clone()))]))
    }

    fn genesis_chunked(&self, chunk: i128) -> Result<J, String> {
        let n = self.genesis.chunks.len();
        if n == 0 {
            return Err("service configuration error, there are no chunks".into());
        }
        let id = usize_of(chunk);
        if id > n - 1 {
            return Err(format!("there are {} chunks, {id} is invalid", n - 1));
        }
        Ok(J::obj([
            ("chunk", J::i64s(i64::try_from(id).unwrap_or(0))),
            ("total", J::i64s(i64::try_from(n).unwrap_or(0))),
            ("data", J::str(&self.genesis.chunks[id])),
        ]))
    }

    // --- blocks ------------------------------------------------------------

    fn blockchain_info(&self, min: i128, max: i128) -> Result<J, String> {
        const LIMIT: i128 = 20;
        let base = i128::from(self.block_store.base());
        let height = i128::from(self.block_store.height());
        if min < 0 || max < 0 {
            return Err("heights must be non-negative".into());
        }
        let mut min = if min == 0 { 1 } else { min };
        let mut max = if max == 0 { height } else { max };
        max = max.min(height);
        min = min.max(base);
        min = min.max(max - LIMIT + 1);
        if min > max {
            return Err(format!(
                "min height {min} can't be greater than max height {max}"
            ));
        }
        let mut metas = Vec::new();
        let mut h = max;
        while h >= min {
            let m = self
                .block_store
                .load_block_meta(i64::try_from(h).unwrap_or(0))
                .map_err(s)?;
            metas.push(m.as_ref().map_or(J::Null, encode::block_meta));
            h -= 1;
        }
        Ok(J::obj([
            ("last_height", J::i64s(self.block_store.height())),
            ("block_metas", J::Arr(metas)),
        ]))
    }

    fn block(&self, h: Option<i128>) -> Result<J, String> {
        let height = self.get_height(self.block_store.height(), h)?;
        let block = self.block_store.load_block(height).map_err(s)?;
        let meta = self.block_store.load_block_meta(height).map_err(s)?;
        Ok(J::obj([
            (
                "block_id",
                encode::block_id(&meta.map(|m| m.block_id).unwrap_or_default()),
            ),
            ("block", block.as_ref().map_or(J::Null, encode::block)),
        ]))
    }

    fn block_by_hash(&self, hash: &[u8]) -> Result<J, String> {
        let block = self.block_store.load_block_by_hash(hash).map_err(s)?;
        let Some(block) = block else {
            return Ok(J::obj([
                ("block_id", encode::block_id(&ctypes::BlockId::default())),
                ("block", J::Null),
            ]));
        };
        let meta = self
            .block_store
            .load_block_meta(block.header.height)
            .map_err(s)?
            .ok_or("missing block meta")?;
        Ok(J::obj([
            ("block_id", encode::block_id(&meta.block_id)),
            ("block", encode::block(&block)),
        ]))
    }

    fn commit(&self, h: Option<i128>) -> Result<J, String> {
        let height = self.get_height(self.block_store.height(), h)?;
        let Some(meta) = self.block_store.load_block_meta(height).map_err(s)? else {
            return Ok(J::Null);
        };
        let (commit, canonical) = if height == self.block_store.height() {
            (self.block_store.load_seen_commit(height).map_err(s)?, false)
        } else {
            (self.block_store.load_block_commit(height).map_err(s)?, true)
        };
        Ok(J::obj([
            (
                "signed_header",
                J::obj([
                    ("header", encode::header(&meta.header)),
                    ("commit", commit.as_ref().map_or(J::Null, encode::commit)),
                ]),
            ),
            ("canonical", J::Bool(canonical)),
        ]))
    }

    fn header(&self, h: Option<i128>) -> Result<J, String> {
        let height = self.get_height(self.block_store.height(), h)?;
        let meta = self.block_store.load_block_meta(height).map_err(s)?;
        Ok(J::obj([(
            "header",
            meta.as_ref().map_or(J::Null, |m| encode::header(&m.header)),
        )]))
    }

    fn header_by_hash(&self, hash: &[u8]) -> Result<J, String> {
        let meta = self.block_store.load_block_meta_by_hash(hash).map_err(s)?;
        Ok(J::obj([(
            "header",
            meta.as_ref().map_or(J::Null, |m| encode::header(&m.header)),
        )]))
    }

    fn block_results(&self, h: Option<i128>) -> Result<J, String> {
        let height = self.get_height(self.block_store.height(), h)?;
        if self.state_store.discard_abci_responses {
            return Err("node is not persisting finalize block responses".to_owned());
        }
        let r = self
            .state_store
            .load_finalize_block_response(height)
            .map_err(s)?
            .ok_or_else(|| format!("could not find results for height #{height}"))?;
        Ok(encode::block_results(height, &r))
    }

    fn signed_block(&self, h: Option<i128>) -> Result<J, String> {
        let height = self.get_height(self.block_store.height(), h)?;
        let block = self
            .block_store
            .load_block(height)
            .map_err(s)?
            .ok_or("block not found")?;
        let seen = self
            .block_store
            .load_seen_commit(height)
            .map_err(s)?
            .ok_or("seen commit not found")?;
        let vals = self.state_store.load_validators(height).map_err(s)?;
        Ok(J::obj([
            ("header", encode::header(&block.header)),
            ("commit", encode::commit(&seen)),
            ("data", encode::data(&block.data)),
            ("validator_set", encode::validator_set(&vals)),
        ]))
    }

    fn validators(
        &self,
        h: Option<i128>,
        page: Option<i128>,
        per_page: Option<i128>,
    ) -> Result<J, String> {
        let height = self.get_height(self.latest_uncommitted_height(), h)?;
        let vals = self.state_store.load_validators(height).map_err(s)?;
        let total = vals.validators.len();
        let per_page = validate_per_page(per_page);
        let page = validate_page(page, per_page, i128::try_from(total).unwrap_or(0))?;
        let skip = usize_of(validate_skip_count(page, per_page));
        let n = usize_of(per_page).min(total.saturating_sub(skip));
        let v = &vals.validators[skip.min(total)..skip.min(total) + n];
        Ok(J::obj([
            ("block_height", J::i64s(height)),
            (
                "validators",
                J::Arr(v.iter().map(encode::validator).collect()),
            ),
            ("count", J::i64s(i64::try_from(v.len()).unwrap_or(0))),
            ("total", J::i64s(i64::try_from(total).unwrap_or(0))),
        ]))
    }

    fn consensus_params(&self, h: Option<i128>) -> Result<J, String> {
        let height = self.get_height(self.latest_uncommitted_height(), h)?;
        let p = self.state_store.load_consensus_params(height).map_err(s)?;
        Ok(J::obj([
            ("block_height", J::i64s(height)),
            ("consensus_params", encode::consensus_params(&p)),
        ]))
    }

    // --- txs ---------------------------------------------------------------

    fn load_raw_block(&self, height: i64) -> Result<Vec<u8>, String> {
        let meta = self
            .block_store
            .load_block_meta(height)
            .map_err(s)?
            .ok_or_else(|| format!("no block found for height {height}"))?;
        let mut buf = Vec::new();
        for i in 0..meta.block_id.part_set_header.total as usize {
            let part = self
                .block_store
                .load_block_part(height, i)
                .map_err(s)?
                .ok_or_else(|| format!("missing block part at height {height} part {i}"))?;
            buf.extend_from_slice(&part.bytes);
        }
        Ok(buf)
    }

    fn prove_tx(&self, height: i64, index: u32) -> Result<ptypes::ShareProof, String> {
        let raw = self.load_raw_block(height)?;
        let res = self.app.query(abci::RequestQuery {
            data: raw,
            path: format!("custom/txInclusionProof/{index}"),
            height: 0,
            prove: false,
        })?;
        ptypes::ShareProof::decode(res.value.as_slice()).map_err(s)
    }

    fn tx_json(&self, hash: &[u8], r: &abci::TxResult, prove: bool) -> Result<J, String> {
        let mut proof = None;
        if prove && self.block_store.load_block(r.height).map_err(s)?.is_some() {
            proof = Some(self.prove_tx(r.height, r.index)?);
        }
        let mut o = J::obj([
            ("hash", J::hex(hash)),
            ("height", J::i64s(r.height)),
            ("index", J::num(r.index)),
            (
                "tx_result",
                encode::exec_tx_result(&r.result.clone().unwrap_or_default()),
            ),
            ("tx", J::b64n(&r.tx)),
        ]);
        if let Some(p) = proof
            && !encode::share_proof_is_zero(&p)
        {
            o.push("proof", encode::share_proof(&p));
        }
        Ok(o)
    }

    fn tx(&self, hash: &[u8], prove: bool) -> Result<J, String> {
        let txi = self
            .tx_indexer
            .as_ref()
            .ok_or("transaction indexing is disabled")?;
        let r = txi
            .get(hash)?
            .ok_or_else(|| format!("tx ({}) not found", hx(hash)))?;
        self.tx_json(hash, &r, prove)
    }

    fn tx_search(
        &self,
        query: &str,
        prove: bool,
        page: Option<i128>,
        per_page: Option<i128>,
        order_by: &str,
    ) -> Result<J, String> {
        let txi = self
            .tx_indexer
            .as_ref()
            .ok_or("transaction indexing is disabled")?;
        if query.len() > MAX_QUERY_LENGTH {
            return Err("maximum query length exceeded".into());
        }
        let q = Query::new(query)?;
        if !matches!(order_by, "desc" | "asc" | "") {
            return Err("expected order_by to be either `asc` or `desc` or empty".into());
        }
        let per_page = validate_per_page(per_page);
        let requested = page.unwrap_or(1);
        let (results, total) = txi.search_paged(
            &q,
            order_by,
            usize_of(validate_skip_count(requested, per_page)),
            usize_of(per_page),
        )?;
        validate_page(page, per_page, i128::try_from(total).unwrap_or(0))?;
        let mut txs = Vec::new();
        for r in &results {
            txs.push(self.tx_json(&ctypes::Tx(r.tx.clone()).hash(), r, prove)?);
        }
        Ok(J::obj([
            ("txs", J::Arr(txs)),
            ("total_count", J::i64s(i64::try_from(total).unwrap_or(0))),
        ]))
    }

    fn block_search(
        &self,
        query: &str,
        page: Option<i128>,
        per_page: Option<i128>,
        order_by: &str,
    ) -> Result<J, String> {
        let bi = self
            .block_indexer
            .as_ref()
            .ok_or("block indexing is disabled")?;
        if query.len() > MAX_QUERY_LENGTH {
            return Err("maximum query length exceeded".into());
        }
        let q = Query::new(query)?;
        let mut results = bi.search(&q)?;
        match order_by {
            "desc" | "" => results.sort_by(|a, b| b.cmp(a)),
            "asc" => results.sort_unstable(),
            _ => return Err("expected order_by to be either `asc` or `desc` or empty".into()),
        }
        let total = results.len();
        let per_page = validate_per_page(per_page);
        let page = validate_page(page, per_page, i128::try_from(total).unwrap_or(0))?;
        let skip = usize_of(validate_skip_count(page, per_page));
        let n = usize_of(per_page).min(total.saturating_sub(skip));
        let mut blocks = Vec::new();
        for &h in &results[skip..skip + n] {
            if let Some(b) = self.block_store.load_block(h).map_err(s)?
                && let Some(m) = self
                    .block_store
                    .load_block_meta(b.header.height)
                    .map_err(s)?
            {
                blocks.push(J::obj([
                    ("block_id", encode::block_id(&m.block_id)),
                    ("block", encode::block(&b)),
                ]));
            }
        }
        Ok(J::obj([
            ("blocks", J::Arr(blocks)),
            ("total_count", J::i64s(i64::try_from(total).unwrap_or(0))),
        ]))
    }

    /// `TxStatus`; also returns whether the tx is committed.
    pub fn tx_status(&self, hash: &[u8]) -> Result<(J, bool), String> {
        if let Some(info) = self.block_store.load_tx_info(hash).map_err(s)? {
            let mut o = J::obj([
                ("height", J::i64s(info.height)),
                ("index", J::num(info.index)),
                ("execution_code", J::num(info.code)),
                ("error", J::str(&info.error)),
                ("status", J::str("COMMITTED")),
            ]);
            if !info.codespace.is_empty() {
                o.push("codespace", J::str(&info.codespace));
            }
            if info.gas_wanted != 0 {
                o.push("gas_wanted", J::i64s(info.gas_wanted));
            }
            if info.gas_used != 0 {
                o.push("gas_used", J::i64s(info.gas_used));
            }
            if !info.signers.is_empty() {
                o.push("signers", J::Arr(info.signers.iter().map(J::str).collect()));
            }
            return Ok((o, true));
        }
        let key: mempool::TxKey = hash.try_into().map_err(|_| {
            format!(
                "failed to get tx key from hash: incorrect tx key size. Expected 32 bytes, got {}",
                hash.len()
            )
        })?;
        let simple = |status: &str, code: u32, err: &str| {
            J::obj([
                ("height", J::i64s(0)),
                ("index", J::num(0)),
                ("execution_code", J::num(code)),
                ("error", J::str(err)),
                ("status", J::str(status)),
            ])
        };
        if self.mempool.get_tx_by_key(&key).is_some() {
            return Ok((simple("PENDING", 0, ""), false));
        }
        if self.mempool.was_recently_evicted(&key) {
            return Ok((simple("EVICTED", 0, ""), false));
        }
        let (rejected, code, log) = self.mempool.was_recently_rejected(&key);
        if rejected {
            return Ok((simple("REJECTED", code, &log), false));
        }
        Ok((simple("UNKNOWN", 0, ""), false))
    }

    fn tx_status_batch(&self, hashes: &Arg) -> Result<J, String> {
        let list = match hashes {
            Arg::BytesList(l) => l.clone(),
            _ => Vec::new(),
        };
        if list.is_empty() {
            return Ok(J::obj([("statuses", J::Arr(Vec::new()))]));
        }
        if list.len() > MAX_TX_STATUS_BATCH_SIZE {
            return Err(format!(
                "batch request exceeds maximum ({MAX_TX_STATUS_BATCH_SIZE}) allowed number of transaction hashes"
            ));
        }
        let mut out = Vec::new();
        for h in &list {
            let (st, _) = self.tx_status(h)?;
            out.push(J::obj([("hash", J::hex(h)), ("result", st)]));
        }
        Ok(J::obj([("statuses", J::Arr(out))]))
    }

    // --- mempool -----------------------------------------------------------

    fn check_tx(&self, tx: &[u8]) -> Result<J, String> {
        let r = self.app.check_tx(tx)?;
        Ok(encode::response_check_tx(&r))
    }

    /// `Mempool.CheckTx` for the broadcast routes.
    pub fn mempool_check_tx(&self, tx: &[u8]) -> Result<abci::ResponseCheckTx, String> {
        let info = mempool::TxInfo {
            from_broadcast: true,
            ..mempool::TxInfo::default()
        };
        self.mempool.check_tx(tx, info).map_err(|e| e.to_string())
    }

    fn broadcast_tx_sync(&self, tx: &[u8]) -> Result<J, String> {
        let r = self.mempool_check_tx(tx)?;
        Ok(J::obj([
            ("code", J::num(r.code)),
            ("data", J::hex(&r.data)),
            ("log", J::str(&r.log)),
            ("codespace", J::str(&r.codespace)),
            ("hash", J::hex(&ctypes::Tx(tx.to_vec()).hash())),
        ]))
    }

    fn broadcast_tx_async(&self, tx: &[u8]) -> Result<J, String> {
        self.mempool_check_tx(tx)?;
        Ok(J::obj([
            ("code", J::num(0)),
            ("data", J::hex(&[])),
            ("log", J::str("")),
            ("codespace", J::str("")),
            ("hash", J::hex(&ctypes::Tx(tx.to_vec()).hash())),
        ]))
    }

    fn unconfirmed_txs(&self, limit: Option<i128>) -> J {
        let limit = match limit {
            None => DEFAULT_PER_PAGE,
            Some(p) if p < -1 || p == 0 => DEFAULT_PER_PAGE,
            Some(p) if p > MAX_PER_PAGE => MAX_PER_PAGE,
            Some(p) => p,
        };
        let txs = self
            .mempool
            .reap_max_txs(i64::try_from(limit).unwrap_or(-1));
        J::obj([
            ("n_txs", J::i64s(i64::try_from(txs.len()).unwrap_or(0))),
            (
                "total",
                J::i64s(i64::try_from(self.mempool.size()).unwrap_or(0)),
            ),
            ("total_bytes", J::i64s(self.mempool.size_bytes())),
            ("txs", J::Arr(txs.iter().map(|t| J::b64(t.tx())).collect())),
        ])
    }

    // --- abci --------------------------------------------------------------

    pub(crate) fn abci_query_response(
        &self,
        path: &str,
        data: &[u8],
        height: i128,
        prove: bool,
    ) -> Result<Option<J>, String> {
        let Some(r) = self.app.query_response(abci::RequestQuery {
            data: data.to_vec(),
            path: path.to_owned(),
            height: i64::try_from(height).unwrap_or(0),
            prove,
        })?
        else {
            return Ok(None);
        };
        Ok(Some(J::obj([(
            "response",
            encode::response_query_for_path(path, &r),
        )])))
    }

    fn broadcast_evidence(&self, ev: &Arg) -> Result<J, String> {
        let ev = match ev {
            Arg::Evidence(e) => (**e).clone(),
            _ => return Err("no evidence was provided".into()),
        };
        ev.validate_basic()
            .map_err(|e| format!("evidence.ValidateBasic failed: {e}"))?;
        self.evidence
            .add_evidence(ev.clone())
            .map_err(|e| format!("failed to add evidence: {e}"))?;
        Ok(J::obj([("hash", J::b64n(&ev.hash()))]))
    }

    // --- celestia ----------------------------------------------------------

    fn prove_shares(&self, height: i128, start: i128, end: i128) -> Result<J, String> {
        let height = i64::try_from(height).unwrap_or(0);
        let raw = self.load_raw_block(height)?;
        let res = self.app.query(abci::RequestQuery {
            data: raw,
            path: format!("custom/shareInclusionProof/{start}/{end}"),
            height: 0,
            prove: false,
        })?;
        if res.value.is_empty() && !res.log.is_empty() {
            return Err(res.log);
        }
        let p = ptypes::ShareProof::decode(res.value.as_slice()).map_err(s)?;
        Ok(encode::share_proof(&p))
    }

    fn validate_data_commitment_range(&self, start: u64, end: u64) -> Result<(), String> {
        if start == 0 {
            return Err("the first block is 0".into());
        }
        let range = end.wrapping_sub(start);
        if range > DATA_COMMITMENT_BLOCKS_LIMIT {
            return Err(format!(
                "the query exceeds the limit of allowed blocks {DATA_COMMITMENT_BLOCKS_LIMIT}"
            ));
        }
        if range == 0 {
            return Err("cannot create the data commitments for an empty set of blocks".into());
        }
        if start >= end {
            return Err("last block is smaller than first block".into());
        }
        let height = u64::try_from(self.block_store.height()).unwrap_or(0);
        if end > height + 1 {
            return Err(format!(
                "end block {end} is higher than current chain height {height}"
            ));
        }
        Ok(())
    }

    fn data_root_tuples(&self, start: u64, end: u64) -> Result<Vec<Vec<u8>>, String> {
        let mut out = Vec::new();
        for h in start..end {
            let meta = self
                .block_store
                .load_block_meta(i64::try_from(h).unwrap_or(i64::MAX))
                .map_err(s)?
                .ok_or_else(|| format!("couldn't load block meta {h}"))?;
            if meta.header.data_hash.len() != 32 {
                return Err(format!(
                    "unexpected data hash length {} at height {h}",
                    meta.header.data_hash.len()
                ));
            }
            out.push(encode_data_root_tuple(
                u64::try_from(meta.header.height).unwrap_or(0),
                &meta.header.data_hash,
            ));
        }
        Ok(out)
    }

    fn data_commitment(&self, start: i128, end: i128) -> Result<J, String> {
        let (start, end) = (
            u64::try_from(start).unwrap_or(0),
            u64::try_from(end).unwrap_or(0),
        );
        self.validate_data_commitment_range(start, end)?;
        let tuples = self.data_root_tuples(start, end)?;
        Ok(J::obj([(
            "data_commitment",
            J::hex(&ctypes::merkle_proof::hash_from_byte_slices(&tuples)),
        )]))
    }

    /// `GenerateDataRootInclusionProof`.
    pub fn generate_data_root_inclusion_proof(
        &self,
        height: i64,
        start: u64,
        end: u64,
    ) -> Result<ctypes::Proof, String> {
        self.validate_data_commitment_range(start, end)?;
        #[allow(clippy::cast_sign_loss)]
        let uh = height as u64;
        if uh < start || uh >= end {
            return Err(format!(
                "height {uh} should be in the end exclusive interval first_block {start} last_block {end}"
            ));
        }
        let tuples = self.data_root_tuples(start, end)?;
        let (_, proofs) = ctypes::merkle_proof::proofs_from_byte_slices(&tuples);
        let idx = usize::try_from(uh - start).unwrap_or(0);
        proofs
            .into_iter()
            .nth(idx)
            .ok_or_else(|| "proof index out of range".into())
    }

    fn data_root_inclusion_proof(&self, height: i128, start: i128, end: i128) -> Result<J, String> {
        let p = self.generate_data_root_inclusion_proof(
            i64::try_from(height).unwrap_or(0),
            u64::try_from(start).unwrap_or(0),
            u64::try_from(end).unwrap_or(0),
        )?;
        Ok(J::obj([(
            "proof",
            encode::merkle_proof(p.total, p.index, &p.leaf_hash, &p.aunts),
        )]))
    }
}

/// The values behind `status` (and the gRPC `BlockAPI/Status`).
#[derive(Debug, Clone, Default)]
pub struct StatusParts {
    /// Node info.
    pub node_info: NodeInfo,
    /// Latest block hash.
    pub latest_hash: Vec<u8>,
    /// Latest app hash.
    pub latest_app_hash: Vec<u8>,
    /// Latest height.
    pub latest_height: i64,
    /// Latest block time.
    pub latest_time: ctypes::Time,
    /// Earliest block hash.
    pub earliest_hash: Vec<u8>,
    /// Earliest app hash.
    pub earliest_app_hash: Vec<u8>,
    /// Earliest height.
    pub earliest_height: i64,
    /// Earliest block time.
    pub earliest_time: ctypes::Time,
    /// Catching up.
    pub catching_up: bool,
    /// Own voting power.
    pub voting_power: i64,
}
