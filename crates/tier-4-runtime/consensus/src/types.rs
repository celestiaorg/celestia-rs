//! Round state (`consensus/types/round_state.go`), consensus wire messages
//! (`consensus/msgs.go`, `tendermint.consensus.Message`) and timeouts.

use ctypes::proto::consensus as pbc;
use ctypes::{Block, BlockId, Part, PartSet, PartSetHeader, Proposal, Time, ValidatorSet, Vote};
use prost::Message as _;

use crate::height_vote_set::HeightVoteSet;
use crate::vote_set::{BitArray, VoteSet};

/// `RoundStepType` (ordered).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum RoundStep {
    /// Waiting for `CommitTime + timeoutCommit`.
    #[default]
    NewHeight = 1,
    /// Setting up a new round.
    NewRound = 2,
    /// Proposed / waiting for a proposal.
    Propose = 3,
    /// Prevoted.
    Prevote = 4,
    /// Saw +2/3 prevotes for anything; waiting.
    PrevoteWait = 5,
    /// Precommitted.
    Precommit = 6,
    /// Saw +2/3 precommits for anything; waiting.
    PrecommitWait = 7,
    /// Committing.
    Commit = 8,
}

impl RoundStep {
    /// From the wire value.
    #[must_use]
    pub fn from_u32(v: u32) -> Option<Self> {
        Some(match v {
            1 => Self::NewHeight,
            2 => Self::NewRound,
            3 => Self::Propose,
            4 => Self::Prevote,
            5 => Self::PrevoteWait,
            6 => Self::Precommit,
            7 => Self::PrecommitWait,
            8 => Self::Commit,
            _ => return None,
        })
    }

    /// Wire value.
    #[must_use]
    pub fn as_u32(self) -> u32 {
        self as u32
    }

    /// `RoundStepType.String`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NewHeight => "RoundStepNewHeight",
            Self::NewRound => "RoundStepNewRound",
            Self::Propose => "RoundStepPropose",
            Self::Prevote => "RoundStepPrevote",
            Self::PrevoteWait => "RoundStepPrevoteWait",
            Self::Precommit => "RoundStepPrecommit",
            Self::PrecommitWait => "RoundStepPrecommitWait",
            Self::Commit => "RoundStepCommit",
        }
    }

    /// Parses [`Self::as_str`].
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        (1..=8)
            .filter_map(Self::from_u32)
            .find(|st| st.as_str() == s)
    }
}

/// `RoundState`.
#[derive(Debug, Clone)]
pub struct RoundState {
    /// Height being decided.
    pub height: i64,
    /// Round.
    pub round: i32,
    /// Step.
    pub step: RoundStep,
    /// When round 0 starts (`CommitTime + timeoutCommit`).
    pub start_time: Time,
    /// When +2/3 precommits were seen (zero when not yet).
    pub commit_time: Time,
    /// Validators (with proposer priorities for `round`).
    pub validators: ValidatorSet,
    /// Proposal of this round.
    pub proposal: Option<Proposal>,
    /// The proposal block once complete.
    pub proposal_block: Option<Block>,
    /// Parts of the proposal block.
    pub proposal_block_parts: Option<PartSet>,
    /// Round we are locked at (-1 if none).
    pub locked_round: i32,
    /// Locked block.
    pub locked_block: Option<Block>,
    /// Locked block parts.
    pub locked_block_parts: Option<PartSet>,
    /// Last round with a polka for a block we have (-1 if none).
    pub valid_round: i32,
    /// That block.
    pub valid_block: Option<Block>,
    /// Its parts.
    pub valid_block_parts: Option<PartSet>,
    /// Votes of this height.
    pub votes: HeightVoteSet,
    /// Round of the commit (-1 until committing).
    pub commit_round: i32,
    /// Precommits of the previous height.
    pub last_commit: Option<VoteSet>,
    /// Validators of the previous height.
    pub last_validators: ValidatorSet,
    /// Whether the precommit timeout was triggered this round.
    pub triggered_timeout_precommit: bool,
}

/// `timeoutInfo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeoutInfo {
    /// Duration (Go `time.Duration` nanoseconds).
    pub duration: i64,
    /// Height.
    pub height: i64,
    /// Round.
    pub round: i32,
    /// Step.
    pub step: RoundStep,
}

impl TimeoutInfo {
    /// `cmtcons.TimeoutInfo`.
    #[must_use]
    pub fn to_proto(&self) -> pbc::TimeoutInfo {
        pbc::TimeoutInfo {
            duration: Some(ctypes::time::duration_to_proto(self.duration)),
            height: self.height,
            round: self.round,
            step: self.step.as_u32(),
        }
    }

