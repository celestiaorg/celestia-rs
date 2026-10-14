//! Gov keeper: proposals, deposits, votes (`x/gov/keeper`).

use celestia_proto::Any;
use celestia_proto::cosmos::gov::v1::{
    Deposit, Params, Proposal, ProposalStatus, TallyResult, Vote, WeightedVoteOption,
};
use math::LegacyDec;
use prost::Message;
use sdk::address::undefined;
use sdk::coins::{Coin, Coins, math_err};
use sdk::error::sdk as sdkerr;
use sdk::event::event;
use sdk::time::add_duration;
use sdk::{Context, Result};

use crate::errors;
use crate::{
    ACTIVE_QUEUE_PREFIX, DEPOSITS_PREFIX, INACTIVE_QUEUE_PREFIX, MODULE_NAME, PROPOSALS_PREFIX,
    STORE_KEY, VOTES_PREFIX, VOTING_PERIOD_PROPOSAL_PREFIX, get_params, insert_queue,
    peek_proposal_id, queue_key, set_proposal, set_proposal_id,
};

/// `types.DefaultConfig().MaxMetadataLen`.
pub const MAX_METADATA_LEN: usize = 255;

pub use sdk::router::MsgRouter;

pub(crate) fn decode<M: Message + Default>(bz: &[u8]) -> Result<M> {
    M::decode(bz).map_err(|e| undefined(e.to_string()))
}

fn id_key(prefix: u8, id: u64) -> Vec<u8> {
    let mut k = vec![prefix];
    k.extend_from_slice(&id.to_be_bytes());
    k
}

/// `Proposals.Get` (`Ok(None)` is `collections.ErrNotFound`).
pub fn get_proposal(ctx: &mut Context<'_>, id: u64) -> Result<Option<Proposal>> {
    ctx.get(STORE_KEY, &id_key(PROPOSALS_PREFIX, id))?
        .map(|b| decode(&b))
        .transpose()
}

/// `Proposals.Get` mapping a missing proposal to Go's collections error.
pub fn must_get_proposal(ctx: &mut Context<'_>, id: u64) -> Result<Proposal> {
    get_proposal(ctx, id)?.ok_or_else(|| {
        undefined(format!(
            "collections: not found: key '{id}' of type cosmos.gov.v1.Proposal"
        ))
    })
}

/// `GetGovernanceAccount().GetAddress()` (creates the account if absent).
pub fn governance_account(ctx: &mut Context<'_>) -> Result<Vec<u8>> {
    let m = x_auth::get_module_account(ctx, MODULE_NAME)?
        .ok_or_else(|| sdkerr::logic("gov module account has not been set"))?;
    x_auth::Account::Module(m).address()
}

/// The gov module address (`authtypes.NewModuleAddress("gov")`).
#[must_use]
pub fn module_address() -> Vec<u8> {
    x_auth::module_address(MODULE_NAME)
}

fn assert_metadata_length(metadata: &str) -> Result<()> {
    if !metadata.is_empty() && metadata.len() > MAX_METADATA_LEN {
        return Err(errors::metadata_too_long()
            .wrap(format!("got metadata with length {}", metadata.len())));
    }
    Ok(())
}

fn assert_summary_length(summary: &str) -> Result<()> {
    if !summary.is_empty() && summary.len() > 40 * MAX_METADATA_LEN {
        return Err(
            errors::summary_too_long().wrap(format!("got summary with length {}", summary.len()))
        );
    }
    Ok(())
}

/// Signer address bytes of a message (`GetMsgV1Signers`).
pub fn msg_signers(msg: &Any) -> Result<Vec<Vec<u8>>> {
    let Some(signers) = celestia_proto::signers::get_signers(&msg.type_url, &msg.value) else {
        return Err(undefined(format!(
            "no cosmos.msg.v1.signer option found for message {}",
            msg.type_url
        )));
    };
    signers
        .map_err(|e| undefined(e.to_string()))?
        .into_iter()
        .map(|s| match s {
            celestia_proto::signers::Signer::Str(s) => encoding::bech32::decode(&s)
                .map(|(_, b)| b)
                .map_err(|e| undefined(e.to_string())),
            celestia_proto::signers::Signer::Raw(b) => Ok(b),
        })
        .collect()
}

