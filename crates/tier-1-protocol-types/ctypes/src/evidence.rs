//! Evidence (`types/evidence.go`): `DuplicateVoteEvidence`,
//! `LightClientAttackEvidence`, `EvidenceList` hashing.

use crate::commit::BLOCK_ID_FLAG_COMMIT;
use crate::error::{Error, Result};
use crate::gofmt::hex_upper;
use crate::header::Header;
use crate::light::{LightBlock, SignedHeader};
use crate::merkle_proof::hash_from_byte_slices;
use crate::proto::{abci, types as pb};
use crate::time::Time;
use crate::validator::Validator;
use crate::validator_set::{ValidatorSet, sort_by_voting_power};
use crate::vote::{Vote, vote_string};
use prost::Message;

/// `abci.MisbehaviorType_DUPLICATE_VOTE`.
pub const MISBEHAVIOR_DUPLICATE_VOTE: i32 = 1;
/// `abci.MisbehaviorType_LIGHT_CLIENT_ATTACK`.
pub const MISBEHAVIOR_LIGHT_CLIENT_ATTACK: i32 = 2;

/// A validator signing two conflicting votes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DuplicateVoteEvidence {
    /// First vote (lower `BlockID.Key`).
    pub vote_a: Option<Vote>,
    /// Second vote.
    pub vote_b: Option<Vote>,
    /// Total voting power of the set.
    pub total_voting_power: i64,
    /// Power of the offending validator.
    pub validator_power: i64,
    /// Block time of the infraction height.
    pub timestamp: Time,
}

impl DuplicateVoteEvidence {
    /// `NewDuplicateVoteEvidence`: orders the votes by `BlockID.Key`.
    pub fn new(vote1: Vote, vote2: Vote, block_time: Time, val_set: &ValidatorSet) -> Result<Self> {
        let Some((_, val)) = val_set.get_by_address(&vote1.validator_address) else {
            return Err(Error::msg(format!(
                "validator {} not in validator set",
                hex_upper(&vote1.validator_address)
            )));
        };
        let (a, b) = if vote1.block_id.key() < vote2.block_id.key() {
            (vote1, vote2)
        } else {
            (vote2, vote1)
        };
        Ok(Self {
            vote_a: Some(a),
            vote_b: Some(b),
            total_voting_power: val_set.total_voting_power(),
            validator_power: val.voting_power,
            timestamp: block_time,
        })
    }

    /// `ABCI()`.
    #[must_use]
    pub fn abci(&self) -> Vec<abci::Misbehavior> {
        let va = self.vote_a.clone().unwrap_or_default();
        vec![abci::Misbehavior {
            r#type: MISBEHAVIOR_DUPLICATE_VOTE,
            validator: Some(abci::Validator {
                address: va.validator_address,
                power: self.validator_power,
            }),
            height: va.height,
            time: Some(self.timestamp.to_proto()),
            total_voting_power: self.total_voting_power,
        }]
    }

