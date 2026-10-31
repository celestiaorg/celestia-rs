//! The JSON-RPC server (`rpc/jsonrpc/server`): URI routes (`GET
//! /block?height=5`), JSON-RPC 2.0 over HTTP POST (single and batch), and
//! WebSocket (`/websocket`, incl. event subscriptions), with Go's routing,
//! status codes, headers and error envelopes.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, State};
use axum::http::{HeaderMap, HeaderValue, Method, Request, Response, StatusCode, header};
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::sync::mpsc;

use crate::env::{ERR_HEAVY_LIMIT, Environment, MAX_QUERY_LENGTH, Route, find_route};
use crate::json::{self, J, Node, Val};
use crate::params::{self, Arg};
use crate::query::Query;

/// A JSON-RPC id (`JSONRPCIntID` / `JSONRPCStringID`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Id {
    /// Integer id.
    Int(i64),
    /// String id.
    Str(String),
}

impl Id {
    fn to_json(&self) -> J {
        match self {
            Self::Int(i) => J::num(i),
            Self::Str(s) => J::str(s),
        }
    }
}

/// `RPCResponse` (compact, as `json.Marshal` writes it).
#[derive(Debug, Clone)]
pub struct RpcResponse {
    id: Option<Id>,
    body: Result<String, (i32, &'static str, String)>,
}

impl RpcResponse {
    /// `NewRPCSuccessResponse`.
    #[must_use]
    pub fn ok(id: Option<Id>, result: &J) -> Self {
        Self {
            id,
            body: Ok(result.encode()),
        }
    }

    fn err(id: Option<Id>, code: i32, msg: &'static str, data: String) -> Self {
        Self {
            id,
            body: Err((code, msg, data)),
        }
    }

    /// `RPCParseError`.
    #[must_use]
    pub fn parse_error(e: &str) -> Self {
        Self::err(None, -32700, "Parse error. Invalid JSON", e.to_owned())
    }

    /// `RPCInvalidRequestError`.
    #[must_use]
    pub fn invalid_request(id: Option<Id>, e: &str) -> Self {
        Self::err(id, -32600, "Invalid Request", e.to_owned())
    }

    /// `RPCMethodNotFoundError`.
    #[must_use]
    pub fn method_not_found(id: Option<Id>) -> Self {
        Self::err(id, -32601, "Method not found", String::new())
    }

    /// `RPCInvalidParamsError`.
    #[must_use]
    pub fn invalid_params(id: Option<Id>, e: &str) -> Self {
        Self::err(id, -32602, "Invalid params", e.to_owned())
    }

    /// `RPCInternalError`.
    #[must_use]
    pub fn internal(id: Option<Id>, e: &str) -> Self {
        Self::err(id, -32603, "Internal error", e.to_owned())
    }

    /// `RPCServerError`.
    #[must_use]
    pub fn server_error(id: Option<Id>, e: &str) -> Self {
        Self::err(id, -32000, "Server error", e.to_owned())
    }

    /// The compact JSON.
    #[must_use]
    pub fn encode(&self) -> String {
        let mut s = String::from("{\"jsonrpc\":\"2.0\"");
        if let Some(id) = &self.id {
            s.push_str(",\"id\":");
            id.to_json().write(&mut s);
        }
        match &self.body {
            Ok(r) => {
                s.push_str(",\"result\":");
                s.push_str(r);
            }
            Err((code, msg, data)) => {
                let mut e = J::obj([("code", J::num(code)), ("message", J::str(*msg))]);
                if !data.is_empty() {
                    e.push("data", J::str(data));
                }
                s.push_str(",\"error\":");
                e.write(&mut s);
            }
        }
        s.push('}');
        s
    }
}

fn encode_responses(rs: &[RpcResponse]) -> String {
    if rs.len() == 1 {
        return rs[0].encode();
    }
    let mut s = String::from("[");
    for (i, r) in rs.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&r.encode());
    }
    s.push(']');
    s
}

/// A parsed `RPCRequest` (`None` id = notification).
#[derive(Debug, Clone)]
pub struct RpcRequest {
    /// Id.
    pub id: Option<Id>,
    /// Method.
    pub method: String,
    /// Raw params (empty when absent).
    pub params: String,
}

const UNSAFE_REQ_TYPE: &str = "struct { JSONRPC string \"json:\\\"jsonrpc\\\"\"; ID interface {} \"json:\\\"id,omitempty\\\"\"; Method string \"json:\\\"method\\\"\"; Params json.RawMessage \"json:\\\"params\\\"\" }";

/// Go `fmt` `%v` of a decoded `interface{}`, without recursion (an id may
/// nest 10 000 levels deep).
fn go_value_string(n: &Node<'_>) -> String {
    enum Item<'b, 'a> {
        Node(&'b Node<'a>),
        Text(String),
    }
    let mut out = String::new();
    let mut stack = vec![Item::Node(n)];
    while let Some(item) = stack.pop() {
        let n = match item {
            Item::Text(s) => {
                out.push_str(&s);
                continue;
            }
            Item::Node(n) => n,
        };
        match &n.v {
            Val::Bool(b) => out.push_str(&b.to_string()),
            Val::Arr(items) => {
                out.push('[');
                stack.push(Item::Text("]".into()));
                for (i, x) in items.iter().enumerate().rev() {
                    stack.push(Item::Node(x));
                    if i > 0 {
                        stack.push(Item::Text(" ".into()));
                    }
                }
            }
            Val::Obj(f) => {
                let mut keys: Vec<_> = f.iter().collect();
                keys.sort_by(|a, b| a.0.cmp(&b.0));
                out.push_str("map[");
                stack.push(Item::Text("]".into()));
                for (i, (k, v)) in keys.iter().enumerate().rev() {
                    stack.push(Item::Node(v));
                    stack.push(Item::Text(format!("{k}:")));
                    if i > 0 {
                        stack.push(Item::Text(" ".into()));
                    }
                }
            }
            Val::Num(t) => out.push_str(t),
            Val::Str(s) => out.push_str(s),
            Val::Null => out.push_str("<nil>"),
        }
    }
    out
}