/// `MsgExecLegacyContent` type URL.
pub const MSG_EXEC_LEGACY_CONTENT: &str = "/cosmos.gov.v1.MsgExecLegacyContent";

/// `Keeper.SubmitProposal`.
#[allow(clippy::too_many_arguments)]
pub fn submit_proposal(
    ctx: &mut Context<'_>,
    router: &dyn MsgRouter,
    messages: &[Any],
    metadata: &str,
    title: &str,
    summary: &str,
    proposer: &[u8],
    expedited: bool,
) -> Result<Proposal> {
    assert_metadata_length(metadata)?;
    assert_summary_length(summary)?;
    assert_metadata_length(title)?;

    let mut msgs_str = String::new();
    for msg in messages {
        msgs_str.push(',');
        msgs_str.push_str(&msg.type_url);
        router
            .validate_basic(msg)
            .map_err(|e| errors::invalid_proposal_msg().wrap(e.log))?;
        let signers = msg_signers(msg)?;
        if signers.len() != 1 {
            return Err(errors::invalid_signer());
        }
        let gov = governance_account(ctx)?;
        if signers[0] != gov {
            return Err(errors::invalid_signer().wrap(x_auth::bech32(&signers[0])?));
        }
        if !router.has_handler(&msg.type_url) {
            return Err(errors::unroutable_proposal_msg().wrap(&msg.type_url));
        }
        if msg.type_url == MSG_EXEC_LEGACY_CONTENT {
            let cp = ctx.branch();
            let res = router.handle(ctx, msg);
            ctx.discard(cp);
            if let Err(e) = res {
                let t = errors::no_proposal_handler_exists();
                if e.codespace == t.codespace && e.code == t.code {
                    return Err(e);
                }
                return Err(errors::invalid_proposal_content().wrap(e.log));
            }
        }
    }

    // ProposalID.Next.
    let id = peek_proposal_id(ctx)?;
    set_proposal_id(ctx, id + 1)?;
    let params = get_params(ctx)?;
    let submit_time = ctx.header.time;
    let deposit_end = add_duration(&submit_time, &params.max_deposit_period.unwrap_or_default());
    let proposal = Proposal {
        id,
        messages: messages.to_vec(),
        status: ProposalStatus::DepositPeriod as i32,
        final_tally_result: Some(empty_tally()),
        submit_time: Some(submit_time),
        deposit_end_time: Some(deposit_end),
        total_deposit: Vec::new(),
        voting_start_time: None,
        voting_end_time: None,
        metadata: metadata.to_owned(),
        title: title.to_owned(),
        summary: summary.to_owned(),
        proposer: x_auth::bech32(proposer)?,
        expedited,
        failed_reason: String::new(),
    };
    set_proposal(ctx, &proposal)?;
    insert_queue(ctx, INACTIVE_QUEUE_PREFIX, &deposit_end, id)?;
    ctx.events.emit(event(
        "submit_proposal",
        [
            ("proposal_id", id.to_string()),
            ("proposal_proposer", proposal.proposer.clone()),
            ("proposal_messages", msgs_str),
        ],
    ));
    Ok(proposal)
}

/// `EmptyTallyResult`.
#[must_use]
pub fn empty_tally() -> TallyResult {
    TallyResult {
        yes_count: "0".into(),
        abstain_count: "0".into(),
        no_count: "0".into(),
        no_with_veto_count: "0".into(),
    }
}

/// `Proposal.GetMinDepositFromParams`.
pub fn min_deposit(p: &Proposal, params: &Params) -> Result<Coins> {
    Coins::from_proto(if p.expedited {
        &params.expedited_min_deposit
    } else {
        &params.min_deposit
    })
}