    /// `ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pb::DuplicateVoteEvidence {
        pb::DuplicateVoteEvidence {
            vote_a: self.vote_a.as_ref().map(Vote::to_proto),
            vote_b: self.vote_b.as_ref().map(Vote::to_proto),
            total_voting_power: self.total_voting_power,
            validator_power: self.validator_power,
            timestamp: Some(self.timestamp.to_proto()),
        }
    }

    /// `Bytes()`.
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        self.to_proto().encode_to_vec()
    }

    /// `Hash()`.
    #[must_use]
    pub fn hash(&self) -> Vec<u8> {
        hash::sha256(&self.bytes()).to_vec()
    }

    /// `Height()`.
    #[must_use]
    pub fn height(&self) -> i64 {
        self.vote_a.as_ref().map_or(0, |v| v.height)
    }

    /// `ValidateBasic`.
    pub fn validate_basic(&self) -> Result<()> {
        let (Some(a), Some(b)) = (&self.vote_a, &self.vote_b) else {
            return Err(Error::msg(format!(
                "one or both of the votes are empty {}, {}",
                vote_string(self.vote_a.as_ref()),
                vote_string(self.vote_b.as_ref())
            )));
        };
        a.validate_basic().map_err(|e| e.wrap("invalid VoteA: "))?;
        b.validate_basic().map_err(|e| e.wrap("invalid VoteB: "))?;
        if a.block_id.key() >= b.block_id.key() {
            return Err(Error::msg("duplicate votes in invalid order"));
        }
        Ok(())
    }

    /// Converts without validation (timestamps are range-checked).
    pub fn from_proto_unvalidated(p: &pb::DuplicateVoteEvidence) -> Result<Self> {
        Ok(Self {
            vote_a: p
                .vote_a
                .as_ref()
                .map(Vote::from_proto_unvalidated)
                .transpose()?,
            vote_b: p
                .vote_b
                .as_ref()
                .map(Vote::from_proto_unvalidated)
                .transpose()?,
            total_voting_power: p.total_voting_power,
            validator_power: p.validator_power,
            timestamp: Time::from_proto(p.timestamp.as_ref())?,
        })
    }

    /// `DuplicateVoteEvidenceFromProto` (validates).
    pub fn from_proto(p: &pb::DuplicateVoteEvidence) -> Result<Self> {
        let decode = |v: &Option<pb::Vote>| -> Result<Option<Vote>> {
            v.as_ref()
                .map(|v| {
                    let v = Vote::from_proto(v)?;
                    v.validate_basic()?;
                    Ok(v)
                })
                .transpose()
        };
        let dve = Self {
            vote_a: decode(&p.vote_a)?,
            vote_b: decode(&p.vote_b)?,
            total_voting_power: p.total_voting_power,
            validator_power: p.validator_power,
            timestamp: Time::from_proto(p.timestamp.as_ref())?,
        };
        dve.validate_basic()?;
        Ok(dve)
    }
}

impl core::fmt::Display for DuplicateVoteEvidence {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "DuplicateVoteEvidence{{VoteA: {}, VoteB: {}}}",
            vote_string(self.vote_a.as_ref()),
            vote_string(self.vote_b.as_ref())
        )
    }
}

/// Evidence of an attack on a light client.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LightClientAttackEvidence {
    /// The conflicting light block.
    pub conflicting_block: Option<LightBlock>,
    /// Last height where both chains agreed.
    pub common_height: i64,
    /// Validators that signed the conflicting block.
    pub byzantine_validators: Vec<Validator>,
    /// Total voting power at the common height.
    pub total_voting_power: i64,
    /// Block time at the common height.
    pub timestamp: Time,
}

impl LightClientAttackEvidence {
    /// `ABCI()`.
    #[must_use]
    pub fn abci(&self) -> Vec<abci::Misbehavior> {
        self.byzantine_validators
            .iter()
            .map(|v| abci::Misbehavior {
                r#type: MISBEHAVIOR_LIGHT_CLIENT_ATTACK,
                validator: Some(abci::Validator {
                    address: v.pub_key.address(),
                    power: v.voting_power,
                }),
                height: self.common_height,
                time: Some(self.timestamp.to_proto()),
                total_voting_power: self.total_voting_power,
            })
            .collect()
    }

    /// `ToProto`.
    pub fn to_proto(&self) -> Result<pb::LightClientAttackEvidence> {
        Ok(pb::LightClientAttackEvidence {
            conflicting_block: self
                .conflicting_block
                .as_ref()
                .map(LightBlock::to_proto)
                .transpose()?,
            common_height: self.common_height,
            byzantine_validators: self
                .byzantine_validators
                .iter()
                .map(Validator::to_proto)
                .collect(),
            total_voting_power: self.total_voting_power,
            timestamp: Some(self.timestamp.to_proto()),
        })
    }

    /// `Bytes()` (Go panics where this errors).
    pub fn bytes(&self) -> Result<Vec<u8>> {
        Ok(self.to_proto()?.encode_to_vec())
    }

    /// `Hash()`: SHA-256 over the conflicting header hash (only its first 31
    /// bytes are copied, a Go quirk preserved here) and the varint common
    /// height.
    #[must_use]
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    pub fn hash(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut ux = (self.common_height as u64) << 1;
        if self.common_height < 0 {
            ux = !ux;
        }
        while ux >= 0x80 {
            buf.push((ux as u8) | 0x80);
            ux >>= 7;
        }
        buf.push(ux as u8);
        let mut bz = vec![0u8; 32 + buf.len()];
        let header_hash = self
            .conflicting_block
            .as_ref()
            .and_then(LightBlock::header)
            .and_then(Header::hash)
            .unwrap_or_default();
        let n = header_hash.len().min(31);
        bz[..n].copy_from_slice(&header_hash[..n]);
        bz[32..].copy_from_slice(&buf);
        hash::sha256(&bz).to_vec()
    }

