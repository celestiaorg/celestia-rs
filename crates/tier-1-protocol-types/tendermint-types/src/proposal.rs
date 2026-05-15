//! CometBFT proposal types.

use crate::block::BlockId;
use crate::commit::MAX_SIGNATURE_SIZE;
use crate::error::{Error, Result};
use crate::proto;
use crate::validator::PublicKey;
use crate::vote::SignedMsgType;

/// A CometBFT proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    /// Proposal height.
    pub height: i64,
    /// Proposal round.
    pub round: i32,
    /// Proof-of-lock round, or -1 when absent.
    pub pol_round: i32,
    /// Proposed block ID.
    pub block_id: BlockId,
    /// Proposal timestamp.
    pub timestamp: Option<proto::google::Timestamp>,
    /// Proposal signature.
    pub signature: Vec<u8>,
}

impl Proposal {
    /// Perform CometBFT stateless proposal validation.
    pub fn validate_basic(&self) -> Result<()> {
        if self.height <= 0 {
            return Err(Error::InvalidHeight);
        }
        if self.round < 0 || self.pol_round < -1 {
            return Err(Error::InvalidRound);
        }
        if !self.block_id.is_complete() {
            return Err(Error::InvalidBlockId);
        }
        if self.signature.is_empty() || self.signature.len() > MAX_SIGNATURE_SIZE {
            return Err(Error::InvalidProposal);
        }
        Ok(())
    }

    /// Return canonical proposal sign bytes.
    pub fn sign_bytes(&self, chain_id: &str) -> Result<Vec<u8>> {
        crate::canonical::proposal_sign_bytes(chain_id, self)
    }

    /// Verify this proposal's canonical signature.
    #[must_use]
    pub fn verify(&self, chain_id: &str, public_key: &PublicKey) -> bool {
        self.sign_bytes(chain_id)
            .is_ok_and(|sign_bytes| public_key.verify_signature(&sign_bytes, &self.signature))
    }

    /// Convert protobuf into a domain proposal.
    pub fn from_proto(proto: proto::types::Proposal) -> Result<Self> {
        if SignedMsgType::try_from(proto.r#type)? != SignedMsgType::Proposal {
            return Err(Error::InvalidProposal);
        }
        let proposal = Self {
            height: proto.height,
            round: proto.round,
            pol_round: proto.pol_round,
            block_id: proto
                .block_id
                .map(BlockId::from_proto)
                .transpose()?
                .unwrap_or_default(),
            timestamp: proto.timestamp,
            signature: proto.signature,
        };
        proposal.validate_basic()?;
        Ok(proposal)
    }

    /// Convert this proposal to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::types::Proposal {
        proto::types::Proposal {
            r#type: SignedMsgType::Proposal.into(),
            height: self.height,
            round: self.round,
            pol_round: self.pol_round,
            block_id: Some(self.block_id.to_proto()),
            timestamp: self.timestamp,
            signature: self.signature.clone(),
        }
    }
}
