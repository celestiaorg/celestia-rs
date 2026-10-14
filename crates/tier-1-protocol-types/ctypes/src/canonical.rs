//! Canonical sign-bytes encodings (`types/canonical.go`, `VoteSignBytes`,
//! `VoteExtensionSignBytes`, `ProposalSignBytes`).

use crate::block_id::BlockId;
use crate::error::Result;
use crate::proto::types as pb;
use crate::vote::PROPOSAL_TYPE;
use prost::Message;

/// `CanonicalizeBlockID`: `None` for the nil block. Go panics when the
/// block id is invalid; this returns that error.
pub fn canonicalize_block_id(bid: Option<&pb::BlockId>) -> Result<Option<pb::CanonicalBlockId>> {
    let default_id = pb::BlockId::default();
    let bid = bid.unwrap_or(&default_id);
    let rbid = BlockId::from_proto(bid)?;
    if rbid.is_zero() {
        return Ok(None);
    }
    let psh = bid.part_set_header.clone().unwrap_or_default();
    Ok(Some(pb::CanonicalBlockId {
        hash: bid.hash.clone(),
        part_set_header: Some(pb::CanonicalPartSetHeader {
            total: psh.total,
            hash: psh.hash,
        }),
    }))
}

/// `CanonicalizeVote`.
pub fn canonicalize_vote(chain_id: &str, vote: &pb::Vote) -> Result<pb::CanonicalVote> {
    Ok(pb::CanonicalVote {
        r#type: vote.r#type,
        height: vote.height,
        round: i64::from(vote.round),
        block_id: canonicalize_block_id(vote.block_id.as_ref())?,
        timestamp: Some(vote.timestamp.unwrap_or(crate::time::Time::ZERO.to_proto())),
        chain_id: chain_id.to_owned(),
    })
}

/// `CanonicalizeVoteExtension`.
#[must_use]
pub fn canonicalize_vote_extension(chain_id: &str, vote: &pb::Vote) -> pb::CanonicalVoteExtension {
    pb::CanonicalVoteExtension {
        extension: vote.extension.clone(),
        height: vote.height,
        round: i64::from(vote.round),
        chain_id: chain_id.to_owned(),
    }
}

/// `CanonicalizeProposal`.
pub fn canonicalize_proposal(chain_id: &str, p: &pb::Proposal) -> Result<pb::CanonicalProposal> {
    Ok(pb::CanonicalProposal {
        r#type: PROPOSAL_TYPE,
        height: p.height,
        round: i64::from(p.round),
        pol_round: i64::from(p.pol_round),
        block_id: canonicalize_block_id(p.block_id.as_ref())?,
        timestamp: Some(p.timestamp.unwrap_or(crate::time::Time::ZERO.to_proto())),
        chain_id: chain_id.to_owned(),
    })
}

/// `protoio.MarshalDelimited`.
fn delimited(m: &impl Message) -> Vec<u8> {
    m.encode_length_delimited_to_vec()
}

/// `VoteSignBytes`: length-delimited `CanonicalVote`.
pub fn vote_sign_bytes(chain_id: &str, vote: &pb::Vote) -> Result<Vec<u8>> {
    Ok(delimited(&canonicalize_vote(chain_id, vote)?))
}

/// `VoteExtensionSignBytes`: length-delimited `CanonicalVoteExtension`.
#[must_use]
pub fn vote_extension_sign_bytes(chain_id: &str, vote: &pb::Vote) -> Vec<u8> {
    delimited(&canonicalize_vote_extension(chain_id, vote))
}

/// `ProposalSignBytes`: length-delimited `CanonicalProposal`.
pub fn proposal_sign_bytes(chain_id: &str, p: &pb::Proposal) -> Result<Vec<u8>> {
    Ok(delimited(&canonicalize_proposal(chain_id, p)?))
}
