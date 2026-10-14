//! Live mempool interop with a Go celestia-core validator (used by
//! `conformance/devnet/mempool-interop.sh`).
//!
//! One process holds:
//!
//! - the Rust state machine (`app::App`), served over ABCI to a Go
//!   celestia-core *follower* that only drives block execution
//!   (InitChain / FinalizeBlock / Commit; this node has no consensus of its
//!   own yet). The follower's own mempool connection is refused, so the
//!   check state belongs to the Rust mempool alone;
//! - the Rust CAT mempool ([`mempool::TxPool`]) calling the same app in
//!   process (CheckTx New / Recheck, QuerySequence), updated on every
//!   Commit under the mempool lock exactly like celestia-core's
//!   `BlockExecutor.Commit`;
//! - a Rust P2P switch with the CAT reactor, peering directly with the Go
//!   validator, plus a consensus stub that advertises our height
//!   (`NewRoundStep`) so the Go node does not skip us as "too far behind",
//!   and sinks for the propagation channels celestia-core requires;
//! - a tiny CometBFT-style RPC: `broadcast_tx_sync` / `broadcast_tx_async`
//!   (JSON-RPC or URI) into the Rust mempool, `num_unconfirmed_txs`,
//!   `unconfirmed_txs`, `rust_status`, `tx_log` (every CheckTx the Rust
//!   mempool ran); any other request is proxied to the Go node's RPC so the
//!   `celestia-appd` CLI can sign and broadcast through this node.
//!
//! ```text
//! cargo run -p mempool --example mempool_interop -- --home DIR --print-id
//! cargo run -p mempool --example mempool_interop -- --home DIR --chain-id C \
//!     --abci 127.0.0.1:A --listen 127.0.0.1:P --dial GOID@127.0.0.1:Q \
//!     --rpc 127.0.0.1:R --go-rpc 127.0.0.1:S
//! ```
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::type_complexity,
    clippy::struct_field_names
)]

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use abci_server::Application;
use base64::Engine as _;
use celestia_core_proto::abci::{
    self, ExecTxResult, RequestCheckTx, ResponseCheckTx, request, response,
};
use celestia_core_proto::tendermint::consensus;
use mempool::cat::{P2pReactor, Reactor, ReactorOptions, TxPool};
use mempool::{AppConnMempool, CachedTx, MempoolConfig, TxInfo};
use p2p::{
    ChannelDescriptor, Envelope, MConnConfig, NodeInfo, NodeInfoOther, NodeKey, ProtocolVersion,
    Switch, SwitchConfig, WeakSwitch,
};
use prost::Message as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

// ---------------------------------------------------------------------------
// The in-process app, shared by the ABCI bridge and the mempool.

/// One CheckTx the Rust mempool ran (New only).
#[derive(Debug, Clone, serde::Serialize)]
struct CheckRecord {
    code: u32,
    log: String,
    signer: String,
    sequence: u64,
    priority: i64,
    gas_wanted: i64,
    height: i64,
    count: u64,
}

struct Shared {
    abci: app::AbciApp,
    /// Committed height.
    height: AtomicI64,
    tx_log: Mutex<BTreeMap<String, CheckRecord>>,
    rpc_keys: Mutex<HashSet<String>>,
}

/// The mempool's ABCI connection: requests go to the same app the ABCI
/// bridge serves.
struct InProcess(Arc<Shared>);

impl AppConnMempool for InProcess {
    fn check_tx(&self, req: RequestCheckTx) -> Result<ResponseCheckTx, String> {
        let is_new = req.r#type == abci::CheckTxType::New as i32;
        let key = hex::encode_upper(mempool::tx_key(&req.tx));
        match self.0.abci.handle(request::Value::CheckTx(req)) {
            response::Value::CheckTx(r) => {
                if is_new {
                    let mut log = self.0.tx_log.lock().unwrap();
                    let e = log.entry(key).or_insert_with(|| CheckRecord {
                        code: r.code,
                        log: String::new(),
                        signer: String::new(),
                        sequence: 0,
                        priority: 0,
                        gas_wanted: 0,
                        height: 0,
                        count: 0,
                    });
                    e.count += 1;
                    // Keep the first acceptance (a later duplicate check
                    // fails on the sequence).
                    if e.count == 1 || r.code == 0 {
                        e.code = r.code;
                        e.log = r.log.chars().take(200).collect();
                        e.signer = hex::encode_upper(&r.address);
                        e.sequence = r.sequence;
                        e.priority = r.priority;
                        e.gas_wanted = r.gas_wanted;
                        e.height = self.0.height.load(Ordering::SeqCst);
                    }
                }
                Ok(r)
            }
            response::Value::Exception(e) => Err(e.error),
            other => Err(format!("unexpected response {other:?}")),
        }
    }