fn go_type_string(n: &Node<'_>) -> &'static str {
    match &n.v {
        Val::Bool(_) => "bool",
        Val::Arr(_) => "[]interface {}",
        Val::Obj(_) => "map[string]interface {}",
        Val::Num(_) => "float64",
        Val::Str(_) => "string",
        Val::Null => "<nil>",
    }
}

/// `RPCRequest.UnmarshalJSON` of a parsed value.
fn request_from_node(n: &Node<'_>) -> Result<RpcRequest, String> {
    let Val::Obj(fields) = &n.v else {
        return Err(format!(
            "json: cannot unmarshal {} into Go value of type {UNSAFE_REQ_TYPE}",
            n.v.kind()
        ));
    };
    let mut id = None;
    let mut method = String::new();
    let mut params = String::new();
    let mut first_err: Option<String> = None;
    for (k, v) in fields {
        let key = k.to_ascii_lowercase();
        match key.as_str() {
            "jsonrpc" | "method" => {
                let val = match &v.v {
                    Val::Str(s) => Some(s.clone()),
                    Val::Null => None,
                    other => {
                        if first_err.is_none() {
                            first_err = Some(format!(
                                "json: cannot unmarshal {} into Go struct field .{key} of type string",
                                other.kind()
                            ));
                        }
                        None
                    }
                };
                if key == "method"
                    && let Some(m) = val
                {
                    method = m;
                }
            }
            "id" => id = Some(v),
            "params" => params = v.raw.to_owned(),
            _ => {}
        }
    }
    if let Some(e) = first_err {
        return Err(e);
    }
    let id = match id {
        None => None,
        Some(v) => match &v.v {
            Val::Null => None,
            Val::Str(s) => Some(Id::Str(s.clone())),
            Val::Num(t) => {
                let f: f64 = t.parse().unwrap_or(0.0);
                #[allow(clippy::cast_possible_truncation)]
                Some(Id::Int(f as i64))
            }
            _ => {
                return Err(format!(
                    "json-rpc ID ({}) is of unknown type ({})",
                    go_value_string(v),
                    go_type_string(v)
                ));
            }
        },
    };
    if id.is_none() {
        // A notification: Go leaves the request empty.
        return Ok(RpcRequest {
            id: None,
            method: String::new(),
            params: String::new(),
        });
    }
    Ok(RpcRequest { id, method, params })
}

/// Parses a request body as Go's handler does: a batch first, then a
/// single request. `Err` is the parse error text.
pub fn parse_requests(body: &str) -> Result<Vec<RpcRequest>, String> {
    let n = json::parse(body).map_err(|e| format!("error unmarshaling request: {e}"))?;
    if let Val::Arr(items) = &n.v {
        let batch: Result<Vec<_>, _> = items.iter().map(request_from_node).collect();
        if let Ok(b) = batch {
            return Ok(b);
        }
    }
    request_from_node(&n)
        .map(|r| vec![r])
        .map_err(|e| format!("error unmarshaling request: {e}"))
}

/// The outcome of calling a route.
enum CallError {
    /// Internal error (`RPCInternalError`).
    Internal(String),
    /// Go recovers while dereferencing an absent Query response. Preserve its
    /// transport-level outcome explicitly without panicking a Rust task.
    EmptyQueryResponse,
    /// Heavy-request limit.
    Busy,
}

/// Calls a route (blocking work on the blocking pool;
/// `broadcast_tx_commit` waits for the tx event).
async fn call_route(
    env: &Arc<Environment>,
    route: &'static Route,
    args: Vec<Arg>,
    remote: &str,
) -> Result<J, CallError> {
    let Some(guard) = env.try_acquire(route) else {
        return Err(CallError::Busy);
    };
    if route.name == "broadcast_tx_commit" {
        return broadcast_tx_commit(
            env,
            args.first().map(|a| a.bytes().to_vec()).unwrap_or_default(),
            remote,
        )
        .await
        .map_err(CallError::Internal);
    }
    let env2 = Arc::clone(env);
    let name = route.name;
    tokio::task::spawn_blocking(move || {
        let _guard = guard;
        if name == "abci_query" {
            let arg = |i: usize| args.get(i).cloned().unwrap_or(Arg::Null);
            env2.abci_query_response(arg(0).str(), arg(1).bytes(), arg(2).int(), arg(3).bool())
                .map_err(CallError::Internal)?
                .ok_or(CallError::EmptyQueryResponse)
        } else {
            env2.call(name, &args).map_err(CallError::Internal)
        }
    })
    .await
    .map_err(|e| CallError::Internal(e.to_string()))?
}

/// Typed result shared by JSON-RPC and core gRPC broadcast adapters.
#[derive(Debug)]
pub(crate) struct BroadcastCommit {
    /// Mempool admission result.
    pub check_tx: ctypes::proto::abci::ResponseCheckTx,
    /// Execution result, or its zero value when admission failed.
    pub tx_result: ctypes::proto::abci::ExecTxResult,
    /// Transaction hash.
    pub hash: Vec<u8>,
    /// Inclusion height, or zero when admission failed.
    pub height: i64,
}

struct TxSubscription {
    bus: Arc<crate::events::EventBus>,
    client: String,
    query: Query,
}

impl Drop for TxSubscription {
    fn drop(&mut self) {
        let _ = self.bus.unsubscribe(&self.client, &self.query);
    }
}

async fn broadcast_tx_commit(
    env: &Arc<Environment>,
    tx: Vec<u8>,
    remote: &str,
) -> Result<J, String> {
    let result = broadcast_commit(env, tx, remote).await?;
    Ok(J::obj([
        (
            "check_tx",
            crate::encode::response_check_tx(&result.check_tx),
        ),
        (
            "tx_result",
            crate::encode::exec_tx_result(&result.tx_result),
        ),
        ("hash", J::hex(&result.hash)),
        ("height", J::i64s(result.height)),
    ]))
}

/// Shared `BroadcastTxCommit` semantics for Comet and core gRPC.
pub(crate) async fn broadcast_commit(
    env: &Arc<Environment>,
    tx: Vec<u8>,
    remote: &str,
) -> Result<BroadcastCommit, String> {
    let bus = &env.event_bus;
    if bus.num_clients() >= env.config.max_subscription_clients {
        return Err(format!(
            "max_subscription_clients {} reached",
            env.config.max_subscription_clients
        ));
    }
    if bus.num_client_subscriptions(remote) >= env.config.max_subscriptions_per_client {
        return Err(format!(
            "max_subscriptions_per_client {} reached",
            env.config.max_subscriptions_per_client
        ));
    }
    let hash = ctypes::Tx(tx.clone()).hash();
    let q = Query::new(&format!(
        "tm.event='Tx' AND tx.hash='{}'",
        hex::encode_upper(&hash)
    ))?;
    let mut sub = bus
        .subscribe(remote, q.clone(), 1)
        .map_err(|e| format!("failed to subscribe to tx: {e}"))?;
    let _subscription = TxSubscription {
        bus: Arc::clone(bus),
        client: remote.to_owned(),
        query: q,
    };
    let env2 = Arc::clone(env);
    let tx2 = tx.clone();
    let check = tokio::task::spawn_blocking(move || env2.mempool_check_tx(&tx2))
        .await
        .map_err(|e| e.to_string())?;
    async {
        let check = check.map_err(|e| format!("error on broadcastTxCommit: {e}"))?;
        if check.code != 0 {
            return Ok(BroadcastCommit {
                check_tx: check,
                tx_result: ctypes::proto::abci::ExecTxResult::default(),
                hash,
                height: 0,
            });
        }
        let timeout = Duration::from_millis(env.config.timeout_broadcast_tx_commit_ms);
        tokio::select! {
            m = sub.out.recv() => match m {
                Some(m) => {
                    let crate::events::EventData::Tx { tx_result } = m.data.as_ref() else {
                        return Err("unexpected event data".into());
                    };
                    Ok(BroadcastCommit {
                        check_tx: check,
                        tx_result: tx_result.result.clone().unwrap_or_default(),
                        hash,
                        height: tx_result.height,
                    })
                }
                None => Err("txSub was canceled (reason: CometBFT exited)".to_owned()),
            },
            r = &mut sub.cancelled => Err(format!(
                "txSub was canceled (reason: {})",
                r.unwrap_or_else(|_| "CometBFT exited".into())
            )),
            () = tokio::time::sleep(timeout) => Err("timed out waiting for tx to be included in a block".into()),
        }
    }
    .await
}

// --- HTTP ------------------------------------------------------------------

#[derive(Clone)]
struct Ctx {
    env: Arc<Environment>,
    remote: SocketAddr,
}

fn json_response(status: StatusCode, body: String, cache: bool) -> Response<Body> {
    let mut r = Response::new(Body::from(body));
    *r.status_mut() = status;
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    if cache {
        r.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=86400"),
        );
    }
    r
}

