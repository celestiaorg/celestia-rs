//! `cosmos.reflection.v1.ReflectionService` (on the query router: every
//! file descriptor linked into celestia-appd, recorded from the Go binary)
//! and `cosmos.base.reflection.v2alpha1.ReflectionService` (registered by
//! Go on the gRPC server only, not reachable over ABCI).
//!
//! Both answer static configuration of the Go binary; the descriptors are
//! the bytes celestia-appd v10.4.0 returns (gzip, `reflection_v1_files`),
//! the v2alpha1 descriptors Go's answers with the lists Go builds from map
//! iteration (random order) put in a canonical order ([`canonical_v2alpha1`]).

use std::io::Read as _;
use std::sync::OnceLock;

use celestia_proto::cosmos::base::reflection::v2alpha1 as v2;
use prost::Message;

use crate::Router;
use crate::error::QueryError;

/// `cosmos.reflection.v1.ReflectionService/FileDescriptors`.
pub const FILE_DESCRIPTORS: &str = "/cosmos.reflection.v1.ReflectionService/FileDescriptors";

/// The `cosmos.base.reflection.v2alpha1.ReflectionService` path prefix.
pub const V2ALPHA1: &str = "/cosmos.base.reflection.v2alpha1.ReflectionService/";

const FILES_GZ: &[u8] = include_bytes!("reflection_v1_files.bin.gz");

/// An empty request message (unknown fields are skipped like gogoproto).
#[derive(Clone, PartialEq, Message)]
struct Empty {}

fn file_descriptors_response() -> &'static [u8] {
    static R: OnceLock<Vec<u8>> = OnceLock::new();
    R.get_or_init(|| {
        let mut out = Vec::new();
        let _ = flate2::read::GzDecoder::new(FILES_GZ).read_to_end(&mut out);
        out
    })
}

pub(crate) fn register(r: &mut Router) {
    r.add_raw(
        FILE_DESCRIPTORS,
        Box::new(|_, data| {
            Empty::decode(data).map_err(|e| crate::decode_error(&e))?;
            Ok(file_descriptors_response().to_vec())
        }),
    );
}

fn sort_by_bytes<M: Message>(v: &mut [M]) {
    v.sort_by_cached_key(Message::encode_to_vec);
}

/// Puts the lists Go fills from map iteration (sign modes, interfaces and
/// their implementers / accepting messages, query services, tx messages)
/// in byte order (query services also sort their methods), so answers can
/// be compared.
#[must_use]
pub fn canonical_v2alpha1(method: &str, bytes: &[u8]) -> Option<Vec<u8>> {
    Some(match method {
        "GetAuthnDescriptor" => {
            let mut r = v2::GetAuthnDescriptorResponse::decode(bytes).ok()?;
            if let Some(a) = r.authn.as_mut() {
                sort_by_bytes(&mut a.sign_modes);
            }
            r.encode_to_vec()
        }
        "GetCodecDescriptor" => {
            let mut r = v2::GetCodecDescriptorResponse::decode(bytes).ok()?;
            if let Some(c) = r.codec.as_mut() {
                for i in &mut c.interfaces {
                    sort_by_bytes(&mut i.interface_implementers);
                    sort_by_bytes(&mut i.interface_accepting_messages);
                }
                sort_by_bytes(&mut c.interfaces);
            }
            r.encode_to_vec()
        }
        "GetQueryServicesDescriptor" => {
            let mut r = v2::GetQueryServicesDescriptorResponse::decode(bytes).ok()?;
            if let Some(q) = r.queries.as_mut() {
                for s in &mut q.query_services {
                    sort_by_bytes(&mut s.methods);
                }
                sort_by_bytes(&mut q.query_services);
            }
            r.encode_to_vec()
        }
        "GetTxDescriptor" => {
            let mut r = v2::GetTxDescriptorResponse::decode(bytes).ok()?;
            if let Some(t) = r.tx.as_mut() {
                sort_by_bytes(&mut t.msgs);
            }
            r.encode_to_vec()
        }
        "GetChainDescriptor" | "GetConfigurationDescriptor" => bytes.to_vec(),
        _ => return None,
    })
}

const AUTHN: &[u8] = include_bytes!("reflection_v2alpha1/GetAuthnDescriptor.bin");
const CODEC: &[u8] = include_bytes!("reflection_v2alpha1/GetCodecDescriptor.bin");
const CONFIG: &[u8] = include_bytes!("reflection_v2alpha1/GetConfigurationDescriptor.bin");
const QUERIES: &[u8] = include_bytes!("reflection_v2alpha1/GetQueryServicesDescriptor.bin");
const TX: &[u8] = include_bytes!("reflection_v2alpha1/GetTxDescriptor.bin");

/// The gRPC server's `v2alpha1` answer for `path` on chain `chain_id`
/// (`None`: not a v2alpha1 path; `Err`: unknown method or bad request).
/// Go serves it outside the query router, so the answer carries no
/// block-height header.
#[must_use]
pub fn v2alpha1(path: &str, data: &[u8], chain_id: &str) -> Option<Result<Vec<u8>, QueryError>> {
    let method = path.strip_prefix(V2ALPHA1)?;
    if let Err(e) = Empty::decode(data) {
        // grpc-go's codec failure (the service is not behind the router).
        let msg = match crate::decode_error(&e) {
            QueryError::Plain(m) | QueryError::Decode(m) => m,
            other => other.grpc_status().1,
        };
        return Some(Err(QueryError::Status(
            crate::Code::Internal,
            format!("grpc: error unmarshalling request: {msg}"),
        )));
    }
    Some(Ok(match method {
        "GetAuthnDescriptor" => AUTHN.to_vec(),
        "GetChainDescriptor" => v2::GetChainDescriptorResponse {
            chain: Some(v2::ChainDescriptor {
                id: chain_id.to_owned(),
            }),
        }
        .encode_to_vec(),
        "GetCodecDescriptor" => CODEC.to_vec(),
        "GetConfigurationDescriptor" => CONFIG.to_vec(),
        "GetQueryServicesDescriptor" => QUERIES.to_vec(),
        "GetTxDescriptor" => TX.to_vec(),
        other => {
            return Some(Err(QueryError::Status(
                crate::Code::Unimplemented,
                format!(
                    "unknown method {other} for service cosmos.base.reflection.v2alpha1.ReflectionService"
                ),
            )));
        }
    }))
}
