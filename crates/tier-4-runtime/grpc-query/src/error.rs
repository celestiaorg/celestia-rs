//! Query handler errors and their two renderings: the ABCI `ResponseQuery`
//! (`baseapp.gRPCErrorToSDKError` + `sdkerrors.QueryResult`) and the gRPC
//! status returned by the gRPC server.

use sdk::SdkError;
use sdk::error::sdk as errs;

/// gRPC status codes (`google.golang.org/grpc/codes`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Code {
    /// `OK`.
    Ok = 0,
    /// `Canceled`.
    Canceled = 1,
    /// `Unknown`.
    Unknown = 2,
    /// `InvalidArgument`.
    InvalidArgument = 3,
    /// `DeadlineExceeded`.
    DeadlineExceeded = 4,
    /// `NotFound`.
    NotFound = 5,
    /// `AlreadyExists`.
    AlreadyExists = 6,
    /// `PermissionDenied`.
    PermissionDenied = 7,
    /// `ResourceExhausted`.
    ResourceExhausted = 8,
    /// `FailedPrecondition`.
    FailedPrecondition = 9,
    /// `Aborted`.
    Aborted = 10,
    /// `OutOfRange`.
    OutOfRange = 11,
    /// `Unimplemented`.
    Unimplemented = 12,
    /// `Internal`.
    Internal = 13,
    /// `Unavailable`.
    Unavailable = 14,
    /// `DataLoss`.
    DataLoss = 15,
    /// `Unauthenticated`.
    Unauthenticated = 16,
}

impl Code {
    /// Go's `codes.Code.String()`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Canceled => "Canceled",
            Self::Unknown => "Unknown",
            Self::InvalidArgument => "InvalidArgument",
            Self::DeadlineExceeded => "DeadlineExceeded",
            Self::NotFound => "NotFound",
            Self::AlreadyExists => "AlreadyExists",
            Self::PermissionDenied => "PermissionDenied",
            Self::ResourceExhausted => "ResourceExhausted",
            Self::FailedPrecondition => "FailedPrecondition",
            Self::Aborted => "Aborted",
            Self::OutOfRange => "OutOfRange",
            Self::Unimplemented => "Unimplemented",
            Self::Internal => "Internal",
            Self::Unavailable => "Unavailable",
            Self::DataLoss => "DataLoss",
            Self::Unauthenticated => "Unauthenticated",
        }
    }
}

/// An error returned by a query handler, keeping the Go error's shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryError {
    /// `status.Error(code, msg)`.
    Status(Code, String),
    /// A registered `errorsmod` error (all of them carry gRPC code
    /// `Unknown` in the pinned SDK).
    Sdk(SdkError),
    /// An unregistered Go error (`errors.New`, `fmt.Errorf`,
    /// `collections.ErrNotFound`, proto decode errors).
    Plain(String),
    /// Protobuf request decoding failed before the transport interceptor.
    /// ABCI renders it like an unregistered error; gRPC adds its codec prefix.
    Decode(String),
    /// A Go panic in the handler (recovered by `BaseApp.Query` as
    /// `ErrPanic`, by the gRPC server's recovery interceptor as `Internal`).
    Panic(String),
}

/// Handler result.
pub type QResult<T> = Result<T, QueryError>;

impl From<SdkError> for QueryError {
    fn from(e: SdkError) -> Self {
        // A Go runtime panic inside the handler (e.g. a nil store of a
        // query view, security.md 70): `BaseApp.Query` recovers it as
        // `ErrPanic`.
        if e.codespace == "undefined" && e.code == 111_222 {
            return Self::Panic(e.log);
        }
        Self::Sdk(e)
    }
}

/// `status.Error(codes.InvalidArgument, msg)`.
pub fn invalid_arg(msg: impl Into<String>) -> QueryError {
    QueryError::Status(Code::InvalidArgument, msg.into())
}

/// `status.Error(codes.NotFound, msg)`.
pub fn not_found(msg: impl Into<String>) -> QueryError {
    QueryError::Status(Code::NotFound, msg.into())
}