fn not_found() -> Response<Body> {
    let mut r = Response::new(Body::from("404 page not found\n"));
    *r.status_mut() = StatusCode::NOT_FOUND;
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    r.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    r
}

fn uri_id() -> Option<Id> {
    Some(Id::Int(-1))
}

/// Go `url.ParseQuery` + `Values.Get` (first value; `+` is a space).
fn query_param(query: &str, name: &str) -> Option<String> {
    for kv in query.split('&') {
        if kv.contains(';') {
            continue;
        }
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        let Some(k) = url_unescape(k) else { continue };
        if k == name {
            return url_unescape(v);
        }
    }
    None
}

fn url_unescape(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                let h = b.get(i + 1..i + 3)?;
                let v = u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()?;
                out.push(v);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

async fn uri_handler(
    ctx: &Ctx,
    route: &'static Route,
    query: &str,
    form: Option<&str>,
) -> Response<Body> {
    if route.ws {
        return json_response(
            StatusCode::NOT_FOUND,
            RpcResponse::method_not_found(uri_id()).encode(),
            false,
        );
    }
    let mut args = Vec::new();
    for (name, ty) in route.args {
        let v = query_param(query, name)
            .filter(|v| !v.is_empty())
            .or_else(|| form.and_then(|f| query_param(f, name)));
        let a = match v.as_deref() {
            None | Some("") => Arg::Null,
            Some(raw) => match params::uri_arg(*ty, raw) {
                Ok(a) => a,
                Err(e) => {
                    return json_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        RpcResponse::invalid_params(
                            uri_id(),
                            &format!("error converting http params to arguments: {e}"),
                        )
                        .encode(),
                        false,
                    );
                }
            },
        };
        args.push(a);
    }
    let cache = route.cacheable_with(&args);
    match call_route(&ctx.env, route, args, &ctx.remote.to_string()).await {
        Ok(j) => json_response(
            StatusCode::OK,
            RpcResponse::ok(uri_id(), &j).encode(),
            cache,
        ),
        Err(CallError::Busy) => json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            RpcResponse::internal(uri_id(), ERR_HEAVY_LIMIT).encode(),
            false,
        ),
        Err(CallError::EmptyQueryResponse) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            RpcResponse::internal(uri_id(), crate::env::ERR_EMPTY_QUERY_RESPONSE).encode(),
            false,
        ),
        Err(CallError::Internal(e)) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            RpcResponse::internal(uri_id(), &e).encode(),
            false,
        ),
    }
}

