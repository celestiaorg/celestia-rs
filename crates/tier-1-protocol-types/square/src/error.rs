//! Error types for data-square helpers.

/// Result alias for this crate.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors returned by Celestia data-square helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A square had an invalid share count or side length.
    InvalidSquareSize {
        /// Observed value.
        actual: usize,
    },
    /// A square-size input was not a power of two.
    NotPowerOfTwo {
        /// Observed value.
        actual: usize,
    },
    /// A multiple or threshold argument was zero.
    ZeroDivisor,
    /// Arithmetic overflowed while calculating layout indexes.
    ArithmeticOverflow,
    /// A share index does not fit in Go's `uint32` fixture/API shape.
    ShareIndexOverflow {
        /// Observed index.
        actual: usize,
    },
    /// A blob transaction wrapper is malformed.
    InvalidBlobTx,
    /// A builder operation referenced a transaction index that is not present.
    TxIndexOutOfRange {
        /// Observed transaction index.
        index: usize,
    },
    /// A builder operation referenced a blob index that is not present.
    BlobIndexOutOfRange {
        /// Observed blob index.
        index: usize,
    },
    /// A requested transaction index does not identify a wrapped PFB.
    NotPayForBlob {
        /// Observed transaction index.
        index: usize,
    },
    /// A normal transaction appeared after a blob transaction in exact mode.
    NormalTxAfterBlobTx {
        /// Observed transaction index.
        index: usize,
    },
    /// Builder accounting invariants were violated.
    BuilderInvariant(&'static str),
    /// Share primitive operation failed.
    Share(shares::Error),
    /// Protobuf decoding failed.
    Decode(String),
    /// Protobuf encoding failed.
    Encode(String),
}

impl From<shares::Error> for Error {
    fn from(error: shares::Error) -> Self {
        Self::Share(error)
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidSquareSize { actual } => {
                write!(f, "invalid square size or share count: {actual}")
            }
            Self::NotPowerOfTwo { actual } => {
                write!(f, "value is not a power of two: {actual}")
            }
            Self::ZeroDivisor => f.write_str("divisor must be non-zero"),
            Self::ArithmeticOverflow => f.write_str("arithmetic overflow"),
            Self::ShareIndexOverflow { actual } => {
                write!(f, "share index overflows uint32: {actual}")
            }
            Self::InvalidBlobTx => f.write_str("invalid blob transaction wrapper"),
            Self::TxIndexOutOfRange { index } => {
                write!(f, "transaction index out of range: {index}")
            }
            Self::BlobIndexOutOfRange { index } => {
                write!(f, "blob index out of range: {index}")
            }
            Self::NotPayForBlob { index } => {
                write!(f, "transaction index does not identify a PFB: {index}")
            }
            Self::NormalTxAfterBlobTx { index } => {
                write!(
                    f,
                    "normal transaction after blob transaction at index {index}"
                )
            }
            Self::BuilderInvariant(message) => write!(f, "builder invariant failed: {message}"),
            Self::Share(error) => write!(f, "share error: {error}"),
            Self::Decode(message) => write!(f, "decode square data: {message}"),
            Self::Encode(message) => write!(f, "encode square data: {message}"),
        }
    }
}

impl std::error::Error for Error {}
