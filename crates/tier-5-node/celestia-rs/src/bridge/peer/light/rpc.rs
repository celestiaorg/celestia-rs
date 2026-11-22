//! Read fully sampled headers from the private light store.
use super::{HeaderWire, Verified};
use crate::bridge::{auth::Auth, blobstream, credentials, header_json, node_rpc};
use anyhow::{Context, Result, ensure};
use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use kvdb::{KvBackend, RedbBackend, WriteBatch};
use prost::Message;
use serde_json::{Value, json};
use std::{future::Future, path::Path, sync::Arc};

pub(super) fn at(height: u64) -> Vec<u8> {
    [b"height/".as_slice(), &height.to_be_bytes()].concat()
}
pub(super) fn by_hash(hash: &[u8]) -> Vec<u8> {
    [b"hash/".as_slice(), hash].concat()
}

fn height_of(raw: &[u8]) -> Result<u64> {
    Ok(u64::from_be_bytes(raw.try_into()?))
}

pub(super) fn store_header(
    db: &RedbBackend,
    batch: &mut WriteBatch,
    block: &Verified,
    raw: &[u8],
) -> Result<()> {
    batch.put(at(block.height), raw.to_vec());
    batch.put(by_hash(&block.hash), block.height.to_be_bytes().to_vec());
    let mut tail = db
        .get(b"tail")?
        .map(|v| height_of(&v))
        .transpose()?
        .unwrap_or(block.height);
    // Go's storage window adds one hour to the seven-day sampling window.
    let cutoff = block
        .signed
        .header
        .as_ref()
        .context("missing header")?
        .time
        .unix_nanos()
        - i128::from(super::TRUST_NS)
        - 3_600_000_000_000;
    while tail < block.height {
        let previous = db.get(&at(tail))?.context("missing light history")?;
        let wire = HeaderWire::decode(previous.as_slice())?;
        let header = ctypes::Header::from_proto(&wire.header.context("missing header")?)?;
        if header.time.unix_nanos() >= cutoff {
            break;
        }
        batch.delete(by_hash(&header.hash().context("missing header hash")?));
        batch.delete(at(tail));
        tail += 1;
    }
    batch.put(b"tail".to_vec(), tail.to_be_bytes().to_vec());
    Ok(())
}

struct Service {
    data: super::data::Client,
    core: Option<super::core::Core>,
    auth: Arc<Auth>,
    db: Arc<RedbBackend>,
    workers: Arc<tokio::sync::Semaphore>,
    waiters: tokio::sync::Semaphore,
    status: tokio::sync::watch::Receiver<super::stats::Sampling>,
}

pub(super) async fn listener(
    args: &super::LightArgs,
    home: &Path,
    db: Arc<RedbBackend>,
    status: tokio::sync::watch::Receiver<super::stats::Sampling>,
    peers: super::Peers,
) -> Result<impl Future<Output = Result<()>> + use<>> {
    let server = if let Some(addr) = args.rpc_listen {
        ensure!(
            addr.ip().is_loopback(),
            "light RPC requires a loopback listener"
        );
        let socket = tokio::net::TcpListener::bind(addr).await?;
        let router = Router::new()
            .route("/", post(handle).get(upgrade))
            .layer(DefaultBodyLimit::max(64 * 1024))
            .with_state(Arc::new(Service {
                auth: Arc::new(Auth::open(home)?),
                data: super::data::Client::new(peers)?,
                core: args
                    .core_ip
                    .as_deref()
                    .map(|host| {
                        super::core::channel(
                            #[cfg(feature = "da-fibre")]
                            &args.network,
                            host,
                            args.core_port.unwrap_or(9090),
                            args.core_tls,
                            args.core_token_path.as_deref(),
                        )
                    })
                    .transpose()?,
                db,
                status,
                waiters: tokio::sync::Semaphore::new(8),
                workers: Arc::new(tokio::sync::Semaphore::new(4)),
            }));
        Some(axum::serve(socket, router))
    } else {
        None
    };
    Ok(async move {
        match server {
            Some(server) => Ok(server.await?),
            None => std::future::pending().await,
        }
    })
}

