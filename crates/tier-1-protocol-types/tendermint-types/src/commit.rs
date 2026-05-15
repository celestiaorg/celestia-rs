//! Commit and commit-signature types.

use crate::block::BlockId;
use crate::error::{Error, Result};
use crate::hash::{Address, address_from_bytes};
use crate::proto;
use crate::vote::{SignedMsgType, Vote};
use prost::Message;

/// Maximum signature size accepted by CometBFT for consensus messages.
pub const MAX_SIGNATURE_SIZE: usize = 64;

/// CometBFT block ID flag used inside commit signatures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BlockIdFlag {
    /// No vote was received.
    Absent = 1,
    /// Validator voted for the commit block ID.
    Commit = 2,
    /// Validator voted for nil.
    Nil = 3,
}

impl TryFrom<i32> for BlockIdFlag {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self> {
        match value {
            1 => Ok(Self::Absent),
            2 => Ok(Self::Commit),
            3 => Ok(Self::Nil),
            _ => Err(Error::InvalidCommitSig),
        }
    }
}

impl From<BlockIdFlag> for i32 {
    fn from(value: BlockIdFlag) -> Self {
        value as Self
    }
}

/// A signature from one validator inside a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitSig {
    /// Block ID flag.
    pub block_id_flag: BlockIdFlag,
    /// Validator address, absent only for `BlockIdFlag::Absent`.
    pub validator_address: Option<Address>,
    /// Commit signature timestamp.
    pub timestamp: Option<proto::google::Timestamp>,
    /// Raw signature bytes.
    pub signature: Vec<u8>,
}

impl CommitSig {
    /// Return an absent commit signature.
    #[must_use]
    pub fn absent() -> Self {
        Self {
            block_id_flag: BlockIdFlag::Absent,
            validator_address: None,
            timestamp: None,
            signature: Vec::new(),
        }
    }

    /// Return the block ID signed by this commit signature.
    pub fn block_id(&self, commit_block_id: &BlockId) -> Result<BlockId> {
        match self.block_id_flag {
            BlockIdFlag::Absent | BlockIdFlag::Nil => Ok(BlockId::default()),
            BlockIdFlag::Commit => Ok(commit_block_id.clone()),
        }
    }

    /// Perform CometBFT stateless commit-signature validation.
    pub fn validate_basic(&self) -> Result<()> {
        match self.block_id_flag {
            BlockIdFlag::Absent => {
                if self.validator_address.is_some()
                    || self.timestamp.is_some()
                    || !self.signature.is_empty()
                {
                    return Err(Error::InvalidCommitSig);
                }
            }
            BlockIdFlag::Commit | BlockIdFlag::Nil => {
                if self.validator_address.is_none()
                    || self.timestamp.is_none()
                    || self.signature.is_empty()
                    || self.signature.len() > MAX_SIGNATURE_SIZE
                {
                    return Err(Error::InvalidCommitSig);
                }
            }
        }
        Ok(())
    }

    /// Convert protobuf into a domain commit signature.
    pub fn from_proto(proto: proto::types::CommitSig) -> Result<Self> {
        let flag = BlockIdFlag::try_from(proto.block_id_flag)?;
        let validator_address = if proto.validator_address.is_empty() {
            None
        } else {
            Some(address_from_bytes(&proto.validator_address)?)
        };
        let commit_sig = Self {
            block_id_flag: flag,
            validator_address,
            timestamp: proto.timestamp,
            signature: proto.signature,
        };
        commit_sig.validate_basic()?;
        Ok(commit_sig)
    }

    /// Convert this commit signature to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::types::CommitSig {
        proto::types::CommitSig {
            block_id_flag: self.block_id_flag.into(),
            validator_address: self
                .validator_address
                .as_ref()
                .map_or_else(Vec::new, |address| address.to_vec()),
            timestamp: self.timestamp,
            signature: self.signature.clone(),
        }
    }
}

/// CometBFT commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// Commit height.
    pub height: i64,
    /// Commit round.
    pub round: i32,
    /// Committed block ID.
    pub block_id: BlockId,
    /// Commit signatures.
    pub signatures: Vec<CommitSig>,
}

impl Commit {
    /// Perform CometBFT stateless commit validation.
    pub fn validate_basic(&self) -> Result<()> {
        if self.height < 0 {
            return Err(Error::InvalidHeight);
        }
        if self.round < 0 {
            return Err(Error::InvalidRound);
        }
        if self.height >= 1 {
            if self.block_id.is_zero() || self.signatures.is_empty() {
                return Err(Error::InvalidCommit);
            }
            for signature in &self.signatures {
                signature.validate_basic()?;
            }
        }
        Ok(())
    }

    /// Return CometBFT's commit hash.
    #[must_use]
    pub fn hash(&self) -> crate::Hash {
        let encoded = self
            .signatures
            .iter()
            .map(|signature| signature.to_proto().encode_to_vec())
            .collect::<Vec<_>>();
        merkle::root_from_byte_slices(&encoded)
    }

    /// Return the number of commit signatures.
    #[must_use]
    pub fn size(&self) -> usize {
        self.signatures.len()
    }

    /// Convert a commit signature at `validator_index` into a precommit vote.
    pub fn get_vote(&self, validator_index: i32) -> Result<Option<Vote>> {
        let signature = self
            .signatures
            .get(usize::try_from(validator_index).map_err(|_| Error::InvalidVote)?)
            .ok_or(Error::InvalidVote)?;
        if signature.block_id_flag == BlockIdFlag::Absent {
            return Ok(None);
        }
        Ok(Some(Vote {
            ty: SignedMsgType::Precommit,
            height: self.height,
            round: self.round,
            block_id: signature.block_id(&self.block_id)?,
            timestamp: signature.timestamp,
            validator_address: signature.validator_address.ok_or(Error::InvalidCommitSig)?,
            validator_index,
            signature: signature.signature.clone(),
            extension: Vec::new(),
            extension_signature: Vec::new(),
        }))
    }

    /// Return canonical vote sign bytes for the signature at `validator_index`.
    pub fn vote_sign_bytes(&self, chain_id: &str, validator_index: i32) -> Result<Vec<u8>> {
        self.get_vote(validator_index)?
            .ok_or(Error::InvalidVote)?
            .sign_bytes(chain_id)
    }

    /// Convert protobuf into a domain commit.
    pub fn from_proto(proto: proto::types::Commit) -> Result<Self> {
        let commit = Self {
            height: proto.height,
            round: proto.round,
            block_id: proto
                .block_id
                .map(BlockId::from_proto)
                .transpose()?
                .unwrap_or_default(),
            signatures: proto
                .signatures
                .into_iter()
                .map(CommitSig::from_proto)
                .collect::<Result<Vec<_>>>()?,
        };
        commit.validate_basic()?;
        Ok(commit)
    }

    /// Convert this commit to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::types::Commit {
        proto::types::Commit {
            height: self.height,
            round: self.round,
            block_id: Some(self.block_id.to_proto()),
            signatures: self.signatures.iter().map(CommitSig::to_proto).collect(),
        }
    }
}