/// `writeListOfEndpoints`.
fn list_endpoints(host: &str, unsafe_routes: bool) -> Response<Body> {
    let mut routes: Vec<&Route> = crate::env::ROUTES.iter().collect();
    if unsafe_routes {
        routes.extend(crate::env::UNSAFE_ROUTES.iter());
    }
    routes.sort_by_key(|r| r.name);
    let mut buf = String::from(
        "<html><body><br>Available endpoints:<br><br>Endpoints that require arguments:<br>",
    );
    for r in routes {
        let mut link = format!("//{host}/{}?", r.name);
        for (i, (a, _)) in r.args.iter().enumerate() {
            link.push_str(a);
            link.push_str("=_");
            if i + 1 < r.args.len() {
                link.push('&');
            }
        }
        buf.push_str(&format!("<a href=\"{link}\">{link}</a></br>"));
    }
    buf.push_str("</body></html>");
    let mut resp = Response::new(Body::from(buf));
    resp.headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html"));
    resp
}

async fn jsonrpc_handler(ctx: &Ctx, body: &[u8], host: &str) -> Response<Body> {
    if body.is_empty() {
        return list_endpoints(host, ctx.env.config.unsafe_routes);
    }
    let text = json::go_string(body);
    let requests = match parse_requests(&text) {
        Ok(r) => r,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                RpcResponse::parse_error(&e).encode(),
                false,
            );
        }
    };
    let mut responses = Vec::new();
    let mut cache = true;
    for req in requests {
        let Some(id) = req.id.clone() else { continue };
        let route = find_route(&req.method, ctx.env.config.unsafe_routes);
        let Some(route) = route.filter(|r| !r.ws) else {
            responses.push(RpcResponse::method_not_found(Some(id)));
            cache = false;
            continue;
        };
        let args = if req.params.is_empty() {
            if !route.args.is_empty() {
                // Go's reflect call panics: the whole response is replaced.
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    RpcResponse::internal(uri_id(), "reflect: Call with too few input arguments")
                        .encode(),
                    false,
                );
            }
            Vec::new()
        } else {
            let names: Vec<&str> = route.args.iter().map(|(n, _)| *n).collect();
            let types: Vec<params::ArgType> = route.args.iter().map(|(_, t)| *t).collect();
            match params::json_args(&names, &types, &req.params) {
                Ok(a) => a,
                Err(e) => {
                    responses.push(RpcResponse::invalid_params(
                        Some(id),
                        &format!("error converting json params to arguments: {e}"),
                    ));
                    cache = false;
                    continue;
                }
            }
        };
        if cache && !route.cacheable_with(&args) {
            cache = false;
        }
        match call_route(&ctx.env, route, args, &ctx.remote.to_string()).await {
            Ok(j) => responses.push(RpcResponse::ok(Some(id), &j)),
            Err(CallError::Busy) => {
                responses.push(RpcResponse::internal(Some(id), ERR_HEAVY_LIMIT));
                cache = false;
            }
            Err(CallError::EmptyQueryResponse) => {
                // Go's outer HTTP recovery replaces the entire response, even
                // for a batch with earlier successful elements.
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    RpcResponse::internal(uri_id(), crate::env::ERR_EMPTY_QUERY_RESPONSE).encode(),
                    false,
                );
            }
            Err(CallError::Internal(e)) => responses.push(RpcResponse::internal(Some(id), &e)),
        }
    }
    if responses.is_empty() {
        return Response::new(Body::empty());
    }
    json_response(StatusCode::OK, encode_responses(&responses), cache)
}

/// Applies the CORS policy (`rs/cors` subset).
fn cors_headers(
    env: &Environment,
    req_headers: &HeaderMap,
    method: &Method,
    resp: &mut Response<Body>,
) {
    let cfg = &env.config;
    if cfg.cors_allowed_origins.is_empty() {
        return;
    }
    resp.headers_mut()
        .append(header::VARY, HeaderValue::from_static("Origin"));
    let Some(origin) = req_headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    else {
        return;
    };
    let all = cfg.cors_allowed_origins.iter().any(|o| o == "*");
    if !all
        && !cfg
            .cors_allowed_origins
            .iter()
            .any(|o| o.eq_ignore_ascii_case(origin))
    {
        return;
    }
    if !cfg
        .cors_allowed_methods
        .iter()
        .any(|m| m.eq_ignore_ascii_case(method.as_str()))
        && *method != Method::OPTIONS
    {
        return;
    }
    let v = if all { "*" } else { origin };
    if let Ok(v) = HeaderValue::from_str(v) {
        resp.headers_mut()
            .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, v);
    }
}

