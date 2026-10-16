//! Differential RPC corpus (`conformance/rpccorpus/<scenario>`): a Go
//! celestia-core v0.42.3 node's responses to URI / JSON-RPC / WebSocket /
//! gRPC requests, replayed against a Rust node built from the same chain.
//!
//! The Rust node is reconstructed the way block sync builds one: genesis
//! through the handshake (InitChain on the Rust app), then every block of
//! the Go chain is saved with its seen commit and applied by the Rust
//! `BlockExecutor` (so validator sets, params, FinalizeBlock responses, tx
//! info, the kv indexers and the event bus are all the Rust node's own).
//! Every recorded request then goes through the Rust server and must
//! produce Go's bytes; only fields that describe the node itself rather
//! than the chain are normalized (`norm` entries: consensus round state,
//! own node identity, the frozen node's key).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::too_many_lines,
    clippy::similar_names
)]

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ctypes::proto::abci;
use ctypes::proto::types as ptypes;
use kvdb::{KvBackend, MemBackend};
use nodestore::{AppConn, BlockExecutor, BlockStore, GenesisDoc, Handshaker, StateStore};
use prost::Message as _;
use rpc::events::{BlockEventPublisher, EventBus};
use rpc::{AppQuery, BlockIndexer, Environment, GenesisData, NodeInfo, NodeStatus, TxIndexer};
use serde_json::Value;

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../conformance/rpccorpus")
}

