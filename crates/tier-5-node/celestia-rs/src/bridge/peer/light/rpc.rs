//! Read fully sampled headers from the private light store.
use super::{HeaderWire, Verified};
use crate::bridge::{auth::Auth, credentials, header_json};
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
use std::{future::Future, net::SocketAddr, path::Path, sync::Arc};

fn at(height: u64) -> Vec<u8> {
    [b"height/".as_slice(), &height.to_be_bytes()].concat()
}
fn by_hash(hash: &[u8]) -> Vec<u8> {
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
    auth: Auth,
    db: Arc<RedbBackend>,
    workers: Arc<tokio::sync::Semaphore>,
    waiters: tokio::sync::Semaphore,
    status: tokio::sync::watch::Receiver<super::stats::Sampling>,
}

pub(super) async fn listener(
    addr: Option<SocketAddr>,
    home: &Path,
    db: Arc<RedbBackend>,
    status: tokio::sync::watch::Receiver<super::stats::Sampling>,
) -> Result<impl Future<Output = Result<()>> + use<>> {
    let server = if let Some(addr) = addr {
        ensure!(
            addr.ip().is_loopback(),
            "light RPC requires a loopback listener"
        );
        let socket = tokio::net::TcpListener::bind(addr).await?;
        let router = Router::new()
            .route("/", post(handle))
            .layer(DefaultBodyLimit::max(64 * 1024))
            .with_state(Arc::new(Service {
                auth: Auth::open(home)?,
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
    let Ok((_, perms)) = credentials(&service.auth, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let mut id = Value::Null;
    let response = (|| {
        let req = crate::bridge::envelope::Request::parse(&body)?;
        let invalid = || (-32600, "invalid request".to_owned());
        id = req.id.ok_or_else(invalid)?;
        let method = req.method.as_str().ok_or_else(invalid)?;
        if !matches!(
            method,
            "header.LocalHead"
                | "header.Tail"
                | "header.GetByHeight"
                | "header.GetByHash"
                | "das.SamplingStats"
                | "das.WaitCatchUp"
        ) {
            return Err((-32601, "method not found".into()));
        }
        if !perms.iter().any(|p| p == "read") {
            return Err((1, "missing read permission".into()));
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
        Ok((method, args)) => dispatch(&service, &method, args).await,
        Err(e) => Err(e),
    };
    let payload = match result {
        Ok(result) => format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{result}}}"#),
        Err((code, message)) => {
            json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}).to_string()
        }
    };
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        payload,
    )
        .into_response()
}
async fn dispatch(
    service: &Service,
    method: &str,
    args: Vec<Value>,
) -> Result<String, (i32, String)> {
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
fn read(db: &RedbBackend, method: &str, args: &[Value]) -> Result<rpc::json::J> {
    let key = match (method, args) {
        ("header.LocalHead", []) => b"head".to_vec(),
        ("header.Tail", []) => at(height_of(
            &db.get(b"tail")?.context("missing sampled tail")?,
        )?),
        ("header.GetByHeight", [h]) => {
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
    Ok(header_json(
        &ctypes::Header::from_proto(&wire.header.context("missing header")?)?,
        &ctypes::Commit::from_proto(&wire.commit.context("missing commit")?)?,
        &ctypes::ValidatorSet::from_proto(&wire.validators.context("missing validators")?)?,
        &square::DataAvailabilityHeader::from_proto_bytes(&wire.dah)?,
    ))
}
