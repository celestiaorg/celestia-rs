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
use futures_util::{Stream, StreamExt as _};
use prost::Message as _;
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::{Code, Status};

use crate::env::{Environment, Route};
use crate::query::Query;

/// Per-message gRPC request and response limits.
pub use grpc_transport::Limits as GrpcLimits;

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

fn broadcast_response(result: crate::server::BroadcastCommit) -> pg::ResponseBroadcastTx {
    // celestia-core's adapter deliberately projects the commit responses to
    // these three fields. In particular, gas and events are not exposed by
    // the core gRPC BroadcastAPI even though BroadcastTxCommit observes them.
    pg::ResponseBroadcastTx {
        check_tx: Some(ctypes::proto::abci::ResponseCheckTx {
            code: result.check_tx.code,
            data: result.check_tx.data,
            log: result.check_tx.log,
            ..Default::default()
        }),
        tx_result: Some(ctypes::proto::abci::ExecTxResult {
            code: result.tx_result.code,
            data: result.tx_result.data,
            log: result.tx_result.log,
            ..Default::default()
        }),
    }
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

struct HeightSubscription {
    bus: Arc<crate::events::EventBus>,
    client: String,
}

impl Drop for HeightSubscription {
    fn drop(&mut self) {
        // A dropped response stream (cancellation, disconnect or send-limit
        // error) must release the event-bus subscription even without a later
        // block publication to notice its closed receiver.
        let _ = self.bus.unsubscribe_all(&self.client);
    }
}

fn subscribe_new_heights(env: &Arc<Environment>) -> Result<BoxStream, Status> {
    let client = format!(
        "block-api-subscription-{}",
        SUB_SEQ.fetch_add(1, Ordering::SeqCst)
    );
    let q = Query::new("tm.event='NewBlock'").map_err(unknown)?;
    let sub = env.event_bus.subscribe(&client, q, 500).map_err(unknown)?;
    let guard = HeightSubscription {
        bus: Arc::clone(&env.event_bus),
        client,
    };
    let env = Arc::clone(env);
    let stream =
        futures_util::stream::unfold((sub, env, guard), |(mut sub, env, guard)| async move {
            let m = sub.out.recv().await?;
            let crate::events::EventData::NewBlock { block, .. } = m.data.as_ref() else {
                return Some((Err(Status::internal("unexpected event")), (sub, env, guard)));
            };
            let mut b = (**block).clone();
            let resp = pg::SubscribeNewHeightsResponse {
                height: b.header.height,
                hash: b.hash().unwrap_or_default(),
                time: Some(b.header.time.to_proto()),
                catching_up: env.node.is_catching_up(),
            };
            Some((Ok(resp.encode_to_vec()), (sub, env, guard)))
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
            if path == "/tendermint.rpc.grpc.BroadcastAPI/BroadcastTx" {
                let request: pg::RequestBroadcastTx = decode(&data)?;
                // Go's gRPC adapter calls BroadcastTxCommit with an empty RPC
                // context, sharing its admission, subscription and timeout rules.
                let result = crate::server::broadcast_commit(&env, request.tx, "")
                    .await
                    .map_err(unknown)?;
                let response = broadcast_response(result);
                return Ok(tonic::Response::new(response.encode_to_vec()));
            }
            let res = tokio::task::spawn_blocking(move || {
                let _g =
                    if HEAVY.contains(&path.as_str()) {
                        Some(env.try_acquire(&HEAVY_ROUTE).ok_or_else(|| {
                            Status::resource_exhausted(crate::env::ERR_HEAVY_LIMIT)
                        })?)
                    } else {
                        None
                    };
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
    max_send: usize,
}

async fn stream_response(
    mut stream: BoxStream,
    max_send: usize,
) -> Result<tonic::Response<BoxStream>, Status> {
    // grpc-go sends headers with its first successful SendMsg, not when a
    // handler creates a stream. In particular, an oversized first response
    // must be a trailers-only error. Waiting also avoids prematurely exposing
    // successful headers for a cancelled subscription with no event yet.
    let Some(first) = stream.next().await else {
        return Ok(tonic::Response::new(stream));
    };
    let first = first?;
    if first.len() > max_send {
        return Err(Status::resource_exhausted(format!(
            "trying to send message larger than max ({} vs. {max_send})",
            first.len()
        )));
    }
    let stream: BoxStream =
        Box::pin(futures_util::stream::once(async move { Ok(first) }).chain(stream));
    Ok(tonic::Response::new(stream))
}

impl tonic::server::ServerStreamingService<Vec<u8>> for StreamSvc {
    type Response = Vec<u8>;
    type ResponseStream = BoxStream;
    type Future = BoxFut<Result<tonic::Response<BoxStream>, Status>>;

    fn call(&mut self, request: tonic::Request<Vec<u8>>) -> Self::Future {
        let env = Arc::clone(&self.env);
        let path = self.path.clone();
        let max_send = self.max_send;
        Box::pin(async move {
            let data = request.into_inner();
            if path == "/tendermint.rpc.grpc.BlockAPI/SubscribeNewHeights" {
                return stream_response(subscribe_new_heights(&env)?, max_send).await;
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
            stream_response(s, max_send).await
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

async fn grpc_handler(
    State((env, limits)): State<(Arc<Environment>, GrpcLimits)>,
    req: Request<Body>,
) -> Response<Body> {
    let path = req.uri().path().to_owned();
    if STREAMING.contains(&path.as_str()) {
        grpc_transport::response(
            limits
                .streaming_grpc()
                .server_streaming(
                    StreamSvc {
                        env,
                        path,
                        max_send: limits.max_send,
                    },
                    req,
                )
                .await,
        )
    } else {
        grpc_transport::response(limits.grpc().unary(UnarySvc { env, path }, req).await)
    }
}

/// An axum router serving the celestia-core gRPC methods (merge it into the
/// app's gRPC router, whose fallback handles every other path).
pub fn router(env: Arc<Environment>) -> axum::Router {
    router_with_limits(env, GrpcLimits::core())
}

/// Core methods mounted on a listener with its own message limits (including
/// the app listener, whose limits differ from the standalone core defaults).
pub fn router_with_limits(env: Arc<Environment>, limits: GrpcLimits) -> axum::Router {
    methods_router(env, limits, false)
}

/// Only BlockAPI is registered on celestia-app's SDK listener. BroadcastAPI
/// and BlobstreamAPI belong to the standalone core listener.
pub fn block_router_with_limits(env: Arc<Environment>, limits: GrpcLimits) -> axum::Router {
    methods_router(env, limits, true)
}

async fn unknown_method(req: Request<Body>) -> Response<Body> {
    let (service, method) = req
        .uri()
        .path()
        .trim_start_matches('/')
        .split_once('/')
        .unwrap_or(("", ""));
    Status::unimplemented(format!("unknown method {method} for service {service}"))
        .into_http()
        .map(Body::new)
}

fn methods_router(env: Arc<Environment>, limits: GrpcLimits, block_only: bool) -> axum::Router {
    let mut r = axum::Router::new();
    for service in [
        "tendermint.rpc.grpc.BlockAPI",
        "tendermint.rpc.grpc.BroadcastAPI",
        "tendermint.rpc.grpc.BlobstreamAPI",
    ] {
        if !block_only || service == "tendermint.rpc.grpc.BlockAPI" {
            r = r.route(
                &format!("/{service}/:method"),
                axum::routing::post(unknown_method),
            );
        }
    }
    for m in METHODS {
        if !block_only || m.starts_with("/tendermint.rpc.grpc.BlockAPI/") {
            r = r.route(m, axum::routing::post(grpc_handler));
        }
    }
    r.with_state((env, limits))
}

/// Serves the celestia-core gRPC services alone (`grpc_laddr`), h2c.
pub async fn serve(
    listener: tokio::net::TcpListener,
    env: Arc<Environment>,
) -> std::io::Result<()> {
    let limit = env.config.grpc_max_open_connections;
    let sem = (limit > 0).then(|| Arc::new(tokio::sync::Semaphore::new(limit)));
    let app = router(env)
        .merge(grpc_reflection::router(grpc_reflection::Profile::Core))
        .fallback(|req: Request<Body>| async move {
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
    loop {
        // Acquire before accepting, like netutil.LimitListener. Waiting TCP
        // connections remain in the kernel backlog until capacity is released.
        let permit = match &sem {
            Some(sem) => Some(
                Arc::clone(sem)
                    .acquire_owned()
                    .await
                    .map_err(|error| std::io::Error::other(error.to_string()))?,
            ),
            None => None,
        };
        let (stream, remote) = listener.accept().await?;
        let service = hyper_util::service::TowerToHyperService::new(app.clone());
        tokio::spawn(async move {
            let _permit = permit;
            let builder =
                hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new());
            if let Err(error) = builder
                .serve_connection_with_upgrades(hyper_util::rt::TokioIo::new(stream), service)
                .await
            {
                tracing::debug!(%error, %remote, "core gRPC connection closed");
            }
        });
    }
}

/// Builds a `ptypes::Part` proof-less copy (helper for tests).
#[must_use]
pub fn strip_proof(mut p: ptypes::Part) -> ptypes::Part {
    p.proof = Some(ctypes::proto::crypto::Proof::default());
    p
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn core_broadcast_projects_the_go_response_fields() {
        let response = broadcast_response(crate::server::BroadcastCommit {
            check_tx: ctypes::proto::abci::ResponseCheckTx {
                code: 3,
                data: vec![1],
                log: "check".into(),
                gas_wanted: 99,
                ..Default::default()
            },
            tx_result: ctypes::proto::abci::ExecTxResult {
                code: 4,
                data: vec![2],
                log: "exec".into(),
                gas_used: 88,
                events: vec![ctypes::proto::abci::Event::default()],
                ..Default::default()
            },
            hash: vec![9; 32],
            height: 12,
        });
        assert_eq!(
            response.check_tx,
            Some(ctypes::proto::abci::ResponseCheckTx {
                code: 3,
                data: vec![1],
                log: "check".into(),
                ..Default::default()
            })
        );
        assert_eq!(
            response.tx_result,
            Some(ctypes::proto::abci::ExecTxResult {
                code: 4,
                data: vec![2],
                log: "exec".into(),
                ..Default::default()
            })
        );
    }

    #[tokio::test]
    async fn core_broadcast_is_served_over_http2() -> Result<(), Box<dyn std::error::Error>> {
        let env = crate::server::broadcast_tests::environment(
            Ok(ctypes::proto::abci::ResponseCheckTx {
                code: 7,
                data: vec![1, 2],
                log: "rejected".into(),
                ..Default::default()
            }),
            1_000,
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server = tokio::spawn(serve(listener, Arc::clone(&env)));

        let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))?
            .connect()
            .await?;
        let mut client = tonic::client::Grpc::new(channel);
        client.ready().await?;
        let path: axum::http::uri::PathAndQuery =
            "/tendermint.rpc.grpc.BroadcastAPI/BroadcastTx".parse()?;
        let response = client
            .unary(
                tonic::Request::new(
                    pg::RequestBroadcastTx {
                        tx: b"rejected transaction".to_vec(),
                    }
                    .encode_to_vec(),
                ),
                path,
                RawCodec,
            )
            .await?;
        let response = pg::ResponseBroadcastTx::decode(response.into_inner().as_slice())?;
        assert_eq!(
            response.check_tx,
            Some(ctypes::proto::abci::ResponseCheckTx {
                code: 7,
                data: vec![1, 2],
                log: "rejected".into(),
                ..Default::default()
            })
        );
        assert_eq!(
            response.tx_result,
            Some(ctypes::proto::abci::ExecTxResult::default())
        );
        assert_eq!(env.event_bus.num_clients(), 0);
        server.abort();
        Ok(())
    }
}

#[cfg(test)]
mod subscription_tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn dropping_stream_guard_cleans_up_without_another_event() {
        let bus = Arc::new(crate::events::EventBus::new());
        let sub = bus
            .subscribe("grpc-height", Query::new("tm.event='NewBlock'").unwrap(), 1)
            .unwrap();
        let guard = HeightSubscription {
            bus: Arc::clone(&bus),
            client: "grpc-height".into(),
        };
        assert_eq!(bus.num_clients(), 1);
        drop(sub);
        assert_eq!(
            bus.num_clients(),
            1,
            "a disconnected receiver alone does not remove a subscription"
        );
        drop(guard);
        assert_eq!(bus.num_clients(), 0);
    }
}