fn gunzip(p: &Path) -> Vec<u8> {
    let f = std::fs::File::open(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(f)
        .read_to_end(&mut out)
        .unwrap();
    out
}

fn now() -> ctypes::Time {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    ctypes::Time::from_unix(
        i64::try_from(d.as_secs()).unwrap(),
        i32::try_from(d.subsec_nanos()).unwrap(),
    )
}

// --- app adapters ------------------------------------------------------------

struct Adapter(Arc<Mutex<app::App>>);

fn lock(a: &Arc<Mutex<app::App>>) -> std::sync::MutexGuard<'_, app::App> {
    a.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn app_err(e: &sdk::SdkError) -> nodestore::Error {
    nodestore::Error::new(e.log.clone())
}

impl AppConn for Adapter {
    fn info(&mut self) -> nodestore::Result<abci::ResponseInfo> {
        Ok(lock(&self.0).info())
    }
    fn init_chain(
        &mut self,
        req: abci::RequestInitChain,
    ) -> nodestore::Result<abci::ResponseInitChain> {
        lock(&self.0).init_chain(req).map_err(|e| app_err(&e))
    }
    fn prepare_proposal(
        &mut self,
        req: abci::RequestPrepareProposal,
    ) -> nodestore::Result<abci::ResponsePrepareProposal> {
        lock(&self.0)
            .prepare_proposal(&req)
            .map_err(|e| app_err(&e))
    }
    fn process_proposal(
        &mut self,
        req: abci::RequestProcessProposal,
    ) -> nodestore::Result<abci::ResponseProcessProposal> {
        lock(&self.0)
            .process_proposal(&req)
            .map_err(|e| app_err(&e))
    }
    fn finalize_block(
        &mut self,
        req: abci::RequestFinalizeBlock,
    ) -> nodestore::Result<abci::ResponseFinalizeBlock> {
        lock(&self.0).finalize_block(req).map_err(|e| app_err(&e))
    }
    fn commit(&mut self) -> nodestore::Result<abci::ResponseCommit> {
        lock(&self.0).commit().map_err(|e| app_err(&e))
    }
}

struct MempoolApp(Arc<Mutex<app::App>>);

impl mempool::AppConnMempool for MempoolApp {
    fn check_tx(&self, req: abci::RequestCheckTx) -> Result<abci::ResponseCheckTx, String> {
        lock(&self.0).check_tx(&req)
    }
    fn query_sequence(&self, signer: &[u8]) -> Result<u64, String> {
        Ok(lock(&self.0).query_sequence(signer).sequence)
    }
}

struct Query(Arc<Mutex<app::App>>);

impl AppQuery for Query {
    fn info(&self) -> Result<abci::ResponseInfo, String> {
        Ok(lock(&self.0).info())
    }
    fn query(&self, req: abci::RequestQuery) -> Result<abci::ResponseQuery, String> {
        Ok(lock(&self.0).query(&req))
    }
    fn check_tx(&self, tx: &[u8]) -> Result<abci::ResponseCheckTx, String> {
        lock(&self.0).check_tx(&abci::RequestCheckTx {
            tx: tx.to_vec(),
            ..Default::default()
        })
    }
}

/// The frozen Go node's identity, taken from its own `/status` (these
/// fields describe the node, not the chain).
struct Node {
    info: NodeInfo,
    catching_up: bool,
}

impl NodeStatus for Node {
    fn node_info(&self) -> NodeInfo {
        self.info.clone()
    }
    fn is_listening(&self) -> bool {
        true
    }
    fn listeners(&self) -> Vec<String> {
        vec!["Listener(@)".into()]
    }
    fn peers(&self) -> Vec<rpc::PeerInfo> {
        Vec::new()
    }
    fn wait_sync(&self) -> bool {
        self.catching_up
    }
    fn is_catching_up(&self) -> bool {
        false
    }
    fn round_state_json(&self) -> String {
        "{}".into()
    }
    fn round_state_simple_json(&self) -> String {
        "{}".into()
    }
}

struct NoEvidence;

impl rpc::EvidenceSink for NoEvidence {
    fn add_evidence(&self, _: ctypes::Evidence) -> Result<(), String> {
        Ok(())
    }
}

fn node_info_from_status(v: &Value) -> NodeInfo {
    let ni = &v["result"]["node_info"];
    let s = |p: &Value| p.as_str().unwrap_or_default().to_owned();
    let u = |p: &Value| s(p).parse::<u64>().unwrap_or(0);
    NodeInfo {
        p2p_version: u(&ni["protocol_version"]["p2p"]),
        block_version: u(&ni["protocol_version"]["block"]),
        app_version: u(&ni["protocol_version"]["app"]),
        id: s(&ni["id"]),
        listen_addr: s(&ni["listen_addr"]),
        network: s(&ni["network"]),
        version: s(&ni["version"]),
        channels: hex::decode(s(&ni["channels"])).unwrap_or_default(),
        moniker: s(&ni["moniker"]),
        tx_index: s(&ni["other"]["tx_index"]),
        rpc_address: s(&ni["other"]["rpc_address"]),
    }
}

fn read_frames(bz: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < bz.len() {
        let mut len: u64 = 0;
        let mut shift = 0;
        loop {
            let b = bz[i];
            i += 1;
            len |= u64::from(b & 0x7f) << shift;
            if b < 0x80 {
                break;
            }
            shift += 7;
        }
        let n = usize::try_from(len).unwrap();
        out.push(bz[i..i + n].to_vec());
        i += n;
    }
    out
}

struct Chain {
    env: Arc<Environment>,
    /// Event frames per recorded live query (Rust side).
    live: BTreeMap<String, Vec<String>>,
    height: i64,
}

const LIVE_QUERIES: &[&str] = &[
    "tm.event='NewBlock'",
    "tm.event='NewBlockHeader'",
    "tm.event='Tx'",
    "tm.event='NewBlockEvents'",
    "tm.event='ValidatorSetUpdates'",
    "tm.event='NewSignedBlock'",
    "tm.event='Tx' AND message.action='/cosmos.bank.v1beta1.MsgSend'",
    "tm.event='NewEvidence'",
    "tx.height>3",
    "tm.event='NewBlock' AND block.height>2",
];

/// App options of the rebuilt node: Go's recorded node keeps every
/// version of these short chains (SDK `pruning = "default"` keeps 362880),
/// so historical `abci_query` heights must not be pruned here either.
fn app_options(genesis: &GenesisDoc) -> app::AppOptions {
    let mut o = app::AppOptions {
        chain_id: Some(genesis.chain_id.clone()),
        ..app::AppOptions::default()
    };
    o.store.pruning.interval = 0;
    o
}

/// Rebuilds the Go chain in a Rust node.
fn build_chain(dir: &Path, status_resp: &Value, frozen_pub_key: ctypes::PubKey) -> Chain {
    let genesis_bytes = std::fs::read(dir.join("genesis.json")).unwrap();
    let genesis = GenesisDoc::from_json(&genesis_bytes, now()).unwrap();
    let app_db: Arc<dyn KvBackend> = Arc::new(MemBackend::new());
    let app = app::App::open_backend(app_db.clone(), &app_options(&genesis))
        .map_err(|e| e.log)
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let bs = BlockStore::new(Arc::new(MemBackend::new())).unwrap();
    let ss = StateStore::new(Arc::new(MemBackend::new()));
    let state = ss.load_from_db_or_genesis(&genesis).unwrap();
    let pool = Arc::new(
        mempool::TxPool::new(
            mempool::MempoolConfig::celestia_app(),
            Arc::new(MempoolApp(app.clone())),
            state.last_block_height,
        )
        .with_pre_check(nodestore::execution::tx_pre_check(&state))
        .with_post_check(nodestore::execution::tx_post_check(&state)),
    );
    let bus = Arc::new(EventBus::new());
    let index_db: Arc<dyn KvBackend> = Arc::new(MemBackend::new());
    let txi = Arc::new(TxIndexer::new(index_db.clone()));
    let bi = Arc::new(BlockIndexer::new(index_db));
    let mut exec = BlockExecutor::new(
        ss.clone(),
        bs.clone(),
        Box::new(Adapter(app.clone())),
        pool.clone(),
        Box::new(nodestore::EmptyEvidencePool),
    );
    exec.set_event_sink(Arc::new(BlockEventPublisher {
        bus: bus.clone(),
        tx_indexer: Some(txi.clone()),
        block_indexer: Some(bi.clone()),
    }));
    let mut hs = Handshaker::new(ss.clone(), state, bs.clone(), &genesis, now());
    let mut state = hs.handshake(&mut exec).unwrap();

    // Live subscriptions, as the Go recorder made them.
    let mut subs = Vec::new();
    for (i, q) in LIVE_QUERIES.iter().enumerate() {
        let sub = bus
            .subscribe("live", rpc::query::Query::new(q).unwrap(), 1 << 20)
            .unwrap();
        subs.push((i, (*q).to_owned(), sub));
    }

    let frames = read_frames(&gunzip(&dir.join("blocks.bin.gz")));
    let mut height = 0;
    for pair in frames.chunks(2) {
        let pb = ptypes::Block::decode(pair[0].as_slice()).unwrap();
        let mut block = ctypes::Block::from_proto(&pb).unwrap();
        let seen = ctypes::Commit::from_proto(&ptypes::Commit::decode(pair[1].as_slice()).unwrap())
            .unwrap();
        let parts = block.make_part_set(ctypes::BLOCK_PART_SIZE_BYTES).unwrap();
        let block_id = nodestore::execution::block_id_of(&mut block, &parts);
        bs.save_block(&mut block, &parts, &seen).unwrap();
        state = exec
            .apply_block(&state, block_id, &mut block, now())
            .unwrap_or_else(|e| panic!("height {}: {e}", block.header.height));
        height = block.header.height;
    }

    let mut live = BTreeMap::new();
    for (i, q, mut sub) in subs {
        let mut v = Vec::new();
        while let Ok(m) = sub.out.try_recv() {
            v.push(rpc::server::event_frame(
                &Some(rpc::server::Id::Int(i64::try_from(i).unwrap())),
                &q,
                &m,
            ));
        }
        live.insert(q, v);
    }
    let _ = bus.unsubscribe_all("live");

    // The Go node was restarted before the requests were recorded: reopen
    // the app from its database (check state back at height 0, as after a
    // restart) with a fresh mempool.
    drop(exec);
    let app = app::App::open_backend(app_db, &app_options(&genesis))
        .map_err(|e| e.log)
        .unwrap();
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
    let genesis_data = GenesisData::from_file(&genesis_bytes, &genesis).unwrap();
    let env = Environment::new(
        bs,
        ss,
        Arc::new(Query(app)),
        pool,
        Arc::new(Node {
            info: node_info_from_status(status_resp),
            catching_up: status_resp["result"]["sync_info"]["catching_up"] == Value::Bool(true),
        }),
        Arc::new(NoEvidence),
        frozen_pub_key,
        genesis_data,
        Some(txi),
        Some(bi),
        bus,
        rpc::RpcConfig::default(),
    );
    Chain {
        env: Arc::new(env),
        live,
        height,
    }
}

// --- comparison ---------------------------------------------------------------

/// Splits `…,"events":{…}}}` so the Go-random map order can be normalized.
fn norm_event_frame(frame: &str) -> String {
    let Some(i) = frame.rfind(",\"events\":") else {
        return frame.to_owned();
    };
    let (head, tail) = frame.split_at(i);
    let tail = &tail[",\"events\":".len()..];
    let events_json = tail.strip_suffix("}}").unwrap_or(tail);
    let v: Value = serde_json::from_str(events_json).unwrap_or(Value::Null);
    format!(
        "{head},\"events\":{}}}}}",
        serde_json::to_string(&v).unwrap()
    )
}

fn normalize(norm: &str, body: &str) -> String {
    let Ok(mut v) = serde_json::from_str::<Value>(body) else {
        return body.to_owned();
    };
    match norm {
        "consensus_state" | "dump_consensus_state" => {
            // Node-local consensus round state: compare only the shape.
            if let Some(r) = v.get_mut("result") {
                *r = Value::String("<round state>".into());
            }
        }
        _ => {}
    }
    serde_json::to_string(&v).unwrap()
}

fn normalize_decode_error(s: &str) -> String {
    let mut s = s.replace("proto:\u{a0}", "").replace("proto: ", "");
    if let (Some(a), Some(b)) = (s.find(" [celestiaorg/"), s.rfind(']')) {
        s.replace_range(a..=b, "");
    }
    s
}

#[derive(Default)]
struct Stats {
    total: usize,
    ok: usize,
    failures: Vec<String>,
    by_route: BTreeMap<String, (usize, usize)>,
}

impl Stats {
    fn record(&mut self, route: &str, ok: bool, detail: impl FnOnce() -> String) {
        self.total += 1;
        let e = self.by_route.entry(route.to_owned()).or_default();
        e.0 += 1;
        if ok {
            self.ok += 1;
            e.1 += 1;
        } else if e.0 - e.1 <= 3 {
            self.failures.push(detail());
        }
    }
}

fn route_of(e: &Value) -> String {
    match e["kind"].as_str().unwrap() {
        "http" => {
            let path = e["path"].as_str().unwrap_or_default();
            if path == "/" {
                let body = e["body"].as_str().unwrap_or_default();
                let m = serde_json::from_str::<Value>(body)
                    .ok()
                    .and_then(|v| v["method"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| "jsonrpc".into());
                format!("POST {m}")
            } else {
                path.split('?').next().unwrap_or(path).to_owned()
            }
        }
        "ws" => "websocket".into(),
        _ => e["grpc"].as_str().unwrap_or_default().to_owned(),
    }
}

fn short(s: &str) -> String {
    if s.len() > 1500 {
        format!("{}…({} bytes)", &s[..1500], s.len())
    } else {
        s.to_owned()
    }
}

fn first_diff(a: &str, b: &str) -> String {
    let pos = a
        .bytes()
        .zip(b.bytes())
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()));
    let from = pos.saturating_sub(120);
    let ctx = |s: &str| {
        s.get(from..(pos + 200).min(s.len()))
            .unwrap_or("")
            .to_owned()
    };
    format!("at byte {pos}:\n   go: …{}\n rust: …{}", ctx(a), ctx(b))
}

