//! `CommitSig`, `Commit`, `ExtendedCommitSig`, `ExtendedCommit`
//! (`types/block.go`).

use crate::block_id::BlockId;
use crate::error::{Error, Result};
use crate::gofmt::{fingerprint_hex, hex_upper};
use crate::header::ADDRESS_SIZE;
use crate::merkle_proof::hash_from_byte_slices;
use crate::proto::types as pb;
use crate::time::Time;
use crate::vote::{MAX_VOTE_EXTENSION_SIZE, PRECOMMIT_TYPE, Vote, vote_sign_bytes};
use prost::Message;

/// `BlockIDFlagAbsent`.
pub const BLOCK_ID_FLAG_ABSENT: u8 = 1;
/// `BlockIDFlagCommit`.
pub const BLOCK_ID_FLAG_COMMIT: u8 = 2;
/// `BlockIDFlagNil`.
pub const BLOCK_ID_FLAG_NIL: u8 = 3;

/// `MaxSignatureSize`.
pub const MAX_SIGNATURE_SIZE: usize = 64;
/// `MaxVotesCount`.
pub const MAX_VOTES_COUNT: usize = 10_000;
/// `MaxCommitOverheadBytes`.
pub const MAX_COMMIT_OVERHEAD_BYTES: i64 = 91;
/// `MaxCommitSigBytes`.
pub const MAX_COMMIT_SIG_BYTES: i64 = 109;

/// `MaxCommitBytes(valCount)`.
#[must_use]
pub fn max_commit_bytes(val_count: usize) -> i64 {
    let proto_encoding_overhead = 2i64;
    #[allow(clippy::cast_possible_wrap)]
    let n = val_count as i64;
    MAX_COMMIT_OVERHEAD_BYTES
        .wrapping_add((MAX_COMMIT_SIG_BYTES + proto_encoding_overhead).wrapping_mul(n))
}

/// One validator's entry in a commit.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommitSig {
    /// `BlockIDFlag` (a Go `byte`).
    pub block_id_flag: u8,
    /// Validator address (empty when absent).
    pub validator_address: Vec<u8>,
    /// Vote timestamp (zero when absent).
    pub timestamp: Time,
    /// Signature (empty when absent).
    pub signature: Vec<u8>,
}

impl CommitSig {
    /// `NewCommitSigAbsent`.
    #[must_use]
    pub fn absent() -> Self {
        Self {
            block_id_flag: BLOCK_ID_FLAG_ABSENT,
            ..Self::default()
        }
    }

    /// `CommitSig.BlockID(commitBlockID)`. Go panics on an unknown flag;
    /// this returns an error with the panic message instead.
    pub fn block_id(&self, commit_block_id: &BlockId) -> Result<BlockId> {
        match self.block_id_flag {
            BLOCK_ID_FLAG_ABSENT | BLOCK_ID_FLAG_NIL => Ok(BlockId::default()),
            BLOCK_ID_FLAG_COMMIT => Ok(commit_block_id.clone()),
            f => Err(Error::msg(format!("Unknown BlockIDFlag: {f}"))),
        }
    }

    /// `CommitSig.ValidateBasic`.
    pub fn validate_basic(&self) -> Result<()> {
        match self.block_id_flag {
            BLOCK_ID_FLAG_ABSENT => {
                if !self.validator_address.is_empty() {
                    return Err(Error::msg("validator address is present"));
                }
                if !self.timestamp.is_zero() {
                    return Err(Error::msg("time is present"));
                }
                if !self.signature.is_empty() {
                    return Err(Error::msg("signature is present"));
                }
            }
            BLOCK_ID_FLAG_COMMIT | BLOCK_ID_FLAG_NIL => {
                if self.validator_address.len() != ADDRESS_SIZE {
                    return Err(Error::msg(format!(
                        "expected ValidatorAddress size to be {ADDRESS_SIZE} bytes, got {} bytes",
                        self.validator_address.len()
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
            }
            f => return Err(Error::msg(format!("unknown BlockIDFlag: {f}"))),
        }
        Ok(())
    }

    /// `CommitSig.ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pb::CommitSig {
        pb::CommitSig {
            block_id_flag: i32::from(self.block_id_flag),
            validator_address: self.validator_address.clone(),
            timestamp: Some(self.timestamp.to_proto()),
            signature: self.signature.clone(),
        }
    }

    /// Converts without validation (`BlockIDFlag(int32)` truncates to a byte).
    pub fn from_proto_unvalidated(p: &pb::CommitSig) -> Result<Self> {
        Ok(Self {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            block_id_flag: p.block_id_flag as u8,
            validator_address: p.validator_address.clone(),
            timestamp: Time::from_proto(p.timestamp.as_ref())?,
            signature: p.signature.clone(),
        })
    }

    /// `CommitSig.FromProto` (validates).
    pub fn from_proto(p: &pb::CommitSig) -> Result<Self> {
        let cs = Self::from_proto_unvalidated(p)?;
        cs.validate_basic()?;
        Ok(cs)
    }
}

impl core::fmt::Display for CommitSig {
    /// `CommitSig.String`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "CommitSig{{{} by {} on {} @ {}}}",
            fingerprint_hex(&self.signature),
            fingerprint_hex(&self.validator_address),
            self.block_id_flag,
            self.timestamp.canonical_string()
        )
    }
}

/// A commit signature with its vote extension.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExtendedCommitSig {
    /// The commit signature.
    pub commit_sig: CommitSig,
    /// Vote extension.
    pub extension: Vec<u8>,
    /// Vote extension signature.
    pub extension_signature: Vec<u8>,
}

