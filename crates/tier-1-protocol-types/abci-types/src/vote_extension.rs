//! Commit and vote-extension types.

use crate::validation::validate_non_negative_round;
use crate::{Error, Result, Validator, proto};
use tendermint_types::BlockIdFlag;

const MAX_EXTENSION_SIGNATURE_SIZE: usize = tendermint_types::commit::MAX_SIGNATURE_SIZE;

/// Validator vote information for the previous commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoteInfo {
    /// Validator that cast the vote.
    pub validator: Validator,
    /// Block ID flag.
    pub block_id_flag: BlockIdFlag,
}

impl VoteInfo {
    /// Perform stateless vote-info validation.
    pub fn validate_basic(&self) -> Result<()> {
        self.validator.validate_basic()
    }

    /// Convert protobuf into a domain vote-info entry.
    pub fn from_proto(proto: proto::abci::VoteInfo) -> Result<Self> {
        let vote_info = Self {
            validator: proto
                .validator
                .map(Validator::from_proto)
                .transpose()?
                .ok_or(Error::InvalidVoteInfo)?,
            block_id_flag: BlockIdFlag::try_from(proto.block_id_flag)
                .map_err(|_| Error::InvalidVoteInfo)?,
        };
        vote_info.validate_basic()?;
        Ok(vote_info)
    }

    /// Convert this vote-info entry to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::abci::VoteInfo {
        proto::abci::VoteInfo {
            validator: Some(self.validator.to_proto()),
            block_id_flag: self.block_id_flag.into(),
        }
    }
}

/// Validator vote information that includes vote-extension bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtendedVoteInfo {
    /// Validator that cast the vote.
    pub validator: Validator,
    /// Application-provided vote extension bytes.
    pub vote_extension: Vec<u8>,
    /// CometBFT vote-extension signature bytes.
    pub extension_signature: Vec<u8>,
    /// Block ID flag.
    pub block_id_flag: BlockIdFlag,
}

impl ExtendedVoteInfo {
    /// Perform stateless extended vote-info validation.
    pub fn validate_basic(&self) -> Result<()> {
        self.validator.validate_basic()?;
        if self.extension_signature.len() > MAX_EXTENSION_SIGNATURE_SIZE {
            return Err(Error::InvalidVoteInfo);
        }
        Ok(())
    }

    /// Convert protobuf into a domain extended vote-info entry.
    pub fn from_proto(proto: proto::abci::ExtendedVoteInfo) -> Result<Self> {
        let vote_info = Self {
            validator: proto
                .validator
                .map(Validator::from_proto)
                .transpose()?
                .ok_or(Error::InvalidVoteInfo)?,
            vote_extension: proto.vote_extension.to_vec(),
            extension_signature: proto.extension_signature.to_vec(),
            block_id_flag: BlockIdFlag::try_from(proto.block_id_flag)
                .map_err(|_| Error::InvalidVoteInfo)?,
        };
        vote_info.validate_basic()?;
        Ok(vote_info)
    }

    /// Convert this extended vote-info entry to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::abci::ExtendedVoteInfo {
        proto::abci::ExtendedVoteInfo {
            validator: Some(self.validator.to_proto()),
            vote_extension: self.vote_extension.clone().into(),
            extension_signature: self.extension_signature.clone().into(),
            block_id_flag: self.block_id_flag.into(),
        }
    }
}

/// Previous-commit information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    /// Commit round.
    pub round: i32,
    /// Ordered validator vote information.
    pub votes: Vec<VoteInfo>,
}

impl CommitInfo {
    /// Perform stateless commit-info validation.
    pub fn validate_basic(&self) -> Result<()> {
        validate_non_negative_round(self.round)?;
        for vote in &self.votes {
            vote.validate_basic()?;
        }
        Ok(())
    }

    /// Convert protobuf into domain commit information.
    pub fn from_proto(proto: proto::abci::CommitInfo) -> Result<Self> {
        let commit_info = Self {
            round: proto.round,
            votes: proto
                .votes
                .into_iter()
                .map(VoteInfo::from_proto)
                .collect::<Result<Vec<_>>>()?,
        };
        commit_info.validate_basic()?;
        Ok(commit_info)
    }

    /// Convert this commit information to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::abci::CommitInfo {
        proto::abci::CommitInfo {
            round: self.round,
            votes: self.votes.iter().map(VoteInfo::to_proto).collect(),
        }
    }
}

/// Previous-commit information with vote extensions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtendedCommitInfo {
    /// Commit round.
    pub round: i32,
    /// Ordered validator vote information with vote extensions.
    pub votes: Vec<ExtendedVoteInfo>,
}

impl ExtendedCommitInfo {
    /// Perform stateless extended commit-info validation.
    pub fn validate_basic(&self) -> Result<()> {
        validate_non_negative_round(self.round)?;
        for vote in &self.votes {
            vote.validate_basic()?;
        }
        Ok(())
    }

    /// Convert protobuf into domain extended commit information.
    pub fn from_proto(proto: proto::abci::ExtendedCommitInfo) -> Result<Self> {
        let commit_info = Self {
            round: proto.round,
            votes: proto
                .votes
                .into_iter()
                .map(ExtendedVoteInfo::from_proto)
                .collect::<Result<Vec<_>>>()?,
        };
        commit_info.validate_basic()?;
        Ok(commit_info)
    }

    /// Convert this extended commit information to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::abci::ExtendedCommitInfo {
        proto::abci::ExtendedCommitInfo {
            round: self.round,
            votes: self.votes.iter().map(ExtendedVoteInfo::to_proto).collect(),
        }
    }
}
