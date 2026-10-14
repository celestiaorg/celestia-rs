//! Light client errors (`light/errors.go`, `light/provider/errors.go`,
//! `light/store/errors.go`). `Display` reproduces Go's error strings.

use core::fmt;

use ctypes::gofmt::hex_upper;
use ctypes::{LightBlock, Time};

use crate::lightblock_hash;

/// Errors a [`crate::provider::Provider`] returns (`light/provider`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// `ErrHeightTooHigh`: the provider does not have the height yet.
    HeightTooHigh,
    /// `ErrLightBlockNotFound` (also `NewNotFound()`, whose Go message
    /// carries a stack trace in front of this text).
    LightBlockNotFound,
    /// `ErrNoResponse`.
    NoResponse,
    /// `ErrBadLightBlock{Reason}`.
    BadLightBlock(String),
    /// `context.Canceled`.
    Canceled,
    /// `context.DeadlineExceeded`.
    DeadlineExceeded,
    /// Any other (malevolent) error, e.g. a connection failure.
    Other(String),
}

impl ProviderError {
    /// Errors the light client treats as benign (`ErrNoResponse`,
    /// `ErrLightBlockNotFound`, `ErrHeightTooHigh`).
    #[must_use]
    pub fn is_benign(&self) -> bool {
        matches!(
            self,
            Self::NoResponse | Self::LightBlockNotFound | Self::HeightTooHigh
        )
    }