/// `validateDepositDenom`.
pub fn validate_deposit_denom(params: &Params, amount: &Coins) -> Result<()> {
    let denoms: Vec<&str> = params
        .min_deposit
        .iter()
        .map(|c| c.denom.as_str())
        .collect();
    for c in amount {
        if !denoms.contains(&c.denom.as_str()) {
            return Err(errors::invalid_deposit_denom().wrap(format!(
                "deposited {amount}, but gov accepts only the following denom(s): [{}]",
                denoms.join(" ")
            )));
        }
    }
    Ok(())
}

/// Go `Coins.IsAllGTE`.
#[must_use]
pub fn is_all_gte(a: &Coins, b: &Coins) -> bool {
    if b.is_empty() {
        return true;
    }
    if a.is_empty() {
        return false;
    }
    b.iter().all(|c| c.amount <= a.amount_of(&c.denom))
}

/// Parses a `LegacyDec` param string (`LegacyNewDecFromStr`).
pub fn parse_dec(s: &str) -> Result<LegacyDec> {
    LegacyDec::from_str_go(s).map_err(|e| undefined(e.to_string()))
}

/// `validateInitialDeposit`.
pub fn validate_initial_deposit(params: &Params, initial: &Coins, expedited: bool) -> Result<()> {
    if !initial.is_valid() || initial.iter().any(|c| c.amount.is_negative()) {
        return Err(sdkerr::invalid_coins(initial.to_string()));
    }
    let ratio = parse_dec(&params.min_initial_deposit_ratio)?;
    if ratio.is_zero() {
        return Ok(());
    }
    let base = Coins::from_proto(if expedited {
        &params.expedited_min_deposit
    } else {
        &params.min_deposit
    })?;
    let mut min = Vec::new();
    for c in &base {
        let amt = c
            .amount
            .to_legacy_dec()
            .checked_mul(&ratio)
            .and_then(|d| d.round_int())
            .map_err(math_err)?;
        min.push(Coin::with_amount(c.denom.clone(), amt));
    }
    let min = Coins(min);
    if !is_all_gte(initial, &min) {
        return Err(errors::min_deposit_too_small().wrap(format!("was ({initial}), need ({min})")));
    }
    Ok(())
}

fn deposit_key(id: u64, depositor: &[u8]) -> Vec<u8> {
    let mut k = id_key(DEPOSITS_PREFIX, id);
    k.extend_from_slice(&sdk::codec::length_prefixed(depositor));
    k
}

fn vote_key(id: u64, voter: &[u8]) -> Vec<u8> {
    let mut k = id_key(VOTES_PREFIX, id);
    k.extend_from_slice(&sdk::codec::length_prefixed(voter));
    k
}

/// `ActivateVotingPeriod`.
pub fn activate_voting_period(ctx: &mut Context<'_>, mut proposal: Proposal) -> Result<()> {
    let start = ctx.header.time;
    proposal.voting_start_time = Some(start);
    let params = get_params(ctx)?;
    let period = if proposal.expedited {
        params.expedited_voting_period
    } else {
        params.voting_period
    }
    .unwrap_or_default();
    let end = add_duration(&start, &period);
    proposal.voting_end_time = Some(end);
    proposal.status = ProposalStatus::VotingPeriod as i32;
    set_proposal(ctx, &proposal)?;
    let dep_end = proposal.deposit_end_time.unwrap_or_default();
    ctx.delete(
        STORE_KEY,
        &queue_key(INACTIVE_QUEUE_PREFIX, &dep_end, proposal.id),
    )?;
    insert_queue(ctx, ACTIVE_QUEUE_PREFIX, &end, proposal.id)
}

