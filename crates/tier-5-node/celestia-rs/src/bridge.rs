//! Opt-in DA RPC backed by the consensus node's committed block and state stores.

pub(crate) mod auth;
mod header_rpc;
mod node_rpc;
mod share_rpc;
mod ws;

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

#[cfg(feature = "bridge-p2p")]
pub(crate) mod peer;

pub(crate) struct Server {
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    pub(crate) fn start(
        addr: SocketAddr,
        home: &std::path::Path,
        blocks: BlockStore,
        states: StateStore,
    ) -> Result<Self> {
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
            waiters: Arc::new(Semaphore::new(32)),
            blocks,
            states,
            worker: Arc::new(Semaphore::new(1)),
        });
        let router = Router::new()
            .route("/", post(handle).get(ws::upgrade))
            .layer(axum::Extension(Arc::new(auth::Auth::open(home)?)))
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
    waiters: Arc<Semaphore>,
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
    BlobProof(i64, shares::Namespace, Vec<u8>, ProofRequest),
    Tail,
    Hash(Vec<u8>),
    ShareRead(i64, share_rpc::Read),
    Share(i64, usize, usize),
}

enum ProofRequest {
    Get,
    Commitment,
    Included(Value),
}

fn bytes(value: &Value) -> Result<Vec<u8>, (i32, &'static str)> {
    use base64::Engine as _;
    let invalid = (-32602, "invalid params");
    base64::engine::general_purpose::STANDARD
        .decode(value.as_str().ok_or(invalid)?)
        .map_err(|_| invalid)
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
        let namespace = |v: &Value| shares::Namespace::from_slice(&bytes(v)?).map_err(|_| bad);
        let index = |v: &Value| v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or(bad);
        match method {
            "header.LocalHead" | "header.NetworkHead" if args.is_empty() => Ok(Self::Head),
            "header.Tail" if args.is_empty() => Ok(Self::Tail),
            "header.GetByHash" if args.len() == 1 => Ok(Self::Hash(
                hex::decode(args[0].as_str().ok_or(bad)?).map_err(|_| bad)?,
            )),
            "share.SharesAvailable" if args.len() == 1 => {
                Ok(Self::ShareRead(height()?, share_rpc::Read::Available))
            }
            "share.GetEDS" if args.len() == 1 => {
                Ok(Self::ShareRead(height()?, share_rpc::Read::Eds))
            }
            "share.GetRow" if args.len() == 2 => Ok(Self::ShareRead(
                height()?,
                share_rpc::Read::Row(index(&args[1])?),
            )),
            "share.GetRange" if args.len() == 3 => Ok(Self::ShareRead(
                height()?,
                share_rpc::Read::Range(index(&args[1])?, index(&args[2])?),
            )),
            "share.GetSamples" if args.len() == 2 => {
                let coords = args[1]
                    .as_array()
                    .ok_or(bad)?
                    .iter()
                    .map(|v| Ok((index(&v["row"])?, index(&v["col"])?)))
                    .collect::<Result<_, _>>()?;
                Ok(Self::ShareRead(height()?, share_rpc::Read::Samples(coords)))
            }
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
            "blob.GetProof" | "blob.GetCommitmentProof" if args.len() == 3 => Ok(Self::BlobProof(
                height()?,
                namespace(&args[1])?,
                bytes(&args[2])?,
                if method == "blob.GetProof" {
                    ProofRequest::Get
                } else {
                    ProofRequest::Commitment
                },
            )),
            "blob.Included" if args.len() == 4 && args[2].is_array() => Ok(Self::BlobProof(
                height()?,
                namespace(&args[1])?,
                bytes(&args[3])?,
                ProofRequest::Included(args[2].clone()),
            )),
            "blob.Get" if args.len() == 3 => Ok(Self::Blob(
                height()?,
                namespace(&args[1])?,
                bytes(&args[2])?,
            )),
            "share.GetNamespaceData" if args.len() == 2 => {
                Ok(Self::Namespace(height()?, namespace(&args[1])?))
            }
            "share.GetShare" if args.len() == 3 => {
                Ok(Self::Share(height()?, index(&args[1])?, index(&args[2])?))
            }
            "header.LocalHead"
            | "header.NetworkHead"
            | "header.Tail"
            | "header.GetByHash"
            | "share.SharesAvailable"
            | "share.GetEDS"
            | "share.GetRow"
            | "share.GetRange"
            | "share.GetSamples"
            | "header.GetByHeight"
            | "blob.Get"
            | "blob.GetProof"
            | "blob.GetCommitmentProof"
            | "blob.Included"
            | "blob.GetAll"
            | "share.GetNamespaceData"
            | "share.GetShare" => Err(bad),
            _ => Err((-32601, "method not found")),
        }
    }
}

