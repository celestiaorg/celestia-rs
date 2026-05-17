//! Error types for Cosmos SDK protocol helpers.

/// Result alias for this crate.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors returned by Cosmos SDK protocol type helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// An address field was not 20 bytes.
    InvalidAddressLength {
        /// Observed byte length.
        actual: usize,
    },
    /// A bech32 address was malformed.
    InvalidAddress,
    /// A bech32 address used the wrong human-readable prefix.
    InvalidAddressPrefix {
        /// Expected human-readable prefix.
        expected: &'static str,
        /// Actual decoded human-readable prefix.
        actual: String,
    },
    /// A protobuf Any was malformed.
    InvalidAny,
    /// A coin was malformed.
    InvalidCoin,
    /// A coin denomination was malformed.
    InvalidDenom,
    /// A coin amount was malformed.
    InvalidAmount,
    /// A fee was malformed.
    InvalidFee,
    /// Mode info was malformed.
    InvalidModeInfo,
    /// A sign mode was malformed or unsupported.
    InvalidSignMode,
    /// Signer info was malformed.
    InvalidSignerInfo,
    /// A sign doc was malformed.
    InvalidSignDoc,
    /// A transaction was malformed.
    InvalidTx,
    /// A public key was malformed.
    InvalidPublicKey,
    /// Protobuf decoding failed.
    Decode(String),
    /// Protobuf encoding failed.
    Encode(String),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidAddressLength { actual } => {
                write!(f, "invalid Cosmos address length: {actual}")
            }
            Self::InvalidAddress => f.write_str("invalid Cosmos address"),
            Self::InvalidAddressPrefix { expected, actual } => {
                write!(
                    f,
                    "invalid Cosmos address prefix: expected {expected}, got {actual}"
                )
            }
            Self::InvalidAny => f.write_str("invalid Cosmos Any"),
            Self::InvalidCoin => f.write_str("invalid Cosmos coin"),
            Self::InvalidDenom => f.write_str("invalid Cosmos denom"),
            Self::InvalidAmount => f.write_str("invalid Cosmos amount"),
            Self::InvalidFee => f.write_str("invalid Cosmos fee"),
            Self::InvalidModeInfo => f.write_str("invalid Cosmos mode info"),
            Self::InvalidSignMode => f.write_str("invalid Cosmos sign mode"),
            Self::InvalidSignerInfo => f.write_str("invalid Cosmos signer info"),
            Self::InvalidSignDoc => f.write_str("invalid Cosmos sign doc"),
            Self::InvalidTx => f.write_str("invalid Cosmos tx"),
            Self::InvalidPublicKey => f.write_str("invalid Cosmos public key"),
            Self::Decode(message) => write!(f, "decode Cosmos protobuf: {message}"),
            Self::Encode(message) => write!(f, "encode Cosmos protobuf: {message}"),
        }
    }
}

impl std::error::Error for Error {}