    /// `Height()`: the common height.
    #[must_use]
    pub fn height(&self) -> i64 {
        self.common_height
    }

    /// `ConflictingHeaderIsInvalid(trustedHeader)`.
    #[must_use]
    pub fn conflicting_header_is_invalid(&self, trusted: &Header) -> bool {
        let Some(c) = self.conflicting_block.as_ref().and_then(LightBlock::header) else {
            return true;
        };
        trusted.validators_hash != c.validators_hash
            || trusted.next_validators_hash != c.next_validators_hash
            || trusted.consensus_hash != c.consensus_hash
            || trusted.app_hash != c.app_hash
            || trusted.last_results_hash != c.last_results_hash
    }

    /// `GetByzantineValidators(commonVals, trusted)`.
    #[must_use]
    pub fn get_byzantine_validators(
        &self,
        common_vals: &ValidatorSet,
        trusted: &SignedHeader,
    ) -> Vec<Validator> {
        let mut validators = Vec::new();
        let Some(cb) = &self.conflicting_block else {
            return validators;
        };
        let Some(csh) = &cb.signed_header else {
            return validators;
        };
        let conflicting_sigs = csh
            .commit
            .as_ref()
            .map(|c| c.signatures.as_slice())
            .unwrap_or_default();
        let trusted_header = trusted.header.clone().unwrap_or_default();
        if self.conflicting_header_is_invalid(&trusted_header) {
            for cs in conflicting_sigs {
                if cs.block_id_flag != BLOCK_ID_FLAG_COMMIT {
                    continue;
                }
                if let Some((_, v)) = common_vals.get_by_address(&cs.validator_address) {
                    validators.push(v);
                }
            }
            sort_by_voting_power(&mut validators);
            return validators;
        }
        let trusted_round = trusted.commit.as_ref().map(|c| c.round);
        let conflicting_round = csh.commit.as_ref().map(|c| c.round);
        if trusted_round == conflicting_round {
            let trusted_sigs = trusted
                .commit
                .as_ref()
                .map(|c| c.signatures.as_slice())
                .unwrap_or_default();
            for (i, sig_a) in conflicting_sigs.iter().enumerate() {
                if sig_a.block_id_flag != BLOCK_ID_FLAG_COMMIT {
                    continue;
                }
                let Some(sig_b) = trusted_sigs.get(i) else {
                    continue;
                };
                if sig_b.block_id_flag != BLOCK_ID_FLAG_COMMIT {
                    continue;
                }
                if let Some((_, v)) = cb
                    .validator_set
                    .as_ref()
                    .and_then(|vs| vs.get_by_address(&sig_a.validator_address))
                {
                    validators.push(v);
                }
            }
            sort_by_voting_power(&mut validators);
        }
        validators
    }

    /// `ValidateBasic`.
    pub fn validate_basic(&self) -> Result<()> {
        let Some(cb) = &self.conflicting_block else {
            return Err(Error::msg("conflicting block is nil"));
        };
        let Some(sh) = &cb.signed_header else {
            return Err(Error::msg("conflicting block missing signed header"));
        };
        let Some(header) = &sh.header else {
            return Err(Error::msg("conflicting block missing header"));
        };
        if self.total_voting_power <= 0 {
            return Err(Error::msg("negative or zero total voting power"));
        }
        if self.common_height <= 0 {
            return Err(Error::msg("negative or zero common height"));
        }
        if self.common_height > header.height {
            return Err(Error::msg(format!(
                "common height is ahead of the conflicting block height ({} > {})",
                self.common_height, header.height
            )));
        }
        cb.validate_basic(&header.chain_id)
            .map_err(|e| e.wrap("invalid conflicting light block: "))
    }

