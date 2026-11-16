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
//! `time(t)` is `sdk.TimeKey` (`sdk.FormatTimeBytes`).
//!
//! Passed proposals execute their messages through the app's message router
//! ([`keeper::MsgRouter`]). celestia registers an empty legacy (v1beta1)
//! content router, so `MsgExecLegacyContent` always fails with
//! `ErrNoProposalHandlerExists`.

pub mod abci;
pub mod errors;
pub mod keeper;
pub mod msgs;
pub mod tally;

pub use abci::end_block;
pub use keeper::MsgRouter;

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
    ctx.set(
        STORE_KEY,
        vec![CONSTITUTION_KEY],
        gostr::to_go_bytes(c).into_owned(),
    )
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

/// Proposal queue key: `prefix || time(end) || u64(id)`.
#[must_use]
pub fn queue_key(prefix: u8, end: &Timestamp, id: u64) -> Vec<u8> {
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

/// `ExportGenesis`: starting proposal id, deposits, votes, proposals
/// (store order), params and constitution.
pub fn export_genesis(ctx: &mut Context<'_>) -> Result<GenesisState> {
    fn all<M: Message + Default>(ctx: &mut Context<'_>, prefix: u8) -> Result<Vec<M>> {
        let mut it = ctx.prefix_iter(STORE_KEY, &[prefix])?;
        let mut out = Vec::new();
        while let Some((_, v)) = it.next(ctx)? {
            out.push(decode(&v)?);
        }
        Ok(out)
    }
    let constitution = ctx
        .get(STORE_KEY, &[CONSTITUTION_KEY])?
        .map(|b| gostr::from_go_bytes(&b).into_owned())
        .unwrap_or_default();
    Ok(GenesisState {
        starting_proposal_id: peek_proposal_id(ctx)?,
        deposits: all(ctx, DEPOSITS_PREFIX)?,
        votes: all(ctx, VOTES_PREFIX)?,
        proposals: all(ctx, PROPOSALS_PREFIX)?,
        params: Some(get_params(ctx)?),
        constitution,
        ..GenesisState::default()
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::coins::Coin;
    use sdk::{ExecMode, Header, Kind, MultiStore, SdkError};

    struct NoRouter;

    impl MsgRouter for NoRouter {
        fn has_handler(&self, _: &str) -> bool {
            false
        }
        fn validate_basic(&self, _: &celestia_proto::Any) -> Result<()> {
            Ok(())
        }
        fn handle(
            &self,
            _: &mut Context<'_>,
            _: &celestia_proto::Any,
        ) -> Result<celestia_proto::Any> {
            Err(sdkerr::unknown_request(""))
        }
    }

    fn ms() -> MultiStore {
        let mut ms = MultiStore::new();
        for s in [
            STORE_KEY,
            x_auth::STORE_KEY,
            x_bank::STORE_KEY,
            x_staking::STORE_KEY,
        ] {
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
        end_block(&mut ctx, &NoRouter)?;
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
        end_block(&mut ctx, &NoRouter)?;
        ctx.header.time = end;
        assert_eq!(
            expired_queue_entries(&mut ctx, ACTIVE_QUEUE_PREFIX, &end)?.len(),
            1
        );
        // No bonded tokens: the proposal is rejected and leaves the queue.
        end_block(&mut ctx, &NoRouter)?;
        assert_eq!(ctx.get(STORE_KEY, &qk)?, None);
        let p = keeper::must_get_proposal(&mut ctx, 1)?;
        assert_eq!(p.status, ProposalStatus::Rejected as i32);
        assert_eq!(p.failed_reason, "proposal did not get enough votes to pass");
        assert_eq!(ctx.get(STORE_KEY, &[0x04, 0, 0, 0, 0, 0, 0, 0, 1])?, None);
        Ok(())
    }

    fn unsupported_proposal(id: u64, end: Timestamp, active: bool) -> Proposal {
        Proposal {
            id,
            messages: vec![celestia_proto::Any {
                type_url: "/review.Unregistered".to_owned(),
                value: vec![],
            }],
            status: if active {
                ProposalStatus::VotingPeriod as i32
            } else {
                ProposalStatus::DepositPeriod as i32
            },
            final_tally_result: Some(keeper::empty_tally()),
            submit_time: Some(Timestamp {
                seconds: 10,
                nanos: 0,
            }),
            deposit_end_time: Some(end),
            total_deposit: vec![],
            voting_start_time: active.then_some(Timestamp {
                seconds: 20,
                nanos: 0,
            }),
            voting_end_time: active.then_some(end),
            metadata: "retained metadata".to_owned(),
            title: "retained title".to_owned(),
            summary: "retained summary".to_owned(),
            proposer: sdk::address::to_bech32("celestia", &[7; 20]),
            expedited: true,
            failed_reason: "old reason".to_owned(),
        }
    }

    fn seed_deposit(ctx: &mut Context<'_>, id: u64, depositor: &[u8]) -> Result<()> {
        x_bank::set_balance(ctx, &keeper::module_address(), &Coin::new("utia", 12))?;
        set_deposit(
            ctx,
            &Deposit {
                proposal_id: id,
                depositor: sdk::address::to_bech32("celestia", depositor),
                amount: vec![celestia_proto::cosmos::base::v1beta1::Coin {
                    denom: "utia".to_owned(),
                    amount: "12".to_owned(),
                }],
            },
        )
    }

    fn assert_failed_event(ctx: &Context<'_>, active: bool, id: u64) {
        let event = ctx.events.events().last().expect("failed event");
        assert_eq!(
            event.r#type,
            if active {
                "active_proposal"
            } else {
                "inactive_proposal"
            }
        );
        assert_eq!(event.attributes[0].key, "proposal_id");
        assert_eq!(event.attributes[0].value, id.to_string());
        assert_eq!(event.attributes[1].key, "proposal_result");
        assert_eq!(event.attributes[1].value, "proposal_failed");
    }

    #[test]
    fn active_unregistered_message_fails_and_preserves_proposal() -> Result<()> {
        let mut ms = ms();
        let end = Timestamp {
            seconds: 100,
            nanos: 0,
        };
        let mut ctx = Context::new(
            &mut ms,
            Header {
                time: Timestamp {
                    seconds: 100,
                    nanos: 0,
                },
                ..Header::default()
            },
            ExecMode::Finalize,
        );
        init_genesis_json(&mut ctx, &devnet_genesis())?;
        let id = 7;
        let proposal = unsupported_proposal(id, end, true);
        set_proposal(&mut ctx, &proposal)?;
        insert_queue(&mut ctx, ACTIVE_QUEUE_PREFIX, &end, id)?;
        let depositor = [8; 20];
        seed_deposit(&mut ctx, id, &depositor)?;
        let vote = Vote {
            proposal_id: id,
            voter: sdk::address::to_bech32("celestia", &[9; 20]),
            options: vec![],
            metadata: String::new(),
        };
        set_vote(&mut ctx, &vote)?;
        let vote_key = pair_addr_key(VOTES_PREFIX, id, &[9; 20]);

        end_block(&mut ctx, &NoRouter)?;

        let failed = keeper::must_get_proposal(&mut ctx, id)?;
        assert_eq!(failed.id, id);
        assert_eq!(failed.status, ProposalStatus::Failed as i32);
        assert!(failed.messages.is_empty());
        assert_eq!(failed.title, "retained title");
        assert_eq!(failed.summary, "retained summary");
        assert_eq!(failed.metadata, "retained metadata");
        assert!(failed.expedited);
        assert_eq!(failed.voting_end_time, Some(end));
        assert_eq!(
            failed.failed_reason,
            "proposal failed because it cannot be processed by gov: collections: encoding error: value decode: no concrete type registered for type URL /review.Unregistered against interface *types.Msg"
        );
        assert!(!ctx.has(STORE_KEY, &queue_key(ACTIVE_QUEUE_PREFIX, &end, id))?);
        let mut voting_index = vec![VOTING_PERIOD_PROPOSAL_PREFIX];
        voting_index.extend_from_slice(&u64_key(id));
        assert!(!ctx.has(STORE_KEY, &voting_index)?);
        assert!(ctx.has(STORE_KEY, &vote_key)?);
        assert!(keeper::get_deposits(&mut ctx, id)?.is_empty());
        assert_eq!(
            x_bank::get_balance(&mut ctx, &depositor, "utia")?,
            Coin::new("utia", 12)
        );
        assert_eq!(
            x_bank::get_balance(&mut ctx, &keeper::module_address(), "utia")?,
            Coin::new("utia", 0)
        );
        assert_failed_event(&ctx, true, id);
        Ok(())
    }

    #[test]
    fn inactive_unregistered_message_refunds_and_deletes_proposal() -> Result<()> {
        let mut ms = ms();
        let end = Timestamp {
            seconds: 100,
            nanos: 0,
        };
        let mut ctx = Context::new(
            &mut ms,
            Header {
                time: end,
                ..Header::default()
            },
            ExecMode::Finalize,
        );
        init_genesis_json(&mut ctx, &devnet_genesis())?;
        let id = 8;
        let proposal = unsupported_proposal(id, end, false);
        set_proposal(&mut ctx, &proposal)?;
        insert_queue(&mut ctx, INACTIVE_QUEUE_PREFIX, &end, id)?;
        let depositor = [10; 20];
        seed_deposit(&mut ctx, id, &depositor)?;

        end_block(&mut ctx, &NoRouter)?;

        assert!(keeper::get_proposal(&mut ctx, id)?.is_none());
        assert!(keeper::get_deposits(&mut ctx, id)?.is_empty());
        assert!(!ctx.has(STORE_KEY, &queue_key(INACTIVE_QUEUE_PREFIX, &end, id))?);
        assert_eq!(
            x_bank::get_balance(&mut ctx, &depositor, "utia")?,
            Coin::new("utia", 12)
        );
        assert_eq!(
            x_bank::get_balance(&mut ctx, &keeper::module_address(), "utia")?,
            Coin::new("utia", 0)
        );
        assert_failed_event(&ctx, false, id);
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
    fn create_validator_msg(
        validator_address: String,
    ) -> celestia_proto::cosmos::staking::v1beta1::MsgCreateValidator {
        use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;
        use celestia_proto::cosmos::crypto::ed25519;
        use celestia_proto::cosmos::staking::v1beta1 as staking_pb;

        #[allow(deprecated)]
        staking_pb::MsgCreateValidator {
            description: Some(staking_pb::Description {
                moniker: "val".into(),
                ..Default::default()
            }),
            commission: Some(staking_pb::CommissionRates {
                rate: "100000000000000000".into(),
                max_rate: "200000000000000000".into(),
                max_change_rate: "10000000000000000".into(),
            }),
            min_self_delegation: "1".into(),
            delegator_address: String::new(),
            validator_address,
            pubkey: Some(celestia_proto::Any::pack(&ed25519::PubKey {
                key: vec![1u8; 32],
            })),
            value: Some(PbCoin {
                denom: "utia".into(),
                amount: "1000000".into(),
            }),
        }
    }

    /// Creates a bonded validator (1 TIA self-delegation, 9 TIA from another
    /// delegator) whose `MsgCreateValidator.validator_address` is the canonical
    /// lowercase or the all-uppercase Bech32 form, has its operator vote YES on
    /// proposal 1, and tallies it.
    fn tally_with_validator_address_case(uppercase: bool) -> Result<Proposal> {
        passed_proposal(uppercase, &NoRouter, 0, false)
    }

    /// The voting flow of `tally_with_validator_address_case`, executed with
    /// `router` at `app_version`. With `with_message`, the proposal holds a
    /// `MsgSend` for the router to execute.
    fn passed_proposal(
        uppercase: bool,
        router: &dyn MsgRouter,
        app_version: u64,
        with_message: bool,
    ) -> Result<Proposal> {
        use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;
        use celestia_proto::cosmos::gov::v1::{MsgVote, VoteOption};
        use celestia_proto::cosmos::staking::v1beta1 as staking_pb;
        use sdk::coins::Coin;

        let mut ms = ms();
        let start = Timestamp {
            seconds: 1_000_000,
            nanos: 0,
        };
        let end = Timestamp {
            seconds: 1_000_100,
            nanos: 0,
        };
        let mut ctx = Context::new(
            &mut ms,
            Header {
                chain_id: "celestia".into(),
                height: 1,
                time: start,
                ..Header::default()
            },
            ExecMode::Finalize,
        );

        x_staking::init_genesis(
            &mut ctx,
            &serde_json::json!({
                "params": {
                    "unbonding_time": "1000s",
                    "max_validators": 10,
                    "max_entries": 7,
                    "historical_entries": 3,
                    "bond_denom": "utia",
                    "min_commission_rate": "0.050000000000000000"
                },
                "last_total_power": "0",
                "exported": false
            }),
        )?;
        let mut g = devnet_genesis();
        g["proposals"] = serde_json::json!([{
            "id": "1", "messages": [], "status": "PROPOSAL_STATUS_VOTING_PERIOD",
            "voting_end_time": "1970-01-12T13:48:20Z", "total_deposit": []
        }]);
        init_genesis_json(&mut ctx, &g)?;
        if with_message {
            let mut p = keeper::must_get_proposal(&mut ctx, 1)?;
            p.messages = vec![celestia_proto::Any {
                type_url: "/cosmos.bank.v1beta1.MsgSend".to_owned(),
                value: vec![],
            }];
            set_proposal(&mut ctx, &p)?;
        }
        if app_version != 0 {
            ctx.consensus_params.version = Some(
                celestia_proto::celestia_core_proto::tendermint::types::VersionParams {
                    app: app_version,
                },
            );
        }

        let op = [1u8; 20];
        let del = [2u8; 20];
        for (addr, amt) in [(&op[..], 1_000_000u64), (&del[..], 9_000_000u64)] {
            let acct = x_auth::new_account_with_address(&mut ctx, addr)?;
            x_auth::set_account(&mut ctx, &acct)?;
            x_bank::set_balance(&mut ctx, addr, &Coin::new("utia", amt))?;
        }

        let val_lower = x_staking::addr::val_to_string(&op)?;
        let val_addr = if uppercase {
            val_lower.to_ascii_uppercase()
        } else {
            val_lower.clone()
        };
        x_staking::handle_create_validator(&mut ctx, &create_validator_msg(val_addr))?;
        x_staking::handle_delegate(
            &mut ctx,
            &staking_pb::MsgDelegate {
                delegator_address: sdk::address::to_bech32("celestia", &del),
                validator_address: val_lower,
                amount: Some(PbCoin {
                    denom: "utia".into(),
                    amount: "9000000".into(),
                }),
            },
        )?;
        x_staking::end_block(&mut ctx)?;

        msgs::vote(
            &mut ctx,
            &MsgVote {
                proposal_id: 1,
                voter: sdk::address::to_bech32("celestia", &op),
                option: VoteOption::Yes as i32,
                metadata: String::new(),
            },
        )?;
        ctx.header.time = end;
        end_block(&mut ctx, router)?;
        keeper::must_get_proposal(&mut ctx, 1)
    }

    /// Fails every message with `error`.
    struct FailingRouter(SdkError);

    impl MsgRouter for FailingRouter {
        fn has_handler(&self, _: &str) -> bool {
            true
        }
        fn validate_basic(&self, _: &celestia_proto::Any) -> Result<()> {
            Ok(())
        }
        fn handle(
            &self,
            _: &mut Context<'_>,
            _: &celestia_proto::Any,
        ) -> Result<celestia_proto::Any> {
            Err(self.0.clone())
        }
        fn msg_text(&self, _: &celestia_proto::Any) -> String {
            "msg".to_owned()
        }
    }

    /// From app version 11, celestia-app's `govMsgRouter` makes x/gov store
    /// only the codespace and code of a failed proposal message, not its
    /// error text (PROTOCO-3759). Earlier versions store the text.
    #[test]
    fn failed_reason_has_no_error_text_from_v11() -> Result<()> {
        let error = SdkError::new(
            "sdk",
            5,
            "spendable balance 0utia is smaller than 1utia: insufficient funds",
        );
        let panic = SdkError::new("undefined", 111_222, "boom [file.go:1]");
        let cases = [
            (
                &error,
                10,
                "spendable balance 0utia is smaller than 1utia: insufficient funds",
            ),
            (
                &panic,
                10,
                "handling x/gov proposal msg [msg] PANICKED: boom [file.go:1]",
            ),
            (&error, 11, "proposal message failed: codespace sdk, code 5"),
            (
                &panic,
                11,
                "proposal message failed: codespace undefined, code 111222",
            ),
        ];
        for (e, version, want) in cases {
            let p = passed_proposal(false, &FailingRouter(e.clone()), version, true)?;
            assert_eq!(p.status, ProposalStatus::Failed as i32);
            assert_eq!(p.failed_reason, want, "v{version}");
        }
        Ok(())
    }

    #[test]
    fn tally_lowercase_operator_inherits_delegator_votes() -> Result<()> {
        let p = tally_with_validator_address_case(false)?;
        let tally = p.final_tally_result.unwrap_or_default();
        assert_eq!(tally.yes_count, "10000000");
        assert_eq!(p.status, ProposalStatus::Passed as i32);
        Ok(())
    }

    /// Go keys the tally's validator map by the stored operator address string
    /// and looks the voter up by its canonical lowercase form, so an operator
    /// registered with an uppercase address does not cast its delegators'
    /// votes: only its own delegation counts and the proposal fails quorum.
    #[test]
    fn tally_uppercase_operator_matches_go() -> Result<()> {
        let p = tally_with_validator_address_case(true)?;
        let tally = p.final_tally_result.unwrap_or_default();
        assert_eq!(tally.yes_count, "1000000");
        assert_eq!(p.status, ProposalStatus::Rejected as i32);
        Ok(())
    }
}