    fn query_sequence(&self, signer: &[u8]) -> Result<u64, String> {
        match self
            .0
            .abci
            .handle(request::Value::QuerySequence(abci::RequestQuerySequence {
                signer: signer.to_vec(),
            })) {
            response::Value::QuerySequence(r) => Ok(r.sequence),
            response::Value::Exception(e) => Err(e.error),
            other => Err(format!("unexpected response {other:?}")),
        }
    }
}

/// Block params for the pre/post-check filters (`TxPreCheck` /
/// `TxPostCheck` with one validator).
#[derive(Default)]
struct Params {
    max_bytes: i64,
    max_gas: i64,
}

/// What the Go follower talks to over ABCI.
struct Bridge {
    shared: Arc<Shared>,
    pool: OnceLock<Arc<TxPool>>,
    block: Mutex<Option<(i64, Vec<Vec<u8>>, Vec<ExecTxResult>)>>,
    params: Mutex<Params>,
}

impl Bridge {
    fn filters(&self) -> (mempool::PreCheckFunc, mempool::PostCheckFunc) {
        let p = self.params.lock().unwrap();
        let max_bytes = if p.max_bytes == -1 {
            ctypes::part_set::MAX_BLOCK_SIZE_BYTES
        } else {
            p.max_bytes
        };
        let max_data = ctypes::max_data_bytes_no_evidence(max_bytes, 1).unwrap_or(max_bytes);
        (
            mempool::pre_check_max_bytes(max_data),
            mempool::post_check_max_gas(p.max_gas),
        )
    }
}

impl Application for Bridge {
    fn handle(&self, req: request::Value) -> response::Value {
        match req {
            // The follower's own mempool: refused, so the app's check state
            // is driven by the Rust mempool only.
            request::Value::CheckTx(_) => response::Value::CheckTx(ResponseCheckTx {
                code: 1,
                log: "check state is owned by the in-process Rust mempool".into(),
                ..ResponseCheckTx::default()
            }),
            request::Value::InitChain(r) => {
                if let Some(b) = r.consensus_params.as_ref().and_then(|c| c.block.as_ref()) {
                    *self.params.lock().unwrap() = Params {
                        max_bytes: b.max_bytes,
                        max_gas: b.max_gas,
                    };
                }
                self.shared.abci.handle(request::Value::InitChain(r))
            }
            request::Value::FinalizeBlock(r) => {
                let (height, txs) = (r.height, r.txs.clone());
                let resp = self.shared.abci.handle(request::Value::FinalizeBlock(r));
                if let response::Value::FinalizeBlock(f) = &resp {
                    if let Some(b) = f
                        .consensus_param_updates
                        .as_ref()
                        .and_then(|c| c.block.as_ref())
                    {
                        *self.params.lock().unwrap() = Params {
                            max_bytes: b.max_bytes,
                            max_gas: b.max_gas,
                        };
                    }
                    *self.block.lock().unwrap() = Some((height, txs, f.tx_results.clone()));
                }
                resp
            }
            request::Value::Commit(c) => {
                let Some(pool) = self.pool.get() else {
                    return self.shared.abci.handle(request::Value::Commit(c));
                };
                // BlockExecutor.Commit: Lock, FlushAppConn, Commit, Update.
                let mut guard = pool.lock();
                let _ = guard.flush_app_conn();
                let resp = self.shared.abci.handle(request::Value::Commit(c));
                if let Some((height, txs, results)) = self.block.lock().unwrap().take() {
                    let block_txs: Vec<CachedTx> = txs.into_iter().map(CachedTx::new).collect();
                    let (pre, post) = self.filters();
                    if let Err(e) =
                        guard.update(height, &block_txs, &results, Some(pre), Some(post))
                    {
                        eprintln!("mempool update at {height}: {e}");
                    }
                    self.shared.height.store(height, Ordering::SeqCst);
                    tracing::info!(
                        height,
                        block_txs = block_txs.len(),
                        mempool = pool.size(),
                        "committed"
                    );
                }
                resp
            }
            other => self.shared.abci.handle(other),
        }
    }
}

// ---------------------------------------------------------------------------
// Consensus stub: advertises our height; sinks consensus / propagation.

const CONSENSUS: [u8; 4] = [0x20, 0x21, 0x22, 0x23];
const PROPAGATION: [u8; 2] = [0x50, 0x51];

struct ConsensusStub {
    switch: Mutex<Option<WeakSwitch>>,
    received: AtomicI64,
}