    /// `LightClientAttackEvidenceFromProto` (validates).
    pub fn from_proto(p: &pb::LightClientAttackEvidence) -> Result<Self> {
        let Some(cb) = &p.conflicting_block else {
            return Err(Error::msg("nil light block"));
        };
        let l = Self {
            conflicting_block: Some(LightBlock::from_proto(cb)?),
            common_height: p.common_height,
            byzantine_validators: p
                .byzantine_validators
                .iter()
                .map(Validator::from_proto)
                .collect::<Result<_>>()?,
            total_voting_power: p.total_voting_power,
            timestamp: Time::from_proto(p.timestamp.as_ref())?,
        };
        l.validate_basic()?;
        Ok(l)
    }
}

/// Any evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evidence {
    /// Duplicate vote.
    DuplicateVote(DuplicateVoteEvidence),
    /// Light client attack.
    LightClientAttack(LightClientAttackEvidence),
}

impl Evidence {
    /// `ABCI()`.
    #[must_use]
    pub fn abci(&self) -> Vec<abci::Misbehavior> {
        match self {
            Self::DuplicateVote(e) => e.abci(),
            Self::LightClientAttack(e) => e.abci(),
        }
    }

    /// `Bytes()`.
    pub fn bytes(&self) -> Result<Vec<u8>> {
        match self {
            Self::DuplicateVote(e) => Ok(e.bytes()),
            Self::LightClientAttack(e) => e.bytes(),
        }
    }

    /// `Hash()`.
    #[must_use]
    pub fn hash(&self) -> Vec<u8> {
        match self {
            Self::DuplicateVote(e) => e.hash(),
            Self::LightClientAttack(e) => e.hash(),
        }
    }

    /// `Height()`.
    #[must_use]
    pub fn height(&self) -> i64 {
        match self {
            Self::DuplicateVote(e) => e.height(),
            Self::LightClientAttack(e) => e.height(),
        }
    }

    /// `Time()`.
    #[must_use]
    pub fn time(&self) -> Time {
        match self {
            Self::DuplicateVote(e) => e.timestamp,
            Self::LightClientAttack(e) => e.timestamp,
        }
    }

    /// `ValidateBasic()`.
    pub fn validate_basic(&self) -> Result<()> {
        match self {
            Self::DuplicateVote(e) => e.validate_basic(),
            Self::LightClientAttack(e) => e.validate_basic(),
        }
    }

    /// `EvidenceToProto`.
    pub fn to_proto(&self) -> Result<pb::Evidence> {
        use pb::evidence::Sum;
        Ok(pb::Evidence {
            sum: Some(match self {
                Self::DuplicateVote(e) => Sum::DuplicateVoteEvidence(e.to_proto()),
                Self::LightClientAttack(e) => Sum::LightClientAttackEvidence(e.to_proto()?),
            }),
        })
    }

    /// `EvidenceFromProto` (validates).
    pub fn from_proto(p: &pb::Evidence) -> Result<Self> {
        use pb::evidence::Sum;
        match &p.sum {
            Some(Sum::DuplicateVoteEvidence(e)) => {
                Ok(Self::DuplicateVote(DuplicateVoteEvidence::from_proto(e)?))
            }
            Some(Sum::LightClientAttackEvidence(e)) => Ok(Self::LightClientAttack(
                LightClientAttackEvidence::from_proto(e)?,
            )),
            None => Err(Error::msg("evidence is not recognized")),
        }
    }
}

/// `EvidenceList.Hash`: Merkle root of `Bytes()` (Go panics on encoding
/// errors; those encode as empty here).
#[must_use]
pub fn evidence_list_hash(evl: &[Evidence]) -> Vec<u8> {
    let bzs: Vec<Vec<u8>> = evl.iter().map(|e| e.bytes().unwrap_or_default()).collect();
    hash_from_byte_slices(&bzs)
}

/// `EvidenceList.Has`.
#[must_use]
pub fn evidence_list_has(evl: &[Evidence], ev: &Evidence) -> bool {
    let h = ev.hash();
    evl.iter().any(|e| e.hash() == h)
}

/// `EvidenceList.ToABCI`.
#[must_use]
pub fn evidence_list_to_abci(evl: &[Evidence]) -> Vec<abci::Misbehavior> {
    evl.iter().flat_map(Evidence::abci).collect()
}

/// `ErrEvidenceOverflow` message.
#[must_use]
pub fn evidence_overflow_message(max: i64, got: i64) -> String {
    format!("Too much evidence: Max {max}, got {got}")
}
