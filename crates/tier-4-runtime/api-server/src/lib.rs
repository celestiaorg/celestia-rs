//! The application's gRPC query server (Go default `localhost:9090`) and
//! REST API (grpc-gateway, Go default `localhost:1317`).
//!
//! Both are thin transports over [`app::App::grpc_query`]: the gRPC server
//! accepts every method of the generated method table
//! (`celestia_proto::services`) through one raw-bytes codec, so handlers,
//! errors and height semantics are shared with the ABCI `Query` path. The
//! REST routes are derived from the methods' `google.api.http` bindings
//! ([`rest`]); no route is written by hand.

pub mod jsonpb;
pub mod rest;

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

use app::AbciApp;
use app::query::GrpcQueryError;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, Request, Response, StatusCode};
use bytes::{Buf, BufMut};
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::{Code, Status};

/// `grpctypes.GRPCBlockHeightHeader`.
pub const BLOCK_HEIGHT_HEADER: &str = "x-cosmos-block-height";

/// A codec passing protobuf message bytes through unchanged.
#[derive(Debug, Clone, Copy, Default)]
pub struct RawCodec;

/// Raw bytes encoder/decoder.
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

fn code_of(c: grpc_query::Code) -> Code {
    Code::from_i32(c as i32)
}

/// Parses the `x-cosmos-block-height` value as the Go interceptor does.
pub fn parse_height(v: Option<&str>) -> Result<i64, (grpc_query::Code, String)> {
    let Some(v) = v else {
        return Ok(0);
    };
    let h: i64 = v.parse().map_err(|e: std::num::ParseIntError| {
        grpc_query::QueryError::Sdk(sdk_invalid_request(&format!(
            "Baseapp.RegisterGRPCServer: invalid height header \"{BLOCK_HEIGHT_HEADER}\": strconv.ParseInt: parsing \"{v}\": {}",
            go_parse_int_reason(&e)
        )))
        .grpc_status()
    })?;
    if h < 0 {
        return Err(grpc_query::QueryError::Sdk(sdk_invalid_request(
            "cannot query with height < 0; please provide a valid height",
        ))
        .grpc_status());
    }
    Ok(h)
}

fn go_parse_int_reason(e: &std::num::ParseIntError) -> &'static str {
    match e.kind() {
        std::num::IntErrorKind::PosOverflow | std::num::IntErrorKind::NegOverflow => {
            "value out of range"
        }
        _ => "invalid syntax",
    }
}

fn sdk_invalid_request(msg: &str) -> sdk::SdkError {
    sdk::error::sdk::invalid_request(msg)
}

/// Runs a gRPC query on the app (blocking; call from a blocking task).
pub fn grpc_call(
    app: &AbciApp,
    path: &str,
    data: &[u8],
    height: i64,
) -> Result<(Vec<u8>, i64), (grpc_query::Code, String)> {
    app.grpc_query(path, data, height)
        .map_err(|e| grpc_error_status(path, e))
}

/// Services Go's gRPC server registers that are answered outside the
/// module query router here (or not ported): an unknown method of these is
/// "unknown method", any other service is "unknown service".
const GO_ONLY_SERVICES: &[&str] = &[
    "cosmos.tx.v1beta1.Service",
    "cosmos.base.node.v1beta1.Service",
    "cosmos.base.tendermint.v1beta1.Service",
    "cosmos.base.reflection.v1beta1.ReflectionService",
    "cosmos.base.reflection.v2alpha1.ReflectionService",
    "cosmos.reflection.v1.ReflectionService",
    "celestia.core.v1.gas_estimation.GasEstimator",
    "celestia.core.v1.tx.Tx",
];

/// The gRPC status Go's server returns for a failed query on `path`.
#[must_use]
pub fn grpc_error_status(path: &str, e: GrpcQueryError) -> (grpc_query::Code, String) {
    match e {
        GrpcQueryError::Unimplemented(_) => {
            let (svc, method) = path
                .trim_start_matches('/')
                .split_once('/')
                .unwrap_or((path, ""));
            let known = grpc_query::router().has_service(svc) || GO_ONLY_SERVICES.contains(&svc);
            let msg = if known {
                format!("unknown method {method} for service {svc}")
            } else {
                format!("unknown service {svc}")
            };
            (grpc_query::Code::Unimplemented, msg)
        }
        other => other.grpc_status(),
    }
}

struct Unary {
    app: Arc<AbciApp>,
    path: String,
}

type BoxFut<T> = Pin<Box<dyn Future<Output = T> + Send>>;

