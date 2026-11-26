//! cosmos-sdk v0.46 gov votes of celestia-app v1 (`x/gov/types/v1beta1`
//! and `v1` `MsgVote.ValidateBasic`, `keeper/msg_server.go` `Vote` and
//! the legacy server wrapping it, `keeper/vote.go` `AddVote`): a vote on
//! a proposal that does not exist fails with `ErrUnknownProposal`. Votes
//! on an existing proposal, weighted votes, deposits and proposals are
//! not ported.

use celestia_proto::Any;
use celestia_proto::cosmos::gov::{v1, v1beta1};
use prost::{Message, Name};
use sdk::error::sdk as errs;
use sdk::{Context, Result, SdkError};

fn decode<M: Message + Default>(msg: &Any) -> Result<M> {
    M::decode(msg.value.as_slice()).map_err(|e| errs::tx_decode(e.to_string()))
}

/// `ValidVoteOption` (v1beta1 and v1 share the option numbers).
fn valid_vote_option(option: i32) -> bool {
    (1..=4).contains(&option)
}

/// `MsgVote.ValidateBasic` (both versions).
fn validate_vote(voter: &str, option: i32) -> Result<()> {
    super::msgs::acc_address(voter, "voter")?;
    if !valid_vote_option(option) {
        return Err(x_gov::errors::invalid_vote().wrap(option_string(option)));
    }
    Ok(())
}

/// Go's `VoteOption.String()`: the enum name, or the number when unknown.
fn option_string(option: i32) -> String {
    match option {
        0 => "VOTE_OPTION_UNSPECIFIED".to_owned(),
        1 => "VOTE_OPTION_YES".to_owned(),
        2 => "VOTE_OPTION_ABSTAIN".to_owned(),
        3 => "VOTE_OPTION_NO".to_owned(),
        4 => "VOTE_OPTION_NO_WITH_VETO".to_owned(),
        n => n.to_string(),
    }
}

/// `ErrUnknownProposal` (gov code 2).
fn unknown_proposal(id: u64) -> SdkError {
    SdkError::new("gov", 2, format!("{id}: unknown proposal"))
}

/// cosmos-sdk v0.46 `ProposalKey`: `0x00 | id (big endian)`.
fn proposal_key(id: u64) -> Vec<u8> {
    let mut k = vec![0x00];
    k.extend_from_slice(&id.to_be_bytes());
    k
}

/// v1 `msgServer.Vote` (the v1beta1 server converts and calls it):
/// `AddVote` reads the proposal first.
fn vote(ctx: &mut Context<'_>, proposal_id: u64, voter: &str) -> Result<()> {
    sdk::address::acc_from_bech32_go(voter).map_err(sdk::error::internal)?;
    if ctx
        .get(x_gov::STORE_KEY, &proposal_key(proposal_id))?
        .is_none()
    {
        return Err(unknown_proposal(proposal_id));
    }
    Err(super::not_ported(errs::not_supported(format!(
        "the cosmos-sdk v0.46 vote on existing proposal {proposal_id} is not ported"
    ))))
}

/// `ValidateBasic` of the gov votes; `None` for any other message.
#[must_use]
pub fn validate_basic(msg: &Any) -> Option<Result<()>> {
    Some(if msg.type_url == v1beta1::MsgVote::type_url() {
        decode::<v1beta1::MsgVote>(msg).and_then(|m| validate_vote(&m.voter, m.option))
    } else if msg.type_url == v1::MsgVote::type_url() {
        decode::<v1::MsgVote>(msg).and_then(|m| validate_vote(&m.voter, m.option))
    } else {
        return None;
    })
}

/// Executes a gov vote; `None` for any other message.
pub fn route(ctx: &mut Context<'_>, msg: &Any) -> Option<Result<Any>> {
    Some(if msg.type_url == v1beta1::MsgVote::type_url() {
        decode::<v1beta1::MsgVote>(msg).and_then(|m| {
            vote(ctx, m.proposal_id, &m.voter).map(|()| Any::pack(&v1beta1::MsgVoteResponse {}))
        })
    } else if msg.type_url == v1::MsgVote::type_url() {
        decode::<v1::MsgVote>(msg).and_then(|m| {
            vote(ctx, m.proposal_id, &m.voter).map(|()| Any::pack(&v1::MsgVoteResponse {}))
        })
    } else {
        return None;
    })
}