    /// From the proto (invalid step = `None`).
    #[must_use]
    pub fn from_proto(p: &pbc::TimeoutInfo) -> Option<Self> {
        Some(Self {
            duration: ctypes::time::duration_from_proto(p.duration.as_ref()).ok()?,
            height: p.height,
            round: p.round,
            step: RoundStep::from_u32(p.step)?,
        })
    }
}

/// A consensus reactor message (`tendermint.consensus.Message`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsensusMessage {
    /// `NewRoundStepMessage`.
    NewRoundStep {
        /// Height.
        height: i64,
        /// Round.
        round: i32,
        /// Step.
        step: RoundStep,
        /// Seconds since round start.
        seconds_since_start_time: i64,
        /// Round of the last commit.
        last_commit_round: i32,
    },
    /// `NewValidBlockMessage`.
    NewValidBlock {
        /// Height.
        height: i64,
        /// Round.
        round: i32,
        /// Part set header.
        block_part_set_header: PartSetHeader,
        /// Parts the sender has.
        block_parts: BitArray,
        /// Whether this is a commit.
        is_commit: bool,
    },
    /// `ProposalMessage`.
    Proposal(Proposal),
    /// `ProposalPOLMessage`.
    ProposalPol {
        /// Height.
        height: i64,
        /// POL round.
        proposal_pol_round: i32,
        /// Prevotes the sender has for that round.
        proposal_pol: BitArray,
    },
    /// `BlockPartMessage`.
    BlockPart {
        /// Height.
        height: i64,
        /// Round.
        round: i32,
        /// Part.
        part: Part,
    },
    /// `VoteMessage`.
    Vote(Vote),
    /// `HasVoteMessage`.
    HasVote {
        /// Height.
        height: i64,
        /// Round.
        round: i32,
        /// Vote type.
        vote_type: i32,
        /// Validator index.
        index: i32,
    },
    /// `VoteSetMaj23Message`.
    VoteSetMaj23 {
        /// Height.
        height: i64,
        /// Round.
        round: i32,
        /// Vote type.
        vote_type: i32,
        /// Claimed majority.
        block_id: BlockId,
    },
    /// `VoteSetBitsMessage`.
    VoteSetBits {
        /// Height.
        height: i64,
        /// Round.
        round: i32,
        /// Vote type.
        vote_type: i32,
        /// Block.
        block_id: BlockId,
        /// Votes the sender has.
        votes: BitArray,
    },
}