/// `AddDeposit`: returns whether the voting period was activated.
pub fn add_deposit(
    ctx: &mut Context<'_>,
    id: u64,
    depositor: &[u8],
    amount: &Coins,
) -> Result<bool> {
    let mut proposal = must_get_proposal(ctx, id)?;
    if proposal.status != ProposalStatus::DepositPeriod as i32
        && proposal.status != ProposalStatus::VotingPeriod as i32
    {
        return Err(errors::inactive_proposal().wrap(id.to_string()));
    }
    let params = get_params(ctx)?;
    let min = min_deposit(&proposal, &params)?;
    let ratio = parse_dec(&params.min_deposit_ratio)?;
    validate_deposit_denom(&params, amount)?;
    if !ratio.is_zero() {
        let mut met = false;
        let mut thresholds = Vec::new();
        for m in &min {
            let t = m
                .amount
                .to_legacy_dec()
                .checked_mul(&ratio)
                .map_err(math_err)?
                .truncate_int();
            thresholds.push(Coin::with_amount(m.denom.clone(), t.clone()).to_string());
            let Some(dep) = amount.iter().find(|c| c.denom == m.denom) else {
                continue;
            };
            if dep.amount >= t {
                met = true;
                break;
            }
        }
        if !met {
            return Err(errors::min_deposit_too_small().wrap(format!(
                "received {amount} but need at least one of the following: {}",
                thresholds.join(",")
            )));
        }
    }
    x_bank::send_from_account_to_module(ctx, depositor, MODULE_NAME, amount)?;
    let total = Coins::sanitized(Coins::from_proto(&proposal.total_deposit)?.0)?.add(amount)?;
    proposal.total_deposit = total.to_proto();
    set_proposal(ctx, &proposal)?;

    let mut activated = false;
    if proposal.status == ProposalStatus::DepositPeriod as i32 && is_all_gte(&total, &min) {
        activate_voting_period(ctx, proposal)?;
        activated = true;
    }

    let key = deposit_key(id, depositor);
    let deposit = match ctx.get(STORE_KEY, &key)? {
        Some(b) => {
            let mut d: Deposit = decode(&b)?;
            let amt = Coins::sanitized(Coins::from_proto(&d.amount)?.0)?.add(amount)?;
            d.amount = amt.to_proto();
            d
        }
        None => Deposit {
            proposal_id: id,
            depositor: x_auth::bech32(depositor)?,
            amount: amount.to_proto(),
        },
    };
    ctx.events.emit(event(
        "proposal_deposit",
        [
            ("depositor", x_auth::bech32(depositor)?),
            ("amount", amount.to_string()),
            ("proposal_id", id.to_string()),
        ],
    ));
    ctx.set(STORE_KEY, key, deposit.encode_to_vec())?;
    Ok(activated)
}

/// Deposits of a proposal in store order (`IterateDeposits`).
pub fn get_deposits(ctx: &mut Context<'_>, id: u64) -> Result<Vec<(Vec<u8>, Deposit)>> {
    let prefix = id_key(DEPOSITS_PREFIX, id);
    let mut it = ctx.prefix_iter(STORE_KEY, &prefix)?;
    let mut out = Vec::new();
    while let Some((k, v)) = it.next(ctx)? {
        out.push((k, decode(&v)?));
    }
    Ok(out)
}

fn depositor_from_key(prefix_len: usize, key: &[u8]) -> Vec<u8> {
    key.get(prefix_len + 1..).unwrap_or_default().to_vec()
}

/// `RefundAndDeleteDeposits` (refund and delete inside one walk).
pub fn refund_and_delete_deposits(ctx: &mut Context<'_>, id: u64) -> Result<()> {
    let prefix = id_key(DEPOSITS_PREFIX, id);
    let mut it = ctx.prefix_iter(STORE_KEY, &prefix)?;
    while let Some((k, v)) = it.next(ctx)? {
        let d: Deposit = decode(&v)?;
        let depositor = depositor_from_key(prefix.len(), &k);
        x_bank::send_from_module_to_account(
            ctx,
            MODULE_NAME,
            &depositor,
            &Coins::from_proto(&d.amount)?,
        )?;
        ctx.delete(STORE_KEY, &k)?;
    }
    Ok(())
}

