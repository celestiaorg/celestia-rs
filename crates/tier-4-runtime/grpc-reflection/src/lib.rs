//! Bidirectional gRPC reflection with Celestia's listener-specific registration.
//!
//! Core registers v1 and v1alpha; the Cosmos SDK listener registers only
//! v1alpha. The registry stores original Go descriptor bytes, preserving custom
//! protobuf options and dependency order. Each response is produced only when
//! polled by the transport: there is no detached task or unbounded send queue.

pub mod proto;
mod registry;

use std::collections::BTreeSet;
use std::future::Future;
use std::io::Read as _;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, Response};
use futures_util::{Stream, StreamExt};
use prost::Message;
use tonic::Status;

pub use registry::Registry;

/// The application's listener and the separate Comet core listener have
/// different reflection registrations and protobuf registries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Cosmos SDK gRPC listener, v1alpha only.
    App,
    /// Comet core gRPC listener, v1 and v1alpha.
    Core,
}

/// Standard stable reflection method.
pub const V1: &str = "/grpc.reflection.v1.ServerReflection/ServerReflectionInfo";
/// Standard alpha reflection method.
pub const V1ALPHA: &str = "/grpc.reflection.v1alpha.ServerReflection/ServerReflectionInfo";

impl Profile {
    /// Original listener-specific Go descriptors and advertised service list.
    #[must_use]
    #[allow(clippy::expect_used)] // Embedded release data is checked by socket/integrity tests.
    pub fn registry(self) -> &'static Arc<Registry> {
        static APP: OnceLock<Arc<Registry>> = OnceLock::new();
        static CORE: OnceLock<Arc<Registry>> = OnceLock::new();
        let (slot, files, services, extensions, lookups): (_, &[u8], &str, &str, &[u8]) = match self
        {
            Self::App => (
                &APP,
                include_bytes!("app-files.bin.gz"),
                include_str!("app-services.txt"),
                include_str!("app-extensions.txt"),
                include_bytes!("app-lookups.bin.gz"),
            ),
            Self::Core => (
                &CORE,
                include_bytes!("core-files.bin.gz"),
                include_str!("core-services.txt"),
                include_str!("core-extensions.txt"),
                include_bytes!("core-lookups.bin.gz"),
            ),
        };
        slot.get_or_init(|| {
            let mut raw = Vec::new();
            flate2::read::GzDecoder::new(files)
                .read_to_end(&mut raw)
                .expect("embedded reflection registry is valid gzip");
            let descriptors = proto::FileDescriptorResponse::decode(raw.as_slice())
                .expect("embedded reflection registry is valid protobuf");
            let mut lookup_bytes = Vec::new();
            flate2::read::GzDecoder::new(lookups)
                .read_to_end(&mut lookup_bytes)
                .expect("embedded lookup registry is valid gzip");
            let lookups = proto::LookupRegistry::decode(lookup_bytes.as_slice())
                .expect("embedded lookups are valid protobuf");
            Arc::new(
                Registry::new(
                    self,
                    services.lines().map(str::to_owned).collect(),
                    descriptors.file_descriptor_proto,
                )
                .expect("embedded reflection descriptors are valid protobuf")
                .with_registered_extensions(extensions.lines().map(|line| {
                    let (name, number) = line
                        .split_once(' ')
                        .expect("embedded extension registration has type and number");
                    (
                        name.to_owned(),
                        number.parse().expect("embedded extension number is valid"),
                    )
                }))
                .with_lookup_overrides(lookups.entries)
                .expect("embedded lookup descriptors are valid protobuf"),
            )
        })
    }
}

// Go's modern protobuf decoder retains a known tag with the wrong wire type
// as an unknown field. prost rejects that tag, so separate it before decoding.
fn request_fields(raw: &[u8]) -> Result<(Vec<u8>, Vec<u8>), prost::DecodeError> {
    let mut known = Vec::new();
    let mut unknown = Vec::new();
    let mut remaining = raw;
    while !remaining.is_empty() {
        let start = raw.len() - remaining.len();
        let (tag, wire_type) = prost::encoding::decode_key(&mut remaining)?;
        prost::encoding::skip_field(
            wire_type,
            tag,
            &mut remaining,
            prost::encoding::DecodeContext::default(),
        )?;
        let end = raw.len() - remaining.len();
        let output = if matches!(tag, 1 | 3..=7)
            && wire_type == prost::encoding::WireType::LengthDelimited
        {
            &mut known
        } else {
            &mut unknown
        };
        output.extend_from_slice(&raw[start..end]);
    }
    Ok((known, unknown))
}

type BoxStream = Pin<Box<dyn Stream<Item = Result<Vec<u8>, Status>> + Send>>;

#[derive(Debug)]
/// A malformed reflection request that terminates its stream.
pub struct ReflectionError {
    code: tonic::Code,
    message: String,
}

impl ReflectionError {
    fn internal(message: impl Into<String>) -> Self {
        Self {
            code: tonic::Code::Internal,
            message: message.into(),
        }
    }

    pub(crate) fn invalid_argument(message: impl Into<String>) -> Self {
        Self {
            code: tonic::Code::InvalidArgument,
            message: message.into(),
        }
    }
}