impl tonic::server::UnaryService<Vec<u8>> for Unary {
    type Response = Vec<u8>;
    type Future = BoxFut<Result<tonic::Response<Vec<u8>>, Status>>;

    fn call(&mut self, request: tonic::Request<Vec<u8>>) -> Self::Future {
        let app = Arc::clone(&self.app);
        let path = self.path.clone();
        Box::pin(async move {
            let height = request
                .metadata()
                .get(BLOCK_HEIGHT_HEADER)
                .and_then(|v| v.to_str().ok().map(str::to_owned));
            let height =
                parse_height(height.as_deref()).map_err(|(c, m)| Status::new(code_of(c), m))?;
            let data = request.into_inner();
            let res = tokio::task::spawn_blocking(move || grpc_call(&app, &path, &data, height))
                .await
                .map_err(|e| Status::internal(e.to_string()))?;
            let (bytes, h) = res.map_err(|(c, m)| Status::new(code_of(c), m))?;
            let mut resp = tonic::Response::new(bytes);
            // A negative height: a service outside the query router (no header).
            if let Some(Ok(v)) = (h >= 0).then(|| h.to_string().parse()) {
                resp.metadata_mut().insert(BLOCK_HEIGHT_HEADER, v);
            }
            Ok(resp)
        })
    }
}

async fn grpc_handler(State(app): State<Arc<AbciApp>>, req: Request<Body>) -> Response<Body> {
    let path = req.uri().path().to_owned();
    let mut grpc = tonic::server::Grpc::new(RawCodec);
    let resp = grpc.unary(Unary { app, path }, req).await;
    resp.map(Body::new)
}

/// The gRPC router: every path is a unary query method.
pub fn grpc_router(app: Arc<AbciApp>) -> axum::Router {
    axum::Router::new().fallback(grpc_handler).with_state(app)
}

/// The bytes of a REST response body. Strings are Go forms (`gostr`): Go's
/// gateway (gogoproto `jsonpb`) writes each invalid UTF-8 byte as `\ufffd`.
#[must_use]
pub fn body_bytes(body: &serde_json::Value) -> Vec<u8> {
    gostr::json_bytes(&body.to_string()).into_owned()
}

async fn rest_handler(State(app): State<Arc<AbciApp>>, req: Request<Body>) -> Response<Body> {
    let method = req.method().as_str().to_owned();
    let path = req.uri().path().to_owned();
    let query = req.uri().query().unwrap_or_default().to_owned();
    let height = req
        .headers()
        .get(BLOCK_HEIGHT_HEADER)
        .and_then(|v| v.to_str().ok().map(str::to_owned));
    let body = axum::body::to_bytes(req.into_body(), 64 << 20)
        .await
        .unwrap_or_default();
    let out = tokio::task::spawn_blocking(move || {
        rest::handle(
            &rest::Request {
                method: &method,
                path: &path,
                query: &query,
                height: height.as_deref(),
                body: &body,
            },
            &mut |p, d, h| grpc_call(&app, p, d, h),
        )
    })
    .await
    .unwrap_or_else(|e| rest::Reply {
        status: 500,
        body: serde_json::json!({"code": 13, "message": e.to_string(), "details": []}),
    });
    let body = body_bytes(&out.body);
    let mut resp = Response::new(Body::from(body));
    *resp.status_mut() = StatusCode::from_u16(out.status).unwrap_or(StatusCode::OK);
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    resp
}

/// The REST router (grpc-gateway routes derived from the descriptors).
pub fn rest_router(app: Arc<AbciApp>) -> axum::Router {
    axum::Router::new().fallback(rest_handler).with_state(app)
}

/// Serves the gRPC query server on `addr` (h2c).
pub async fn serve_grpc(addr: SocketAddr, app: Arc<AbciApp>) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "gRPC query server listening");
    serve_grpc_on(listener, app).await
}

/// Serves the gRPC query server on a bound listener.
pub async fn serve_grpc_on(
    listener: tokio::net::TcpListener,
    app: Arc<AbciApp>,
) -> std::io::Result<()> {
    axum::serve(listener, grpc_router(app)).await
}

/// Serves the REST API on a bound listener.
pub async fn serve_rest_on(
    listener: tokio::net::TcpListener,
    app: Arc<AbciApp>,
) -> std::io::Result<()> {
    axum::serve(listener, rest_router(app)).await
}

/// Serves the REST API on `addr`.
pub async fn serve_rest(addr: SocketAddr, app: Arc<AbciApp>) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "REST API listening");
    axum::serve(listener, rest_router(app)).await
}