impl ExtendedCommitSig {
    /// `NewExtendedCommitSigAbsent`.
    #[must_use]
    pub fn absent() -> Self {
        Self {
            commit_sig: CommitSig::absent(),
            ..Self::default()
        }
    }

    /// `ExtendedCommitSig.ValidateBasic`.
    pub fn validate_basic(&self) -> Result<()> {
        self.commit_sig.validate_basic()?;
        if self.commit_sig.block_id_flag == BLOCK_ID_FLAG_COMMIT {
            if self.extension.len() > MAX_VOTE_EXTENSION_SIZE {
                return Err(Error::msg(format!(
                    "vote extension is too big (max: {MAX_VOTE_EXTENSION_SIZE})"
                )));
            }
            if self.extension_signature.len() > MAX_SIGNATURE_SIZE {
                return Err(Error::msg(format!(
                    "vote extension signature is too big (max: {MAX_SIGNATURE_SIZE})"
                )));
            }
            return Ok(());
        }
        if self.extension_signature.is_empty() && !self.extension.is_empty() {
            return Err(Error::msg(
                "vote extension signature absent on vote with extension",
            ));
        }
        Ok(())
    }

    /// `ExtendedCommitSig.EnsureExtension`.
    pub fn ensure_extension(&self, ext_enabled: bool) -> Result<()> {
        let cs = &self.commit_sig;
        let who = format!(
            "validator addr {}, timestamp {}",
            hex_upper(&cs.validator_address),
            cs.timestamp.go_string()
        );
        if ext_enabled {
            if cs.block_id_flag == BLOCK_ID_FLAG_COMMIT && self.extension_signature.is_empty() {
                return Err(Error::msg(format!(
                    "vote extension signature is missing; {who}"
                )));
            }
            if cs.block_id_flag != BLOCK_ID_FLAG_COMMIT && !self.extension.is_empty() {
                return Err(Error::msg(format!(
                    "non-commit vote extension present; {who}"
                )));
            }
            if cs.block_id_flag != BLOCK_ID_FLAG_COMMIT && !self.extension_signature.is_empty() {
                return Err(Error::msg(format!(
                    "non-commit vote extension signature present; {who}"
                )));
            }
        } else {
            if !self.extension.is_empty() {
                return Err(Error::msg(format!(
                    "vote extension present but extensions disabled; {who}"
                )));
            }
            if !self.extension_signature.is_empty() {
                return Err(Error::msg(format!(
                    "vote extension signature present but extensions disabled; {who}"
                )));
            }
        }
        Ok(())
    }

    /// `ExtendedCommitSig.ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pb::ExtendedCommitSig {
        let cs = &self.commit_sig;
        pb::ExtendedCommitSig {
            block_id_flag: i32::from(cs.block_id_flag),
            validator_address: cs.validator_address.clone(),
            timestamp: Some(cs.timestamp.to_proto()),
            signature: cs.signature.clone(),
            extension: self.extension.clone(),
            extension_signature: self.extension_signature.clone(),
        }
    }

    /// `ExtendedCommitSig.FromProto` (validates).
    pub fn from_proto(p: &pb::ExtendedCommitSig) -> Result<Self> {
        let ecs = Self {
            commit_sig: CommitSig {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                block_id_flag: p.block_id_flag as u8,
                validator_address: p.validator_address.clone(),
                timestamp: Time::from_proto(p.timestamp.as_ref())?,
                signature: p.signature.clone(),
            },
            extension: p.extension.clone(),
            extension_signature: p.extension_signature.clone(),
        };
        ecs.validate_basic()?;
        Ok(ecs)
    }
}

