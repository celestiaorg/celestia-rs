//! celestia-core's gRPC services (`rpc/grpc`): `tendermint.rpc.grpc.BlockAPI`
//! (BlockByHash, BlockByHeight, Commit, ValidatorSet, SubscribeNewHeights,
//! Status), `BlobstreamAPI` (DataRootInclusionProof) and `BroadcastAPI`
//! (Ping, BroadcastTx), as served on `grpc_laddr` and, by celestia-app,
//! on the app's gRPC server (what celestia-node's `core.grpc` uses).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, Response};
use bytes::{Buf, BufMut};
use ctypes::proto::rpc::grpc as pg;
use ctypes::proto::{p2p as pp2p, types as ptypes};
use futures_util::Stream;
use prost::Message as _;
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::{Code, Status};

use crate::env::{Environment, Route};
use crate::query::Query;

/// Raw protobuf bytes codec.
#[derive(Debug, Clone, Copy, Default)]
pub struct RawCodec;

/// Raw encoder/decoder.
#[derive(Debug, Clone, Copy, Default)]
pub struct RawCoder;

impl Encoder for RawCoder {
    type Item = Vec<u8>;
    type Error = Status;

    fn encode(&mut self, item: Vec<u8>, dst: &mut EncodeBuf<'_>) -> Result<(), Status> {
        dst.put_slice(&item);
        Ok(())
    }
}

impl Decoder for RawCoder {
    type Item = Vec<u8>;
    type Error = Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Vec<u8>>, Status> {
        let mut out = vec![0u8; src.remaining()];
        src.copy_to_slice(&mut out);
        Ok(Some(out))
    }
}

impl Codec for RawCodec {
    type Encode = Vec<u8>;
    type Decode = Vec<u8>;
    type Encoder = RawCoder;
    type Decoder = RawCoder;

    fn encoder(&mut self) -> RawCoder {
        RawCoder
    }

    fn decoder(&mut self) -> RawCoder {
        RawCoder
    }
}

type BoxFut<T> = Pin<Box<dyn Future<Output = T> + Send>>;
type BoxStream = Pin<Box<dyn Stream<Item = Result<Vec<u8>, Status>> + Send>>;

/// Go handlers return plain errors: gRPC `Unknown`.
fn unknown(msg: impl Into<String>) -> Status {
    Status::new(Code::Unknown, msg.into())
}

fn decode<M: prost::Message + Default>(b: &[u8]) -> Result<M, Status> {
    M::decode(b).map_err(|e| Status::internal(format!("grpc: error unmarshalling request: {e}")))
}

/// `%d` of a byte slice in Go: `[1 2 3]`.
fn go_bytes_d(b: &[u8]) -> String {
    format!(
        "[{}]",
        b.iter().map(u8::to_string).collect::<Vec<_>>().join(" ")
    )
}

const HEAVY: &[&str] = &[
    "/tendermint.rpc.grpc.BlockAPI/BlockByHash",
    "/tendermint.rpc.grpc.BlockAPI/BlockByHeight",
    "/tendermint.rpc.grpc.BlockAPI/ValidatorSet",
    "/tendermint.rpc.grpc.BlobstreamAPI/DataRootInclusionProof",
];

const HEAVY_ROUTE: Route = Route {
    name: "grpc",
    args: &[],
    ws: false,
    cacheable: None,
    heavy: true,
};

