//! `Proposal` (`types/proposal.go`).

use crate::block_id::BlockId;
use crate::canonical::proposal_sign_bytes;
use crate::commit::MAX_SIGNATURE_SIZE;
use crate::error::{Error, Result};
use crate::gofmt::fingerprint_hex;
use crate::proto::types as pb;
use crate::time::Time;
use crate::vote::PROPOSAL_TYPE;

/// A block proposal.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Proposal {
    /// `SignedMsgType` (must be `ProposalType`).
    pub msg_type: i32,
    /// Height.
    pub height: i64,
    /// Round.
    pub round: i32,
    /// Proof-of-lock round, -1 if none.
    pub pol_round: i32,
    /// Proposed block.
    pub block_id: BlockId,
    /// Timestamp.
    pub timestamp: Time,
    /// Proposer signature over `ProposalSignBytes`.
    pub signature: Vec<u8>,
}

impl Proposal {
    /// `NewProposal` with an explicit timestamp.
    #[must_use]
    pub fn new(
        height: i64,
        round: i32,
        pol_round: i32,
        block_id: BlockId,
        timestamp: Time,
    ) -> Self {
        Self {
            msg_type: PROPOSAL_TYPE,
            height,
            round,
            pol_round,
            block_id,
            timestamp,
            signature: Vec::new(),
        }
    }

    /// `Proposal.ValidateBasic`.
    pub fn validate_basic(&self) -> Result<()> {
        if self.msg_type != PROPOSAL_TYPE {
            return Err(Error::msg("invalid Type"));
        }
        if self.height < 0 {
            return Err(Error::msg("negative Height"));
        }
        if self.round < 0 {
            return Err(Error::msg("negative Round"));
        }
        if self.pol_round < -1 {
            return Err(Error::msg("negative POLRound (exception: -1)"));
        }
        self.block_id
            .validate_basic()
            .map_err(|e| e.wrap("wrong BlockID: "))?;
        if !self.block_id.is_complete() {
            return Err(Error::msg(format!(
                "expected a complete, non-empty BlockID, got: {}",
                self.block_id
            )));
        }
        if self.signature.is_empty() {
            return Err(Error::msg("signature is missing"));
        }
        if self.signature.len() > MAX_SIGNATURE_SIZE {
            return Err(Error::msg(format!(
                "signature is too big (max: {MAX_SIGNATURE_SIZE})"
            )));
        }
        Ok(())
    }

    /// `ProposalSignBytes(chainID, p.ToProto())`.
    pub fn sign_bytes(&self, chain_id: &str) -> Result<Vec<u8>> {
        proposal_sign_bytes(chain_id, &self.to_proto())
    }

    /// `Proposal.ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pb::Proposal {
        pb::Proposal {
            r#type: self.msg_type,
            height: self.height,
            round: self.round,
            pol_round: self.pol_round,
            block_id: Some(self.block_id.to_proto()),
            timestamp: Some(self.timestamp.to_proto()),
            signature: self.signature.clone(),
        }
    }

    /// Converts without validating anything but the timestamp range.
    pub fn from_proto_unvalidated(pp: &pb::Proposal) -> Result<Self> {
        Ok(Self {
            msg_type: pp.r#type,
            height: pp.height,
            round: pp.round,
            pol_round: pp.pol_round,
            block_id: pp
                .block_id
                .as_ref()
                .map(BlockId::from_proto_unvalidated)
                .unwrap_or_default(),
            timestamp: Time::from_proto(pp.timestamp.as_ref())?,
            signature: pp.signature.clone(),
        })
    }

    /// `ProposalFromProto` (validates).
    pub fn from_proto(pp: &pb::Proposal) -> Result<Self> {
        let default_id = pb::BlockId::default();
        let block_id = BlockId::from_proto(pp.block_id.as_ref().unwrap_or(&default_id))?;
        let p = Self {
            msg_type: pp.r#type,
            height: pp.height,
            round: pp.round,
            pol_round: pp.pol_round,
            block_id,
            timestamp: Time::from_proto(pp.timestamp.as_ref())?,
            signature: pp.signature.clone(),
        };
        p.validate_basic()?;
        Ok(p)
    }
}

impl core::fmt::Display for Proposal {
    /// `Proposal.String`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "Proposal{{{}/{} ({}, {}) {} @ {}}}",
            self.height,
            self.round,
            self.block_id,
            self.pol_round,
            fingerprint_hex(&self.signature),
            self.timestamp.canonical_string()
        )
    }
}