async fn http_call(
    env: &Arc<Environment>,
    host: &str,
    method: &str,
    path: &str,
    body: &str,
) -> (u16, String, String, String) {
    use tower::ServiceExt as _;
    let req = axum::http::Request::builder()
        .method(method)
        .uri(path)
        .header("host", host)
        .body(axum::body::Body::from(body.to_owned()))
        .unwrap();
    let app = rpc::server::router(env.clone(), "127.0.0.1:55555".parse().unwrap());
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status().as_u16();
    let h = |n: &str| {
        resp.headers()
            .get(n)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    };
    let (ct, cc) = (h("content-type"), h("cache-control"));
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, ct, cc, String::from_utf8_lossy(&bytes).into_owned())
}

async fn ws_call(addr: std::net::SocketAddr, send: &[String]) -> Vec<String> {
    use futures_util::{SinkExt as _, StreamExt as _};
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/websocket"))
        .await
        .unwrap();
    let mut out = Vec::new();
    // gorilla/websocket: after a read timeout the connection is unusable,
    // so the Go recorder saw nothing more.
    let mut broken = false;
    for m in send {
        if broken {
            out.push("<none>".into());
            continue;
        }
        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            m.clone().into(),
        ))
        .await
        .unwrap();
        let r = tokio::time::timeout(std::time::Duration::from_millis(1500), ws.next()).await;
        if let Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t)))) = r {
            out.push(t.to_string());
        } else {
            broken = true;
            out.push("<none>".into());
        }
    }
    out
}

