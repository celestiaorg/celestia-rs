//! `x/gov` v1 (celestiaorg/cosmos-sdk v0.52.12).
//!
//! Store layout (collections):
//! - `0x00 || u64(id)` → `Proposal`
//! - `0x01 || time(end) || u64(id)` → `u64(id)` (active proposals queue)
//! - `0x02 || time(end) || u64(id)` → `u64(id)` (inactive proposals queue)
//! - `0x03` → next proposal id (`Sequence`)
//! - `0x04 || u64(id)` → `[1]` (proposals in voting period)
//! - `0x10 || u64(id) || len || addr` → `Deposit`
//! - `0x20 || u64(id) || len || addr` → `Vote`
//! - `0x30` → `Params`, `0x31` → constitution (raw string)
//!
//! `time(t)` is `sdk.TimeKey` (`sdk.FormatTimeBytes`). Proposal tallying and
//! execution of passed proposals are not ported yet: [`end_block`] returns
//! a `not supported` error when a queue holds an expired proposal, so a
//! divergence is loud rather than silent.

use celestia_proto::cosmos::gov::v1::{
    Deposit, GenesisState, Params, Proposal, ProposalStatus, Vote,
};
use pbjson_types::Timestamp;
use prost::Message;
use sdk::address::{acc_from_bech32, undefined};
use sdk::codec::{length_prefixed, u64_key};
use sdk::coins::Coins;
use sdk::error::sdk as sdkerr;
use sdk::time::format_time_bytes;
use sdk::{Context, Result};

/// Store key.
pub const STORE_KEY: &str = "gov";
/// Module name.
pub const MODULE_NAME: &str = "gov";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 5;
/// `ProposalsKeyPrefix`.
pub const PROPOSALS_PREFIX: u8 = 0x00;
/// `ActiveProposalQueuePrefix`.
pub const ACTIVE_QUEUE_PREFIX: u8 = 0x01;
/// `InactiveProposalQueuePrefix`.
pub const INACTIVE_QUEUE_PREFIX: u8 = 0x02;
/// `ProposalIDKey`.
pub const PROPOSAL_ID_KEY: u8 = 0x03;
/// `VotingPeriodProposalKeyPrefix`.
pub const VOTING_PERIOD_PROPOSAL_PREFIX: u8 = 0x04;
/// `DepositsKeyPrefix`.
pub const DEPOSITS_PREFIX: u8 = 0x10;
/// `VotesKeyPrefix`.
pub const VOTES_PREFIX: u8 = 0x20;
/// `ParamsKey`.
pub const PARAMS_KEY: u8 = 0x30;
/// `ConstitutionKey`.
pub const CONSTITUTION_KEY: u8 = 0x31;

fn decode<M: Message + Default>(bz: &[u8]) -> Result<M> {
    M::decode(bz).map_err(|e| undefined(e.to_string()))
}

/// `ProposalID.Set`.
pub fn set_proposal_id(ctx: &mut Context<'_>, id: u64) -> Result<()> {
    ctx.set(STORE_KEY, vec![PROPOSAL_ID_KEY], u64_key(id).to_vec())
}

/// `ProposalID.Peek` (0 when unset).
pub fn peek_proposal_id(ctx: &mut Context<'_>) -> Result<u64> {
    Ok(ctx
        .get(STORE_KEY, &[PROPOSAL_ID_KEY])?
        .and_then(|b| sdk::codec::decode_u64(&b))
        .unwrap_or(0))
}

/// `Params.Set`.
pub fn set_params(ctx: &mut Context<'_>, p: &Params) -> Result<()> {
    ctx.set(STORE_KEY, vec![PARAMS_KEY], p.encode_to_vec())
}

/// `Params.Get`.
pub fn get_params(ctx: &mut Context<'_>) -> Result<Params> {
    let bz = ctx.get(STORE_KEY, &[PARAMS_KEY])?.ok_or_else(|| {
        sdkerr::not_found("collections: not found: key 'no_key' of type v1.Params")
    })?;
    decode(&bz)
}

/// `Constitution.Set`.
pub fn set_constitution(ctx: &mut Context<'_>, c: &str) -> Result<()> {
    ctx.set(STORE_KEY, vec![CONSTITUTION_KEY], c.as_bytes().to_vec())
}

fn pair_addr_key(prefix: u8, id: u64, addr: &[u8]) -> Vec<u8> {
    let mut k = vec![prefix];
    k.extend_from_slice(&u64_key(id));
    k.extend_from_slice(&length_prefixed(addr));
    k
}

/// `SetDeposit`.
pub fn set_deposit(ctx: &mut Context<'_>, d: &Deposit) -> Result<()> {
    let addr = acc_from_bech32(&d.depositor)?;
    ctx.set(
        STORE_KEY,
        pair_addr_key(DEPOSITS_PREFIX, d.proposal_id, &addr),
        d.encode_to_vec(),
    )
}