fn cors_preflight(env: &Environment, req_headers: &HeaderMap) -> Option<Response<Body>> {
    let cfg = &env.config;
    if cfg.cors_allowed_origins.is_empty() {
        return None;
    }
    let req_method = req_headers.get(header::ACCESS_CONTROL_REQUEST_METHOD)?;
    let mut r = Response::new(Body::empty());
    *r.status_mut() = StatusCode::NO_CONTENT;
    for v in [
        "Origin",
        "Access-Control-Request-Method",
        "Access-Control-Request-Headers",
    ] {
        r.headers_mut()
            .append(header::VARY, HeaderValue::from_static(v));
    }
    let origin = req_headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())?;
    let all = cfg.cors_allowed_origins.iter().any(|o| o == "*");
    if !all
        && !cfg
            .cors_allowed_origins
            .iter()
            .any(|o| o.eq_ignore_ascii_case(origin))
    {
        return Some(r);
    }
    let m = req_method.to_str().unwrap_or_default().to_ascii_uppercase();
    if !cfg
        .cors_allowed_methods
        .iter()
        .any(|x| x.eq_ignore_ascii_case(&m))
    {
        return Some(r);
    }
    if let Ok(v) = HeaderValue::from_str(if all { "*" } else { origin }) {
        r.headers_mut()
            .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, v);
    }
    if let Ok(v) = HeaderValue::from_str(&m) {
        r.headers_mut()
            .insert(header::ACCESS_CONTROL_ALLOW_METHODS, v);
    }
    if let Some(h) = req_headers.get(header::ACCESS_CONTROL_REQUEST_HEADERS) {
        r.headers_mut()
            .insert(header::ACCESS_CONTROL_ALLOW_HEADERS, h.clone());
    }
    Some(r)
}

async fn handle(State(ctx): State<Ctx>, req: Request<Body>) -> Response<Body> {
    let mut resp = handle_inner(&ctx, req).await;
    if let Ok(v) = HeaderValue::from_str(&unix_now().to_string()) {
        resp.headers_mut().insert("x-server-time", v);
    }
    resp
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

async fn handle_inner(ctx: &Ctx, req: Request<Body>) -> Response<Body> {
    let (mut parts, body) = req.into_parts();
    let path = parts.uri.path().to_owned();
    if path == "/websocket" {
        let upgrade = WebSocketUpgrade::from_request_parts(&mut parts, &()).await;
        return if let Ok(up) = upgrade {
            let env = Arc::clone(&ctx.env);
            let remote = ctx.remote.to_string();
            up.max_message_size(ctx.env.config.max_body_bytes.max(1))
                .on_upgrade(move |socket| ws_connection(socket, env, remote))
        } else {
            let mut r = Response::new(Body::from("Bad Request\n"));
            *r.status_mut() = StatusCode::BAD_REQUEST;
            r.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; charset=utf-8"),
            );
            r.headers_mut()
                .insert("sec-websocket-version", HeaderValue::from_static("13"));
            r
        };
    }
    if parts.method == Method::OPTIONS
        && let Some(r) = cors_preflight(&ctx.env, &parts.headers)
    {
        return r;
    }
    let max = ctx.env.config.max_body_bytes;
    let Ok(body) = axum::body::to_bytes(body, max).await else {
        {
            return json_response(
                StatusCode::BAD_REQUEST,
                RpcResponse::invalid_request(
                    None,
                    "error reading request body: http: request body too large",
                )
                .encode(),
                false,
            );
        }
    };
    // PreChecksHandler: oversized batches are rejected on every path.
    if ctx.env.config.max_request_batch_size > 0 {
        let text = json::go_string(&body);
        if let Ok(n) = json::parse(&text)
            && let Val::Arr(items) = &n.v
        {
            let all_requests = items.iter().all(|i| request_from_node(i).is_ok());
            if all_requests && items.len() > ctx.env.config.max_request_batch_size {
                return json_response(
                    StatusCode::OK,
                    RpcResponse::invalid_request(
                        None,
                        &format!(
                            "batch request exceeds maximum ({}) allowed number of requests",
                            ctx.env.config.max_request_batch_size
                        ),
                    )
                    .encode(),
                    false,
                );
            }
        }
    }
    let mut resp = if path == "/" {
        let host = parts
            .headers
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        jsonrpc_handler(ctx, &body, &host).await
    } else if let Some(route) = path
        .strip_prefix('/')
        .and_then(|n| find_route(n, ctx.env.config.unsafe_routes))
    {
        let query = parts.uri.query().unwrap_or_default().to_owned();
        let is_form = parts
            .headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("application/x-www-form-urlencoded"));
        let form = (is_form && matches!(parts.method, Method::POST | Method::PUT | Method::PATCH))
            .then(|| String::from_utf8_lossy(&body).into_owned());
        uri_handler(ctx, route, &query, form.as_deref()).await
    } else {
        not_found()
    };
    cors_headers(&ctx.env, &parts.headers, &parts.method, &mut resp);
    resp
}

/// The HTTP router (every path goes through Go's mux logic).
pub fn router(env: Arc<Environment>, remote: SocketAddr) -> axum::Router {
    axum::Router::new()
        .fallback(handle)
        .with_state(Ctx { env, remote })
}

