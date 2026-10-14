//! `Vote` (`types/vote.go`, `types/signed_msg_type.go`).

use crate::block_id::{BlockId, proto_block_id_is_nil};
pub use crate::canonical::{vote_extension_sign_bytes, vote_sign_bytes};
use crate::commit::{
    BLOCK_ID_FLAG_COMMIT, BLOCK_ID_FLAG_NIL, CommitSig, ExtendedCommitSig, MAX_SIGNATURE_SIZE,
};
use crate::error::{Error, Result};
use crate::gofmt::fingerprint_hex;
use crate::header::ADDRESS_SIZE;
use crate::proto::types as pb;
use crate::pubkey::PubKey;
use crate::time::Time;

/// `UnknownType`.
pub const UNKNOWN_TYPE: i32 = 0;
/// `PrevoteType`.
pub const PREVOTE_TYPE: i32 = 1;
/// `PrecommitType`.
pub const PRECOMMIT_TYPE: i32 = 2;
/// `ProposalType`.
pub const PROPOSAL_TYPE: i32 = 32;

/// `MaxVoteExtensionSize`.
pub const MAX_VOTE_EXTENSION_SIZE: usize = 1024 * 1024;

/// `IsVoteTypeValid`.
#[must_use]
pub fn is_vote_type_valid(t: i32) -> bool {
    t == PREVOTE_TYPE || t == PRECOMMIT_TYPE
}

/// `SignedMsgTypeToShortString`.
#[must_use]
pub fn signed_msg_type_short_string(t: i32) -> &'static str {
    match t {
        PREVOTE_TYPE => "prevote",
        PRECOMMIT_TYPE => "precommit",
        PROPOSAL_TYPE => "proposal",
        _ => "unknown",
    }
}

/// `SignedMsgType.String()` (proto enum name; the number when unknown).
#[must_use]
pub fn signed_msg_type_string(t: i32) -> String {
    match t {
        UNKNOWN_TYPE => "SIGNED_MSG_TYPE_UNKNOWN".to_owned(),
        PREVOTE_TYPE => "SIGNED_MSG_TYPE_PREVOTE".to_owned(),
        PRECOMMIT_TYPE => "SIGNED_MSG_TYPE_PRECOMMIT".to_owned(),
        PROPOSAL_TYPE => "SIGNED_MSG_TYPE_PROPOSAL".to_owned(),
        n => n.to_string(),
    }
}

/// A prevote or precommit.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Vote {
    /// `SignedMsgType`.
    pub vote_type: i32,
    /// Height.
    pub height: i64,
    /// Round.
    pub round: i32,
    /// Voted block; zero for nil.
    pub block_id: BlockId,
    /// Timestamp.
    pub timestamp: Time,
    /// Validator address.
    pub validator_address: Vec<u8>,
    /// Validator index.
    pub validator_index: i32,
    /// Signature over `VoteSignBytes`.
    pub signature: Vec<u8>,
    /// Vote extension (non-nil precommits only).
    pub extension: Vec<u8>,
    /// Signature over `VoteExtensionSignBytes`.
    pub extension_signature: Vec<u8>,
}

impl Vote {
    /// `VoteFromProto` (only the block id is validated).
    pub fn from_proto(pv: &pb::Vote) -> Result<Self> {
        let default_id = pb::BlockId::default();
        let block_id = BlockId::from_proto(pv.block_id.as_ref().unwrap_or(&default_id))?;
        Ok(Self {
            vote_type: pv.r#type,
            height: pv.height,
            round: pv.round,
            block_id,
            timestamp: Time::from_proto(pv.timestamp.as_ref())?,
            validator_address: pv.validator_address.clone(),
            validator_index: pv.validator_index,
            signature: pv.signature.clone(),
            extension: pv.extension.clone(),
            extension_signature: pv.extension_signature.clone(),
        })
    }