fn new_round_step(height: i64) -> Vec<u8> {
    consensus::Message {
        sum: Some(consensus::message::Sum::NewRoundStep(
            consensus::NewRoundStep {
                height,
                round: 0,
                step: 1, // RoundStepNewHeight
                seconds_since_start_time: 0,
                last_commit_round: if height <= 1 { -1 } else { 0 },
            },
        )),
    }
    .encode_to_vec()
}

impl p2p::Reactor for ConsensusStub {
    fn name(&self) -> String {
        "CONSENSUS".into()
    }

    fn channels(&self) -> Vec<ChannelDescriptor> {
        CONSENSUS
            .iter()
            .chain(PROPAGATION.iter())
            .map(|&id| ChannelDescriptor {
                send_queue_capacity: 100,
                recv_message_capacity: 64 << 20,
                ..ChannelDescriptor::new(id, 5)
            })
            .collect()
    }

    fn set_switch(&self, sw: WeakSwitch) {
        *self.switch.lock().unwrap() = Some(sw);
    }

    fn receive(&self, _env: Envelope) -> Result<(), String> {
        self.received.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn queue_size(&self) -> usize {
        1000
    }
}

// ---------------------------------------------------------------------------
// RPC

struct Rpc {
    pool: Arc<TxPool>,
    shared: Arc<Shared>,
    switch: Switch,
    go_rpc: String,
    go_id: Option<p2p::NodeId>,
}

struct HttpRequest {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    raw_head: Vec<u8>,
}

async fn read_request(s: &mut TcpStream) -> Option<HttpRequest> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let head_end = loop {
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
        let n = s.read(&mut tmp).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_string();
    let target = first.next()?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let len: usize = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = buf[head_end..].to_vec();
    while body.len() < len {
        let n = s.read(&mut tmp).await.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    Some(HttpRequest {
        method,
        target,
        headers,
        body,
        raw_head: buf[..head_end].to_vec(),
    })
}

async fn respond(s: &mut TcpStream, body: &str) {
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = s.write_all(resp.as_bytes()).await;
    let _ = s.shutdown().await;
}

fn query_param(target: &str, name: &str) -> Option<String> {
    let q = target.split_once('?')?.1;
    q.split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.trim_matches('"').to_string())
}

impl Rpc {
    fn broadcast(&self, tx: &[u8]) -> Result<serde_json::Value, String> {
        let key = hex::encode_upper(mempool::tx_key(tx));
        self.shared.rpc_keys.lock().unwrap().insert(key);
        let hash = hex::encode_upper(ctypes::Tx(tx.to_vec()).hash());
        let rsp = self
            .pool
            .check_tx(
                tx,
                TxInfo {
                    from_broadcast: true,
                    ..TxInfo::default()
                },
            )
            .map_err(|e| e.to_string())?;
        Ok(serde_json::json!({
            "code": rsp.code,
            "data": hex::encode_upper(&rsp.data),
            "log": rsp.log,
            "codespace": rsp.codespace,
            "hash": hash,
        }))
    }

    fn local(
        &self,
        method: &str,
        tx: Option<Vec<u8>>,
        limit: usize,
    ) -> Option<Result<serde_json::Value, String>> {
        Some(match method {
            "broadcast_tx_sync" | "broadcast_tx_async" => {
                let tx = tx.unwrap_or_default();
                // CheckTx can be slow (blob commitments in a debug build).
                tokio::task::block_in_place(|| self.broadcast(&tx))
            }
            "num_unconfirmed_txs" => Ok(serde_json::json!({
                "n_txs": self.pool.size().to_string(),
                "total": self.pool.size().to_string(),
                "total_bytes": self.pool.size_bytes().to_string(),
            })),
            "unconfirmed_txs" => {
                let txs: Vec<String> = self
                    .pool
                    .reap_max_txs(limit as i64)
                    .iter()
                    .map(|t| base64::engine::general_purpose::STANDARD.encode(t.tx()))
                    .collect();
                Ok(serde_json::json!({
                    "n_txs": txs.len().to_string(),
                    "total": self.pool.size().to_string(),
                    "total_bytes": self.pool.size_bytes().to_string(),
                    "txs": txs,
                }))
            }
            "rust_status" => {
                let peers: Vec<String> = self
                    .switch
                    .peers()
                    .iter()
                    .map(|p| p.id().to_string())
                    .collect();
                let go = self.go_id.and_then(|id| self.switch.get_peer(&id));
                Ok(serde_json::json!({
                    "height": self.shared.height.load(Ordering::SeqCst),
                    "mempool_size": self.pool.size(),
                    "mempool_bytes": self.pool.size_bytes(),
                    "peers": peers,
                    "go_peer_connected": go.is_some(),
                    "go_peer_outbound": go.as_ref().map(|p| p.is_outbound()),
                    "go_peer_channels": go.as_ref().map(|p| hex::encode(&p.node_info().channels)),
                }))
            }
            "tx_log" => {
                let log = self.shared.tx_log.lock().unwrap().clone();
                let rpc = self.shared.rpc_keys.lock().unwrap().clone();
                Ok(serde_json::json!({ "checks": log, "rpc_keys": rpc }))
            }
            _ => return None,
        })
    }