/// Serves the RPC on a bound listener, at most `max_open_connections`
/// connections at a time (`netutil.LimitListener`).
pub async fn serve(
    listener: tokio::net::TcpListener,
    env: Arc<Environment>,
) -> std::io::Result<()> {
    let limit = env.config.max_open_connections;
    let sem = (limit > 0).then(|| Arc::new(tokio::sync::Semaphore::new(limit)));
    loop {
        let permit = match &sem {
            Some(s) => Some(
                Arc::clone(s)
                    .acquire_owned()
                    .await
                    .map_err(|e| std::io::Error::other(e.to_string()))?,
            ),
            None => None,
        };
        let (stream, remote) = listener.accept().await?;
        let app = router(Arc::clone(&env), remote);
        tokio::spawn(async move {
            let _permit = permit;
            let svc = hyper_util::service::TowerToHyperService::new(app);
            let builder =
                hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new());
            if let Err(e) = builder
                .serve_connection_with_upgrades(hyper_util::rt::TokioIo::new(stream), svc)
                .await
            {
                tracing::debug!(error = %e, %remote, "rpc connection error");
            }
        });
    }
}

// --- WebSocket -------------------------------------------------------------

/// Builds the event frame of a subscription (`ResultEvent`).
#[must_use]
pub fn event_frame(id: &Option<Id>, query: &str, msg: &crate::events::Message) -> String {
    let mut events = String::from("{");
    for (i, (k, vs)) in msg.events.iter().enumerate() {
        if i > 0 {
            events.push(',');
        }
        events.push_str(&json::quote(k));
        events.push(':');
        J::Arr(vs.iter().map(J::str).collect()).write(&mut events);
    }
    events.push('}');
    let mut result = String::from("{\"query\":");
    result.push_str(&json::quote(query));
    result.push_str(",\"data\":");
    result.push_str(msg.data_json());
    result.push_str(",\"events\":");
    result.push_str(&events);
    result.push('}');
    RpcResponse {
        id: id.clone(),
        body: Ok(result),
    }
    .encode()
}

async fn ws_connection(socket: WebSocket, env: Arc<Environment>, remote: String) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::channel::<String>(env.config.websocket_write_buffer_size.max(1));
    let writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(Duration::from_secs(27));
        ping.tick().await;
        loop {
            tokio::select! {
                m = rx.recv() => match m {
                    Some(m) => if sink.send(WsMessage::Text(m)).await.is_err() { break },
                    None => break,
                },
                _ = ping.tick() => if sink.send(WsMessage::Ping(Vec::new())).await.is_err() { break },
            }
        }
    });
    while let Some(msg) = stream.next().await {
        let text = match msg {
            Ok(WsMessage::Text(t)) => t,
            Ok(WsMessage::Binary(b)) => json::go_string(&b),
            Ok(WsMessage::Close(_)) | Err(_) => break,
            Ok(_) => continue,
        };
        if let Some(r) = ws_request(&env, &remote, &text, &tx).await
            && tx.send(r).await.is_err()
        {
            break;
        }
    }
    let _ = env.event_bus.unsubscribe_all(&remote);
    drop(tx);
    let _ = writer.await;
}

/// Handles one WebSocket message; `None` when nothing is written back
/// (notifications).
async fn ws_request(
    env: &Arc<Environment>,
    remote: &str,
    text: &str,
    tx: &mpsc::Sender<String>,
) -> Option<String> {
    // `json.NewDecoder(r).Decode(&request)`: one value, trailing data ignored.
    let n = match json::parse(text) {
        Ok(n) => n,
        Err(e) if e.contains("after top-level value") => {
            let end = first_value_end(text);
            match json::parse(&text[..end]) {
                Ok(n) => n,
                Err(e) => {
                    return Some(
                        RpcResponse::parse_error(&format!("error unmarshaling request: {e}"))
                            .encode(),
                    );
                }
            }
        }
        Err(e) => {
            return Some(
                RpcResponse::parse_error(&format!("error unmarshaling request: {e}")).encode(),
            );
        }
    };
    let req = match request_from_node(&n) {
        Ok(r) => r,
        Err(e) => {
            return Some(
                RpcResponse::parse_error(&format!("error unmarshaling request: {e}")).encode(),
            );
        }
    };
    let id = req.id.clone()?;
    let Some(route) = find_route(&req.method, env.config.unsafe_routes) else {
        return Some(RpcResponse::method_not_found(Some(id)).encode());
    };
    let args = if req.params.is_empty() {
        if !route.args.is_empty() {
            return Some(
                RpcResponse::internal(
                    uri_id(),
                    "WSJSONRPC: reflect: Call with too few input arguments",
                )
                .encode(),
            );
        }
        Vec::new()
    } else {
        let names: Vec<&str> = route.args.iter().map(|(n, _)| *n).collect();
        let types: Vec<params::ArgType> = route.args.iter().map(|(_, t)| *t).collect();
        match params::json_args(&names, &types, &req.params) {
            Ok(a) => a,
            Err(e) => {
                return Some(
                    RpcResponse::internal(
                        Some(id),
                        &format!("error converting json params to arguments: {e}"),
                    )
                    .encode(),
                );
            }
        }
    };
    let res = match route.name {
        "subscribe" => ws_subscribe(
            env,
            remote,
            args.first().map(|a| a.str().to_owned()).unwrap_or_default(),
            &id,
            tx,
        ),
        "unsubscribe" => {
            let q = args.first().map(|a| a.str().to_owned()).unwrap_or_default();
            Query::new(&q)
                .map_err(|e| format!("failed to parse query: {e}"))
                .and_then(|q| env.event_bus.unsubscribe(remote, &q))
                .map(|()| J::Obj(Vec::new()))
        }
        "unsubscribe_all" => env
            .event_bus
            .unsubscribe_all(remote)
            .map(|()| J::Obj(Vec::new())),
        _ => match call_route(env, route, args, remote).await {
            Ok(j) => Ok(j),
            Err(CallError::Busy) => Err(ERR_HEAVY_LIMIT.to_owned()),
            Err(CallError::EmptyQueryResponse) => {
                return Some(
                    RpcResponse::internal(uri_id(), crate::env::ERR_EMPTY_QUERY_RESPONSE).encode(),
                );
            }
            Err(CallError::Internal(e)) => Err(e),
        },
    };
    Some(match res {
        Ok(j) => RpcResponse::ok(Some(id), &j).encode(),
        Err(e) => RpcResponse::internal(Some(id), &e).encode(),
    })
}

