//! Error types for IAVL helpers.

/// Result alias for this crate.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors returned by IAVL tree helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A mutation or proof request used an empty key.
    EmptyKey,
    /// A version value was invalid.
    InvalidVersion {
        /// Observed version.
        version: i64,
    },
    /// A range had incompatible bounds.
    InvalidRange,
    /// A proof was malformed or did not verify.
    InvalidProof(&'static str),
    /// An internal node shape is impossible for a valid IAVL tree.
    MalformedNode(&'static str),
    /// A committed version is not available (never written or pruned).
    VersionNotFound {
        /// Requested version.
        version: i64,
    },
    /// A persisted node referenced by the tree is missing from storage.
    MissingNode {
        /// Node key version.
        version: i64,
        /// Node key nonce.
        nonce: u32,
    },
    /// Persisted data could not be decoded.
    Corrupt(&'static str),
    /// Storage backend error.
    Db(String),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EmptyKey => f.write_str("empty key"),
            Self::InvalidVersion { version } => write!(f, "invalid version {version}"),
            Self::InvalidRange => f.write_str("invalid range"),
            Self::InvalidProof(message) => write!(f, "invalid proof: {message}"),
            Self::MalformedNode(message) => write!(f, "malformed node: {message}"),
            Self::VersionNotFound { version } => write!(f, "version {version} not found"),
            Self::MissingNode { version, nonce } => {
                write!(f, "missing node ({version}, {nonce})")
            }
            Self::Corrupt(message) => write!(f, "corrupt storage: {message}"),
            Self::Db(message) => write!(f, "storage: {message}"),
        }
    }
}

impl std::error::Error for Error {}