    async fn proxy(&self, req: &HttpRequest, s: &mut TcpStream) {
        let Ok(mut up) = TcpStream::connect(&self.go_rpc).await else {
            respond(s, r#"{"error":"go rpc unreachable"}"#).await;
            return;
        };
        // One request per connection.
        let mut head = String::new();
        for l in String::from_utf8_lossy(&req.raw_head).split("\r\n") {
            let lower = l.to_ascii_lowercase();
            if l.is_empty() || lower.starts_with("connection:") || lower.starts_with("keep-alive:")
            {
                continue;
            }
            head.push_str(l);
            head.push_str("\r\n");
        }
        head.push_str("Connection: close\r\n\r\n");
        let _ = up.write_all(head.as_bytes()).await;
        let _ = up.write_all(&req.body).await;
        let mut out = Vec::new();
        let _ = up.read_to_end(&mut out).await;
        let _ = s.write_all(&out).await;
        let _ = s.shutdown().await;
    }

    async fn serve(self: Arc<Self>, s: &mut TcpStream) {
        let Some(req) = read_request(s).await else {
            return;
        };
        let path = req
            .target
            .split('?')
            .next()
            .unwrap_or("")
            .trim_start_matches('/')
            .to_string();
        if req.method == "GET" && !path.is_empty() {
            let tx = query_param(&req.target, "tx").and_then(|t| {
                let t = t.trim_start_matches("0x");
                hex::decode(t).ok()
            });
            let limit = query_param(&req.target, "limit")
                .and_then(|l| l.parse().ok())
                .unwrap_or(-1i64);
            let limit = if limit < 0 {
                usize::MAX >> 1
            } else {
                limit as usize
            };
            if let Some(res) = self.local(&path, tx, limit) {
                let body = match res {
                    Ok(r) => serde_json::json!({"jsonrpc": "2.0", "id": -1, "result": r}),
                    Err(e) => rpc_error(&serde_json::json!(-1), &e),
                };
                respond(s, &body.to_string()).await;
                return;
            }
        }
        if req.method == "POST"
            && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&req.body)
            && let Some(method) = v.get("method").and_then(|m| m.as_str())
        {
            let id = v.get("id").cloned().unwrap_or(serde_json::Value::Null);
            let tx = v
                .pointer("/params/tx")
                .and_then(|t| t.as_str())
                .and_then(|t| base64::engine::general_purpose::STANDARD.decode(t).ok());
            let limit = v
                .pointer("/params/limit")
                .and_then(|l| {
                    l.as_str()
                        .and_then(|s| s.parse::<usize>().ok())
                        .or(l.as_u64().map(|n| n as usize))
                })
                .unwrap_or(usize::MAX >> 1);
            if let Some(res) = self.local(method, tx, limit) {
                let body = match res {
                    Ok(r) => serde_json::json!({"jsonrpc": "2.0", "id": id, "result": r}),
                    Err(e) => rpc_error(&id, &e),
                };
                respond(s, &body.to_string()).await;
                return;
            }
        }
        let _ = &req.headers;
        self.proxy(&req, s).await;
    }
}

fn rpc_error(id: &serde_json::Value, e: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": -32603, "message": "Internal error", "data": e},
    })
}

// ---------------------------------------------------------------------------

struct Args {
    home: PathBuf,
    print_id: bool,
    chain_id: String,
    abci: String,
    listen: String,
    dial: Option<String>,
    rpc: String,
    go_rpc: String,
}