async fn handle(State(service): State<Arc<Service>>, headers: HeaderMap, body: Bytes) -> Response {
    let changed = service.auth.subscribe();
    let Ok((token, perms)) = credentials(&service.auth, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let payload = tokio::select! {
        result = call(&service,&perms,&body) => result,
        () = service.auth.revoked(changed, token) => {
            let id = crate::bridge::envelope::Request::parse(&body).ok().and_then(|req| req.id);
            json!({"jsonrpc":"2.0","id":id,"error":{"code":1,"message":"token revoked"}}).to_string()
        }
    };
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        payload,
    )
        .into_response()
}

async fn upgrade(
    State(service): State<Arc<Service>>,
    headers: HeaderMap,
    ws: axum::extract::WebSocketUpgrade,
) -> Response {
    crate::bridge::ws::upgrade_endpoint(service.clone(), service.auth.clone(), headers, ws)
}

async fn call(service: &Service, perms: &[String], body: &[u8]) -> String {
    let mut id = Value::Null;
    let response = (|| {
        let req = crate::bridge::envelope::Request::parse(body)?;
        let invalid = || (-32600, "invalid request".to_owned());
        id = req.id.ok_or_else(invalid)?;
        let method = req.method.as_str().ok_or_else(invalid)?;
        let node_permission = node_rpc::permission(method);
        if node_permission.is_none()
            && !blobstream::handles(method)
            && !(cfg!(feature = "da-fibre") && method == "fibre.Download")
            && !super::data::handles(method)
            && !matches!(
                method,
                "state.BalanceForAddress"
                    | "header.LocalHead"
                    | "header.NetworkHead"
                    | "header.SyncState"
                    | "header.SyncWait"
                    | "header.Tail"
                    | "header.GetByHeight"
                    | "header.GetByHash"
                    | "header.GetRangeByHeight"
                    | "header.WaitForHeight"
                    | "das.SamplingStats"
                    | "das.WaitCatchUp"
            )
        {
            return Err((-32601, "method not found".into()));
        }
        let permission = node_permission.unwrap_or("read");
        if !perms.iter().any(|p| p == permission) {
            return Err((1, format!("missing {permission} permission")));
        }
        let empty = Vec::new();
        let args = if req.params.is_null() {
            &empty
        } else {
            req.params.as_array().ok_or_else(invalid)?
        };
        Ok((method.to_owned(), args.clone()))
    })();
    let result = match response {
        Ok((method, args)) => dispatch(service, &method, args).await,
        Err(e) => Err(e),
    };
    match result {
        Ok(result) => format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{result}}}"#),
        Err((code, message)) => {
            json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}).to_string()
        }
    }
}
async fn dispatch(
    service: &Service,
    method: &str,
    args: Vec<Value>,
) -> Result<String, (i32, String)> {
    if method == "state.BalanceForAddress"
        || (cfg!(feature = "da-fibre") && method == "fibre.Download")
    {
        return core_read(service, method, &args).await;
    }
    if matches!(method, "header.SyncState" | "header.SyncWait") {
        if !args.is_empty() {
            return Err((-32602, "invalid parameters".into()));
        }
        let mut status = service.status.clone();
        if method == "header.SyncState" {
            return Ok(status.borrow().sync_state().to_string());
        }
        let target = status.borrow().sync_target();
        let _permit = service
            .waiters
            .try_acquire()
            .map_err(|_| (-32000, "too many header waiters".to_owned()))?;
        status
            .wait_for(|s| s.sampled >= target)
            .await
            .map_err(|_| (1, "sampler stopped".to_owned()))?;
        return Ok("null".into());
    }
    if method == "header.NetworkHead" {
        if !args.is_empty() {
            return Err((-32602, "invalid parameters".into()));
        }
        let _permit = service
            .workers
            .try_acquire()
            .map_err(|_| (-32000, "light RPC busy".into()))?;
        return network_head(service)
            .await
            .map(|head| head.encode())
            .map_err(|e| (1, e.to_string()));
    }
    if super::data::handles(method) {
        let query = crate::bridge::Query::parse(method, &Value::Array(args))
            .map_err(|(code, text)| (code, text.to_owned()))?;
        return service
            .data
            .read(&service.db, query)
            .await
            .map_err(|e| (1, e.to_string()));
    }
    if node_rpc::permission(method).is_some() {
        return node_rpc::call(&service.auth, "Light", method, &args)
            .map(|v| v.to_string())
            .map_err(|e| (1, e.to_string()));
    }
    if method.starts_with("das.") {
        if !args.is_empty() {
            return Err((-32602, "invalid parameters".into()));
        }
        if method == "das.SamplingStats" {
            return Ok(service.status.borrow().json().to_string());
        }
        let _permit = service
            .waiters
            .try_acquire()
            .map_err(|_| (-32000, "too many catch-up waiters".to_owned()))?;
        let mut status = service.status.clone();
        status
            .wait_for(|s| s.done)
            .await
            .map_err(|_| (1, "sampler stopped".to_owned()))?;
        return Ok("null".into());
    }
    if method == "header.WaitForHeight" {
        let height = match args.as_slice() {
            [h] => h.as_u64().filter(|h| *h > 0),
            _ => None,
        }
        .ok_or_else(|| (-32602, "invalid height".to_owned()))?;
        let _permit = service
            .waiters
            .try_acquire()
            .map_err(|_| (-32000, "too many header waiters".to_owned()))?;
        let mut status = service.status.clone();
        status
            .wait_for(|s| s.sampled >= height)
            .await
            .map_err(|_| (1, "sampler stopped".to_owned()))?;
    }
    let worker = service
        .workers
        .clone()
        .try_acquire_owned()
        .map_err(|_| (-32000, "light RPC busy".to_owned()))?;
    let db = service.db.clone();
    let method = method.to_owned();
    tokio::task::spawn_blocking(move || {
        let _permit = worker;
        read(&db, &method, &args).map(|h| h.encode())
    })
    .await
    .map_err(|e| (1, e.to_string()))?
    .map_err(|e| (1, e.to_string()))
}
async fn core_read(
    service: &Service,
    method: &str,
    args: &[Value],
) -> Result<String, (i32, String)> {
    let [Value::String(value)] = args else {
        return Err((-32602, "expected one string argument".into()));
    };
    let core = service.core.clone().ok_or_else(|| {
        let reason = if method == "fibre.Download" {
            "fibre module is not available: node is not connected to a core endpoint"
        } else {
            "node is running without state access. run with --core.ip <CORE NODE IP> to resolve"
        };
        (1, reason.to_owned())
    })?;
    let _permit = service
        .workers
        .try_acquire()
        .map_err(|_| (-32000, "light RPC busy".into()))?;
    let result = match method {
        #[cfg(feature = "da-fibre")]
        "fibre.Download" => super::core::download(core, value).await,
        _ => super::core::balance(core, &service.db, value).await,
    };
    result
        .map(|v| v.to_string())
        .map_err(|e| (1, e.to_string()))
}