impl ConsensusMessage {
    /// `MsgToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pbc::Message {
        use pbc::message::Sum;
        let sum = match self {
            Self::NewRoundStep {
                height,
                round,
                step,
                seconds_since_start_time,
                last_commit_round,
            } => Sum::NewRoundStep(pbc::NewRoundStep {
                height: *height,
                round: *round,
                step: step.as_u32(),
                seconds_since_start_time: *seconds_since_start_time,
                last_commit_round: *last_commit_round,
            }),
            Self::NewValidBlock {
                height,
                round,
                block_part_set_header,
                block_parts,
                is_commit,
            } => Sum::NewValidBlock(pbc::NewValidBlock {
                height: *height,
                round: *round,
                block_part_set_header: Some(block_part_set_header.to_proto()),
                block_parts: Some(block_parts.to_proto()),
                is_commit: *is_commit,
            }),
            Self::Proposal(p) => Sum::Proposal(pbc::Proposal {
                proposal: Some(p.to_proto()),
            }),
            Self::ProposalPol {
                height,
                proposal_pol_round,
                proposal_pol,
            } => Sum::ProposalPol(pbc::ProposalPol {
                height: *height,
                proposal_pol_round: *proposal_pol_round,
                proposal_pol: Some(proposal_pol.to_proto()),
            }),
            Self::BlockPart {
                height,
                round,
                part,
            } => Sum::BlockPart(pbc::BlockPart {
                height: *height,
                round: *round,
                part: Some(part.to_proto()),
            }),
            Self::Vote(v) => Sum::Vote(pbc::Vote {
                vote: Some(v.to_proto()),
            }),
            Self::HasVote {
                height,
                round,
                vote_type,
                index,
            } => Sum::HasVote(pbc::HasVote {
                height: *height,
                round: *round,
                r#type: *vote_type,
                index: *index,
            }),
            Self::VoteSetMaj23 {
                height,
                round,
                vote_type,
                block_id,
            } => Sum::VoteSetMaj23(pbc::VoteSetMaj23 {
                height: *height,
                round: *round,
                r#type: *vote_type,
                block_id: Some(block_id.to_proto()),
            }),
            Self::VoteSetBits {
                height,
                round,
                vote_type,
                block_id,
                votes,
            } => Sum::VoteSetBits(pbc::VoteSetBits {
                height: *height,
                round: *round,
                r#type: *vote_type,
                block_id: Some(block_id.to_proto()),
                votes: Some(votes.to_proto()),
            }),
        };
        pbc::Message { sum: Some(sum) }
    }

    /// `MsgFromProto` (plus `ValidateBasic` of the embedded types).
    pub fn from_proto(m: &pbc::Message) -> Result<Self, String> {
        use pbc::message::Sum;
        let e = |x: ctypes::Error| x.to_string();
        let msg = match m.sum.as_ref().ok_or("nil message")? {
            Sum::NewRoundStep(p) => Self::NewRoundStep {
                height: p.height,
                round: p.round,
                step: RoundStep::from_u32(p.step).ok_or("invalid Step")?,
                seconds_since_start_time: p.seconds_since_start_time,
                last_commit_round: p.last_commit_round,
            },
            Sum::NewValidBlock(p) => {
                if let Some(b) = &p.block_parts {
                    BitArray::validate_proto(b)
                        .map_err(|e| format!("validating BlockParts: {e}"))?;
                }
                Self::NewValidBlock {
                    height: p.height,
                    round: p.round,
                    block_part_set_header: PartSetHeader::from_proto(
                        &p.block_part_set_header.clone().unwrap_or_default(),
                    )
                    .map_err(e)?,
                    block_parts: p
                        .block_parts
                        .as_ref()
                        .map(BitArray::from_proto)
                        .unwrap_or_default(),
                    is_commit: p.is_commit,
                }
            }
            Sum::Proposal(p) => Self::Proposal(
                Proposal::from_proto(p.proposal.as_ref().ok_or("nil proposal")?).map_err(e)?,
            ),
            Sum::ProposalPol(p) => {
                if let Some(b) = &p.proposal_pol {
                    BitArray::validate_proto(b)
                        .map_err(|e| format!("validating ProposalPOL: {e}"))?;
                }
                Self::ProposalPol {
                    height: p.height,
                    proposal_pol_round: p.proposal_pol_round,
                    proposal_pol: p
                        .proposal_pol
                        .as_ref()
                        .map(BitArray::from_proto)
                        .unwrap_or_default(),
                }
            }
            Sum::BlockPart(p) => Self::BlockPart {
                height: p.height,
                round: p.round,
                part: Part::from_proto(p.part.as_ref().ok_or("nil part")?).map_err(e)?,
            },
            Sum::Vote(p) => {
                Self::Vote(Vote::from_proto(p.vote.as_ref().ok_or("nil vote")?).map_err(e)?)
            }
            Sum::HasVote(p) => Self::HasVote {
                height: p.height,
                round: p.round,
                vote_type: p.r#type,
                index: p.index,
            },
            Sum::VoteSetMaj23(p) => Self::VoteSetMaj23 {
                height: p.height,
                round: p.round,
                vote_type: p.r#type,
                block_id: BlockId::from_proto(&p.block_id.clone().unwrap_or_default())
                    .map_err(e)?,
            },
            Sum::VoteSetBits(p) => {
                if let Some(b) = &p.votes {
                    BitArray::validate_proto(b).map_err(|e| format!("validating Votes: {e}"))?;
                }
                Self::VoteSetBits {
                    height: p.height,
                    round: p.round,
                    vote_type: p.r#type,
                    block_id: BlockId::from_proto(&p.block_id.clone().unwrap_or_default())
                        .map_err(e)?,
                    votes: p
                        .votes
                        .as_ref()
                        .map(BitArray::from_proto)
                        .unwrap_or_default(),
                }
            }
        };
        Ok(msg)
    }

    /// `ValidateBasic` of the reactor message (the embedded proposal,
    /// part and vote are validated when decoded).
    pub fn validate_basic(&self) -> Result<(), String> {
        use ctypes::commit::MAX_VOTES_COUNT;
        use ctypes::part_set::MAX_BLOCK_PARTS_COUNT;
        let valid_type =
            |t: i32| t == ctypes::vote::PREVOTE_TYPE || t == ctypes::vote::PRECOMMIT_TYPE;
        match self {
            Self::NewRoundStep {
                height,
                round,
                last_commit_round,
                ..
            } => {
                if *height < 0 {
                    return Err("negative Height".into());
                }
                if *round < 0 {
                    return Err("negative Round".into());
                }
                if *last_commit_round < -1 {
                    return Err("invalid LastCommitRound (cannot be < -1)".into());
                }
            }
            Self::NewValidBlock {
                height,
                round,
                block_part_set_header,
                block_parts,
                ..
            } => {
                if *height < 0 {
                    return Err("negative Height".into());
                }
                if *round < 0 {
                    return Err("negative Round".into());
                }
                block_part_set_header
                    .validate_basic()
                    .map_err(|e| format!("wrong BlockPartSetHeader: {e}"))?;
                if block_parts.size() == 0 {
                    return Err("empty blockParts".into());
                }
                if block_parts.size() != block_part_set_header.total as usize {
                    return Err(format!(
                        "blockParts bit array size {} not equal to BlockPartSetHeader.Total {}",
                        block_parts.size(),
                        block_part_set_header.total
                    ));
                }
                if block_parts.size() > MAX_BLOCK_PARTS_COUNT as usize {
                    return Err(format!(
                        "blockParts bit array is too big: {}, max: {MAX_BLOCK_PARTS_COUNT}",
                        block_parts.size()
                    ));
                }
            }
            Self::Proposal(_) | Self::BlockPart { .. } => {
                if let Self::BlockPart { height, round, .. } = self {
                    if *height < 0 {
                        return Err("negative Height".into());
                    }
                    if *round < 0 {
                        return Err("negative Round".into());
                    }
                }
            }
            Self::ProposalPol {
                height,
                proposal_pol_round,
                proposal_pol,
            } => {
                if *height < 0 {
                    return Err("negative Height".into());
                }
                if *proposal_pol_round < 0 {
                    return Err("negative ProposalPOLRound".into());
                }
                if proposal_pol.size() == 0 {
                    return Err("empty ProposalPOL bit array".into());
                }
                if proposal_pol.size() > MAX_VOTES_COUNT {
                    return Err(format!(
                        "proposalPOL bit array is too big: {}, max: {MAX_VOTES_COUNT}",
                        proposal_pol.size()
                    ));
                }
            }
            Self::Vote(v) => v.validate_basic().map_err(|e| e.to_string())?,
            Self::HasVote {
                height,
                round,
                vote_type,
                index,
            } => {
                if *height < 0 {
                    return Err("negative Height".into());
                }
                if *round < 0 {
                    return Err("negative Round".into());
                }
                if !valid_type(*vote_type) {
                    return Err("invalid Type".into());
                }
                if *index < 0 {
                    return Err("negative Index".into());
                }
            }
            Self::VoteSetMaj23 {
                height,
                round,
                vote_type,
                block_id,
            } => {
                if *height < 0 {
                    return Err("negative Height".into());
                }
                if *round < 0 {
                    return Err("negative Round".into());
                }
                if !valid_type(*vote_type) {
                    return Err("invalid Type".into());
                }
                block_id
                    .validate_basic()
                    .map_err(|e| format!("wrong BlockID: {e}"))?;
            }
            Self::VoteSetBits {
                height,
                vote_type,
                block_id,
                votes,
                ..
            } => {
                if *height < 0 {
                    return Err("negative Height".into());
                }
                if !valid_type(*vote_type) {
                    return Err("invalid Type".into());
                }
                block_id
                    .validate_basic()
                    .map_err(|e| format!("wrong BlockID: {e}"))?;
                if votes.size() > MAX_VOTES_COUNT {
                    return Err(format!(
                        "votes bit array is too big: {}, max: {MAX_VOTES_COUNT}",
                        votes.size()
                    ));
                }
            }
        }
        Ok(())
    }

    /// Encoded wire bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        self.to_proto().encode_to_vec()
    }

    /// Decodes wire bytes.
    pub fn decode(bz: &[u8]) -> Result<Self, String> {
        let m = pbc::Message::decode(bz).map_err(|e| e.to_string())?;
        Self::from_proto(&m)
    }
}

/// `CompareHRS`.
#[must_use]
pub fn compare_hrs(
    h1: i64,
    r1: i32,
    s1: RoundStep,
    h2: i64,
    r2: i32,
    s2: RoundStep,
) -> std::cmp::Ordering {
    (h1, r1, s1).cmp(&(h2, r2, s2))
}