/// `status.Error(codes.Internal, msg)`.
pub fn internal(msg: impl Into<String>) -> QueryError {
    QueryError::Status(Code::Internal, msg.into())
}

/// `status.Error(codes.InvalidArgument, "empty request")`, the most common
/// nil-request guard.
#[must_use]
pub fn empty_request() -> QueryError {
    invalid_arg("empty request")
}

/// Go's description of a registered error, used to rebuild the gRPC status
/// message (`codespace X code N: desc: wrap`).
fn split_desc(e: &SdkError) -> (Option<String>, String) {
    // `SdkError` logs are `wrap: …: desc`; the description is the part after
    // the last ": " for wrapped errors and the whole log otherwise.
    match e.log.rfind(": ") {
        Some(i) => (Some(e.log[..i].to_owned()), e.log[i + 2..].to_owned()),
        None => (None, e.log.clone()),
    }
}

impl QueryError {
    /// `err.Error()` in Go.
    #[must_use]
    pub fn go_error_string(&self) -> String {
        match self {
            Self::Status(code, msg) => format!("rpc error: code = {} desc = {msg}", code.name()),
            Self::Sdk(e) => e.log.clone(),
            Self::Plain(msg) | Self::Decode(msg) | Self::Panic(msg) => msg.clone(),
        }
    }

    /// Whether this is a recovered panic (the ABCI response then carries
    /// no height).
    #[must_use]
    pub fn is_panic(&self) -> bool {
        matches!(self, Self::Panic(_))
    }

    /// `gRPCErrorToSDKError`: the error reported by an ABCI query.
    #[must_use]
    pub fn to_sdk(&self) -> SdkError {
        let msg = self.go_error_string();
        match self {
            Self::Status(code, _) => match code {
                Code::NotFound => errs::key_not_found(msg),
                Code::InvalidArgument | Code::FailedPrecondition => errs::invalid_request(msg),
                Code::Unauthenticated => errs::unauthorized(msg),
                _ => errs::unknown_request(msg),
            },
            // Registered errors carry gRPC code Unknown.
            Self::Sdk(_) => errs::unknown_request(msg),
            Self::Plain(_) | Self::Decode(_) => errs::invalid_request(msg),
            Self::Panic(_) => sdk::error::go_panic(format!("{msg}: panic")),
        }
    }

    /// The gRPC status (code, message) the Go gRPC server returns.
    #[must_use]
    pub fn grpc_status(&self) -> (Code, String) {
        match self {
            Self::Status(code, msg) => (*code, msg.clone()),
            Self::Sdk(e) => {
                let (wrap, desc) = split_desc(e);
                let base = format!("codespace {} code {}: {desc}", e.codespace, e.code);
                (
                    Code::Unknown,
                    match wrap {
                        Some(w) => format!("{base}: {w}"),
                        None => base,
                    },
                )
            }
            Self::Plain(msg) | Self::Decode(msg) => (Code::Unknown, msg.clone()),
            Self::Panic(msg) => (Code::Internal, msg.clone()),
        }
    }
}

impl core::fmt::Display for QueryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.go_error_string())
    }
}

impl std::error::Error for QueryError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abci_mapping() {
        let e = not_found("account x not found").to_sdk();
        assert_eq!((e.codespace, e.code), ("sdk", 22));
        assert_eq!(
            e.log,
            "rpc error: code = NotFound desc = account x not found: key not found"
        );
        let e = QueryError::Sdk(errs::invalid_address("bad")).to_sdk();
        assert_eq!(e.code, 6);
        assert_eq!(e.log, "bad: invalid address: unknown request");
        let e = QueryError::Plain("collections: not found".into()).to_sdk();
        assert_eq!(e.log, "collections: not found: invalid request");
    }

    #[test]
    fn grpc_status_of_sdk_error() {
        let (c, m) = QueryError::Sdk(errs::invalid_address("bad")).grpc_status();
        assert_eq!(c, Code::Unknown);
        assert_eq!(m, "codespace sdk code 7: invalid address: bad");
    }
}
