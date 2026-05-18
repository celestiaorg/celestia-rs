//! Error types for namespaced Merkle tree helpers.

/// Result alias for this crate.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors returned by namespaced Merkle tree helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A namespace ID did not have the fixed Celestia M1 namespace length.
    InvalidNamespaceLength {
        /// Observed byte length.
        actual: usize,
    },
    /// A leaf did not include the required namespace prefix.
    InvalidLeafLength {
        /// Observed byte length.
        actual: usize,
        /// Required minimum byte length.
        min: usize,
    },
    /// A namespaced hash did not have the expected encoded length.
    InvalidNodeLength {
        /// Observed byte length.
        actual: usize,
        /// Required byte length.
        expected: usize,
    },
    /// A namespaced hash had `max_namespace < min_namespace`.
    InvalidNodeNamespaceOrder,
    /// Left and right child namespace ranges were not ordered.
    UnorderedSiblings,
    /// Leaves were not pushed in non-decreasing namespace order.
    UnorderedLeaves,
    /// A requested leaf range was invalid.
    InvalidRange,
    /// A proof was malformed.
    InvalidProof,
    /// A namespace proof failed the completeness check.
    FailedCompletenessCheck,
    /// Subroot path inputs were invalid.
    InvalidSubrootPath,
    /// Decoding failed.
    Decode(String),
    /// Encoding failed.
    Encode(String),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidNamespaceLength { actual } => {
                write!(f, "invalid namespace length: {actual}")
            }
            Self::InvalidLeafLength { actual, min } => {
                write!(f, "invalid NMT leaf length: got {actual}, want >= {min}")
            }
            Self::InvalidNodeLength { actual, expected } => {
                write!(f, "invalid NMT node length: got {actual}, want {expected}")
            }
            Self::InvalidNodeNamespaceOrder => f.write_str("invalid NMT node namespace order"),
            Self::UnorderedSiblings => {
                f.write_str("NMT sibling nodes are not ordered by namespace")
            }
            Self::UnorderedLeaves => f.write_str("NMT leaves are not ordered by namespace"),
            Self::InvalidRange => f.write_str("invalid NMT proof range"),
            Self::InvalidProof => f.write_str("invalid NMT proof"),
            Self::FailedCompletenessCheck => f.write_str("failed NMT completeness check"),
            Self::InvalidSubrootPath => f.write_str("invalid NMT subroot path input"),
            Self::Decode(message) => write!(f, "decode NMT data: {message}"),
            Self::Encode(message) => write!(f, "encode NMT data: {message}"),
        }
    }
}

impl std::error::Error for Error {}
