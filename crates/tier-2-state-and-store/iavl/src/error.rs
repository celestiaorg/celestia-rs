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
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EmptyKey => f.write_str("empty key"),
            Self::InvalidVersion { version } => write!(f, "invalid version {version}"),
            Self::InvalidRange => f.write_str("invalid range"),
            Self::InvalidProof(message) => write!(f, "invalid proof: {message}"),
            Self::MalformedNode(message) => write!(f, "malformed node: {message}"),
        }
    }
}

impl std::error::Error for Error {}