    /// Converts without validating anything but the timestamp range.
    pub fn from_proto_unvalidated(pv: &pb::Vote) -> Result<Self> {
        Ok(Self {
            vote_type: pv.r#type,
            height: pv.height,
            round: pv.round,
            block_id: pv
                .block_id
                .as_ref()
                .map(BlockId::from_proto_unvalidated)
                .unwrap_or_default(),
            timestamp: Time::from_proto(pv.timestamp.as_ref())?,
            validator_address: pv.validator_address.clone(),
            validator_index: pv.validator_index,
            signature: pv.signature.clone(),
            extension: pv.extension.clone(),
            extension_signature: pv.extension_signature.clone(),
        })
    }

    /// `Vote.ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pb::Vote {
        pb::Vote {
            r#type: self.vote_type,
            height: self.height,
            round: self.round,
            block_id: Some(self.block_id.to_proto()),
            timestamp: Some(self.timestamp.to_proto()),
            validator_address: self.validator_address.clone(),
            validator_index: self.validator_index,
            signature: self.signature.clone(),
            extension: self.extension.clone(),
            extension_signature: self.extension_signature.clone(),
        }
    }

    /// `Vote.CommitSig`. Go panics when the block id is neither empty nor
    /// complete; this returns that message as an error.
    pub fn commit_sig(&self) -> Result<CommitSig> {
        let flag = if self.block_id.is_complete() {
            BLOCK_ID_FLAG_COMMIT
        } else if self.block_id.is_zero() {
            BLOCK_ID_FLAG_NIL
        } else {
            return Err(Error::msg(format!(
                "Invalid vote {self} - expected BlockID to be either empty or complete"
            )));
        };
        Ok(CommitSig {
            block_id_flag: flag,
            validator_address: self.validator_address.clone(),
            timestamp: self.timestamp,
            signature: self.signature.clone(),
        })
    }

    /// `Vote.ExtendedCommitSig`.
    pub fn extended_commit_sig(&self) -> Result<ExtendedCommitSig> {
        Ok(ExtendedCommitSig {
            commit_sig: self.commit_sig()?,
            extension: self.extension.clone(),
            extension_signature: self.extension_signature.clone(),
        })
    }

    fn verify_and_return_proto(&self, chain_id: &str, pub_key: &PubKey) -> Result<pb::Vote> {
        if pub_key.address() != self.validator_address {
            return Err(Error::VoteInvalidValidatorAddress);
        }
        let v = self.to_proto();
        if !pub_key.verify_signature(&vote_sign_bytes(chain_id, &v)?, &self.signature) {
            return Err(Error::VoteInvalidSignature);
        }
        Ok(v)
    }

    /// `Vote.Verify`: address and signature (not the extension).
    pub fn verify(&self, chain_id: &str, pub_key: &PubKey) -> Result<()> {
        self.verify_and_return_proto(chain_id, pub_key).map(|_| ())
    }

    /// `Vote.VerifyVoteAndExtension`.
    pub fn verify_vote_and_extension(&self, chain_id: &str, pub_key: &PubKey) -> Result<()> {
        let v = self.verify_and_return_proto(chain_id, pub_key)?;
        let default_id = pb::BlockId::default();
        if self.vote_type == PRECOMMIT_TYPE
            && !proto_block_id_is_nil(v.block_id.as_ref().unwrap_or(&default_id))
        {
            if self.extension_signature.is_empty() {
                return Err(Error::msg("expected vote extension signature"));
            }
            let ext = vote_extension_sign_bytes(chain_id, &v);
            if !pub_key.verify_signature(&ext, &self.extension_signature) {
                return Err(Error::VoteInvalidSignature);
            }
        }
        Ok(())
    }

    /// `Vote.VerifyExtension`.
    pub fn verify_extension(&self, chain_id: &str, pub_key: &PubKey) -> Result<()> {
        if self.vote_type != PRECOMMIT_TYPE || self.block_id.is_zero() {
            return Ok(());
        }
        let ext = vote_extension_sign_bytes(chain_id, &self.to_proto());
        if !pub_key.verify_signature(&ext, &self.extension_signature) {
            return Err(Error::VoteInvalidSignature);
        }
        Ok(())
    }

    /// `Vote.ValidateBasic`.
    pub fn validate_basic(&self) -> Result<()> {
        if !is_vote_type_valid(self.vote_type) {
            return Err(Error::msg("invalid Type"));
        }
        if self.height <= 0 {
            return Err(Error::msg("negative or zero Height"));
        }
        if self.round < 0 {
            return Err(Error::msg("negative Round"));
        }
        self.block_id
            .validate_basic()
            .map_err(|e| e.wrap("wrong BlockID: "))?;
        if !self.block_id.is_zero() && !self.block_id.is_complete() {
            return Err(Error::msg(format!(
                "blockID must be either empty or complete, got: {}",
                self.block_id
            )));
        }
        if self.validator_address.len() != ADDRESS_SIZE {
            return Err(Error::msg(format!(
                "expected ValidatorAddress size to be {ADDRESS_SIZE} bytes, got {} bytes",
                self.validator_address.len()
            )));
        }
        if self.validator_index < 0 {
            return Err(Error::msg("negative ValidatorIndex"));
        }
        if self.signature.is_empty() {
            return Err(Error::msg("signature is missing"));
        }
        if self.signature.len() > MAX_SIGNATURE_SIZE {
            return Err(Error::msg(format!(
                "signature is too big (max: {MAX_SIGNATURE_SIZE})"
            )));
        }
        if self.vote_type != PRECOMMIT_TYPE || self.block_id.is_zero() {
            if !self.extension.is_empty() {
                return Err(Error::msg(format!(
                    "unexpected vote extension; vote type {}, isNil {}",
                    self.vote_type,
                    self.block_id.is_zero()
                )));
            }
            if !self.extension_signature.is_empty() {
                return Err(Error::msg("unexpected vote extension signature"));
            }
        }
        if self.vote_type == PRECOMMIT_TYPE && !self.block_id.is_zero() {
            if self.extension_signature.len() > MAX_SIGNATURE_SIZE {
                return Err(Error::msg(format!(
                    "vote extension signature is too big (max: {MAX_SIGNATURE_SIZE})"
                )));
            }
            if self.extension_signature.is_empty() && !self.extension.is_empty() {
                return Err(Error::msg(
                    "vote extension signature absent on vote with extension",
                ));
            }
        }
        Ok(())
    }

    /// `Vote.EnsureExtension`.
    pub fn ensure_extension(&self) -> Result<()> {
        if self.vote_type != PRECOMMIT_TYPE
            || self.block_id.is_zero()
            || !self.extension_signature.is_empty()
        {
            return Ok(());
        }
        Err(Error::VoteExtensionAbsent)
    }
}

impl core::fmt::Display for Vote {
    /// `Vote.String` (Go panics for non-vote types; those print `Unknown`).
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let type_string = match self.vote_type {
            PREVOTE_TYPE => "Prevote",
            PRECOMMIT_TYPE => "Precommit",
            _ => "Unknown",
        };
        write!(
            f,
            "Vote{{{}:{} {}/{:02}/{}({}) {} {} {} @ {}}}",
            self.validator_index,
            fingerprint_hex(&self.validator_address),
            self.height,
            self.round,
            signed_msg_type_string(self.vote_type),
            type_string,
            fingerprint_hex(&self.block_id.hash),
            fingerprint_hex(&self.signature),
            fingerprint_hex(&self.extension),
            self.timestamp.canonical_string()
        )
    }
}

/// `Vote.String()` for an optional vote (`nil-Vote`).
#[must_use]
pub fn vote_string(v: Option<&Vote>) -> String {
    v.map_or_else(|| "nil-Vote".to_owned(), ToString::to_string)
}