fn parse_args() -> Args {
    let mut a = Args {
        home: PathBuf::from("."),
        print_id: false,
        chain_id: String::new(),
        abci: "127.0.0.1:26658".into(),
        listen: "127.0.0.1:0".into(),
        dial: None,
        rpc: "127.0.0.1:26657".into(),
        go_rpc: String::new(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| panic!("missing value for {k}"));
        match k.as_str() {
            "--home" => a.home = PathBuf::from(v()),
            "--print-id" => a.print_id = true,
            "--chain-id" => a.chain_id = v(),
            "--abci" => a.abci = v(),
            "--listen" => a.listen = v(),
            "--dial" => a.dial = Some(v()),
            "--rpc" => a.rpc = v(),
            "--go-rpc" => a.go_rpc = v(),
            other => panic!("unknown argument {other}"),
        }
    }
    a
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let args = parse_args();
    std::fs::create_dir_all(&args.home).unwrap();
    let key = NodeKey::load_or_generate(&args.home.join("node_key.json")).unwrap();
    if args.print_id {
        println!("{}", key.id());
        return;
    }

    let shared = Arc::new(Shared {
        abci: app::AbciApp::new(app::App::new()),
        height: AtomicI64::new(0),
        tx_log: Mutex::new(BTreeMap::new()),
        rpc_keys: Mutex::new(HashSet::new()),
    });
    let cfg = MempoolConfig::celestia_app();
    let pool = Arc::new(TxPool::new(
        cfg.clone(),
        Arc::new(InProcess(shared.clone())),
        0,
    ));
    let bridge = Arc::new(Bridge {
        shared: shared.clone(),
        pool: OnceLock::new(),
        block: Mutex::new(None),
        params: Mutex::new(Params {
            max_bytes: -1,
            max_gas: -1,
        }),
    });
    let _ = bridge.pool.set(pool.clone());
    let abci_listener = TcpListener::bind(&args.abci).await.unwrap();
    tokio::spawn(abci_server::serve(abci_listener, bridge.clone()));

    let reactor = Reactor::new(
        pool.clone(),
        ReactorOptions {
            listen_only: !cfg.broadcast,
            max_tx_size: cfg.max_tx_bytes as i64,
            max_gossip_delay: cfg.max_gossip_delay,
            max_persistent_sticky_peers: cfg.max_persistent_sticky_peers,
            ..ReactorOptions::default()
        },
    )
    .unwrap();

    let info = NodeInfo {
        protocol_version: ProtocolVersion {
            p2p: p2p::node_info::P2P_PROTOCOL,
            block: p2p::node_info::BLOCK_PROTOCOL,
            app: 0,
        },
        network: args.chain_id.clone(),
        version: p2p::node_info::TM_CORE_SEMVER.into(),
        moniker: "rust-mempool".into(),
        listen_addr: format!("tcp://{}", args.listen),
        other: NodeInfoOther {
            tx_index: "off".into(),
            rpc_address: format!("tcp://{}", args.rpc),
        },
        ..NodeInfo::default()
    };
    let sw = Switch::new(
        SwitchConfig {
            listen_address: Some(format!("tcp://{}", args.listen)),
            allow_duplicate_ip: true,
            persistent_peers: args.dial.iter().cloned().collect(),
            mconn: MConnConfig {
                send_rate: 50 << 20,
                recv_rate: 50 << 20,
                flush_throttle: Duration::from_millis(10),
                ..MConnConfig::default()
            },
            ..SwitchConfig::default()
        },
        key,
        info,
    )
    .unwrap();
    sw.add_peer_filter(Arc::new(p2p::switch::celestia_peer_filter));
    let stub = Arc::new(ConsensusStub {
        switch: Mutex::new(None),
        received: AtomicI64::new(0),
    });
    sw.add_reactor(stub.clone()).unwrap();
    sw.add_reactor(P2pReactor::new(reactor.clone())).unwrap();
    sw.start().await.unwrap();
    eprintln!(
        "rust node {} listening on {:?}",
        sw.node_id(),
        sw.net_address()
    );

    // Advertise our height to the Go consensus reactor.
    {
        let sw = sw.clone();
        let shared = shared.clone();
        tokio::spawn(async move {
            loop {
                let h = shared.height.load(Ordering::SeqCst) + 1;
                for p in sw.peers() {
                    let _ = p.try_send(0x20, new_round_step(h));
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        });
    }

    let go_id = args
        .dial
        .as_ref()
        .and_then(|d| d.split('@').next())
        .and_then(|id| id.parse().ok());
    let rpc = Arc::new(Rpc {
        pool: pool.clone(),
        shared: shared.clone(),
        switch: sw.clone(),
        go_rpc: args.go_rpc.clone(),
        go_id,
    });
    let listener = TcpListener::bind(&args.rpc).await.unwrap();
    eprintln!("rpc on {}", args.rpc);
    loop {
        let Ok((mut s, _)) = listener.accept().await else {
            continue;
        };
        let rpc = rpc.clone();
        tokio::spawn(async move { rpc.serve(&mut s).await });
    }
}