fn block_parts_stream(
    env: &Environment,
    height: i64,
    by_hash: Option<&[u8]>,
    prove: bool,
) -> Result<Vec<Vec<u8>>, Status> {
    let bs = &env.block_store;
    let what = |what: &str| match by_hash {
        Some(h) => format!("{what} for block hash {}", go_bytes_d(h)),
        None => format!("{what} for height {height}"),
    };
    let meta = match by_hash {
        Some(h) => bs.load_block_meta_by_hash(h),
        None => bs.load_block_meta(height),
    }
    .map_err(|e| unknown(e.to_string()))?;
    let Some(meta) = meta else {
        return Err(unknown(what("nil block meta")));
    };
    let h = meta.header.height;
    let commit = match by_hash {
        Some(_) => bs.load_block_commit(h),
        None => bs.load_seen_commit(h),
    }
    .map_err(|e| unknown(e.to_string()))?;
    let Some(commit) = commit else {
        return Err(unknown(match by_hash {
            Some(_) => what("nil commit"),
            None => what("nil block commit"),
        }));
    };
    let vals = env
        .state_store
        .load_validators(h)
        .map_err(|e| unknown(e.to_string()))?;
    let pvals = vals.to_proto().map_err(|e| unknown(e.to_string()))?;
    let pcommit = commit.to_proto();
    let total = meta.block_id.part_set_header.total;
    let mut out = Vec::new();
    for i in 0..total {
        let part = bs
            .load_block_part(h, i as usize)
            .map_err(|e| unknown(e.to_string()))?;
        let Some(part) = part else {
            return Err(unknown(match by_hash {
                Some(hash) => format!("nil block part {i} for block hash {}", go_bytes_d(hash)),
                None => format!("nil block part {i} for height {h}"),
            }));
        };
        let mut pp = part.to_proto();
        if !prove {
            pp.proof = Some(ctypes::proto::crypto::Proof::default());
        }
        let first = i == 0;
        let is_last = i + 1 == total;
        let bytes = if by_hash.is_some() {
            pg::BlockByHashResponse {
                block_part: Some(pp),
                commit: first.then(|| pcommit.clone()),
                validator_set: first.then(|| pvals.clone()),
                is_last,
            }
            .encode_to_vec()
        } else {
            pg::BlockByHeightResponse {
                block_part: Some(pp),
                commit: first.then(|| pcommit.clone()),
                validator_set: first.then(|| pvals.clone()),
                is_last,
            }
            .encode_to_vec()
        };
        out.push(bytes);
    }
    Ok(out)
}

/// Runs a unary method.
fn unary(env: &Arc<Environment>, method: &str, req: &[u8]) -> Result<Vec<u8>, Status> {
    match method {
        "/tendermint.rpc.grpc.BlockAPI/Commit" => {
            let r: pg::CommitRequest = decode(req)?;
            let height = if r.height == 0 {
                env.block_store.height()
            } else {
                r.height
            };
            let c = env
                .block_store
                .load_seen_commit(height)
                .map_err(|e| unknown(e.to_string()))?
                .ok_or_else(|| unknown(format!("nil block commit for height {height}")))?;
            Ok(pg::CommitResponse {
                commit: Some(c.to_proto()),
            }
            .encode_to_vec())
        }
        "/tendermint.rpc.grpc.BlockAPI/ValidatorSet" => {
            let r: pg::ValidatorSetRequest = decode(req)?;
            let height = if r.height == 0 {
                env.block_store.height()
            } else {
                r.height
            };
            let vals = env
                .state_store
                .load_validators(height)
                .map_err(|e| unknown(e.to_string()))?;
            Ok(pg::ValidatorSetResponse {
                validator_set: Some(vals.to_proto().map_err(|e| unknown(e.to_string()))?),
                height,
            }
            .encode_to_vec())
        }
        "/tendermint.rpc.grpc.BlockAPI/Status" => {
            let st = env.status_parts().map_err(unknown)?;
            let ni = &st.node_info;
            Ok(pg::StatusResponse {
                node_info: Some(pp2p::DefaultNodeInfo {
                    protocol_version: Some(pp2p::ProtocolVersion {
                        p2p: ni.p2p_version,
                        block: ni.block_version,
                        app: ni.app_version,
                    }),
                    default_node_id: ni.id.clone(),
                    listen_addr: ni.listen_addr.clone(),
                    network: ni.network.clone(),
                    version: ni.version.clone(),
                    channels: ni.channels.clone(),
                    moniker: ni.moniker.clone(),
                    other: Some(pp2p::DefaultNodeInfoOther {
                        tx_index: ni.tx_index.clone(),
                        rpc_address: ni.rpc_address.clone(),
                    }),
                }),
                sync_info: Some(pg::SyncInfo {
                    latest_block_hash: st.latest_hash.clone(),
                    latest_app_hash: st.latest_app_hash.clone(),
                    latest_block_height: st.latest_height,
                    latest_block_time: Some(st.latest_time.to_proto()),
                    earliest_block_hash: st.earliest_hash.clone(),
                    earliest_app_hash: st.earliest_app_hash.clone(),
                    earliest_block_height: st.earliest_height,
                    earliest_block_time: Some(st.earliest_time.to_proto()),
                    catching_up: st.catching_up,
                }),
                validator_info: Some(pg::ValidatorInfo {
                    address: env.pub_key.address(),
                    pub_key: Some(env.pub_key.to_proto()),
                    voting_power: st.voting_power,
                }),
            }
            .encode_to_vec())
        }
        "/tendermint.rpc.grpc.BlobstreamAPI/DataRootInclusionProof" => {
            let r: pg::DataRootInclusionProofRequest = decode(req)?;
            let p = env
                .generate_data_root_inclusion_proof(r.height, r.start, r.end)
                .map_err(unknown)?;
            Ok(pg::DataRootInclusionProofResponse {
                proof: Some(p.to_proto()),
            }
            .encode_to_vec())
        }
        "/tendermint.rpc.grpc.BroadcastAPI/Ping" => Ok(pg::ResponsePing {}.encode_to_vec()),
        _ => Err(Status::unimplemented(format!(
            "unknown method {}",
            method.rsplit('/').next().unwrap_or(method)
        ))),
    }
}