/// `Votes.Set`.
pub fn set_vote(ctx: &mut Context<'_>, v: &Vote) -> Result<()> {
    let addr = acc_from_bech32(&v.voter)?;
    ctx.set(
        STORE_KEY,
        pair_addr_key(VOTES_PREFIX, v.proposal_id, &addr),
        v.encode_to_vec(),
    )
}

fn queue_key(prefix: u8, end: &Timestamp, id: u64) -> Vec<u8> {
    let mut k = vec![prefix];
    k.extend_from_slice(&format_time_bytes(end));
    k.extend_from_slice(&u64_key(id));
    k
}

/// `ActiveProposalsQueue.Set` / `InactiveProposalsQueue.Set`.
pub fn insert_queue(ctx: &mut Context<'_>, prefix: u8, end: &Timestamp, id: u64) -> Result<()> {
    ctx.set(STORE_KEY, queue_key(prefix, end, id), u64_key(id).to_vec())
}

/// `SetProposal` (also maintains the voting-period index).
pub fn set_proposal(ctx: &mut Context<'_>, p: &Proposal) -> Result<()> {
    let mut vk = vec![VOTING_PERIOD_PROPOSAL_PREFIX];
    vk.extend_from_slice(&u64_key(p.id));
    if p.status == ProposalStatus::VotingPeriod as i32 {
        ctx.set(STORE_KEY, vk, vec![1])?;
    } else {
        ctx.delete(STORE_KEY, &vk)?;
    }
    let mut k = vec![PROPOSALS_PREFIX];
    k.extend_from_slice(&u64_key(p.id));
    ctx.set(STORE_KEY, k, p.encode_to_vec())
}

/// `InitGenesis`.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &GenesisState) -> Result<()> {
    set_proposal_id(ctx, gs.starting_proposal_id)?;
    // Go dereferences `*data.Params` (panics when nil).
    let params = gs
        .params
        .clone()
        .ok_or_else(|| sdkerr::logic("gov genesis params are nil"))?;
    set_params(ctx, &params)?;
    set_constitution(ctx, &gs.constitution)?;

    let module_acc = x_auth::get_module_account(ctx, MODULE_NAME)?
        .ok_or_else(|| sdkerr::logic(format!("{MODULE_NAME} module account has not been set")))?;

    let mut total = Coins::new();
    for d in &gs.deposits {
        set_deposit(ctx, d)?;
        total = total.add(&Coins::from_proto(&d.amount)?)?;
    }
    for v in &gs.votes {
        set_vote(ctx, v)?;
    }
    for p in &gs.proposals {
        if p.status == ProposalStatus::DepositPeriod as i32 {
            let end = p
                .deposit_end_time
                .ok_or_else(|| sdkerr::logic("proposal deposit end time is nil"))?;
            insert_queue(ctx, INACTIVE_QUEUE_PREFIX, &end, p.id)?;
        } else if p.status == ProposalStatus::VotingPeriod as i32 {
            let end = p
                .voting_end_time
                .ok_or_else(|| sdkerr::logic("proposal voting end time is nil"))?;
            insert_queue(ctx, ACTIVE_QUEUE_PREFIX, &end, p.id)?;
        }
        set_proposal(ctx, p)?;
    }

    let acc = x_auth::Account::Module(module_acc);
    let addr = acc.address()?;
    let balance = x_bank::get_all_balances(ctx, &addr)?;
    if balance.is_empty() {
        x_auth::set_account(ctx, &acc)?;
    }
    if balance != total {
        return Err(sdkerr::logic(format!(
            "expected module account was {balance} but we got {total}"
        )));
    }
    Ok(())
}

/// `InitGenesis` from the module's genesis JSON.
pub fn init_genesis_json(ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    let gs: GenesisState = serde_json::from_value(genesis.clone())
        .map_err(|e| sdkerr::json_unmarshal(e.to_string()))?;
    init_genesis(ctx, &gs)
}

/// Entries of a proposal queue whose end time is `<= until`
/// (`collections.NewPrefixUntilPairRange`).
pub fn expired_queue_entries(
    ctx: &mut Context<'_>,
    prefix: u8,
    until: &Timestamp,
) -> Result<Vec<(Vec<u8>, u64)>> {
    let mut upto = vec![prefix];
    upto.extend_from_slice(&format_time_bytes(until));
    let end = sdk::context::prefix_end(&upto);
    let mut it = ctx.iterator(STORE_KEY, Some(&[prefix]), end.as_deref())?;
    let mut out = Vec::new();
    while let Some((k, v)) = it.next(ctx)? {
        let id = sdk::codec::decode_u64(&v).ok_or_else(|| undefined("invalid queue value"))?;
        out.push((k, id));
    }
    Ok(out)
}