/// End offset of the first JSON value in `s` (for decoder semantics).
fn first_value_end(s: &str) -> usize {
    let b = s.as_bytes();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    let mut started = false;
    for (i, &c) in b.iter().enumerate() {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
                if depth == 0 {
                    return i + 1;
                }
            }
            continue;
        }
        match c {
            b'"' => {
                in_str = true;
                started = true;
            }
            b'{' | b'[' => {
                depth += 1;
                started = true;
            }
            b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            b' ' | b'\t' | b'\n' | b'\r' if started && depth == 0 => return i,
            _ => started = true,
        }
    }
    b.len()
}

fn ws_subscribe(
    env: &Arc<Environment>,
    remote: &str,
    query: String,
    id: &Id,
    tx: &mpsc::Sender<String>,
) -> Result<J, String> {
    let bus = &env.event_bus;
    if bus.num_clients() >= env.config.max_subscription_clients {
        return Err(format!(
            "max_subscription_clients {} reached",
            env.config.max_subscription_clients
        ));
    }
    if bus.num_client_subscriptions(remote) >= env.config.max_subscriptions_per_client {
        return Err(format!(
            "max_subscriptions_per_client {} reached",
            env.config.max_subscriptions_per_client
        ));
    }
    if query.len() > MAX_QUERY_LENGTH {
        return Err("maximum query length exceeded".into());
    }
    let q = Query::new(&query).map_err(|e| format!("failed to parse query: {e}"))?;
    let mut sub = bus.subscribe(remote, q, env.config.subscription_buffer_size)?;
    let tx = tx.clone();
    let id = Some(id.clone());
    let close_if_slow = env.config.close_on_slow_client;
    tokio::spawn(async move {
        loop {
            tokio::select! {
                m = sub.out.recv() => {
                    let Some(m) = m else { return };
                    let frame = event_frame(&id, &query, &m);
                    let sent = tokio::time::timeout(Duration::from_secs(10), tx.send(frame)).await;
                    if !matches!(sent, Ok(Ok(()))) && close_if_slow {
                        let _ = tx.try_send(
                            RpcResponse::server_error(id.clone(), "subscription was canceled (reason: slow client)").encode(),
                        );
                        return;
                    }
                }
                r = &mut sub.cancelled => {
                    let reason = r.unwrap_or_else(|_| "CometBFT exited".into());
                    if reason != crate::events::ERR_UNSUBSCRIBED {
                        let _ = tx.try_send(
                            RpcResponse::server_error(
                                id.clone(),
                                &format!("subscription was canceled (reason: {reason})"),
                            )
                            .encode(),
                        );
                    }
                    return;
                }
            }
        }
    });
    Ok(J::Obj(Vec::new()))
}

/// Events of a message as a sorted map (for tests and tools).
#[must_use]
pub fn events_map(m: &crate::events::Message) -> BTreeMap<String, Vec<String>> {
    (*m.events).clone()
}

#[cfg(test)]
#[allow(clippy::expect_used, missing_docs)]
pub(crate) mod broadcast_tests {
    use super::*;
    use std::sync::Mutex;

    use ctypes::proto::abci;
    use kvdb::MemBackend;
    use mempool::{CachedTx, Mempool, MempoolError, MempoolLock, TxInfo, TxKey};
    use nodestore::{BlockStore, StateStore};

    struct TestApp;

    impl crate::AppQuery for TestApp {
        fn info(&self) -> Result<abci::ResponseInfo, String> {
            Ok(abci::ResponseInfo::default())
        }

        fn query(&self, _: abci::RequestQuery) -> Result<abci::ResponseQuery, String> {
            Ok(abci::ResponseQuery::default())
        }

        fn check_tx(&self, _: &[u8]) -> Result<abci::ResponseCheckTx, String> {
            Ok(abci::ResponseCheckTx::default())
        }
    }

    struct TestNode;

    impl crate::NodeStatus for TestNode {
        fn node_info(&self) -> crate::NodeInfo {
            crate::NodeInfo::default()
        }

        fn is_listening(&self) -> bool {
            true
        }

        fn listeners(&self) -> Vec<String> {
            Vec::new()
        }

        fn peers(&self) -> Vec<crate::PeerInfo> {
            Vec::new()
        }

