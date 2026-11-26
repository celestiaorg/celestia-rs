//! Protobuf messages the server speaks besides `celestia.fibre.v1`:
//! `grpc.health.v1` and the `google.rpc` error details.

/// `google.rpc.Status` (the `grpc-status-details-bin` payload).
#[derive(Clone, PartialEq, prost::Message)]
pub struct RpcStatus {
    /// gRPC code.
    #[prost(int32, tag = "1")]
    pub code: i32,
    /// Message.
    #[prost(string, tag = "2")]
    pub message: String,
    /// Details.
    #[prost(message, repeated, tag = "3")]
    pub details: Vec<pbjson_types::Any>,
}

/// `google.rpc.RetryInfo`.
#[derive(Clone, PartialEq, prost::Message)]
pub struct RetryInfo {
    /// How long the client should wait before retrying.
    #[prost(message, optional, tag = "1")]
    pub retry_delay: Option<pbjson_types::Duration>,
}

/// Type URL of [`RetryInfo`] in an `Any`.
pub const RETRY_INFO_TYPE_URL: &str = "type.googleapis.com/google.rpc.RetryInfo";

/// `grpc.health.v1.HealthCheckRequest`.
#[derive(Clone, PartialEq, prost::Message)]
pub struct HealthCheckRequest {
    /// Service name; empty for the server as a whole.
    #[prost(string, tag = "1")]
    pub service: String,
}

/// `grpc.health.v1.HealthCheckResponse.ServingStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum ServingStatus {
    /// UNKNOWN.
    Unknown = 0,
    /// SERVING.
    Serving = 1,
    /// NOT_SERVING.
    NotServing = 2,
    /// SERVICE_UNKNOWN (Watch only).
    ServiceUnknown = 3,
}

/// `grpc.health.v1.HealthCheckResponse`.
#[derive(Clone, PartialEq, prost::Message)]
pub struct HealthCheckResponse {
    /// The status.
    #[prost(enumeration = "ServingStatus", tag = "1")]
    pub status: i32,
}

/// `grpc.health.v1.HealthListRequest`.
#[derive(Clone, PartialEq, prost::Message)]
pub struct HealthListRequest {}

/// `grpc.health.v1.HealthListResponse`.
#[derive(Clone, PartialEq, prost::Message)]
pub struct HealthListResponse {
    /// Status of every known service.
    #[prost(btree_map = "string, message", tag = "1")]
    pub statuses: std::collections::BTreeMap<String, HealthCheckResponse>,
}
