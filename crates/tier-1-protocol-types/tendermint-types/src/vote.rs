//! CometBFT vote types.

use crate::block::BlockId;
use crate::commit::{BlockIdFlag, CommitSig, MAX_SIGNATURE_SIZE};
use crate::error::{Error, Result};
use crate::hash::{Address, address_from_bytes};
use crate::proto;
use crate::validator::PublicKey;

/// CometBFT signed message type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum SignedMsgType {
    /// Unknown signed message type.
    Unknown = 0,
    /// Prevote.
    Prevote = 1,
    /// Precommit.
    Precommit = 2,
    /// Proposal.
    Proposal = 32,
}

impl TryFrom<i32> for SignedMsgType {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self> {
        match value {
            0 => Ok(Self::Unknown),
            1 => Ok(Self::Prevote),
            2 => Ok(Self::Precommit),
            32 => Ok(Self::Proposal),
            _ => Err(Error::InvalidVote),
        }
    }
}

impl From<SignedMsgType> for i32 {
    fn from(value: SignedMsgType) -> Self {
        value as Self
    }
}

/// A CometBFT vote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vote {
    /// Signed message type.
    pub ty: SignedMsgType,
    /// Vote height.
    pub height: i64,
    /// Vote round.
    pub round: i32,
    /// Voted block ID.
    pub block_id: BlockId,
    /// Vote timestamp.
    pub timestamp: Option<proto::google::Timestamp>,
    /// Validator address.
    pub validator_address: Address,
    /// Validator index.
    pub validator_index: i32,
    /// Vote signature.
    pub signature: Vec<u8>,
    /// Vote extension.
    pub extension: Vec<u8>,
    /// Vote extension signature.
    pub extension_signature: Vec<u8>,
}

impl Vote {
    /// Perform CometBFT stateless vote validation.
    pub fn validate_basic(&self) -> Result<()> {
        if !matches!(self.ty, SignedMsgType::Prevote | SignedMsgType::Precommit) {
            return Err(Error::InvalidVote);
        }
        if self.height <= 0 {
            return Err(Error::InvalidHeight);
        }
        if self.round < 0 {
            return Err(Error::InvalidRound);
        }
        if self.validator_index < 0 {
            return Err(Error::InvalidVote);
        }
        self.block_id.validate_basic()?;
        if !self.block_id.is_zero() && !self.block_id.is_complete() {
            return Err(Error::InvalidBlockId);
        }
        if self.signature.is_empty() || self.signature.len() > MAX_SIGNATURE_SIZE {
            return Err(Error::InvalidVote);
        }
        if self.ty != SignedMsgType::Precommit || self.block_id.is_zero() {
            if !self.extension.is_empty() || !self.extension_signature.is_empty() {
                return Err(Error::InvalidVote);
            }
        } else {
            if self.extension_signature.len() > MAX_SIGNATURE_SIZE {
                return Err(Error::InvalidVote);
            }
            if !self.extension.is_empty() && self.extension_signature.is_empty() {
                return Err(Error::InvalidVote);
            }
        }
        Ok(())
    }

    /// Convert this vote to a commit signature.
    pub fn commit_sig(&self) -> Result<CommitSig> {
        let block_id_flag = if self.block_id.is_complete() {
            BlockIdFlag::Commit
        } else if self.block_id.is_zero() {
            BlockIdFlag::Nil
        } else {
            return Err(Error::InvalidBlockId);
        };
        Ok(CommitSig {
            block_id_flag,
            validator_address: Some(self.validator_address),
            timestamp: self.timestamp,
            signature: self.signature.clone(),
        })
    }

    /// Return canonical vote sign bytes.
    pub fn sign_bytes(&self, chain_id: &str) -> Result<Vec<u8>> {
        crate::canonical::vote_sign_bytes(chain_id, self)
    }

    /// Return canonical vote-extension sign bytes.
    pub fn extension_sign_bytes(&self, chain_id: &str) -> Result<Vec<u8>> {
        crate::canonical::vote_extension_sign_bytes(chain_id, self)
    }

    /// Verify this vote's canonical signature.
    #[must_use]
    pub fn verify(&self, chain_id: &str, public_key: &PublicKey) -> bool {
        self.sign_bytes(chain_id)
            .is_ok_and(|sign_bytes| public_key.verify_signature(&sign_bytes, &self.signature))
    }

    /// Convert protobuf into a domain vote.
    pub fn from_proto(proto: proto::types::Vote) -> Result<Self> {
        let vote = Self {
            ty: SignedMsgType::try_from(proto.r#type)?,
            height: proto.height,
            round: proto.round,
            block_id: proto
                .block_id
                .map(BlockId::from_proto)
                .transpose()?
                .unwrap_or_default(),
            timestamp: proto.timestamp,
            validator_address: address_from_bytes(&proto.validator_address)?,
            validator_index: proto.validator_index,
            signature: proto.signature,
            extension: proto.extension,
            extension_signature: proto.extension_signature,
        };
        vote.validate_basic()?;
        Ok(vote)
    }

    /// Convert this vote to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::types::Vote {
        proto::types::Vote {
            r#type: self.ty.into(),
            height: self.height,
            round: self.round,
            block_id: Some(self.block_id.to_proto()),
            timestamp: self.timestamp,
            validator_address: self.validator_address.to_vec(),
            validator_index: self.validator_index,
            signature: self.signature.clone(),
            extension: self.extension.clone(),
            extension_signature: self.extension_signature.clone(),
        }
    }
}