/// +2/3 precommits for a block.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Commit {
    /// Height.
    pub height: i64,
    /// Round.
    pub round: i32,
    /// Committed block.
    pub block_id: BlockId,
    /// One entry per validator, in validator-set order.
    pub signatures: Vec<CommitSig>,
}

impl Commit {
    /// `Commit.GetVote(valIdx)`. Errors where Go panics (index out of range,
    /// unknown flag).
    pub fn get_vote(&self, val_idx: i32) -> Result<Vote> {
        let cs = usize::try_from(val_idx)
            .ok()
            .and_then(|i| self.signatures.get(i))
            .ok_or_else(|| Error::msg(format!("index out of range [{val_idx}]")))?;
        Ok(Vote {
            vote_type: PRECOMMIT_TYPE,
            height: self.height,
            round: self.round,
            block_id: cs.block_id(&self.block_id)?,
            timestamp: cs.timestamp,
            validator_address: cs.validator_address.clone(),
            validator_index: val_idx,
            signature: cs.signature.clone(),
            extension: Vec::new(),
            extension_signature: Vec::new(),
        })
    }

    /// `Commit.VoteSignBytes(chainID, valIdx)`.
    pub fn vote_sign_bytes(&self, chain_id: &str, val_idx: i32) -> Result<Vec<u8>> {
        vote_sign_bytes(chain_id, &self.get_vote(val_idx)?.to_proto())
    }

    /// `Commit.Size`.
    #[must_use]
    pub fn size(&self) -> usize {
        self.signatures.len()
    }

    /// `Commit.ValidateBasic`.
    pub fn validate_basic(&self) -> Result<()> {
        if self.height < 0 {
            return Err(Error::msg("negative Height"));
        }
        if self.round < 0 {
            return Err(Error::msg("negative Round"));
        }
        if self.height >= 1 {
            if self.block_id.is_zero() {
                return Err(Error::msg("commit cannot be for nil block"));
            }
            self.block_id
                .validate_basic()
                .map_err(|e| e.wrap("wrong BlockID: "))?;
            if self.signatures.is_empty() {
                return Err(Error::msg("no signatures in commit"));
            }
            for (i, cs) in self.signatures.iter().enumerate() {
                cs.validate_basic()
                    .map_err(|e| e.wrap(&format!("wrong CommitSig #{i}: ")))?;
            }
        }
        Ok(())
    }

    /// `Commit.Hash`: Merkle root of the encoded commit signatures.
    #[must_use]
    pub fn hash(&self) -> Vec<u8> {
        let bs: Vec<Vec<u8>> = self
            .signatures
            .iter()
            .map(|cs| cs.to_proto().encode_to_vec())
            .collect();
        hash_from_byte_slices(&bs)
    }

    /// `Commit.WrappedExtendedCommit`.
    #[must_use]
    pub fn wrapped_extended_commit(&self) -> ExtendedCommit {
        ExtendedCommit {
            height: self.height,
            round: self.round,
            block_id: self.block_id.clone(),
            extended_signatures: self
                .signatures
                .iter()
                .map(|s| ExtendedCommitSig {
                    commit_sig: s.clone(),
                    ..ExtendedCommitSig::default()
                })
                .collect(),
        }
    }

    /// `Commit.ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pb::Commit {
        pb::Commit {
            height: self.height,
            round: self.round,
            block_id: Some(self.block_id.to_proto()),
            signatures: self.signatures.iter().map(CommitSig::to_proto).collect(),
        }
    }

    /// Converts without validation.
    pub fn from_proto_unvalidated(p: &pb::Commit) -> Result<Self> {
        Ok(Self {
            height: p.height,
            round: p.round,
            block_id: p
                .block_id
                .as_ref()
                .map(BlockId::from_proto_unvalidated)
                .unwrap_or_default(),
            signatures: p
                .signatures
                .iter()
                .map(CommitSig::from_proto_unvalidated)
                .collect::<Result<_>>()?,
        })
    }

    /// `CommitFromProto` (validates).
    pub fn from_proto(p: &pb::Commit) -> Result<Self> {
        if p.signatures.len() > MAX_VOTES_COUNT {
            return Err(Error::msg(format!(
                "commit has too many signatures: {}, max: {MAX_VOTES_COUNT}",
                p.signatures.len()
            )));
        }
        let default_id = pb::BlockId::default();
        let block_id = BlockId::from_proto(p.block_id.as_ref().unwrap_or(&default_id))?;
        let signatures = p
            .signatures
            .iter()
            .map(CommitSig::from_proto)
            .collect::<Result<_>>()?;
        let c = Self {
            height: p.height,
            round: p.round,
            block_id,
            signatures,
        };
        c.validate_basic()?;
        Ok(c)
    }
}