struct Committed {
    header: ctypes::Header,
    commit: ctypes::Commit,
    validators: ctypes::ValidatorSet,
    eds: square::ExtendedDataSquare,
    dah: square::DataAvailabilityHeader,
}

impl Bridge {
    fn committed(&self, height: i64) -> Result<Committed> {
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
        Ok(Committed {
            header: h,
            commit,
            validators,
            eds,
            dah,
        })
    }

    fn read(&self, query: Query) -> Result<J> {
        let head = self
            .states
            .load()?
            .context("no committed state")?
            .last_block_height;
        let height = match query {
            Query::Head => head,
            Query::Tail => self.blocks.base(),
            Query::Hash(ref hash) => {
                self.blocks
                    .load_block_meta_by_hash(hash)?
                    .context("header not found")?
                    .header
                    .height
            }
            Query::ShareRead(h, _)
            | Query::Header(h)
            | Query::Share(h, ..)
            | Query::Namespace(h, _)
            | Query::Blobs(h, _)
            | Query::BlobProof(h, ..)
            | Query::Blob(h, ..) => h,
        };
        ensure!(
            height > 0 && height <= head,
            "height {height} is not committed"
        );
        let block = self.committed(height)?;
        match query {
            Query::Header(_) | Query::Head | Query::Tail | Query::Hash(_) => {
                Ok(block.header_json())
            }
            _ => query.answer_square(&block.eds, &block.dah),
        }
    }
}

impl Query {
    fn answer_square(
        self,
        eds: &square::ExtendedDataSquare,
        dah: &square::DataAvailabilityHeader,
    ) -> Result<J> {
        if let Query::BlobProof(_, ns, hash, kind) = self {
            return prove_blob(eds, dah, ns, &hash, kind);
        }
        if let Query::ShareRead(_, method) = self {
            return method.response(eds, dah);
        }
        if let Query::Share(_, row, col) = self {
            return Ok(J::b64(eds.share(row, col)?));
        }
        if let Query::Namespace(_, ns) = self {
            let rows = app::inclusion_proof::namespace_rows(eds, dah, ns)
                .map_err(|e| anyhow::anyhow!("{}", e.log))?;
            return Ok(J::Arr(rows.into_iter().map(namespace_row_json).collect()));
        }
        if let Query::Blobs(_, namespaces) = self {
            let mut blobs = Vec::new();
            for ns in namespaces {
                blobs.extend(read_blobs(eds, dah, ns)?.into_iter().map(|(_, blob)| blob));
            }
            return Ok(if blobs.is_empty() {
                J::Null
            } else {
                J::Arr(blobs)
            });
        }
        if let Query::Blob(_, ns, commitment) = self {
            return read_blobs(eds, dah, ns)?
                .into_iter()
                .find(|(hash, _)| hash.as_slice() == commitment)
                .map(|(_, blob)| blob)
                .context("blob: not found");
        }
        anyhow::bail!("expected a blob or share query")
    }
}

impl Committed {
    fn header_json(&self) -> J {
        header_json(&self.header, &self.commit, &self.validators, &self.dah)
    }
}

fn header_json(
    header: &ctypes::Header,
    commit: &ctypes::Commit,
    validators: &ctypes::ValidatorSet,
    dah: &square::DataAvailabilityHeader,
) -> J {
    let roots = |rs: &[[u8; square::DAH_ROOT_SIZE]]| J::Arr(rs.iter().map(|r| J::b64(r)).collect());
    J::obj([
        ("header", rpc::encode::header(header)),
        ("commit", rpc::encode::commit(commit)),
        ("validator_set", rpc::encode::validator_set(validators)),
        (
            "dah",
            J::obj([
                ("row_roots", roots(&dah.row_roots)),
                ("column_roots", roots(&dah.column_roots)),
            ]),
        ),
    ])
}