async fn network_head(service: &Service) -> Result<rpc::json::J> {
    let raw = service.db.get(b"head")?.context("missing sampled head")?;
    let wire = HeaderWire::decode(raw.as_slice())?;
    let chain = wire.header.context("missing sampled header")?.chain_id;
    let local = Verified::decode(&raw, &chain, None)?;
    let now = light::verifier::now();
    let age = now.unix_nanos()
        - local
            .signed
            .header
            .as_ref()
            .context("missing header")?
            .time
            .unix_nanos();
    // The DA releases configure the syncer's recency threshold to three four-second blocks.
    let head = if age > 12_000_000_000 {
        service
            .data
            .peers
            .network_tip(&local, &chain)
            .await
            .0
            .unwrap_or(local)
    } else {
        local
    };
    Ok(header_json(
        head.signed.header.as_ref().context("missing header")?,
        head.signed.commit.as_ref().context("missing commit")?,
        &head.validators,
        &head.dah,
    ))
}
fn read(db: &RedbBackend, method: &str, args: &[Value]) -> Result<rpc::json::J> {
    if blobstream::handles(method) {
        let request = blobstream::Request::parse(method, args)?;
        let head = db.get(b"head")?.context("missing sampled head")?;
        let height = HeaderWire::decode(head.as_slice())?
            .header
            .context("missing header")?
            .height;
        return request.answer(u64::try_from(height)?, |start, end| {
            (start..end)
                .map(|h| {
                    let bytes = db.get(&at(h))?.context("header not found")?;
                    let header = HeaderWire::decode(bytes.as_slice())?
                        .header
                        .context("missing header")?;
                    ensure!(
                        u64::try_from(header.height)? == h && header.data_hash.len() == 32,
                        "invalid sampled data root tuple"
                    );
                    Ok(rpc::env::encode_data_root_tuple(h, &header.data_hash))
                })
                .collect()
        });
    }
    if let ("header.GetRangeByHeight", [from, to]) = (method, args) {
        let (range, mut previous) = crate::bridge::header_rpc::range_bounds(from, to)?;
        let mut headers = Vec::new();
        for h in range {
            let raw = db
                .get(&at(u64::try_from(h)?))?
                .context("header not found")?;
            let wire = HeaderWire::decode(raw.as_slice())?;
            let header = ctypes::Header::from_proto(&wire.header.context("missing header")?)?;
            ensure!(
                header.height == h && header.last_block_id.hash == previous,
                "discontinuous header range"
            );
            previous = header.hash().context("missing header hash")?;
            headers.push(decode_header(
                &header,
                wire.commit,
                wire.validators,
                &wire.dah,
            )?);
        }
        return Ok(rpc::json::J::Arr(headers));
    }
    let key = match (method, args) {
        ("header.LocalHead", []) => b"head".to_vec(),
        ("header.Tail", []) => at(height_of(
            &db.get(b"tail")?.context("missing sampled tail")?,
        )?),
        ("header.GetByHeight" | "header.WaitForHeight", [h]) => {
            at(h.as_u64().filter(|h| *h > 0).context("invalid height")?)
        }
        ("header.GetByHash", [h]) => {
            let hash = hex::decode(h.as_str().context("invalid hash")?)?;
            ensure!(hash.len() == 32, "invalid hash length");
            at(height_of(
                &db.get(&by_hash(&hash))?.context("header not found")?,
            )?)
        }
        _ => anyhow::bail!("invalid parameters"),
    };
    let raw = db.get(&key)?.context("header not found")?;
    let wire = HeaderWire::decode(raw.as_slice())?;
    let header = ctypes::Header::from_proto(&wire.header.context("missing header")?)?;
    decode_header(&header, wire.commit, wire.validators, &wire.dah)
}