/// `EndBlocker`: walks the inactive and active proposal queues up to the
/// block time. Processing an expired proposal (refund/burn deposits,
/// tally, executing messages) is not ported; it is reported as an error.
pub fn end_block(ctx: &mut Context<'_>) -> Result<()> {
    let now = ctx.header.time;
    for (prefix, what) in [
        (INACTIVE_QUEUE_PREFIX, "inactive"),
        (ACTIVE_QUEUE_PREFIX, "active"),
    ] {
        if let Some((_, id)) = expired_queue_entries(ctx, prefix, &now)?.first() {
            return Err(sdkerr::not_supported(format!(
                "gov: processing {what} proposal {id} is not implemented"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::{ExecMode, Header, Kind, MultiStore};

    fn ms() -> MultiStore {
        let mut ms = MultiStore::new();
        for s in [STORE_KEY, x_auth::STORE_KEY, x_bank::STORE_KEY] {
            ms.mount(s, Kind::Iavl).unwrap_or_default();
        }
        ms
    }

    fn hex(b: &[u8]) -> String {
        b.iter().fold(String::new(), |mut s, x| {
            use std::fmt::Write as _;
            let _ = write!(s, "{x:02x}");
            s
        })
    }

    fn devnet_genesis() -> serde_json::Value {
        serde_json::json!({"starting_proposal_id": "1", "deposits": [], "votes": [], "proposals": [],
            "deposit_params": null, "voting_params": null, "tally_params": null,
            "params": {"min_deposit": [{"denom": "utia", "amount": "10000000000"}],
                "max_deposit_period": "604800s", "voting_period": "604800s",
                "quorum": "0.334000000000000000", "threshold": "0.500000000000000000",
                "veto_threshold": "0.334000000000000000",
                "min_initial_deposit_ratio": "0.000000000000000000",
                "proposal_cancel_ratio": "0.500000000000000000", "proposal_cancel_dest": "",
                "expedited_voting_period": "86400s", "expedited_threshold": "0.667000000000000000",
                "expedited_min_deposit": [{"denom": "utia", "amount": "50000000000"}],
                "burn_vote_quorum": false, "burn_proposal_deposit_prevote": false,
                "burn_vote_veto": true, "min_deposit_ratio": "0.010000000000000000"},
            "constitution": ""})
    }

    #[test]
    fn genesis_matches_devnet() -> Result<()> {
        let mut ms = ms();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        init_genesis_json(&mut ctx, &devnet_genesis())?;
        assert_eq!(
            ctx.get(STORE_KEY, &[0x03])?,
            Some(vec![0, 0, 0, 0, 0, 0, 0, 1])
        );
        assert_eq!(ctx.get(STORE_KEY, &[0x31])?, Some(vec![]));
        assert_eq!(
            hex(&ctx.get(STORE_KEY, &[0x30])?.unwrap_or_default()),
            "0a130a0475746961120b313030303030303030303012040880f5241a040880f5242214302e3333343030303030303030303030303030302a14302e3530303030303030303030303030303030303214302e3333343030303030303030303030303030303a14302e3030303030303030303030303030303030304214302e35303030303030303030303030303030303052040880a3055a14302e36363730303030303030303030303030303062130a0475746961120b35303030303030303030307801820114302e303130303030303030303030303030303030"
        );
        assert!(x_auth::get_account(&mut ctx, &x_auth::module_address(MODULE_NAME))?.is_some());
        assert_eq!(peek_proposal_id(&mut ctx)?, 1);
        assert_eq!(get_params(&mut ctx)?.quorum, "0.334000000000000000");
        end_block(&mut ctx)?;
        Ok(())
    }

    #[test]
    fn queues_and_end_block() -> Result<()> {
        let mut ms = ms();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let end = Timestamp {
            seconds: 100,
            nanos: 0,
        };
        let mut g = devnet_genesis();
        g["proposals"] = serde_json::json!([{
            "id": "1", "messages": [], "status": "PROPOSAL_STATUS_VOTING_PERIOD",
            "voting_end_time": "1970-01-01T00:01:40Z", "total_deposit": []
        }]);
        init_genesis_json(&mut ctx, &g)?;
        let mut qk = vec![ACTIVE_QUEUE_PREFIX];
        qk.extend_from_slice(b"1970-01-01T00:01:40.000000000");
        qk.extend_from_slice(&u64_key(1));
        assert_eq!(ctx.get(STORE_KEY, &qk)?, Some(u64_key(1).to_vec()));
        assert_eq!(
            ctx.get(STORE_KEY, &[0x04, 0, 0, 0, 0, 0, 0, 0, 1])?,
            Some(vec![1])
        );
        // Before the end time nothing expires.
        end_block(&mut ctx)?;
        ctx.header.time = end;
        assert_eq!(
            expired_queue_entries(&mut ctx, ACTIVE_QUEUE_PREFIX, &end)?.len(),
            1
        );
        assert!(end_block(&mut ctx).is_err());
        Ok(())
    }

    #[test]
    fn deposit_keys_are_length_prefixed() -> Result<()> {
        let mut ms = ms();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let addr = [9u8; 20];
        set_deposit(
            &mut ctx,
            &Deposit {
                proposal_id: 2,
                depositor: sdk::address::to_bech32("celestia", &addr),
                amount: vec![],
            },
        )?;
        let mut k = vec![0x10, 0, 0, 0, 0, 0, 0, 0, 2, 20];
        k.extend_from_slice(&addr);
        assert!(ctx.has(STORE_KEY, &k)?);
        Ok(())
    }
}