/// Walk Q0 in block order, retaining the EDS index used by the Go blob API.
fn read_blobs(
    eds: &square::ExtendedDataSquare,
    dah: &square::DataAvailabilityHeader,
    ns: shares::Namespace,
) -> Result<Vec<([u8; 32], J)>> {
    Ok(blob_records(eds, dah, ns)?
        .into_iter()
        .map(|r| (r.commitment, r.value))
        .collect())
}

struct BlobRecord {
    commitment: [u8; 32],
    value: J,
    start: usize,
    end: usize,
    roots: Vec<Vec<u8>>,
}

fn blob_records(
    eds: &square::ExtendedDataSquare,
    dah: &square::DataAvailabilityHeader,
    ns: shares::Namespace,
) -> Result<Vec<BlobRecord>> {
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
        let roots = square::commitment::commitment_subtree_roots(
            &blob,
            square::DEFAULT_SUBTREE_ROOT_THRESHOLD,
        )?;
        let commitment = merkle::root_from_byte_slices(&roots);
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
        let ods_start = index / eds.width() * eds.original_width() + index % eds.width();
        result.push(BlobRecord {
            commitment,
            value,
            start: ods_start,
            end: ods_start + end - start,
            roots,
        });
    }
    Ok(result)
}

fn namespace_row_json(row: app::inclusion_proof::NamespaceRow) -> J {
    let proof = proof_json(&row.proof, true);
    let shares = if row.shares.is_empty() {
        J::Null
    } else {
        J::Arr(row.shares.iter().map(|s| J::b64(s)).collect())
    };
    J::obj([("shares", shares), ("proof", proof)])
}

async fn handle(
    State(bridge): State<Arc<Bridge>>,
    axum::Extension(auth): axum::Extension<Arc<auth::Auth>>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    use axum::response::IntoResponse as _;
    let changed = auth.subscribe();
    let Ok((token, permissions)) = credentials(&auth, &headers) else {
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    };
    tokio::select! {
        value=dispatch(bridge,&auth,&permissions,&body) => ([(CONTENT_TYPE, "application/json")], value).into_response(),
        ()=auth.revoked(changed,token)=>axum::http::StatusCode::UNAUTHORIZED.into_response(),
    }
}

fn credentials<'a>(
    auth: &auth::Auth,
    headers: &'a axum::http::HeaderMap,
) -> Result<(Option<&'a str>, Vec<String>)> {
    let token = match headers.get(axum::http::header::AUTHORIZATION) {
        None => None,
        Some(raw) => match raw.to_str().ok().and_then(|s| s.strip_prefix("Bearer ")) {
            Some(token) => Some(token),
            None => anyhow::bail!("invalid authorization header"),
        },
    };
    let permissions = token
        .map(|t| auth.verify(t))
        .transpose()?
        .unwrap_or_else(|| vec!["public".to_owned()]);
    Ok((token, permissions))
}