/// `DeleteAndBurnDeposits`.
pub fn delete_and_burn_deposits(ctx: &mut Context<'_>, id: u64) -> Result<()> {
    let prefix = id_key(DEPOSITS_PREFIX, id);
    let mut it = ctx.prefix_iter(STORE_KEY, &prefix)?;
    let mut burn = Coins::new();
    while let Some((k, v)) = it.next(ctx)? {
        let d: Deposit = decode(&v)?;
        burn = burn.add(&Coins::from_proto(&d.amount)?)?;
        ctx.delete(STORE_KEY, &k)?;
    }
    x_bank::burn_coins(ctx, MODULE_NAME, &burn)
}

/// `ChargeDeposit`: burns (or sends to `dest`) `rate` of every deposit and
/// refunds the rest.
pub fn charge_deposit(ctx: &mut Context<'_>, id: u64, dest: &str, rate: &str) -> Result<()> {
    let rate = parse_dec(rate).map_err(|e| sdk::error::go_panic(e.log))?;
    let mut charges = Coins::new();
    for (_, d) in get_deposits(ctx, id)? {
        let depositor = sdk::address::acc_from_bech32(&d.depositor)?;
        let mut remaining = Coins::new();
        for c in &Coins::from_proto(&d.amount)? {
            let burn = c
                .amount
                .to_legacy_dec()
                .checked_mul(&rate)
                .map_err(math_err)?
                .truncate_int();
            let rest = c.amount.checked_sub(&burn).map_err(math_err)?;
            remaining = remaining.add(&Coins(vec![Coin::with_amount(c.denom.clone(), rest)]))?;
            charges = charges.add(&Coins(vec![Coin::with_amount(c.denom.clone(), burn)]))?;
        }
        if !remaining.is_empty() {
            x_bank::send_from_module_to_account(ctx, MODULE_NAME, &depositor, &remaining)?;
        }
        ctx.delete(STORE_KEY, &deposit_key(d.proposal_id, &depositor))?;
    }
    if !charges.is_empty() {
        let distr = x_auth::bech32(&x_auth::module_address("distribution"))?;
        if dest.is_empty() {
            x_bank::burn_coins(ctx, MODULE_NAME, &charges)?;
        } else if dest == distr {
            x_distribution::keeper::fund_community_pool(ctx, &charges, &module_address())?;
        } else {
            let to = sdk::address::acc_from_bech32(dest)?;
            x_bank::send_from_module_to_account(ctx, MODULE_NAME, &to, &charges)?;
        }
    }
    Ok(())
}

/// `deleteVotes` (`Votes.Clear`: collect keys, then delete).
pub fn delete_votes(ctx: &mut Context<'_>, id: u64) -> Result<()> {
    let prefix = id_key(VOTES_PREFIX, id);
    let mut it = ctx.prefix_iter(STORE_KEY, &prefix)?;
    let mut keys = Vec::new();
    while let Some((k, _)) = it.next(ctx)? {
        keys.push(k);
    }
    for k in keys {
        ctx.delete(STORE_KEY, &k)?;
    }
    Ok(())
}

/// `DeleteProposal`.
pub fn delete_proposal(ctx: &mut Context<'_>, id: u64) -> Result<()> {
    let p = must_get_proposal(ctx, id)?;
    if let Some(t) = &p.deposit_end_time {
        ctx.delete(STORE_KEY, &queue_key(INACTIVE_QUEUE_PREFIX, t, id))?;
    }
    if let Some(t) = &p.voting_end_time {
        ctx.delete(STORE_KEY, &queue_key(ACTIVE_QUEUE_PREFIX, t, id))?;
        ctx.delete(STORE_KEY, &id_key(VOTING_PERIOD_PROPOSAL_PREFIX, id))?;
    }
    ctx.delete(STORE_KEY, &id_key(PROPOSALS_PREFIX, id))
}