async fn grpc_call(
    addr: std::net::SocketAddr,
    method: &str,
    req: Vec<u8>,
) -> (i32, String, Vec<String>) {
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut client = tonic::client::Grpc::new(channel);
    client.ready().await.unwrap();
    let path: axum::http::uri::PathAndQuery = method.parse().unwrap();
    let streaming = method.ends_with("/BlockByHeight") || method.ends_with("/BlockByHash");
    if streaming {
        match client
            .server_streaming(tonic::Request::new(req), path, rpc::grpc::RawCodec)
            .await
        {
            Ok(resp) => {
                let mut s = resp.into_inner();
                let mut frames = Vec::new();
                loop {
                    match s.message().await {
                        Ok(Some(m)) => frames.push(hex::encode(m)),
                        Ok(None) => return (0, String::new(), frames),
                        Err(st) => return (st.code() as i32, st.message().to_owned(), frames),
                    }
                }
            }
            Err(st) => (st.code() as i32, st.message().to_owned(), Vec::new()),
        }
    } else {
        match client
            .unary(tonic::Request::new(req), path, rpc::grpc::RawCodec)
            .await
        {
            Ok(r) => (0, String::new(), vec![hex::encode(r.into_inner())]),
            Err(st) => (st.code() as i32, st.message().to_owned(), Vec::new()),
        }
    }
}