static SUB_SEQ: AtomicU64 = AtomicU64::new(0);

fn subscribe_new_heights(env: &Arc<Environment>) -> Result<BoxStream, Status> {
    let client = format!(
        "block-api-subscription-{}",
        SUB_SEQ.fetch_add(1, Ordering::SeqCst)
    );
    let q = Query::new("tm.event='NewBlock'").map_err(unknown)?;
    let sub = env.event_bus.subscribe(&client, q, 500).map_err(unknown)?;
    let env = Arc::clone(env);
    let stream =
        futures_util::stream::unfold((sub, env, client), |(mut sub, env, client)| async move {
            let m = sub.out.recv().await?;
            let crate::events::EventData::NewBlock { block, .. } = m.data.as_ref() else {
                return Some((
                    Err(Status::internal("unexpected event")),
                    (sub, env, client),
                ));
            };
            let mut b = (**block).clone();
            let resp = pg::SubscribeNewHeightsResponse {
                height: b.header.height,
                hash: b.hash().unwrap_or_default(),
                time: Some(b.header.time.to_proto()),
                catching_up: env.node.is_catching_up(),
            };
            Some((Ok(resp.encode_to_vec()), (sub, env, client)))
        });
    Ok(Box::pin(stream))
}

struct UnarySvc {
    env: Arc<Environment>,
    path: String,
}

impl tonic::server::UnaryService<Vec<u8>> for UnarySvc {
    type Response = Vec<u8>;
    type Future = BoxFut<Result<tonic::Response<Vec<u8>>, Status>>;

    fn call(&mut self, request: tonic::Request<Vec<u8>>) -> Self::Future {
        let env = Arc::clone(&self.env);
        let path = self.path.clone();
        Box::pin(async move {
            let data = request.into_inner();
            let res = tokio::task::spawn_blocking(move || {
                let _g =
                    if HEAVY.contains(&path.as_str()) {
                        Some(env.try_acquire(&HEAVY_ROUTE).ok_or_else(|| {
                            Status::resource_exhausted(crate::env::ERR_HEAVY_LIMIT)
                        })?)
                    } else {
                        None
                    };
                if path == "/tendermint.rpc.grpc.BroadcastAPI/BroadcastTx" {
                    return Err(Status::unimplemented(
                        "BroadcastTx: use broadcast_tx_commit",
                    ));
                }
                unary(&env, &path, &data)
            })
            .await
            .map_err(|e| Status::internal(e.to_string()))??;
            Ok(tonic::Response::new(res))
        })
    }
}

struct StreamSvc {
    env: Arc<Environment>,
    path: String,
}

impl tonic::server::ServerStreamingService<Vec<u8>> for StreamSvc {
    type Response = Vec<u8>;
    type ResponseStream = BoxStream;
    type Future = BoxFut<Result<tonic::Response<BoxStream>, Status>>;