/// A commit whose signatures retain vote extensions.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExtendedCommit {
    /// Height.
    pub height: i64,
    /// Round.
    pub round: i32,
    /// Committed block.
    pub block_id: BlockId,
    /// Extended signatures.
    pub extended_signatures: Vec<ExtendedCommitSig>,
}

impl ExtendedCommit {
    /// `ExtendedCommit.EnsureExtensions`.
    pub fn ensure_extensions(&self, ext_enabled: bool) -> Result<()> {
        self.extended_signatures
            .iter()
            .try_for_each(|e| e.ensure_extension(ext_enabled))
    }

    /// `ExtendedCommit.ToCommit`.
    #[must_use]
    pub fn to_commit(&self) -> Commit {
        Commit {
            height: self.height,
            round: self.round,
            block_id: self.block_id.clone(),
            signatures: self
                .extended_signatures
                .iter()
                .map(|e| e.commit_sig.clone())
                .collect(),
        }
    }

    /// `ExtendedCommit.GetExtendedVote(valIndex)`.
    pub fn get_extended_vote(&self, val_idx: i32) -> Result<Vote> {
        let ecs = usize::try_from(val_idx)
            .ok()
            .and_then(|i| self.extended_signatures.get(i))
            .ok_or_else(|| Error::msg(format!("index out of range [{val_idx}]")))?;
        let cs = &ecs.commit_sig;
        Ok(Vote {
            vote_type: PRECOMMIT_TYPE,
            height: self.height,
            round: self.round,
            block_id: cs.block_id(&self.block_id)?,
            timestamp: cs.timestamp,
            validator_address: cs.validator_address.clone(),
            validator_index: val_idx,
            signature: cs.signature.clone(),
            extension: ecs.extension.clone(),
            extension_signature: ecs.extension_signature.clone(),
        })
    }

    /// `ExtendedCommit.Size`.
    #[must_use]
    pub fn size(&self) -> usize {
        self.extended_signatures.len()
    }

    /// `ExtendedCommit.BitArray`: which validators voted (block or nil).
    #[must_use]
    pub fn bit_array(&self) -> Vec<bool> {
        self.extended_signatures
            .iter()
            .map(|e| e.commit_sig.block_id_flag != BLOCK_ID_FLAG_ABSENT)
            .collect()
    }

    /// `ExtendedCommit.IsCommit`.
    #[must_use]
    pub fn is_commit(&self) -> bool {
        !self.extended_signatures.is_empty()
    }

    /// `ExtendedCommit.ValidateBasic`.
    pub fn validate_basic(&self) -> Result<()> {
        if self.height < 0 {
            return Err(Error::msg("negative Height"));
        }
        if self.round < 0 {
            return Err(Error::msg("negative Round"));
        }
        if self.height >= 1 {
            if self.block_id.is_zero() {
                return Err(Error::msg("extended commit cannot be for nil block"));
            }
            if self.extended_signatures.is_empty() {
                return Err(Error::msg("no signatures in commit"));
            }
            for (i, e) in self.extended_signatures.iter().enumerate() {
                e.validate_basic()
                    .map_err(|err| err.wrap(&format!("wrong ExtendedCommitSig #{i}: ")))?;
            }
        }
        Ok(())
    }

    /// `ExtendedCommit.ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pb::ExtendedCommit {
        pb::ExtendedCommit {
            height: self.height,
            round: self.round,
            block_id: Some(self.block_id.to_proto()),
            extended_signatures: self
                .extended_signatures
                .iter()
                .map(ExtendedCommitSig::to_proto)
                .collect(),
        }
    }

    /// `ExtendedCommitFromProto` (validates).
    pub fn from_proto(p: &pb::ExtendedCommit) -> Result<Self> {
        if p.extended_signatures.len() > MAX_VOTES_COUNT {
            return Err(Error::msg(format!(
                "extended commit has too many signatures: {}, max: {MAX_VOTES_COUNT}",
                p.extended_signatures.len()
            )));
        }
        let default_id = pb::BlockId::default();
        let block_id = BlockId::from_proto(p.block_id.as_ref().unwrap_or(&default_id))?;
        let extended_signatures = p
            .extended_signatures
            .iter()
            .map(ExtendedCommitSig::from_proto)
            .collect::<Result<_>>()?;
        let ec = Self {
            height: p.height,
            round: p.round,
            block_id,
            extended_signatures,
        };
        ec.validate_basic()?;
        Ok(ec)
    }
}
