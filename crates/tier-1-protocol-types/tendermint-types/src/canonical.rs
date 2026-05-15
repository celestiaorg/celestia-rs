//! Canonical sign-byte encoding helpers.

use crate::block::{BlockId, PartSetHeader};
use crate::error::{Error, Result};
use crate::proposal::Proposal;
use crate::proto;
use crate::vote::Vote;
use encoding::protobuf;
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

/// Return CometBFT's canonical timestamp string.
pub fn canonical_time(timestamp: &proto::google::Timestamp) -> Result<String> {
    if !(0..1_000_000_000).contains(&timestamp.nanos) {
        return Err(Error::InvalidTimestamp);
    }
    let date_time = OffsetDateTime::from_unix_timestamp(timestamp.seconds)
        .map_err(|_| Error::InvalidTimestamp)?
        .checked_add(Duration::nanoseconds(i64::from(timestamp.nanos)))
        .ok_or(Error::InvalidTimestamp)?;
    date_time
        .format(&Rfc3339)
        .map_err(|e| Error::Encode(e.to_string()))
}

/// Encode canonical vote sign bytes with the CometBFT length-delimited
/// protobuf wrapper.
pub fn vote_sign_bytes(chain_id: &str, vote: &Vote) -> Result<Vec<u8>> {
    let canonical = proto::types::CanonicalVote {
        r#type: vote.ty.into(),
        height: vote.height,
        round: i64::from(vote.round),
        block_id: canonical_block_id(&vote.block_id),
        timestamp: vote.timestamp,
        chain_id: chain_id.to_owned(),
    };
    Ok(protobuf::encode_length_delimited_to_vec(&canonical))
}

/// Encode canonical proposal sign bytes with the CometBFT length-delimited
/// protobuf wrapper.
pub fn proposal_sign_bytes(chain_id: &str, proposal: &Proposal) -> Result<Vec<u8>> {
    let canonical = proto::types::CanonicalProposal {
        r#type: crate::vote::SignedMsgType::Proposal.into(),
        height: proposal.height,
        round: i64::from(proposal.round),
        pol_round: i64::from(proposal.pol_round),
        block_id: canonical_block_id(&proposal.block_id),
        timestamp: proposal.timestamp,
        chain_id: chain_id.to_owned(),
    };
    Ok(protobuf::encode_length_delimited_to_vec(&canonical))
}

/// Encode canonical vote-extension sign bytes.
pub fn vote_extension_sign_bytes(chain_id: &str, vote: &Vote) -> Result<Vec<u8>> {
    let canonical = proto::types::CanonicalVoteExtension {
        extension: vote.extension.clone(),
        height: vote.height,
        round: i64::from(vote.round),
        chain_id: chain_id.to_owned(),
    };
    Ok(protobuf::encode_length_delimited_to_vec(&canonical))
}

fn canonical_block_id(block_id: &BlockId) -> Option<proto::types::CanonicalBlockId> {
    if block_id.is_zero() {
        return None;
    }
    Some(proto::types::CanonicalBlockId {
        hash: block_id.hash.map_or_else(Vec::new, |hash| hash.to_vec()),
        part_set_header: Some(canonical_part_set_header(&block_id.part_set_header)),
    })
}

fn canonical_part_set_header(
    part_set_header: &PartSetHeader,
) -> proto::types::CanonicalPartSetHeader {
    proto::types::CanonicalPartSetHeader {
        total: part_set_header.total,
        hash: part_set_header
            .hash
            .map_or_else(Vec::new, |hash| hash.to_vec()),
    }
}