async fn dispatch(
    bridge: Arc<Bridge>,
    auth: &auth::Auth,
    permissions: &[String],
    body: &[u8],
) -> String {
    let require = |method: &str, perm: &str| {
        if permissions.iter().any(|p| p == perm) {
            Ok(())
        } else {
            Err((
                1,
                format!(
                    "missing permission to invoke '{}' (need '{perm}')",
                    method.rsplit('.').next().unwrap_or(method)
                ),
            ))
        }
    };
    let mut id = J::Null;
    let result = async {
        let req: Value =
            serde_json::from_slice(body).map_err(|_| (-32700, "parse error".to_owned()))?;
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
        if let Some(permission) = node_rpc::permission(method) {
            require(method, permission)?;
            let args = req
                .get("params")
                .filter(|v| !v.is_null())
                .unwrap_or(&empty)
                .as_array()
                .ok_or_else(invalid)?;
            return node_rpc::call(auth, "Bridge", method, args)
                .map(|value| J::Raw(value.to_string()))
                .map_err(|e| (1, e.to_string()));
        }
        if matches!(method, "das.SamplingStats" | "das.WaitCatchUp") {
            require(method, "read")?;
            if req
                .get("params")
                .is_some_and(|v| !v.is_null() && v != &empty)
            {
                return Err((-32602, "invalid parameters".to_owned()));
            }
            return Err((1, "module/das: stubbed: dasing is disabled".to_owned()));
        }
        if header_rpc::handles(method) {
            require(method, "read")?;
            let params = req.get("params").filter(|v| !v.is_null()).unwrap_or(&empty);
            return header_rpc::handle(bridge, method, params)
                .await
                .map_err(|e| (1, e.to_string()));
        }
        let query = Query::parse(
            method,
            req.get("params").filter(|v| !v.is_null()).unwrap_or(&empty),
        )
        .map_err(|(code, msg)| (code, msg.to_owned()))?;
        require(method, "read")?;
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
    J::obj([("jsonrpc", J::str("2.0")), ("id", id), payload]).encode()
}

fn proof_json(p: &celestia_proto::celestia::core::v1::proof::NmtProof, shwap: bool) -> J {
    let mut proof = J::obj([("end", J::num(p.end))]);
    if shwap {
        proof.push("is_max_namespace_ignored", J::Bool(true));
    }
    if p.start != 0 {
        proof.push("start", J::num(p.start));
    }
    if !p.nodes.is_empty() {
        proof.push("nodes", J::Arr(p.nodes.iter().map(|n| J::b64(n)).collect()));
    }
    if !p.leaf_hash.is_empty() {
        proof.push("leaf_hash", J::b64(&p.leaf_hash));
    }
    proof
}

fn prove_blob(
    eds: &square::ExtendedDataSquare,
    dah: &square::DataAvailabilityHeader,
    ns: shares::Namespace,
    hash: &[u8],
    kind: ProofRequest,
) -> Result<J> {
    let Some(record) = blob_records(eds, dah, ns)?
        .into_iter()
        .find(|r| r.commitment == hash)
    else {
        if matches!(kind, ProofRequest::Included(_)) {
            return Ok(J::Bool(false));
        }
        anyhow::bail!("blob: not found")
    };
    if matches!(kind, ProofRequest::Commitment) {
        return share_rpc::range(eds, dah, record.start, record.end, Some(record.roots));
    }
    let width = eds.original_width();
    let covered = record.start / width..=(record.end - 1) / width;
    let rows = app::inclusion_proof::namespace_rows(eds, dah, ns)
        .map_err(|e| anyhow::anyhow!("{}", e.log))?;
    let mut expected = Vec::new();
    for (i, row) in dah.rows_with_namespace(ns)?.into_iter().zip(rows) {
        if covered.contains(&i) {
            expected.push(row.proof);
        }
    }
    if let ProofRequest::Included(input) = kind {
        let supplied = input.as_array().context("blob: proof cannot be nil")?;
        ensure!(expected.len() == supplied.len(), "blob: invalid proof");
        for (p, v) in expected.iter().zip(supplied) {
            let equivalent = serde_json::from_value::<InputProof>(v.clone())?;
            ensure!(equivalent.matches(p), "blob: invalid proof");
        }
        Ok(J::Bool(true))
    } else {
        Ok(J::Arr(
            expected.iter().map(|p| proof_json(p, true)).collect(),
        ))
    }
}

#[derive(serde::Deserialize)]
#[serde(default)]
#[derive(Default)]
struct InputProof {
    start: i32,
    end: i32,
    nodes: Option<Vec<String>>,
    leaf_hash: Option<String>,
}
impl InputProof {
    fn matches(&self, p: &celestia_proto::celestia::core::v1::proof::NmtProof) -> bool {
        use base64::Engine as _;
        let enc = base64::engine::general_purpose::STANDARD;
        let nodes = self.nodes.as_deref().unwrap_or_default();
        self.start == p.start
            && self.end == p.end
            && self.leaf_hash.as_deref().unwrap_or_default() == enc.encode(&p.leaf_hash)
            && nodes.len() == p.nodes.len()
            && nodes.iter().zip(&p.nodes).all(|(a, b)| a == &enc.encode(b))
    }
}
