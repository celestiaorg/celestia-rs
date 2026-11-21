//! Opt-in DA RPC backed by the consensus node's committed block and state stores.

use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::thread::JoinHandle;

use anyhow::{Context, Result, ensure};
use axum::extract::{DefaultBodyLimit, State};
use axum::http::header::CONTENT_TYPE;
use axum::{Router, body::Bytes, routing::post};
use nodestore::{BlockStore, StateStore};
use rpc::json::J;
use serde_json::Value;
use tokio::sync::{Semaphore, oneshot};

pub(crate) struct Server {
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    pub(crate) fn start(addr: SocketAddr, blocks: BlockStore, states: StateStore) -> Result<Self> {
        ensure!(
            addr.ip().is_loopback(),
            "bridge RPC requires a loopback listener"
        );
        let listener = TcpListener::bind(addr).context("bind bridge RPC")?;
        listener.set_nonblocking(true)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let listener = {
            let _enter = runtime.enter();
            tokio::net::TcpListener::from_std(listener)?
        };
        let (stop, stopped) = oneshot::channel();
        let state = Arc::new(Bridge {
            blocks,
            states,
            worker: Arc::new(Semaphore::new(1)),
        });
        let router = Router::new()
            .route("/", post(handle))
            .layer(DefaultBodyLimit::max(64 * 1024))
            .with_state(state);
        let thread = std::thread::Builder::new().name("da-bridge".into()).spawn(move || {
            runtime.block_on(async move {
                tokio::select! {
                    result = axum::serve(listener, router).into_future() => {
                        if let Err(error) = result { tracing::error!(%error, "bridge listener stopped"); }
                    }
                    _ = stopped => {}
                }
            });
        })?;
        Ok(Self {
            stop: Some(stop),
            thread: Some(thread),
        })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Bridge {
    blocks: BlockStore,
    states: StateStore,
    worker: Arc<Semaphore>,
}

enum Query {
    Blobs(i64, Vec<shares::Namespace>),
    Blob(i64, shares::Namespace, Vec<u8>),
    Namespace(i64, shares::Namespace),
    Header(i64),
    Head,
    Share(i64, usize, usize),
}

impl Query {
    fn parse(method: &str, params: &Value) -> Result<Self, (i32, &'static str)> {
        let bad = (-32602, "invalid params");
        let args = params.as_array().ok_or(bad)?;
        let height = || {
            args.first()
                .and_then(Value::as_i64)
                .filter(|h| *h > 0)
                .ok_or(bad)
        };
        let bytes = |v: &Value| {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD
                .decode(v.as_str().ok_or(bad)?)
                .map_err(|_| bad)
        };
        let namespace = |v: &Value| shares::Namespace::from_slice(&bytes(v)?).map_err(|_| bad);
        match method {
            "header.LocalHead" if args.is_empty() => Ok(Self::Head),
            "header.GetByHeight" if args.len() == 1 => Ok(Self::Header(height()?)),
            "blob.GetAll" if args.len() == 2 => {
                let namespaces = args[1]
                    .as_array()
                    .ok_or(bad)?
                    .iter()
                    .map(namespace)
                    .collect::<Result<_, _>>()?;
                Ok(Self::Blobs(height()?, namespaces))
            }
            "blob.Get" if args.len() == 3 => Ok(Self::Blob(
                height()?,
                namespace(&args[1])?,
                bytes(&args[2])?,
            )),
            "share.GetNamespaceData" if args.len() == 2 => {
                Ok(Self::Namespace(height()?, namespace(&args[1])?))
            }
            "share.GetShare" if args.len() == 3 => {
                let index = |v: &Value| v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or(bad);
                Ok(Self::Share(height()?, index(&args[1])?, index(&args[2])?))
            }
            "header.LocalHead"
            | "header.GetByHeight"
            | "share.GetNamespaceData"
            | "share.GetShare" => Err(bad),
            _ => Err((-32601, "method not found")),
        }
    }
}

impl Bridge {
    fn read(&self, query: Query) -> Result<J> {
        let head = self
            .states
            .load()?
            .context("no committed state")?
            .last_block_height;
        let height = match query {
            Query::Head => head,
            Query::Header(h)
            | Query::Share(h, ..)
            | Query::Namespace(h, _)
            | Query::Blobs(h, _)
            | Query::Blob(h, ..) => h,
        };
        ensure!(
            height > 0 && height <= head,
            "height {height} is not committed"
        );
        let block = self
            .blocks
            .load_block(height)?
            .context("block unavailable")?;
        let commit = match self.blocks.load_block_commit(height)? {
            Some(commit) => commit,
            None => self
                .blocks
                .load_seen_commit(height)?
                .context("commit unavailable")?,
        };
        let h = block.header;
        ensure!(
            h.height == height && h.hash().as_ref() == Some(&commit.block_id.hash),
            "block and commit disagree"
        );
        let validators = self.states.load_validators(height)?;
        ensure!(
            h.validators_hash == validators.hash(),
            "validator set does not match block"
        );
        ctypes::verify_commit_light(&h.chain_id, &validators, &commit.block_id, height, &commit)?;
        let txs = block
            .data
            .txs
            .into_iter()
            .map(|tx| tx.0)
            .collect::<Vec<_>>();
        let (eds, dah) = app::proposal::verified_block_eds(h.version.app, &txs, &h.data_hash)
            .map_err(anyhow::Error::msg)?;
        if let Query::Share(_, row, col) = query {
            return Ok(J::b64(eds.share(row, col)?));
        }
        if let Query::Namespace(_, ns) = query {
            let rows = app::inclusion_proof::namespace_rows(&eds, &dah, ns)
                .map_err(|e| anyhow::anyhow!("{}", e.log))?;
            return Ok(J::Arr(rows.into_iter().map(namespace_row_json).collect()));
        }
        if let Query::Blobs(_, namespaces) = query {
            let mut blobs = Vec::new();
            for ns in namespaces {
                blobs.extend(
                    read_blobs(&eds, &dah, ns)?
                        .into_iter()
                        .map(|(_, blob)| blob),
                );
            }
            return Ok(if blobs.is_empty() {
                J::Null
            } else {
                J::Arr(blobs)
            });
        }
        if let Query::Blob(_, ns, commitment) = query {
            return read_blobs(&eds, &dah, ns)?
                .into_iter()
                .find(|(hash, _)| hash.as_slice() == commitment)
                .map(|(_, blob)| blob)
                .context("blob: not found");
        }
        let roots =
            |rs: &[[u8; square::DAH_ROOT_SIZE]]| J::Arr(rs.iter().map(|r| J::b64(r)).collect());
        Ok(J::obj([
            ("header", rpc::encode::header(&h)),
            ("commit", rpc::encode::commit(&commit)),
            ("validator_set", rpc::encode::validator_set(&validators)),
            (
                "dah",
                J::obj([
                    ("row_roots", roots(&dah.row_roots)),
                    ("column_roots", roots(&dah.column_roots)),
                ]),
            ),
        ]))
    }
}

/// Walk Q0 in block order, retaining the EDS index used by the Go blob API.
fn read_blobs(
    eds: &square::ExtendedDataSquare,
    dah: &square::DataAvailabilityHeader,
    ns: shares::Namespace,
) -> Result<Vec<([u8; 32], J)>> {
    let mut shares = Vec::new();
    let mut starts = Vec::new();
    for row in dah.rows_with_namespace(ns)? {
        for col in 0..eds.original_width() {
            let bytes = eds.share(row, col)?;
            if bytes.get(..shares::NAMESPACE_SIZE) != Some(ns.as_bytes().as_slice()) {
                continue;
            }
            let share = shares::Share::from_slice(bytes)?;
            if share.is_padding()? {
                continue;
            }
            if share.is_sequence_start()? {
                starts.push((shares.len(), row * eds.width() + col));
            }
            shares.push(share);
        }
    }
    let mut result = Vec::with_capacity(starts.len());
    for (i, &(start, index)) in starts.iter().enumerate() {
        let end = starts.get(i + 1).map_or(shares.len(), |(next, _)| *next);
        let first = &shares[start];
        let blob = if first.version()? == shares::SHARE_VERSION_TWO {
            let len = first
                .sequence_len()?
                .context("missing fibre sequence length")? as usize;
            let data = first
                .raw_data()?
                .get(..len)
                .context("invalid fibre share length")?;
            shares::Blob::new_with_signer(
                ns,
                data.to_vec(),
                shares::SHARE_VERSION_TWO,
                first.signer()?,
            )?
        } else {
            shares::parse_blobs(&shares[start..end])?
                .pop()
                .context("invalid blob sequence")?
        };
        let commitment = square::create_commitment(
            &blob,
            merkle::root_from_byte_slices,
            square::DEFAULT_SUBTREE_ROOT_THRESHOLD,
        )?;
        let mut value = J::obj([
            ("namespace", J::b64(ns.as_bytes())),
            ("data", J::b64(&blob.data)),
            ("share_version", J::num(blob.share_version)),
            ("commitment", J::b64(&commitment)),
            ("index", J::num(index)),
        ]);
        if let Some(signer) = blob.signer {
            value.push("signer", J::b64(&signer));
        }
        result.push((commitment, value));
    }
    Ok(result)
}

fn namespace_row_json(row: app::inclusion_proof::NamespaceRow) -> J {
    let p = row.proof;
    let mut proof = J::obj([
        ("end", J::num(p.end)),
        ("is_max_namespace_ignored", J::Bool(true)),
    ]);
    if p.start != 0 {
        proof.push("start", J::num(p.start));
    }
    if !p.nodes.is_empty() {
        proof.push("nodes", J::Arr(p.nodes.iter().map(|b| J::b64(b)).collect()));
    }
    if !p.leaf_hash.is_empty() {
        proof.push("leaf_hash", J::b64(&p.leaf_hash));
    }
    let shares = if row.shares.is_empty() {
        J::Null
    } else {
        J::Arr(row.shares.iter().map(|s| J::b64(s)).collect())
    };
    J::obj([("shares", shares), ("proof", proof)])
}

async fn handle(
    State(bridge): State<Arc<Bridge>>,
    body: Bytes,
) -> ([(axum::http::HeaderName, &'static str); 1], String) {
    let mut id = J::Null;
    let result = async {
        let req: Value =
            serde_json::from_slice(&body).map_err(|_| (-32700, "parse error".to_owned()))?;
        let invalid = || (-32600, "invalid request".to_owned());
        if req["jsonrpc"] != "2.0" {
            return Err(invalid());
        }
        let request_id = req.get("id").ok_or_else(invalid)?;
        if !(request_id.is_string() || request_id.is_number() || request_id.is_null()) {
            return Err(invalid());
        }
        id = J::Raw(request_id.to_string());
        let method = req["method"].as_str().ok_or_else(invalid)?;
        let empty = Value::Array(Vec::new());
        let query = Query::parse(
            method,
            req.get("params").filter(|v| !v.is_null()).unwrap_or(&empty),
        )
        .map_err(|(code, msg)| (code, msg.to_owned()))?;
        let permit = bridge
            .worker
            .clone()
            .try_acquire_owned()
            .map_err(|_| (-32000, "bridge busy".to_owned()))?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            bridge.read(query)
        })
        .await
        .map_err(|e| (-32000, e.to_string()))?
        .map_err(|e| (-32000, e.to_string()))
    }
    .await;
    let payload = match result {
        Ok(value) => ("result", value),
        Err((code, msg)) => (
            "error",
            J::obj([("code", J::num(code)), ("message", J::str(msg))]),
        ),
    };
    (
        [(CONTENT_TYPE, "application/json")],
        J::obj([("jsonrpc", J::str("2.0")), ("id", id), payload]).encode(),
    )
}
