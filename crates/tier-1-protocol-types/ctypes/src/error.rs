//! Errors. `Display` reproduces the Go error string exactly (including the
//! wrapping prefixes added by `fmt.Errorf`), because these strings surface to
//! peers and RPC clients.

use core::fmt;

/// A celestia-core `types` error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// `ErrNotEnoughVotingPowerSigned`.
    NotEnoughVotingPowerSigned {
        /// Tallied voting power.
        got: i64,
        /// Voting power that had to be exceeded.
        needed: i64,
    },
    /// `ErrInvalidCommitHeight`.
    InvalidCommitHeight {
        /// Expected height.
        expected: i64,
        /// Commit height.
        actual: i64,
    },
    /// `ErrInvalidCommitSignatures`.
    InvalidCommitSignatures {
        /// Validator set size.
        expected: usize,
        /// Number of commit signatures.
        actual: usize,
    },
    /// `ErrTotalVotingPowerOverflow`.
    TotalVotingPowerOverflow,
    /// `ErrProposerNotInVals`.
    ProposerNotInVals,
    /// `ErrPartSetUnexpectedIndex`.
    PartSetUnexpectedIndex,
    /// `ErrPartTooBig`.
    PartTooBig,
    /// `ErrPartInvalidSize`.
    PartInvalidSize,
    /// `ErrVoteInvalidValidatorAddress`.
    VoteInvalidValidatorAddress,
    /// `ErrVoteInvalidSignature`.
    VoteInvalidSignature,
    /// `ErrVoteNil`.
    VoteNil,
    /// `ErrVoteExtensionAbsent`.
    VoteExtensionAbsent,
    /// Any other error, carrying the exact Go message.
    Msg(String),
}

impl Error {
    /// An error with the given Go message.
    #[must_use]
    pub fn msg(s: impl Into<String>) -> Self {
        Self::Msg(s.into())
    }

    /// Wraps `self` the way `fmt.Errorf("<prefix>%v", err)` / `%w` does.
    #[must_use]
    pub fn wrap(self, prefix: &str) -> Self {
        Self::Msg(format!("{prefix}{self}"))
    }

    /// True for `ErrNotEnoughVotingPowerSigned` (Go `IsErrNotEnoughVotingPowerSigned`).
    #[must_use]
    pub fn is_not_enough_voting_power_signed(&self) -> bool {
        matches!(self, Self::NotEnoughVotingPowerSigned { .. })
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotEnoughVotingPowerSigned { got, needed } => write!(
                f,
                "invalid commit -- insufficient voting power: got {got}, needed more than {needed}"
            ),
            Self::InvalidCommitHeight { expected, actual } => {
                write!(f, "Invalid commit -- wrong height: {expected} vs {actual}")
            }
            Self::InvalidCommitSignatures { expected, actual } => write!(
                f,
                "Invalid commit -- wrong set size: {expected} vs {actual}"
            ),
            Self::TotalVotingPowerOverflow => write!(
                f,
                "total voting power of resulting valset exceeds max {}",
                crate::validator_set::MAX_TOTAL_VOTING_POWER
            ),
            Self::ProposerNotInVals => f.write_str("proposer not in validator set"),
            Self::PartSetUnexpectedIndex => f.write_str("error part set unexpected index"),
            Self::PartTooBig => f.write_str("error part size too big"),
            Self::PartInvalidSize => f.write_str("error inner part with invalid size"),
            Self::VoteInvalidValidatorAddress => f.write_str("invalid validator address"),
            Self::VoteInvalidSignature => f.write_str("invalid signature"),
            Self::VoteNil => f.write_str("nil vote"),
            Self::VoteExtensionAbsent => f.write_str("vote extension absent"),
            Self::Msg(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for Error {}

/// Result alias.
pub type Result<T, E = Error> = core::result::Result<T, E>;
