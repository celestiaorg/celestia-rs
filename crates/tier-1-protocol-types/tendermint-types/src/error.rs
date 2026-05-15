//! Error types for CometBFT protocol helpers.

/// Result alias for this crate.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors returned by CometBFT protocol type helpers.
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
    /// A chain ID exceeded CometBFT's maximum length.
    InvalidChainIdLength {
        /// Observed byte length.
        actual: usize,
        /// Maximum allowed byte length.
        max: usize,
    },
    /// A height was zero or negative where CometBFT requires a positive height.
    InvalidHeight,
    /// A round was negative where CometBFT requires a non-negative round.
    InvalidRound,
    /// A block ID was malformed.
    InvalidBlockId,
    /// A part-set header was malformed.
    InvalidPartSetHeader,
    /// A commit was malformed.
    InvalidCommit,
    /// A commit signature was malformed.
    InvalidCommitSig,
    /// A validator was malformed.
    InvalidValidator,
    /// A validator set was malformed.
    InvalidValidatorSet,
    /// A vote was malformed.
    InvalidVote,
    /// A proposal was malformed.
    InvalidProposal,
    /// A public key was malformed or unsupported.
    InvalidPublicKey,
    /// A timestamp was absent or malformed.
    InvalidTimestamp,
    /// Protobuf decoding failed.
    Decode(String),
    /// Protobuf encoding failed.
    Encode(String),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidHashLength { actual } => {
                write!(f, "invalid CometBFT hash length: {actual}")
            }
            Self::InvalidAddressLength { actual } => {
                write!(f, "invalid CometBFT address length: {actual}")
            }
            Self::InvalidChainIdLength { actual, max } => {
                write!(f, "invalid CometBFT chain ID length: {actual} > {max}")
            }
            Self::InvalidHeight => f.write_str("invalid CometBFT height"),
            Self::InvalidRound => f.write_str("invalid CometBFT round"),
            Self::InvalidBlockId => f.write_str("invalid CometBFT block ID"),
            Self::InvalidPartSetHeader => f.write_str("invalid CometBFT part-set header"),
            Self::InvalidCommit => f.write_str("invalid CometBFT commit"),
            Self::InvalidCommitSig => f.write_str("invalid CometBFT commit signature"),
            Self::InvalidValidator => f.write_str("invalid CometBFT validator"),
            Self::InvalidValidatorSet => f.write_str("invalid CometBFT validator set"),
            Self::InvalidVote => f.write_str("invalid CometBFT vote"),
            Self::InvalidProposal => f.write_str("invalid CometBFT proposal"),
            Self::InvalidPublicKey => f.write_str("invalid CometBFT public key"),
            Self::InvalidTimestamp => f.write_str("invalid CometBFT timestamp"),
            Self::Decode(message) => write!(f, "decode CometBFT protobuf: {message}"),
            Self::Encode(message) => write!(f, "encode CometBFT protobuf: {message}"),
        }
    }
}

impl std::error::Error for Error {}
