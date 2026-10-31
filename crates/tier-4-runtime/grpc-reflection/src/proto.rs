//! The common wire schema of grpc.reflection.v1 and v1alpha.
//!
//! They differ in package name only. Keeping descriptor bytes opaque preserves
//! custom Go protobuf options instead of re-encoding them through prost-types.

/// A reflection request.
#[derive(Clone, PartialEq, prost::Message)]
pub struct Request {
    /// Requested host (echoed without interpreting it).
    #[prost(string, tag = "1")]
    pub host: String,
    /// Lookup operation.
    #[prost(oneof = "request::Operation", tags = "3, 4, 5, 6, 7")]
    pub operation: Option<request::Operation>,
}

/// Request operations.
pub mod request {
    /// Lookup operation.
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Operation {
        /// Filename lookup.
        #[prost(string, tag = "3")]
        FileByFilename(String),
        /// Symbol lookup.
        #[prost(string, tag = "4")]
        FileContainingSymbol(String),
        /// Extension lookup.
        #[prost(message, tag = "5")]
        FileContainingExtension(super::ExtensionRequest),
        /// Extension enumeration.
        #[prost(string, tag = "6")]
        AllExtensionNumbersOfType(String),
        /// Registered service names.
        #[prost(string, tag = "7")]
        ListServices(String),
    }
}

/// An extension lookup.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ExtensionRequest {
    /// Fully qualified message name.
    #[prost(string, tag = "1")]
    pub containing_type: String,
    /// Extension tag.
    #[prost(int32, tag = "2")]
    pub extension_number: i32,
}

/// A reflection response.
#[derive(Clone, PartialEq, prost::Message)]
pub struct Response {
    /// Host echoed from the request.
    #[prost(string, tag = "1")]
    pub valid_host: String,
    /// Original request echoed from the stream.
    #[prost(message, optional, tag = "2")]
    pub original_request: Option<Request>,
    /// Lookup result or in-band error.
    #[prost(oneof = "response::Result", tags = "4, 5, 6, 7")]
    pub result: Option<response::Result>,
}

/// Response variants.
pub mod response {
    /// Lookup result.
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Result {
        /// Descriptor and unsent transitive dependencies.
        #[prost(message, tag = "4")]
        FileDescriptorResponse(super::FileDescriptorResponse),
        /// Extension enumeration.
        #[prost(message, tag = "5")]
        AllExtensionNumbersResponse(super::ExtensionNumberResponse),
        /// Service list.
        #[prost(message, tag = "6")]
        ListServicesResponse(super::ListServiceResponse),
        /// Non-terminal lookup failure.
        #[prost(message, tag = "7")]
        ErrorResponse(super::ErrorResponse),
    }
}

/// Raw FileDescriptorProto messages.
#[derive(Clone, PartialEq, prost::Message)]
pub struct FileDescriptorResponse {
    /// The requested descriptor first, then previously unsent dependencies.
    #[prost(bytes = "vec", repeated, tag = "1")]
    pub file_descriptor_proto: Vec<Vec<u8>>,
}

/// Registered extensions of a message.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ExtensionNumberResponse {
    /// Fully qualified message name.
    #[prost(string, tag = "1")]
    pub base_type_name: String,
    /// Extension tags (Go map iteration order is unspecified).
    #[prost(int32, repeated, tag = "2")]
    pub extension_number: Vec<i32>,
}

/// Registered services on this listener.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ListServiceResponse {
    /// Service names, in byte order.
    #[prost(message, repeated, tag = "1")]
    pub service: Vec<ServiceResponse>,
}

/// A registered service name.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ServiceResponse {
    /// Fully qualified service name.
    #[prost(string, tag = "1")]
    pub name: String,
}

/// An in-band reflection lookup failure.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ErrorResponse {
    /// gRPC status code.
    #[prost(int32, tag = "1")]
    pub error_code: i32,
    /// Go listener's lookup error.
    #[prost(string, tag = "2")]
    pub error_message: String,
}

// `original_request` is a message on the wire. Encoding it as opaque bytes in
// the transport preserves unknown fields from forward-compatible clients.
#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct WireResponse {
    #[prost(string, tag = "1")]
    pub valid_host: String,
    #[prost(bytes = "vec", optional, tag = "2")]
    pub original_request: Option<Vec<u8>>,
    #[prost(oneof = "response::Result", tags = "4, 5, 6, 7")]
    pub result: Option<response::Result>,
}

/// Captured runtime lookup closures whose Go resolver differs from filename
/// resolution (SDK's gogo and modern protobuf registries can share a filename).
#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct LookupRegistry {
    #[prost(message, repeated, tag = "1")]
    pub entries: Vec<LookupEntry>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct LookupEntry {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(bytes = "vec", repeated, tag = "2")]
    pub files: Vec<Vec<u8>>,
}