    fn call(&mut self, request: tonic::Request<Vec<u8>>) -> Self::Future {
        let env = Arc::clone(&self.env);
        let path = self.path.clone();
        Box::pin(async move {
            let data = request.into_inner();
            if path == "/tendermint.rpc.grpc.BlockAPI/SubscribeNewHeights" {
                return Ok(tonic::Response::new(subscribe_new_heights(&env)?));
            }
            let items = tokio::task::spawn_blocking(move || -> Result<Vec<Vec<u8>>, Status> {
                let _g = env
                    .try_acquire(&HEAVY_ROUTE)
                    .ok_or_else(|| Status::resource_exhausted(crate::env::ERR_HEAVY_LIMIT))?;
                if path == "/tendermint.rpc.grpc.BlockAPI/BlockByHash" {
                    let r: pg::BlockByHashRequest = decode(&data)?;
                    block_parts_stream(&env, 0, Some(&r.hash), r.prove)
                } else {
                    let r: pg::BlockByHeightRequest = decode(&data)?;
                    let h = if r.height == 0 {
                        env.block_store.height()
                    } else {
                        r.height
                    };
                    block_parts_stream(&env, h, None, r.prove)
                }
            })
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
            // Go sends the parts it has before failing; an error before the
            // first part is the call's status.
            let items = items?;
            let s: BoxStream = Box::pin(futures_util::stream::iter(items.into_iter().map(Ok)));
            Ok(tonic::Response::new(s))
        })
    }
}

const STREAMING: &[&str] = &[
    "/tendermint.rpc.grpc.BlockAPI/BlockByHash",
    "/tendermint.rpc.grpc.BlockAPI/BlockByHeight",
    "/tendermint.rpc.grpc.BlockAPI/SubscribeNewHeights",
];

/// Every method path served here.
pub const METHODS: &[&str] = &[
    "/tendermint.rpc.grpc.BlockAPI/BlockByHash",
    "/tendermint.rpc.grpc.BlockAPI/BlockByHeight",
    "/tendermint.rpc.grpc.BlockAPI/Commit",
    "/tendermint.rpc.grpc.BlockAPI/ValidatorSet",
    "/tendermint.rpc.grpc.BlockAPI/SubscribeNewHeights",
    "/tendermint.rpc.grpc.BlockAPI/Status",
    "/tendermint.rpc.grpc.BlobstreamAPI/DataRootInclusionProof",
    "/tendermint.rpc.grpc.BroadcastAPI/Ping",
    "/tendermint.rpc.grpc.BroadcastAPI/BroadcastTx",
];

async fn grpc_handler(State(env): State<Arc<Environment>>, req: Request<Body>) -> Response<Body> {
    let path = req.uri().path().to_owned();
    let mut grpc = tonic::server::Grpc::new(RawCodec);
    if STREAMING.contains(&path.as_str()) {
        grpc.server_streaming(StreamSvc { env, path }, req)
            .await
            .map(Body::new)
    } else {
        grpc.unary(UnarySvc { env, path }, req).await.map(Body::new)
    }
}

/// An axum router serving the celestia-core gRPC methods (merge it into the
/// app's gRPC router, whose fallback handles every other path).
pub fn router(env: Arc<Environment>) -> axum::Router {
    let mut r = axum::Router::new();
    for m in METHODS {
        r = r.route(m, axum::routing::post(grpc_handler));
    }
    r.with_state(env)
}

/// Serves the celestia-core gRPC services alone (`grpc_laddr`), h2c.
pub async fn serve(
    listener: tokio::net::TcpListener,
    env: Arc<Environment>,
) -> std::io::Result<()> {
    let app = router(env).fallback(|req: Request<Body>| async move {
        let path = req.uri().path().to_owned();
        let svc = path
            .trim_start_matches('/')
            .split('/')
            .next()
            .unwrap_or("")
            .to_owned();
        let status = Status::unimplemented(format!("unknown service {svc}"));
        status.into_http().map(Body::new)
    });
    axum::serve(listener, app).await
}

/// Builds a `ptypes::Part` proof-less copy (helper for tests).
#[must_use]
pub fn strip_proof(mut p: ptypes::Part) -> ptypes::Part {
    p.proof = Some(ctypes::proto::crypto::Proof::default());
    p
}