/// `CancelProposal`.
pub fn cancel_proposal(ctx: &mut Context<'_>, id: u64, proposer: &str) -> Result<()> {
    let p = must_get_proposal(ctx, id)?;
    if p.proposer.is_empty() {
        return Err(errors::invalid_proposal().wrap(format!(
            "proposal {id} doesn't have proposer {proposer}, so cannot be canceled"
        )));
    }
    if p.proposer != proposer {
        return Err(errors::invalid_proposer().wrap(format!("invalid proposer {proposer}")));
    }
    if p.status != ProposalStatus::DepositPeriod as i32
        && p.status != ProposalStatus::VotingPeriod as i32
    {
        return Err(
            errors::invalid_proposal().wrap("proposal should be in the deposit or voting period")
        );
    }
    if let Some(end) = &p.voting_end_time {
        if sdk::time::before(end, &ctx.header.time) {
            return Err(errors::voting_period_ended().wrap(format!(
                "voting period is already ended for this proposal {id}"
            )));
        }
    }
    let params = get_params(ctx)?;
    charge_deposit(
        ctx,
        p.id,
        &params.proposal_cancel_dest,
        &params.proposal_cancel_ratio,
    )?;
    if p.voting_start_time.is_some() {
        delete_votes(ctx, p.id)?;
    }
    delete_proposal(ctx, p.id)
}

/// `v1.ValidVoteOption`.
#[must_use]
pub fn valid_vote_option(o: i32) -> bool {
    (1..=4).contains(&o)
}

/// `ValidWeightedVoteOption`.
#[must_use]
pub fn valid_weighted_option(o: &WeightedVoteOption) -> bool {
    match LegacyDec::from_str_go(&o.weight) {
        Ok(w) => w.is_positive() && w <= LegacyDec::one() && valid_vote_option(o.option),
        Err(_) => false,
    }
}

/// `VoteOption.String()`.
#[must_use]
pub fn vote_option_name(o: i32) -> String {
    celestia_proto::cosmos::gov::v1::VoteOption::try_from(o)
        .map_or_else(|_| o.to_string(), |v| v.as_str_name().to_owned())
}

/// `WeightedVoteOption.String()` (gogoproto `CompactTextString`).
#[must_use]
pub fn weighted_option_string(o: &WeightedVoteOption) -> String {
    let mut parts = Vec::new();
    if o.option != 0 {
        parts.push(format!("option:{}", vote_option_name(o.option)));
    }
    if !o.weight.is_empty() {
        parts.push(format!("weight:{:?}", o.weight));
    }
    parts.join(" ")
}

/// `WeightedVoteOptions.String()`: `json.Marshal` of the options.
#[must_use]
pub fn weighted_options_json(opts: &[WeightedVoteOption]) -> String {
    let items: Vec<String> = opts
        .iter()
        .map(|o| {
            let mut fields = Vec::new();
            if o.option != 0 {
                fields.push(format!("\"option\":{}", o.option));
            }
            if !o.weight.is_empty() {
                fields.push(format!(
                    "\"weight\":{}",
                    serde_json::to_string(&o.weight).unwrap_or_default()
                ));
            }
            format!("{{{}}}", fields.join(","))
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// `AddVote`.
pub fn add_vote(
    ctx: &mut Context<'_>,
    id: u64,
    voter: &[u8],
    options: &[WeightedVoteOption],
    metadata: &str,
) -> Result<()> {
    if !ctx.has(STORE_KEY, &id_key(VOTING_PERIOD_PROPOSAL_PREFIX, id))? {
        return Err(errors::inactive_proposal().wrap(id.to_string()));
    }
    assert_metadata_length(metadata)?;
    for o in options {
        if !valid_weighted_option(o) {
            return Err(errors::invalid_vote().wrap(weighted_option_string(o)));
        }
    }
    let vote = Vote {
        proposal_id: id,
        voter: x_auth::bech32(voter)?,
        options: options.to_vec(),
        metadata: metadata.to_owned(),
    };
    ctx.set(STORE_KEY, vote_key(id, voter), vote.encode_to_vec())?;
    ctx.events.emit(event(
        "proposal_vote",
        [
            ("voter", vote.voter.clone()),
            ("option", weighted_options_json(options)),
            ("proposal_id", id.to_string()),
        ],
    ));
    Ok(())
}
