//! Error types for ICS-23 proof helpers.

/// Result alias for this crate.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors returned by ICS-23 proof operation helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A commitment operation used an empty key.
    EmptyKey,
    /// A proof operation type was not one of the SDK ICS-23 operations.
    InvalidProofOpType(String),
    /// A proof was malformed or did not verify.
    InvalidProof(&'static str),
    /// Protobuf bytes could not be decoded as the expected proof type.
    MalformedProto(&'static str),
    /// The proof kind is recognized but unsupported by the requested operation.
    UnsupportedProofKind(&'static str),
    /// Error forwarded from the local IAVL crate.
    Iavl(iavl::Error),
}

impl From<iavl::Error> for Error {
    fn from(value: iavl::Error) -> Self {
        Self::Iavl(value)
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EmptyKey => f.write_str("empty key"),
            Self::InvalidProofOpType(field_type) => {
                write!(f, "invalid proof operation type: {field_type}")
            }
            Self::InvalidProof(message) => write!(f, "invalid proof: {message}"),
            Self::MalformedProto(message) => write!(f, "malformed protobuf: {message}"),
            Self::UnsupportedProofKind(message) => {
                write!(f, "unsupported proof kind: {message}")
            }
            Self::Iavl(err) => write!(f, "iavl: {err}"),
        }
    }
}

impl std::error::Error for Error {}
