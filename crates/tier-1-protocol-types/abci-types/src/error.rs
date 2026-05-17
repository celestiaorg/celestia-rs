//! Error types for CometBFT ABCI protocol helpers.

/// Result alias for this crate.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors returned by ABCI protocol type helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A hash field was neither empty nor 32 bytes.
    InvalidHashLength {
        /// Observed byte length.
        actual: usize,
    },
    /// An address field was not 20 bytes.
    InvalidAddressLength {
        /// Observed byte length.
        actual: usize,
    },
    /// A height was zero or negative where ABCI requires a positive height.
    InvalidHeight,
    /// A round was negative where ABCI requires a non-negative round.
    InvalidRound,
    /// An ABCI event was malformed.
    InvalidEvent,
    /// A transaction execution result was malformed.
    InvalidExecTxResult,
    /// A validator was malformed.
    InvalidValidator,
    /// A validator update was malformed.
    InvalidValidatorUpdate,
    /// A vote-info entry was malformed.
    InvalidVoteInfo,
    /// Commit information was malformed.
    InvalidCommitInfo,
    /// A misbehavior descriptor was malformed.
    InvalidMisbehavior,
    /// A state-sync snapshot was malformed.
    InvalidSnapshot,
    /// An ABCI request envelope was malformed.
    InvalidRequest,
    /// An ABCI response envelope was malformed.
    InvalidResponse,
    /// An enum status was unknown or unsupported for its context.
    InvalidStatus,
    /// A public key was malformed or unsupported.
    InvalidPublicKey,
    /// Protobuf decoding failed.
    Decode(String),
    /// Protobuf encoding failed.
    Encode(String),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidHashLength { actual } => {
                write!(f, "invalid ABCI hash length: {actual}")
            }
            Self::InvalidAddressLength { actual } => {
                write!(f, "invalid ABCI address length: {actual}")
            }
            Self::InvalidHeight => f.write_str("invalid ABCI height"),
            Self::InvalidRound => f.write_str("invalid ABCI round"),
            Self::InvalidEvent => f.write_str("invalid ABCI event"),
            Self::InvalidExecTxResult => f.write_str("invalid ABCI tx execution result"),
            Self::InvalidValidator => f.write_str("invalid ABCI validator"),
            Self::InvalidValidatorUpdate => f.write_str("invalid ABCI validator update"),
            Self::InvalidVoteInfo => f.write_str("invalid ABCI vote info"),
            Self::InvalidCommitInfo => f.write_str("invalid ABCI commit info"),
            Self::InvalidMisbehavior => f.write_str("invalid ABCI misbehavior"),
            Self::InvalidSnapshot => f.write_str("invalid ABCI snapshot"),
            Self::InvalidRequest => f.write_str("invalid ABCI request"),
            Self::InvalidResponse => f.write_str("invalid ABCI response"),
            Self::InvalidStatus => f.write_str("invalid ABCI status"),
            Self::InvalidPublicKey => f.write_str("invalid ABCI public key"),
            Self::Decode(message) => write!(f, "decode ABCI protobuf: {message}"),
            Self::Encode(message) => write!(f, "encode ABCI protobuf: {message}"),
        }
    }
}

impl std::error::Error for Error {}
