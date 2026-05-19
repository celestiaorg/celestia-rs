//! Error types for share helpers.

/// Result alias for this crate.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors returned by Celestia share helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A namespace did not have the fixed share namespace length.
    InvalidNamespaceLength {
        /// Observed byte length.
        actual: usize,
        /// Required byte length.
        expected: usize,
    },
    /// A namespace version is not supported by go-square v1 share namespaces.
    InvalidNamespaceVersion {
        /// Observed namespace version.
        version: u8,
    },
    /// A namespace ID did not have the fixed namespace ID length.
    InvalidNamespaceIdLength {
        /// Observed byte length.
        actual: usize,
        /// Required byte length.
        expected: usize,
    },
    /// A version-zero namespace ID did not start with the required zero prefix.
    InvalidNamespaceV0Prefix,
    /// A share did not have the fixed share length.
    InvalidShareLength {
        /// Observed byte length.
        actual: usize,
        /// Required byte length.
        expected: usize,
    },
    /// A share version exceeded the supported bit width.
    InvalidShareVersion {
        /// Observed share version.
        version: u8,
        /// Maximum supported share version.
        max: u8,
    },
    /// A share version is not present in the caller supplied supported list.
    UnsupportedShareVersion {
        /// Observed share version.
        version: u8,
        /// Supported versions.
        supported: Vec<u8>,
    },
    /// A share info byte is malformed.
    InvalidInfoByte,
    /// Compact reserved bytes had the wrong length.
    InvalidReservedBytesLength {
        /// Observed byte length.
        actual: usize,
        /// Required byte length.
        expected: usize,
    },
    /// Compact reserved bytes pointed outside a share.
    InvalidReservedByteIndex {
        /// Observed byte index.
        actual: u32,
        /// Exclusive upper bound.
        max_exclusive: u32,
    },
    /// A sequence length field is malformed.
    InvalidSequenceLen,
    /// A share range is malformed.
    InvalidShareRange,
    /// A padding share is malformed.
    InvalidPadding,
    /// A blob is malformed.
    InvalidBlob,
    /// A signer had the wrong length for a version-one blob.
    InvalidSignerLength {
        /// Observed byte length.
        actual: usize,
        /// Required byte length.
        expected: usize,
    },
    /// A compact share operation is malformed.
    InvalidCompactShare,
    /// A sparse share operation is malformed.
    InvalidSparseShare,
    /// An index-wrapper transaction is malformed.
    InvalidIndexWrapper,
    /// Decoding failed.
    Decode(String),
    /// Encoding failed.
    Encode(String),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidNamespaceLength { actual, expected } => {
                write!(f, "invalid namespace length: got {actual}, want {expected}")
            }
            Self::InvalidNamespaceVersion { version } => {
                write!(f, "invalid namespace version: {version}")
            }
            Self::InvalidNamespaceIdLength { actual, expected } => {
                write!(
                    f,
                    "invalid namespace ID length: got {actual}, want {expected}"
                )
            }
            Self::InvalidNamespaceV0Prefix => f.write_str("invalid version-zero namespace prefix"),
            Self::InvalidShareLength { actual, expected } => {
                write!(f, "invalid share length: got {actual}, want {expected}")
            }
            Self::InvalidShareVersion { version, max } => {
                write!(f, "invalid share version: got {version}, max {max}")
            }
            Self::UnsupportedShareVersion { version, supported } => {
                write!(
                    f,
                    "unsupported share version {version}; supported versions: {supported:?}"
                )
            }
            Self::InvalidInfoByte => f.write_str("invalid share info byte"),
            Self::InvalidReservedBytesLength { actual, expected } => {
                write!(
                    f,
                    "invalid reserved bytes length: got {actual}, want {expected}"
                )
            }
            Self::InvalidReservedByteIndex {
                actual,
                max_exclusive,
            } => write!(
                f,
                "invalid reserved byte index: got {actual}, want < {max_exclusive}"
            ),
            Self::InvalidSequenceLen => f.write_str("invalid share sequence length"),
            Self::InvalidShareRange => f.write_str("invalid share range"),
            Self::InvalidPadding => f.write_str("invalid share padding"),
            Self::InvalidBlob => f.write_str("invalid blob"),
            Self::InvalidSignerLength { actual, expected } => {
                write!(f, "invalid signer length: got {actual}, want {expected}")
            }
            Self::InvalidCompactShare => f.write_str("invalid compact share"),
            Self::InvalidSparseShare => f.write_str("invalid sparse share"),
            Self::InvalidIndexWrapper => f.write_str("invalid index wrapper"),
            Self::Decode(message) => write!(f, "decode share data: {message}"),
            Self::Encode(message) => write!(f, "encode share data: {message}"),
        }
    }
}

impl std::error::Error for Error {}