    /// `context.Canceled` / `context.DeadlineExceeded`.
    #[must_use]
    pub fn is_context(&self) -> bool {
        matches!(self, Self::Canceled | Self::DeadlineExceeded)
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HeightTooHigh => f.write_str("height requested is too high"),
            Self::LightBlockNotFound => f.write_str("light block not found"),
            Self::NoResponse => f.write_str("client failed to respond"),
            Self::BadLightBlock(r) => write!(f, "client provided bad signed header: {r}"),
            Self::Canceled => f.write_str("context canceled"),
            Self::DeadlineExceeded => f.write_str("context deadline exceeded"),
            Self::Other(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for ProviderError {}

/// Light store errors (`light/store`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// `store.ErrLightBlockNotFound`.
    LightBlockNotFound,
    /// Backend / encoding failure (Go message).
    Msg(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LightBlockNotFound => f.write_str("light block not found"),
            Self::Msg(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for StoreError {}

/// Light client errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// `ErrOldHeaderExpired`.
    OldHeaderExpired {
        /// Expiration time.
        at: Time,
        /// Current time.
        now: Time,
    },
    /// `ErrNewValSetCantBeTrusted` (reason is `ErrNotEnoughVotingPowerSigned`).
    NewValSetCantBeTrusted(ctypes::Error),
    /// `ErrInvalidHeader`.
    InvalidHeader(Box<Error>),
    /// `ErrVerificationFailed`.
    VerificationFailed {
        /// Trusted height.
        from: i64,
        /// Height that failed.
        to: i64,
        /// Cause.
        reason: Box<Error>,
    },
    /// `ErrFailedHeaderCrossReferencing`.
    FailedHeaderCrossReferencing,
    /// `ErrLightClientAttack`.
    LightClientAttack,
    /// `ErrNoWitnesses`.
    NoWitnesses,
    /// `ErrConflictingHeaders`.
    ConflictingHeaders {
        /// The witness's block.
        block: Box<LightBlock>,
        /// Witness index.
        witness_index: usize,
    },
    /// `ErrProposerPrioritiesDiverge`.
    ProposerPrioritiesDiverge {
        /// Witness proposer-priority hash.
        witness_hash: Vec<u8>,
        /// Witness index.
        witness_index: usize,
        /// Primary proposer-priority hash.
        primary_hash: Vec<u8>,
    },
    /// `errBadWitness`.
    BadWitness {
        /// Cause.
        reason: Box<Error>,
        /// Witness index.
        witness_index: usize,
    },
    /// `errNoDivergence`.
    NoDivergence,
    /// A `types` error (commit verification, validation).
    Types(ctypes::Error),
    /// A provider error.
    Provider(ProviderError),
    /// A store error.
    Store(StoreError),
    /// Any other error, with Go's message.
    Msg(String),
}

impl Error {
    /// An error with the given message.
    #[must_use]
    pub fn msg(s: impl Into<String>) -> Self {
        Self::Msg(s.into())
    }

    /// `fmt.Errorf("<prefix>%w", self)`, keeping the cause inspectable for
    /// `errors.Is`-style checks via [`Error::root`].
    #[must_use]
    pub fn wrap(self, prefix: impl Into<String>) -> Self {
        Self::Msg(format!("{}{self}", prefix.into()))
    }

    /// `errors.Unwrap` for `ErrVerificationFailed` (Go's `Unwrap` returns
    /// the reason); other errors unwrap to themselves here.
    #[must_use]
    pub fn unwrap_reason(&self) -> &Self {
        match self {
            Self::VerificationFailed { reason, .. } => reason,
            other => other,
        }
    }

    /// `errors.Is(err, provider.ErrLightBlockNotFound)` on an unwrapped error.
    #[must_use]
    pub fn is_light_block_not_found(&self) -> bool {
        matches!(
            self,
            Self::Provider(ProviderError::LightBlockNotFound)
                | Self::Store(StoreError::LightBlockNotFound)
        )
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OldHeaderExpired { at, now } => write!(
                f,
                "old header has expired at {} (now: {})",
                at.go_string(),
                now.go_string()
            ),
            Self::NewValSetCantBeTrusted(r) => write!(f, "cant trust new val set: {r}"),
            Self::InvalidHeader(r) => write!(f, "invalid header: {r}"),
            Self::VerificationFailed { from, to, reason } => {
                write!(f, "verify from #{from} to #{to} failed: {reason}")
            }
            Self::FailedHeaderCrossReferencing => f.write_str(
                "all witnesses have either not responded, don't have the  blocks or sent invalid blocks. You should look to change your witnesses  or review the light client's logs for more information",
            ),
            Self::LightClientAttack => f.write_str(
                "attempted attack detected.\n\tLight client received valid conflicting header from witness.\n\tUnable to verify header. Evidence has been sent to both providers.\n\tCheck logs for full evidence and trace",
            ),
            Self::NoWitnesses => f.write_str("no witnesses connected. please reset light client"),
            Self::ConflictingHeaders {
                block,
                witness_index,
            } => write!(
                f,
                "header hash ({}) from witness ({witness_index}) does not match primary",
                hex_upper(&lightblock_hash(block))
            ),
            Self::ProposerPrioritiesDiverge {
                witness_hash,
                witness_index,
                primary_hash,
            } => write!(
                f,
                "validator set's proposer priority hashes do not match: witness[{witness_index}]={}, primary={}",
                hex_upper(witness_hash),
                hex_upper(primary_hash)
            ),
            Self::BadWitness {
                reason,
                witness_index,
            } => write!(f, "Witness {witness_index} returned error: {reason}"),
            Self::NoDivergence => f.write_str(
                "sanity check failed: no divergence between the original trace and the provider's new trace",
            ),
            Self::Types(e) => write!(f, "{e}"),
            Self::Provider(e) => write!(f, "{e}"),
            Self::Store(e) => write!(f, "{e}"),
            Self::Msg(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for Error {}

impl From<ctypes::Error> for Error {
    fn from(e: ctypes::Error) -> Self {
        Self::Types(e)
    }
}

impl From<ProviderError> for Error {
    fn from(e: ProviderError) -> Self {
        Self::Provider(e)
    }
}

impl From<StoreError> for Error {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

/// Result alias.
pub type Result<T, E = Error> = core::result::Result<T, E>;