fn decode_header(
    header: &ctypes::Header,
    commit: Option<ctypes::proto::types::Commit>,
    validators: Option<ctypes::proto::types::ValidatorSet>,
    dah: &[u8],
) -> Result<rpc::json::J> {
    Ok(header_json(
        header,
        &ctypes::Commit::from_proto(&commit.context("missing commit")?)?,
        &ctypes::ValidatorSet::from_proto(&validators.context("missing validators")?)?,
        &square::DataAvailabilityHeader::from_proto_bytes(dah)?,
    ))
}

impl crate::bridge::ws::Endpoint for Service {
    fn body_limit(&self, _: &[String]) -> usize {
        64 * 1024
    }

    fn bounds(&self) -> Result<(i64, i64)> {
        let tail = self
            .db
            .get(b"tail")?
            .map(|v| height_of(&v))
            .transpose()?
            .context("missing sampled tail")?;
        Ok((
            i64::try_from(tail)?,
            i64::try_from(self.status.borrow().sampled)?,
        ))
    }

    async fn call(
        self: Arc<Self>,
        _: Arc<Auth>,
        permissions: Arc<[String]>,
        bytes: Vec<u8>,
    ) -> String {
        call(&self, &permissions, &bytes).await
    }

    async fn event(
        self: Arc<Self>,
        height: i64,
        ns: Option<shares::Namespace>,
    ) -> Result<Option<String>> {
        let _permit = self.workers.try_acquire().context("light RPC busy")?;
        let height_u = u64::try_from(height)?;
        let raw = self
            .db
            .get(&at(height_u))?
            .context("sampled header unavailable")?;
        let wire = HeaderWire::decode(raw.as_slice())?;
        let header = ctypes::Header::from_proto(&wire.header.context("missing header")?)?;
        let value = if let Some(ns) = ns {
            let blobs = self
                .data
                .read(&self.db, crate::bridge::Query::Blobs(height, vec![ns]))
                .await?;
            rpc::json::J::obj([
                ("Blobs", rpc::json::J::Raw(blobs)),
                ("Height", rpc::json::J::num(height)),
                ("Header", crate::bridge::ws::raw_header(&header)?),
            ])
        } else {
            decode_header(&header, wire.commit, wire.validators, &wire.dah)?
        };
        Ok(Some(value.encode()))
    }
}