impl From<ReflectionError> for Status {
    fn from(error: ReflectionError) -> Self {
        Self::new(error.code, error.message)
    }
}

fn process(
    registry: &Registry,
    raw: &[u8],
    sent: &mut BTreeSet<String>,
    preserve_unknown: bool,
) -> Result<Vec<u8>, ReflectionError> {
    let invalid = |error: prost::DecodeError| {
        let reason = if error.to_string().contains("UTF-8") {
            "string field contains invalid UTF-8"
        } else {
            "proto: cannot parse invalid wire-format data"
        };
        ReflectionError::internal(format!(
            "grpc: failed to unmarshal the received message: {reason}"
        ))
    };
    let (known, unknown) = request_fields(raw).map_err(invalid)?;
    let request = proto::Request::decode(known.as_slice()).map_err(invalid)?;
    let mut original = request.encode_to_vec();
    if preserve_unknown {
        original.extend_from_slice(&unknown);
    }
    let response = registry.answer(request, sent)?;
    Ok(proto::WireResponse {
        valid_host: response.valid_host,
        original_request: Some(original),
        result: response.result,
    }
    .encode_to_vec())
}

struct Reflection {
    registry: Arc<Registry>,
    limits: grpc_transport::Limits,
    preserve_unknown: bool,
}

impl tonic::server::StreamingService<Vec<u8>> for Reflection {
    type Response = Vec<u8>;
    type ResponseStream = BoxStream;
    type Future = Pin<Box<dyn Future<Output = Result<tonic::Response<BoxStream>, Status>> + Send>>;

    fn call(&mut self, request: tonic::Request<tonic::Streaming<Vec<u8>>>) -> Self::Future {
        let registry = Arc::clone(&self.registry);
        let limits = self.limits;
        let preserve_unknown = self.preserve_unknown;
        Box::pin(async move {
            let mut incoming = request.into_inner();
            let mut sent = BTreeSet::new();
            // grpc-go sends initial headers only with its first response. An
            // empty stream or an initial codec/lookup-operation failure is a
            // trailers-only response, not an initial success followed by error.
            let Some(first) = incoming.next().await else {
                return Err(Status::new(tonic::Code::Ok, ""));
            };
            let first =
                process(&registry, &first?, &mut sent, preserve_unknown).map_err(Status::from)?;
            if first.len() > limits.max_send {
                return Err(Status::resource_exhausted(format!(
                    "trying to send message larger than max ({} vs. {})",
                    first.len(),
                    limits.max_send
                )));
            }
            let state = (incoming, sent, registry, false, preserve_unknown);
            let rest = futures_util::stream::unfold(
                state,
                |(mut incoming, mut sent, registry, ended, preserve_unknown)| async move {
                    if ended {
                        return None;
                    }
                    let result = match incoming.next().await? {
                        Ok(raw) => process(&registry, &raw, &mut sent, preserve_unknown)
                            .map_err(Status::from),
                        Err(error) => Err(error),
                    };
                    let ended = result.is_err();
                    Some((result, (incoming, sent, registry, ended, preserve_unknown)))
                },
            );
            let outgoing = futures_util::stream::iter([Ok(first)]).chain(rest);
            Ok(tonic::Response::new(Box::pin(outgoing) as BoxStream))
        })
    }
}

#[derive(Clone)]
struct Config {
    registry: Arc<Registry>,
    profile: Profile,
    limits: grpc_transport::Limits,
}

async fn handler(State(config): State<Config>, request: Request<Body>) -> Response<Body> {
    let path = request.uri().path();
    let (service, method) = path
        .trim_start_matches('/')
        .rsplit_once('/')
        .unwrap_or(("", ""));
    if method != "ServerReflectionInfo" {
        return Status::unimplemented(format!("unknown method {method} for service {service}"))
            .into_http()
            .map(Body::new);
    }
    let preserve_unknown = config.profile == Profile::App || path == V1;
    let mut grpc = config.limits.streaming_grpc();
    let response = grpc
        .streaming(
            Reflection {
                registry: config.registry,
                limits: config.limits,
                preserve_unknown,
            },
            request,
        )
        .await;
    grpc_transport::response(response)
}

/// Routes only the reflection versions registered on this listener. Merge the
/// core router only into the standalone core server, never the SDK listener.
pub fn router(profile: Profile) -> axum::Router {
    let limits = match profile {
        Profile::App => grpc_transport::Limits::sdk(),
        Profile::Core => grpc_transport::Limits::core(),
    };
    router_with_limits(profile, limits)
}

/// Reflection routes using the operator's listener-specific message limits.
pub fn router_with_limits(profile: Profile, limits: grpc_transport::Limits) -> axum::Router {
    let mut router = axum::Router::new()
        .route(V1ALPHA, axum::routing::post(handler))
        .route(
            "/grpc.reflection.v1alpha.ServerReflection/:method",
            axum::routing::post(handler),
        );
    if profile == Profile::Core {
        router = router.route(V1, axum::routing::post(handler)).route(
            "/grpc.reflection.v1.ServerReflection/:method",
            axum::routing::post(handler),
        );
    }
    router.with_state(Config {
        registry: Arc::clone(profile.registry()),
        profile,
        limits,
    })
}