fn status_norm_grpc(hex_frame: &str) -> String {
    // Keep the chain fields (sync info); the node identity comes from the
    // Go node and the validator key is the frozen one in both.
    let bz = hex::decode(hex_frame).unwrap_or_default();
    let mut r = ctypes::proto::rpc::grpc::StatusResponse::decode(bz.as_slice()).unwrap_or_default();
    if let Some(s) = r.sync_info.as_mut() {
        s.catching_up = false;
    }
    hex::encode(r.encode_to_vec())
}

async fn run_scenario(dir: &Path) -> Stats {
    let corpus: Value = serde_json::from_slice(&gunzip(&dir.join("corpus.json.gz"))).unwrap();
    let meta: Value =
        serde_json::from_slice(&std::fs::read(dir.join("meta.json")).unwrap()).unwrap();
    let host = corpus["host"].as_str().unwrap().to_owned();
    let entries = corpus["entries"].as_array().unwrap();
    let status = entries
        .iter()
        .find(|e| e["path"] == "/status")
        .map(|e| serde_json::from_str::<Value>(e["resp"].as_str().unwrap()).unwrap())
        .unwrap();
    let _ = &meta;
    let frozen =
        nodestore::genesis::pub_key_from_json(&status["result"]["validator_info"]["pub_key"])
            .unwrap();
    let dir2 = dir.to_owned();
    let chain = tokio::task::spawn_blocking(move || build_chain(&dir2, &status, frozen))
        .await
        .unwrap();
    let env = chain.env.clone();
    println!(
        "{}: rebuilt {} heights",
        dir.file_name().unwrap().to_string_lossy(),
        chain.height
    );

    let rpc_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rpc_addr = rpc_listener.local_addr().unwrap();
    tokio::spawn(rpc::server::serve(rpc_listener, env.clone()));
    let grpc_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grpc_addr = grpc_listener.local_addr().unwrap();
    tokio::spawn(rpc::grpc::serve(grpc_listener, env.clone()));

    let mut st = Stats::default();
    for e in entries {
        let route = route_of(e);
        let norm = e["norm"].as_str().unwrap_or_default();
        match e["kind"].as_str().unwrap() {
            "http" => {
                let method = e["method"].as_str().unwrap();
                let path = e["path"].as_str().unwrap();
                let body = e["body"].as_str().unwrap_or_default();
                let (status, ct, cc, resp) = http_call(&env, &host, method, path, body).await;
                let go = e["resp"].as_str().unwrap_or_default();
                let go_status = e["status"].as_u64().unwrap_or(0);
                let go_ct = e["content_type"].as_str().unwrap_or_default();
                let go_cc = e["cache_control"].as_str().unwrap_or_default();
                let (g, r) = if go.contains("tx parse error [") {
                    // CheckTx of an undecodable tx: Go's error text embeds the
                    // Go binary's protobuf-go `detrand` separator after
                    // "proto:" and an error-stack location; compare the rest.
                    (normalize_decode_error(go), normalize_decode_error(&resp))
                } else if norm.is_empty() {
                    (go.to_owned(), resp.clone())
                } else {
                    (normalize(norm, go), normalize(norm, &resp))
                };
                let ok = u64::from(status) == go_status && ct == go_ct && cc == go_cc && g == r;
                st.record(&route, ok, || {
                    format!(
                        "{method} {path} {}\n  go:   {go_status} [{go_ct}] [{go_cc}] {}\n  rust: {status} [{ct}] [{cc}] {}\n  {}",
                        short(body),
                        short(&g),
                        short(&r),
                        first_diff(&g, &r)
                    )
                });
            }
            "ws" => {
                let send: Vec<String> = e["send"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|s| s.as_str().unwrap().to_owned())
                    .collect();
                let go: Vec<String> = e["frames"]
                    .as_array()
                    .map(|a| a.iter().map(|s| s.as_str().unwrap().to_owned()).collect())
                    .unwrap_or_default();
                let rust = ws_call(rpc_addr, &send).await;
                for (i, m) in send.iter().enumerate() {
                    let g = go.get(i).cloned().unwrap_or_default();
                    let r = rust.get(i).cloned().unwrap_or_default();
                    st.record("websocket", g == r, || {
                        format!("WS {m}\n  go:   {}\n  rust: {}", short(&g), short(&r))
                    });
                }
            }
            "grpc" => {
                let method = e["grpc"].as_str().unwrap();
                let req = hex::decode(e["req"].as_str().unwrap_or_default()).unwrap();
                let (code, msg, frames) = grpc_call(grpc_addr, method, req).await;
                let mut go_frames: Vec<String> = e["frames"]
                    .as_array()
                    .map(|a| a.iter().map(|s| s.as_str().unwrap().to_owned()).collect())
                    .unwrap_or_default();
                let mut frames = frames;
                if norm == "grpc-status" {
                    go_frames = go_frames.iter().map(|f| status_norm_grpc(f)).collect();
                    frames = frames.iter().map(|f| status_norm_grpc(f)).collect();
                }
                let go_code = i32::try_from(e["code"].as_i64().unwrap_or(0)).unwrap();
                let go_msg = e["message"].as_str().unwrap_or_default();
                let ok = code == go_code && msg == go_msg && frames == go_frames;
                st.record(method, ok, || {
                    format!(
                        "GRPC {method} {}\n  go:   {go_code} {go_msg} {:?}\n  rust: {code} {msg} {:?}",
                        e["req"],
                        go_frames.iter().map(|f| short(f)).collect::<Vec<_>>(),
                        frames.iter().map(|f| short(f)).collect::<Vec<_>>()
                    )
                });
            }
            k => panic!("unknown kind {k}"),
        }
    }

    // Live WebSocket events: Go's frames per query must appear, in order,
    // among the Rust node's (Go subscribed only after block 1 or 2).
    let ws_lines = String::from_utf8(gunzip(&dir.join("ws.jsonl.gz"))).unwrap();
    let mut go_live: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in ws_lines.lines() {
        let v: Value = serde_json::from_str(line).unwrap();
        let frame = v["frame"].as_str().unwrap().to_owned();
        let fv: Value = serde_json::from_str(&frame).unwrap();
        let Some(q) = fv["result"]["query"].as_str() else {
            continue; // subscribe acknowledgements
        };
        go_live.entry(q.to_owned()).or_default().push(frame);
    }
    for (q, go_frames) in &go_live {
        let rust_frames: Vec<String> = chain
            .live
            .get(q)
            .map(|v| v.iter().map(|f| norm_event_frame(f)).collect())
            .unwrap_or_default();
        let go_frames: Vec<String> = go_frames.iter().map(|f| norm_event_frame(f)).collect();
        let start = rust_frames.iter().position(|r| *r == go_frames[0]);
        match start {
            None => st.record(&format!("live {q}"), false, || {
                format!(
                    "LIVE {q}: first Go frame not produced by Rust\n  go:   {}\n  rust[0]: {}",
                    short(&go_frames[0]),
                    short(rust_frames.first().map_or("", String::as_str))
                )
            }),
            Some(s) => {
                for (i, g) in go_frames.iter().enumerate() {
                    let r = rust_frames.get(s + i).cloned().unwrap_or_default();
                    st.record(&format!("live {q}"), *g == r, || {
                        format!(
                            "LIVE {q} #{i}\n  go:   {}\n  rust: {}\n  {}",
                            short(g),
                            short(&r),
                            first_diff(g, &r)
                        )
                    });
                }
            }
        }
    }
    st
}

#[test]
fn rpc_corpus_matches_go() {
    let root = corpus_root();
    let mut scenarios: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.join("corpus.json.gz").exists())
        .collect();
    scenarios.sort();
    if let Ok(only) = std::env::var("RPC_CORPUS") {
        scenarios.retain(|p| p.file_name().is_some_and(|n| n == only.as_str()));
    }
    assert!(!scenarios.is_empty(), "no corpus under {}", root.display());
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut failed = false;
    for dir in scenarios {
        let st = rt.block_on(run_scenario(&dir));
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        println!("{name}: {}/{} identical", st.ok, st.total);
        for (r, (n, ok)) in &st.by_route {
            if ok != n {
                println!("  {r}: {ok}/{n}");
            }
        }
        for f in &st.failures {
            println!("--- {name}\n{f}");
        }
        failed |= st.ok != st.total;
    }
    assert!(!failed, "RPC responses differ from Go");
}