        fn wait_sync(&self) -> bool {
            false
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

    impl crate::EvidenceSink for NoEvidence {
        fn add_evidence(&self, _: ctypes::Evidence) -> Result<(), String> {
            Ok(())
        }
    }

    struct TestLock;

    impl MempoolLock for TestLock {
        fn flush_app_conn(&mut self) -> Result<(), MempoolError> {
            Ok(())
        }

        fn update(
            &mut self,
            _: i64,
            _: &[CachedTx],
            _: &[abci::ExecTxResult],
            _: Option<mempool::PreCheckFunc>,
            _: Option<mempool::PostCheckFunc>,
        ) -> Result<(), MempoolError> {
            Ok(())
        }
    }

    struct TestMempool {
        response: Mutex<Result<abci::ResponseCheckTx, MempoolError>>,
    }

    impl TestMempool {
        fn new(response: Result<abci::ResponseCheckTx, MempoolError>) -> Self {
            Self {
                response: Mutex::new(response),
            }
        }
    }

    impl Mempool for TestMempool {
        fn check_tx(&self, _: &[u8], info: TxInfo) -> Result<abci::ResponseCheckTx, MempoolError> {
            assert!(info.from_broadcast);
            self.response
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }

        fn remove_tx_by_key(&self, _: &TxKey) -> Result<(), MempoolError> {
            Ok(())
        }

        fn reap_max_bytes_max_gas(&self, _: i64, _: i64) -> Vec<CachedTx> {
            Vec::new()
        }

        fn reap_max_txs(&self, _: i64) -> Vec<CachedTx> {
            Vec::new()
        }

        fn lock(&self) -> Box<dyn MempoolLock + '_> {
            Box::new(TestLock)
        }

        fn flush(&self) {}

        fn enable_txs_available(&self) {}

        fn txs_available(&self) -> Option<Arc<tokio::sync::Notify>> {
            None
        }

        fn size(&self) -> usize {
            0
        }

        fn size_bytes(&self) -> i64 {
            0
        }

        fn get_tx_by_key(&self, _: &TxKey) -> Option<CachedTx> {
            None
        }

        fn was_recently_evicted(&self, _: &TxKey) -> bool {
            false
        }

        fn was_recently_rejected(&self, _: &TxKey) -> (bool, u32, String) {
            (false, 0, String::new())
        }
    }

    pub(crate) fn environment(
        response: Result<abci::ResponseCheckTx, MempoolError>,
        timeout_ms: u64,
    ) -> Arc<Environment> {
        let db: Arc<dyn kvdb::KvBackend> = Arc::new(MemBackend::new());
        let config = crate::RpcConfig {
            timeout_broadcast_tx_commit_ms: timeout_ms,
            ..Default::default()
        };
        Arc::new(Environment::new(
            BlockStore::new(Arc::clone(&db)).expect("memory block store"),
            StateStore::new(db),
            Arc::new(TestApp),
            Arc::new(TestMempool::new(response)),
            Arc::new(TestNode),
            Arc::new(NoEvidence),
            ctypes::PubKey::Ed25519([0; 32]),
            crate::GenesisData::default(),
            None,
            None,
            Arc::new(crate::EventBus::new()),
            config,
        ))
    }

    async fn wait_for_subscription(env: &Environment, client: &str) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while env.event_bus.num_client_subscriptions(client) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("broadcast subscription");
    }

    #[tokio::test]
    async fn commit_returns_full_result_and_releases_subscription() {
        let check = abci::ResponseCheckTx {
            gas_wanted: 7,
            ..Default::default()
        };
        let env = environment(Ok(check.clone()), 1_000);
        let tx = b"accepted transaction".to_vec();
        let task_env = Arc::clone(&env);
        let task_tx = tx.clone();
        let task =
            tokio::spawn(async move { broadcast_commit(&task_env, task_tx, "core-grpc").await });
        wait_for_subscription(&env, "core-grpc").await;

        let exec = abci::ExecTxResult {
            code: 9,
            log: "execution failed".into(),
            gas_used: 5,
            ..Default::default()
        };
        env.event_bus.publish_tx(abci::TxResult {
            height: 12,
            index: 0,
            tx: tx.clone(),
            result: Some(exec.clone()),
        });

        let result = task.await.expect("join").expect("commit result");
        assert_eq!(result.check_tx, check);
        assert_eq!(result.tx_result, exec);
        assert_eq!(result.hash, ctypes::Tx(tx).hash());
        assert_eq!(result.height, 12);
        assert_eq!(env.event_bus.num_client_subscriptions("core-grpc"), 0);
    }

    #[tokio::test]
    async fn rejection_timeout_and_cancellation_release_subscription() {
        let rejected = abci::ResponseCheckTx {
            code: 7,
            log: "rejected".into(),
            ..Default::default()
        };
        let env = environment(Ok(rejected.clone()), 1_000);
        let result = broadcast_commit(&env, b"rejected".to_vec(), "reject")
            .await
            .expect("rejection response");
        assert_eq!(result.check_tx, rejected);
        assert_eq!(result.tx_result, abci::ExecTxResult::default());
        assert_eq!(result.height, 0);
        assert_eq!(env.event_bus.num_clients(), 0);

        let env = environment(Err(MempoolError::App("app unavailable".into())), 1_000);
        let error = broadcast_commit(&env, b"app-error".to_vec(), "app-error")
            .await
            .expect_err("application error");
        assert_eq!(
            error,
            "error on broadcastTxCommit: app unavailable".to_owned()
        );
        assert_eq!(env.event_bus.num_clients(), 0);

        let env = environment(Ok(abci::ResponseCheckTx::default()), 1);
        let error = broadcast_commit(&env, b"timeout".to_vec(), "timeout")
            .await
            .expect_err("timeout");
        assert_eq!(error, "timed out waiting for tx to be included in a block");
        assert_eq!(env.event_bus.num_clients(), 0);

        let env = environment(Ok(abci::ResponseCheckTx::default()), 1_000);
        let task_env = Arc::clone(&env);
        let task =
            tokio::spawn(
                async move { broadcast_commit(&task_env, b"cancel".to_vec(), "cancel").await },
            );
        wait_for_subscription(&env, "cancel").await;
        task.abort();
        let _ = task.await;
        assert_eq!(env.event_bus.num_clients(), 0);
    }
}
