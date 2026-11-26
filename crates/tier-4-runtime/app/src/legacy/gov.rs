//! cosmos-sdk v0.46 `x/gov` of celestia-app v1 (`v1.31.2-sdk-v0.46.16`
//! `x/gov`): `ValidateBasic` of the v1beta1 and v1 messages, the v1 msg
//! server with the v1beta1 server that wraps it (`keeper/msg_server.go`),
//! the keeper flows they use (`keeper/proposal.go`, `deposit.go`,
//! `vote.go`, `tally.go`) and the `EndBlocker` (`abci.go`).
//!
//! The store layout is the one later releases kept (proposal, queue,
//! deposit and vote keys), without v0.50's voting-period index. The
//! parameters live in the `x/params` subspace `gov` (amino JSON).
//!
//! Proposal content: celestia-app v1's legacy router has the routes
//! `params` (parameter changes behind the param filter), `distribution`
//! (community pool spends) and `ibc` (client proposals). Community pool
//! spends and parameter changes ([`super::param_change`]) run here; an IBC
//! client proposal or a v1 message other than `MsgExecLegacyContent` is
//! not ported. A text proposal has no route in
//! celestia-app v1, so its submission fails as in Go.

use celestia_proto::Any;
use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;
use celestia_proto::cosmos::distribution::v1beta1 as distr;
use celestia_proto::cosmos::gov::{v1, v1beta1};
use math::LegacyDec;
use pbjson_types::Timestamp;
use prost::{Message, Name};
use sdk::coins::Coins;
use sdk::error::sdk as errs;
use sdk::event::event;
use sdk::{Context, Result, SdkError};
use x_gov::errors as gerr;

fn decode<M: Message + Default>(msg: &Any) -> Result<M> {
    M::decode(msg.value.as_slice()).map_err(|e| errs::tx_decode(e.to_string()))
}

fn not_ported(what: impl AsRef<str>) -> SdkError {
    super::not_ported(errs::not_supported(format!(
        "the cosmos-sdk v0.46 {} is not ported",
        what.as_ref()
    )))
}

/// `ErrUnknownProposal` (gov code 2).
fn unknown_proposal(id: u64) -> SdkError {
    SdkError::new("gov", 2, format!("{id}: unknown proposal"))
}

/// `types.DefaultConfig().MaxMetadataLen`.
const MAX_METADATA_LEN: usize = 255;
/// `MaxTitleLength` / `MaxDescriptionLength` of v1beta1 content.
const MAX_TITLE_LENGTH: usize = 140;
const MAX_DESCRIPTION_LENGTH: usize = 10_000;
/// `KVGasConfig().WriteCostPerByte`.
const WRITE_COST_PER_BYTE: u64 = 30;

const COMMUNITY_POOL_SPEND: &str = "/cosmos.distribution.v1beta1.CommunityPoolSpendProposal";
const TEXT_PROPOSAL: &str = "/cosmos.gov.v1beta1.TextProposal";
const PARAMETER_CHANGE: &str = "/cosmos.params.v1beta1.ParameterChangeProposal";
const CLIENT_UPDATE: &str = "/ibc.core.client.v1.ClientUpdateProposal";

// ---- keys ---------------------------------------------------------------

fn proposal_key(id: u64) -> Vec<u8> {
    let mut k = vec![x_gov::PROPOSALS_PREFIX];
    k.extend_from_slice(&id.to_be_bytes());
    k
}

/// `DepositKey` / `VoteKey`: prefix, id, length-prefixed address.
fn pair_key(prefix: u8, id: u64, addr: &[u8]) -> Vec<u8> {
    let mut k = vec![prefix];
    k.extend_from_slice(&id.to_be_bytes());
    #[allow(clippy::cast_possible_truncation)]
    k.push(addr.len() as u8);
    k.extend_from_slice(addr);
    k
}

fn pair_prefix(prefix: u8, id: u64) -> Vec<u8> {
    let mut k = vec![prefix];
    k.extend_from_slice(&id.to_be_bytes());
    k
}

// ---- parameters (subspace `gov`) -----------------------------------------

fn params_json(ctx: &mut Context<'_>, key: &str) -> Result<serde_json::Value> {
    let raw = x_params::get_raw(ctx, x_gov::MODULE_NAME, key.as_bytes())?.unwrap_or_default();
    serde_json::from_slice(&raw)
        .map_err(|e| sdk::error::go_panic(format!("subspace gov: {key}: {e}")))
}

fn duration_param(v: &serde_json::Value, field: &str) -> Result<i128> {
    v.get(field)
        .and_then(serde_json::Value::as_str)
        .and_then(|s| s.parse::<i64>().ok())
        .map(i128::from)
        .ok_or_else(|| {
            sdk::error::go_panic(format!("subspace gov: {field}: unexpected amino JSON"))
        })
}

/// `GetDepositParams`: the minimum deposit and the deposit period (ns).
fn deposit_params(ctx: &mut Context<'_>) -> Result<(Coins, i128)> {
    let v = params_json(ctx, x_gov::V046_KEY_DEPOSIT_PARAMS)?;
    let mut min = Vec::new();
    if let Some(a) = v.get("min_deposit").and_then(serde_json::Value::as_array) {
        for c in a {
            let pb = PbCoin {
                denom: c["denom"].as_str().unwrap_or_default().to_owned(),
                amount: c["amount"].as_str().unwrap_or_default().to_owned(),
            };
            min.push(sdk::coins::Coin::from_proto(&pb)?);
        }
    }
    Ok((Coins(min), duration_param(&v, "max_deposit_period")?))
}

/// `GetVotingParams().VotingPeriod` (ns).
fn voting_period(ctx: &mut Context<'_>) -> Result<i128> {
    let v = params_json(ctx, x_gov::V046_KEY_VOTING_PARAMS)?;
    duration_param(&v, "voting_period")
}

/// `GetTallyParams`: quorum, threshold, veto threshold (Go ignores parse
/// errors and uses a nil `Dec`, which only a parameter change could set).
fn tally_params(ctx: &mut Context<'_>) -> Result<[LegacyDec; 3]> {
    let v = params_json(ctx, x_gov::V046_KEY_TALLY_PARAMS)?;
    let dec = |f: &str| {
        v.get(f)
            .and_then(serde_json::Value::as_str)
            .and_then(|s| LegacyDec::from_str_go(s).ok())
            .ok_or_else(|| not_ported(format!("gov tally parameter {f}")))
    };
    Ok([dec("quorum")?, dec("threshold")?, dec("veto_threshold")?])
}

// ---- store ----------------------------------------------------------------

fn get_proposal(ctx: &mut Context<'_>, id: u64) -> Result<Option<v1::Proposal>> {
    ctx.get(x_gov::STORE_KEY, &proposal_key(id))?
        .map(|b| {
            v1::Proposal::decode(b.as_slice()).map_err(|e| sdk::error::go_panic(e.to_string()))
        })
        .transpose()
}

fn set_proposal(ctx: &mut Context<'_>, p: &v1::Proposal) -> Result<()> {
    ctx.set(x_gov::STORE_KEY, proposal_key(p.id), p.encode_to_vec())
}

fn get_deposit(ctx: &mut Context<'_>, id: u64, depositor: &[u8]) -> Result<Option<v1::Deposit>> {
    ctx.get(
        x_gov::STORE_KEY,
        &pair_key(x_gov::DEPOSITS_PREFIX, id, depositor),
    )?
    .map(|b| v1::Deposit::decode(b.as_slice()).map_err(|e| sdk::error::go_panic(e.to_string())))
    .transpose()
}

/// The deposits of a proposal in key order, with their depositor
/// addresses.
fn deposits(ctx: &mut Context<'_>, id: u64) -> Result<Vec<(Vec<u8>, v1::Deposit)>> {
    let prefix = pair_prefix(x_gov::DEPOSITS_PREFIX, id);
    let mut it = ctx.prefix_iter(x_gov::STORE_KEY, &prefix)?;
    let mut out = Vec::new();
    while let Some((k, v)) = it.next(ctx)? {
        let d =
            v1::Deposit::decode(v.as_slice()).map_err(|e| sdk::error::go_panic(e.to_string()))?;
        out.push((k, d));
    }
    Ok(out)
}

/// The proposal ID (`GetProposalID`).
fn proposal_id(ctx: &mut Context<'_>) -> Result<u64> {
    let b = ctx
        .get(x_gov::STORE_KEY, &[x_gov::PROPOSAL_ID_KEY])?
        .ok_or_else(|| gerr::invalid_genesis().wrap("initial proposal ID hasn't been set"))?;
    sdk::codec::decode_u64(&b).ok_or_else(|| sdk::error::go_panic("invalid proposal ID bytes"))
}

/// `GetGovernanceAccount().GetAddress()`.
fn gov_account(ctx: &mut Context<'_>) -> Result<Vec<u8>> {
    let acc = x_auth::get_module_account(ctx, x_gov::MODULE_NAME)?
        .ok_or_else(|| sdk::error::go_panic("gov module account has not been set"))?;
    x_auth::Account::Module(acc).address()
}

// ---- coins ----------------------------------------------------------------

/// Go `sdk.NewCoins(...).Add(...)` of two coin sets (both valid).
fn coins_add(a: &[PbCoin], b: &[PbCoin]) -> Result<Vec<PbCoin>> {
    let a = Coins::sanitized(Coins::from_proto(a)?.0)?;
    let b = Coins::from_proto(b)?;
    Ok(a.add(&b)?.to_proto())
}

/// v0.46 `Coins.IsValid() && !IsAnyNegative()` of raw coins (v1beta1).
fn validate_raw_deposit(coins: &[PbCoin]) -> Result<()> {
    if !sdk::coins::raw_coins_is_valid(coins)? {
        return Err(errs::invalid_coins(sdk::coins::raw_coins_string(coins)));
    }
    Ok(())
}

/// v1 `sdk.NewCoins(m.Amount...)` then `IsValid`: a coin set that
/// `NewCoins` rejects panics (as Go's `NewCoins` does).
fn validate_v1_deposit(coins: &[PbCoin]) -> Result<()> {
    for c in coins {
        if c.amount.is_empty() {
            return Err(sdk::error::go_panic(
                "runtime error: invalid memory address or nil pointer dereference",
            ));
        }
    }
    let parsed = Coins::from_proto(coins).map_err(|_| not_ported("v1 deposit amount"))?;
    Coins::sanitized(parsed.0).map_err(|e| sdk::error::go_panic(e.log))?;
    Ok(())
}

/// The deposit `AddDeposit` sends: the raw coins of the message, which
/// the bank keeper takes as given. Raw coins that are not valid (a zero
/// coin that the v1 `ValidateBasic` sanitized away) are not ported.
fn deposit_coins(coins: &[PbCoin]) -> Result<Coins> {
    if !sdk::coins::raw_coins_is_valid(coins)? {
        return Err(not_ported("deposit of coins that are not valid as sent"));
    }
    Coins::from_proto(coins)
}

// ---- ValidateBasic --------------------------------------------------------

fn option_string(option: i32) -> String {
    x_gov::keeper::vote_option_name(option)
}

/// `MsgVote.ValidateBasic` (both versions).
fn validate_vote(voter: &str, option: i32) -> Result<()> {
    super::msgs::acc_address(voter, "voter")?;
    if !x_gov::keeper::valid_vote_option(option) {
        return Err(gerr::invalid_vote().wrap(option_string(option)));
    }
    Ok(())
}

/// `WeightedVoteOptions.String()` (v1): each option's compact text, one
/// per line, trimmed.
fn options_string(opts: &[v1::WeightedVoteOption]) -> String {
    opts.iter()
        .map(|o| x_gov::keeper::weighted_option_string(o) + "\n")
        .collect::<String>()
        .trim()
        .to_owned()
}

/// `MsgVoteWeighted.ValidateBasic` (v1; v1beta1 differs only in reading
/// the weights as `sdk.Dec` already).
fn validate_vote_weighted(voter: &str, opts: &[v1::WeightedVoteOption]) -> Result<()> {
    super::msgs::acc_address(voter, "voter")?;
    if opts.is_empty() {
        return Err(errs::invalid_request(options_string(opts)));
    }
    let mut total = LegacyDec::zero();
    let mut used = std::collections::HashSet::new();
    for o in opts {
        if !x_gov::keeper::valid_weighted_option(o) {
            return Err(gerr::invalid_vote().wrap(x_gov::keeper::weighted_option_string(o)));
        }
        let w = LegacyDec::from_str_go(&o.weight)
            .map_err(|e| gerr::invalid_vote().wrap(format!("Invalid weight: {e}")))?;
        total = total
            .checked_add(&w)
            .map_err(|e| sdk::error::go_panic(e.to_string()))?;
        if !used.insert(o.option) {
            return Err(gerr::invalid_vote().wrap("Duplicated vote option"));
        }
    }
    if total > LegacyDec::one() {
        return Err(gerr::invalid_vote().wrap("Total weight overflow 1.00"));
    }
    if total < LegacyDec::one() {
        return Err(gerr::invalid_vote().wrap("Total weight lower than 1.00"));
    }
    Ok(())
}

/// v1beta1 weighted options as the v1 server receives them
/// (`opt.Weight.String()`). gogoproto decodes each weight as an `sdk.Dec`
/// customtype; one it cannot decode is not ported.
fn v1beta1_options(opts: &[v1beta1::WeightedVoteOption]) -> Result<Vec<v1::WeightedVoteOption>> {
    opts.iter()
        .map(|o| {
            // An empty weight is a nil `Dec`, which Go cannot print.
            if o.weight.is_empty() {
                return Err(not_ported("v1beta1 weighted vote with an empty weight"));
            }
            let w = LegacyDec::unmarshal(o.weight.as_bytes())
                .map_err(|_| not_ported("v1beta1 weighted vote weight decoding"))?;
            Ok(v1::WeightedVoteOption {
                option: o.option,
                weight: w.to_string(),
            })
        })
        .collect()
}

/// The content of a v1beta1 proposal as the v1 registry decodes it, after
/// `ValidateBasic` of the content (`ValidateAbstract` plus its own checks).
fn validate_content(content: &Any) -> Result<()> {
    let abstract_ = |title: &str, description: &str| -> Result<()> {
        if title.trim().is_empty() {
            return Err(gerr::invalid_proposal_content().wrap("proposal title cannot be blank"));
        }
        if title.len() > MAX_TITLE_LENGTH {
            return Err(gerr::invalid_proposal_content().wrap(format!(
                "proposal title is longer than max length of {MAX_TITLE_LENGTH}"
            )));
        }
        if description.is_empty() {
            return Err(
                gerr::invalid_proposal_content().wrap("proposal description cannot be blank")
            );
        }
        if description.len() > MAX_DESCRIPTION_LENGTH {
            return Err(gerr::invalid_proposal_content().wrap(format!(
                "proposal description is longer than max length of {MAX_DESCRIPTION_LENGTH}"
            )));
        }
        Ok(())
    };
    match content.type_url.as_str() {
        COMMUNITY_POOL_SPEND => {
            let c: distr::CommunityPoolSpendProposal = decode(content)?;
            abstract_(&c.title, &c.description)?;
            if !sdk::coins::raw_coins_is_valid(&c.amount)? {
                // `ErrInvalidProposalAmount` (distribution code 9).
                return Err(SdkError::new(
                    "distribution",
                    9,
                    "invalid community pool spend proposal amount",
                ));
            }
            if c.recipient.is_empty() {
                // `ErrEmptyProposalRecipient` (distribution code 10).
                return Err(SdkError::new(
                    "distribution",
                    10,
                    "invalid community pool spend proposal recipient",
                ));
            }
            Ok(())
        }
        TEXT_PROPOSAL => {
            let c: v1beta1::TextProposal = decode(content)?;
            abstract_(&c.title, &c.description)
        }
        PARAMETER_CHANGE => {
            let c: celestia_proto::cosmos::params::v1beta1::ParameterChangeProposal =
                decode(content)?;
            abstract_(&c.title, &c.description)?;
            super::param_change::validate_changes(&c)
        }
        CLIENT_UPDATE => {
            let c: celestia_proto::ibc::core::client::v1::ClientUpdateProposal = decode(content)?;
            abstract_(&c.title, &c.description)?;
            if c.subject_client_id == c.substitute_client_id {
                return Err(x_ibc::errors::client::invalid_substitute(
                    "subject and substitute client identifiers are equal",
                ));
            }
            parse_client_identifier(&c.subject_client_id)?;
            parse_client_identifier(&c.substitute_client_id)
        }
        other => Err(not_ported(format!("gov proposal content {other}"))),
    }
}

/// ibc-go v6 `ParseClientIdentifier` (`{client-type}-{N}`).
fn parse_client_identifier(id: &str) -> Result<()> {
    // `IsClientIDFormat`: `^.*[^\n-]-[0-9]{1,20}$`.
    let format = id.rsplit_once('-').is_some_and(|(head, n)| {
        !n.is_empty()
            && n.len() <= 20
            && n.bytes().all(|b| b.is_ascii_digit())
            && head.chars().last().is_some_and(|c| c != '\n' && c != '-')
            && !head.contains('\n')
    });
    if !format {
        return Err(x_ibc::errors::host::invalid_id(format!(
            "invalid client identifier {id} is not in format: `{{client-type}}-{{N}}`"
        )));
    }
    let (head, n) = id.rsplit_once('-').unwrap_or_default();
    if head.trim().is_empty() {
        return Err(x_ibc::errors::host::invalid_id(
            "client identifier must be in format: `{client-type}-{N}` and client type cannot be blank",
        ));
    }
    n.parse::<u64>()
        .map(|_| ())
        .map_err(|_| not_ported("client identifier sequence beyond uint64"))
}

/// `MsgSubmitProposal.ValidateBasic` (v1beta1).
fn validate_submit_v1beta1(m: &v1beta1::MsgSubmitProposal) -> Result<()> {
    super::msgs::acc_address(&m.proposer, "proposer")?;
    validate_raw_deposit(&m.initial_deposit)?;
    let Some(content) = m.content.as_ref().filter(|c| !c.type_url.is_empty()) else {
        return Err(gerr::invalid_proposal_content().wrap("missing content"));
    };
    validate_content(content)
}

/// `MsgSubmitProposal.ValidateBasic` (v1).
fn validate_submit_v1(m: &v1::MsgSubmitProposal) -> Result<()> {
    super::msgs::acc_address(&m.proposer, "proposer")?;
    validate_v1_deposit(&m.initial_deposit)?;
    if m.messages.is_empty() && m.metadata.is_empty() {
        return Err(gerr::no_proposal_msgs().wrap("either metadata or Msgs length must be non-nil"));
    }
    for (idx, msg) in m.messages.iter().enumerate() {
        if msg.type_url != v1::MsgExecLegacyContent::type_url() {
            return Err(not_ported(format!(
                "gov v1 proposal message {}",
                msg.type_url
            )));
        }
        let e: v1::MsgExecLegacyContent = decode(msg)?;
        if let Err(err) = sdk::address::acc_from_bech32_go(&e.authority) {
            return Err(gerr::invalid_proposal_msg().wrap(format!(
                "msg: {idx}, err: {}",
                sdk::error::internal(err).log
            )));
        }
    }
    Ok(())
}

/// `ValidateBasic` of the gov messages; `None` for any other message.
#[must_use]
pub fn validate_basic(msg: &Any) -> Option<Result<()>> {
    let u = msg.type_url.as_str();
    Some(if u == v1beta1::MsgVote::type_url() {
        decode::<v1beta1::MsgVote>(msg).and_then(|m| validate_vote(&m.voter, m.option))
    } else if u == v1::MsgVote::type_url() {
        decode::<v1::MsgVote>(msg).and_then(|m| validate_vote(&m.voter, m.option))
    } else if u == v1::MsgVoteWeighted::type_url() {
        decode::<v1::MsgVoteWeighted>(msg)
            .and_then(|m| validate_vote_weighted(&m.voter, &m.options))
    } else if u == v1beta1::MsgVoteWeighted::type_url() {
        decode::<v1beta1::MsgVoteWeighted>(msg)
            .and_then(|m| validate_vote_weighted(&m.voter, &v1beta1_options(&m.options)?))
    } else if u == v1beta1::MsgDeposit::type_url() {
        decode::<v1beta1::MsgDeposit>(msg).and_then(|m| {
            super::msgs::acc_address(&m.depositor, "depositor")?;
            validate_raw_deposit(&m.amount)
        })
    } else if u == v1::MsgDeposit::type_url() {
        decode::<v1::MsgDeposit>(msg).and_then(|m| {
            super::msgs::acc_address(&m.depositor, "depositor")?;
            validate_v1_deposit(&m.amount)
        })
    } else if u == v1beta1::MsgSubmitProposal::type_url() {
        decode::<v1beta1::MsgSubmitProposal>(msg).and_then(|m| validate_submit_v1beta1(&m))
    } else if u == v1::MsgSubmitProposal::type_url() {
        decode::<v1::MsgSubmitProposal>(msg).and_then(|m| validate_submit_v1(&m))
    } else if u == v1::MsgExecLegacyContent::type_url() {
        decode::<v1::MsgExecLegacyContent>(msg).and_then(|m| {
            sdk::address::acc_from_bech32_go(&m.authority)
                .map(|_| ())
                .map_err(sdk::error::internal)
        })
    } else {
        return None;
    })
}

// ---- keeper ---------------------------------------------------------------

fn assert_metadata_length(metadata: &str) -> Result<()> {
    if !metadata.is_empty() && metadata.len() > MAX_METADATA_LEN {
        return Err(
            gerr::metadata_too_long().wrap(format!("got metadata with length {}", metadata.len()))
        );
    }
    Ok(())
}

/// The legacy content handler of a proposal (celestia-app v1's legacy
/// router): only the community pool spend runs.
fn run_legacy_content(ctx: &mut Context<'_>, content: &Any) -> Result<()> {
    match content.type_url.as_str() {
        COMMUNITY_POOL_SPEND => {
            let c: distr::CommunityPoolSpendProposal = decode(content)?;
            let recipient =
                sdk::address::acc_from_bech32_go(&c.recipient).map_err(sdk::error::internal)?;
            if x_bank::is_blocked(&recipient) {
                return Err(errs::unauthorized(format!(
                    "{} is not allowed to receive external funds",
                    c.recipient
                )));
            }
            x_distribution::keeper::distribute_from_fee_pool(
                ctx,
                &Coins::from_proto(&c.amount)?,
                &recipient,
            )
        }
        PARAMETER_CHANGE => super::param_change::handle(ctx, content),
        // celestia-app v1 `NewClientProposalHandler` → ibc-go v6
        // `Keeper.ClientUpdateProposal`, up to the subject client's
        // lookup.
        CLIENT_UPDATE => {
            let c: celestia_proto::ibc::core::client::v1::ClientUpdateProposal = decode(content)?;
            if c.subject_client_id == "09-localhost" || c.substitute_client_id == "09-localhost" {
                // `ErrInvalidUpdateClientProposal` (client code 24 in v6).
                return Err(SdkError::new("client", 24, "invalid update client proposal")
                    .wrap("cannot update localhost client with proposal"));
            }
            match x_ibc::client::get_client_state(ctx, &c.subject_client_id)? {
                None => Err(x_ibc::errors::client::client_not_found(format!(
                    "subject client with ID {}",
                    c.subject_client_id
                ))),
                Some(_) => Err(not_ported("ibc client update proposal of an existing client")),
            }
        }
        other => Err(not_ported(format!("gov proposal content {other}"))),
    }
}

/// The route of a legacy content (`content.ProposalRoute()`).
fn content_route(content: &Any) -> Result<&'static str> {
    match content.type_url.as_str() {
        COMMUNITY_POOL_SPEND => Ok("distribution"),
        TEXT_PROPOSAL => Ok("gov"),
        PARAMETER_CHANGE => Ok("params"),
        CLIENT_UPDATE => Ok("client"),
        other => Err(not_ported(format!("gov proposal content {other}"))),
    }
}

/// The msg service router's handler of `MsgExecLegacyContent` (a fresh
/// event manager, `ValidateBasic`, then `msgServer.ExecLegacyContent`);
/// returns the handler's events.
fn exec_legacy_content(
    ctx: &mut Context<'_>,
    m: &v1::MsgExecLegacyContent,
) -> Result<Vec<celestia_proto::celestia_core_proto::abci::Event>> {
    sdk::address::acc_from_bech32_go(&m.authority).map_err(sdk::error::internal)?;
    let outer = std::mem::take(&mut ctx.events);
    let r = (|| {
        let gov = x_auth::bech32(&gov_account(ctx)?)?;
        if gov != m.authority {
            return Err(gerr::invalid_signer().wrap(format!("expected {gov} got {}", m.authority)));
        }
        let content = m.content.clone().unwrap_or_default();
        let route = content_route(&content)?;
        if route != "distribution" && route != "params" && route != "client" {
            return Err(gerr::no_proposal_handler_exists().wrap(route));
        }
        run_legacy_content(ctx, &content).map_err(|e| {
            if e.is_panic() {
                return e;
            }
            // Go formats the handler's error with `%+v`: the ibc client
            // handler's errors carry a stack, printed after a newline
            // (the oracle drops the frames: they name the build's paths).
            let sep = if route == "client" { ", \n" } else { ", " };
            gerr::invalid_proposal_content()
                .wrap(format!("failed to run legacy handler {route},{}{}", &sep[1..], e.log))
        })
    })();
    let events = std::mem::replace(&mut ctx.events, outer).take();
    r.map(|()| events)
}

/// `Keeper.SubmitProposal` (v0.46) of `MsgExecLegacyContent` messages.
fn keeper_submit(
    ctx: &mut Context<'_>,
    msgs: &[v1::MsgExecLegacyContent],
    metadata: &str,
) -> Result<v1::Proposal> {
    assert_metadata_length(metadata)?;
    let mut msgs_str = String::new();
    let mut anys = Vec::with_capacity(msgs.len());
    for m in msgs {
        msgs_str.push(',');
        msgs_str.push_str(&v1::MsgExecLegacyContent::type_url());
        // `msg.ValidateBasic()`.
        let authority = sdk::address::acc_from_bech32_go(&m.authority)
            .map_err(|e| gerr::invalid_proposal_msg().wrap(sdk::error::internal(e).log))?;
        // `GetSigners` then the governance account.
        let gov = gov_account(ctx)?;
        if authority != gov {
            return Err(gerr::invalid_signer().wrap(x_auth::bech32(&authority)?));
        }
        // The handler on a cache context (writes and events discarded,
        // gas kept).
        let cp = ctx.branch();
        let r = exec_legacy_content(ctx, m);
        ctx.discard(cp);
        if let Err(e) = r {
            if e.is_panic() {
                return Err(e);
            }
            return Err(gerr::no_proposal_handler_exists().wrap(e.log));
        }
        anys.push(txdecode::gogo_repack(&Any::pack(m)));
    }
    let id = proposal_id(ctx)?;
    let submit = ctx.header.time;
    let (_, period) = deposit_params(ctx)?;
    let end = sdk::time::add(&submit, period);
    let p = v1::Proposal {
        id,
        messages: anys,
        status: v1::ProposalStatus::DepositPeriod as i32,
        final_tally_result: Some(x_gov::keeper::empty_tally()),
        submit_time: Some(submit),
        deposit_end_time: Some(end),
        metadata: metadata.to_owned(),
        ..Default::default()
    };
    set_proposal(ctx, &p)?;
    x_gov::insert_queue(ctx, x_gov::INACTIVE_QUEUE_PREFIX, &end, id)?;
    ctx.set(
        x_gov::STORE_KEY,
        vec![x_gov::PROPOSAL_ID_KEY],
        (id + 1).to_be_bytes().to_vec(),
    )?;
    ctx.events.emit(event(
        "submit_proposal",
        [
            ("proposal_id", id.to_string()),
            ("proposal_messages", msgs_str),
        ],
    ));
    Ok(p)
}

/// `ActivateVotingPeriod`.
fn activate_voting_period(ctx: &mut Context<'_>, mut p: v1::Proposal) -> Result<()> {
    let start = ctx.header.time;
    let period = voting_period(ctx)?;
    let end = sdk::time::add(&start, period);
    p.voting_start_time = Some(start);
    p.voting_end_time = Some(end);
    p.status = v1::ProposalStatus::VotingPeriod as i32;
    set_proposal(ctx, &p)?;
    let dep_end = p.deposit_end_time.unwrap_or_default();
    ctx.delete(
        x_gov::STORE_KEY,
        &x_gov::queue_key(x_gov::INACTIVE_QUEUE_PREFIX, &dep_end, p.id),
    )?;
    x_gov::insert_queue(ctx, x_gov::ACTIVE_QUEUE_PREFIX, &end, p.id)
}

/// `AddDeposit` (v0.46): whether the voting period started.
fn add_deposit(
    ctx: &mut Context<'_>,
    id: u64,
    depositor: &[u8],
    amount: &[PbCoin],
) -> Result<bool> {
    let Some(mut p) = get_proposal(ctx, id)? else {
        return Err(unknown_proposal(id));
    };
    let deposit_period = v1::ProposalStatus::DepositPeriod as i32;
    if p.status != deposit_period && p.status != v1::ProposalStatus::VotingPeriod as i32 {
        return Err(gerr::inactive_proposal().wrap(id.to_string()));
    }
    let coins = deposit_coins(amount)?;
    x_bank::send_from_account_to_module(ctx, depositor, x_gov::MODULE_NAME, &coins)?;
    p.total_deposit = coins_add(&p.total_deposit, amount)?;
    set_proposal(ctx, &p)?;
    let mut activated = false;
    if p.status == deposit_period {
        let (min, _) = deposit_params(ctx)?;
        let total = Coins::from_proto(&p.total_deposit)?;
        if x_gov::keeper::is_all_gte(&total, &min) {
            activate_voting_period(ctx, p)?;
            activated = true;
        }
    }
    let deposit = match get_deposit(ctx, id, depositor)? {
        Some(mut d) => {
            d.amount = coins_add(&d.amount, amount)?;
            d
        }
        None => v1::Deposit {
            proposal_id: id,
            depositor: x_auth::bech32(depositor)?,
            amount: amount.to_vec(),
        },
    };
    ctx.events.emit(event(
        "proposal_deposit",
        [
            ("amount", sdk::coins::raw_coins_string(amount)),
            ("proposal_id", id.to_string()),
        ],
    ));
    ctx.set(
        x_gov::STORE_KEY,
        pair_key(x_gov::DEPOSITS_PREFIX, id, depositor),
        deposit.encode_to_vec(),
    )?;
    Ok(activated)
}

/// `AddVote` (v0.46).
fn add_vote(
    ctx: &mut Context<'_>,
    id: u64,
    voter: &[u8],
    options: Vec<v1::WeightedVoteOption>,
    metadata: &str,
) -> Result<()> {
    let Some(p) = get_proposal(ctx, id)? else {
        return Err(unknown_proposal(id));
    };
    if p.status != v1::ProposalStatus::VotingPeriod as i32 {
        return Err(gerr::inactive_proposal().wrap(id.to_string()));
    }
    assert_metadata_length(metadata)?;
    for o in &options {
        if !x_gov::keeper::valid_weighted_option(o) {
            return Err(gerr::invalid_vote().wrap(x_gov::keeper::weighted_option_string(o)));
        }
    }
    let option = options_string(&options);
    let vote = v1::Vote {
        proposal_id: id,
        voter: x_auth::bech32(voter)?,
        options,
        metadata: metadata.to_owned(),
    };
    ctx.set(
        x_gov::STORE_KEY,
        pair_key(x_gov::VOTES_PREFIX, id, voter),
        vote.encode_to_vec(),
    )?;
    ctx.events.emit(event(
        "proposal_vote",
        [
            ("voter", vote.voter.clone()),
            ("option", option),
            ("proposal_id", id.to_string()),
        ],
    ));
    Ok(())
}

// ---- msg server -----------------------------------------------------------

/// `sdk.AccAddressFromBech32` in a msg server: a plain error.
fn server_acc(s: &str) -> Result<Vec<u8>> {
    sdk::address::acc_from_bech32_go(s).map_err(sdk::error::internal)
}

fn message_event(ctx: &mut Context<'_>, sender: &str) {
    ctx.events.emit(event(
        "message",
        [
            ("module", "governance".to_owned()),
            ("sender", sender.to_owned()),
        ],
    ));
}

/// v1 `msgServer.SubmitProposal`.
fn submit_proposal(
    ctx: &mut Context<'_>,
    msgs: &[v1::MsgExecLegacyContent],
    initial_deposit: &[PbCoin],
    proposer: &str,
    metadata: &str,
) -> Result<u64> {
    let p = keeper_submit(ctx, msgs, metadata)?;
    let bytes = p.encode_to_vec().len() as u64;
    ctx.gas
        .consume(3 * WRITE_COST_PER_BYTE * bytes, "submit proposal")
        .map_err(|_| errs::out_of_gas("out of gas in location: submit proposal"))?;
    let proposer_addr = sdk::address::acc_from_bech32_go(proposer).unwrap_or_default();
    let started = add_deposit(ctx, p.id, &proposer_addr, initial_deposit)?;
    message_event(ctx, proposer);
    if started {
        ctx.events.emit(event(
            "submit_proposal",
            [("voting_period_start", p.id.to_string())],
        ));
    }
    Ok(p.id)
}

fn deposit(ctx: &mut Context<'_>, id: u64, depositor: &str, amount: &[PbCoin]) -> Result<()> {
    let addr = server_acc(depositor)?;
    let started = add_deposit(ctx, id, &addr, amount)?;
    message_event(ctx, depositor);
    if started {
        ctx.events.emit(event(
            "proposal_deposit",
            [("voting_period_start", id.to_string())],
        ));
    }
    Ok(())
}

fn vote(
    ctx: &mut Context<'_>,
    id: u64,
    voter: &str,
    options: Vec<v1::WeightedVoteOption>,
    metadata: &str,
) -> Result<()> {
    let addr = server_acc(voter)?;
    add_vote(ctx, id, &addr, options, metadata)?;
    message_event(ctx, voter);
    Ok(())
}

/// `NewNonSplitVoteOption`.
fn non_split(option: i32) -> Vec<v1::WeightedVoteOption> {
    vec![v1::WeightedVoteOption {
        option,
        weight: "1.000000000000000000".to_owned(),
    }]
}

/// The v1 messages of a v1 `MsgSubmitProposal` (only
/// `MsgExecLegacyContent`, which `ValidateBasic` ensured).
fn exec_msgs(m: &v1::MsgSubmitProposal) -> Result<Vec<v1::MsgExecLegacyContent>> {
    m.messages
        .iter()
        .map(|a| super::msgs::gogo_view(a).and_then(|a| decode(&a)))
        .collect()
}

/// Executes a gov message; `None` for any other message.
pub fn route(ctx: &mut Context<'_>, msg: &Any) -> Option<Result<Any>> {
    let u = msg.type_url.as_str();
    Some(if u == v1beta1::MsgVote::type_url() {
        decode::<v1beta1::MsgVote>(msg).and_then(|m| {
            vote(ctx, m.proposal_id, &m.voter, non_split(m.option), "")
                .map(|()| Any::pack(&v1beta1::MsgVoteResponse {}))
        })
    } else if u == v1::MsgVote::type_url() {
        decode::<v1::MsgVote>(msg).and_then(|m| {
            vote(
                ctx,
                m.proposal_id,
                &m.voter,
                non_split(m.option),
                &m.metadata,
            )
            .map(|()| Any::pack(&v1::MsgVoteResponse {}))
        })
    } else if u == v1::MsgVoteWeighted::type_url() {
        decode::<v1::MsgVoteWeighted>(msg).and_then(|m| {
            vote(ctx, m.proposal_id, &m.voter, m.options, &m.metadata)
                .map(|()| Any::pack(&v1::MsgVoteWeightedResponse {}))
        })
    } else if u == v1beta1::MsgVoteWeighted::type_url() {
        decode::<v1beta1::MsgVoteWeighted>(msg).and_then(|m| {
            let opts = v1beta1_options(&m.options)?;
            vote(ctx, m.proposal_id, &m.voter, opts, "")
                .map(|()| Any::pack(&v1beta1::MsgVoteWeightedResponse {}))
        })
    } else if u == v1beta1::MsgDeposit::type_url() {
        decode::<v1beta1::MsgDeposit>(msg).and_then(|m| {
            deposit(ctx, m.proposal_id, &m.depositor, &m.amount)
                .map(|()| Any::pack(&v1beta1::MsgDepositResponse {}))
        })
    } else if u == v1::MsgDeposit::type_url() {
        decode::<v1::MsgDeposit>(msg).and_then(|m| {
            deposit(ctx, m.proposal_id, &m.depositor, &m.amount)
                .map(|()| Any::pack(&v1::MsgDepositResponse {}))
        })
    } else if u == v1beta1::MsgSubmitProposal::type_url() {
        decode::<v1beta1::MsgSubmitProposal>(msg).and_then(|m| {
            // `v1.NewLegacyContent(msg.GetContent(), govAcct)`: the content
            // re-marshaled by Go.
            let content = txdecode::gogo_repack(&m.content.clone().unwrap_or_default());
            let gov = x_auth::bech32(&x_gov::keeper::module_address())?;
            let exec = v1::MsgExecLegacyContent {
                content: Some(content),
                authority: gov,
            };
            submit_proposal(ctx, &[exec], &m.initial_deposit, &m.proposer, "")
                .map(|id| Any::pack(&v1beta1::MsgSubmitProposalResponse { proposal_id: id }))
        })
    } else if u == v1::MsgSubmitProposal::type_url() {
        decode::<v1::MsgSubmitProposal>(msg).and_then(|m| {
            let msgs = exec_msgs(&m)?;
            submit_proposal(ctx, &msgs, &m.initial_deposit, &m.proposer, &m.metadata)
                .map(|id| Any::pack(&v1::MsgSubmitProposalResponse { proposal_id: id }))
        })
    } else if u == v1::MsgExecLegacyContent::type_url() {
        // Only the gov account can sign it (only a proposal runs it).
        decode::<v1::MsgExecLegacyContent>(msg).and_then(|m| {
            let events = exec_legacy_content(ctx, &m)?;
            ctx.events.emit_all(events);
            Ok(Any::pack(&v1::MsgExecLegacyContentResponse {}))
        })
    } else {
        return None;
    })
}

// ---- EndBlocker -----------------------------------------------------------

fn must_get_proposal(ctx: &mut Context<'_>, id: u64) -> Result<v1::Proposal> {
    get_proposal(ctx, id)?
        .ok_or_else(|| sdk::error::go_panic(format!("proposal {id} does not exist")))
}

fn refund_and_delete_deposits(ctx: &mut Context<'_>, id: u64) -> Result<()> {
    for (key, d) in deposits(ctx, id)? {
        let depositor =
            sdk::address::acc_from_bech32_go(&d.depositor).map_err(sdk::error::go_panic)?;
        x_bank::send_from_module_to_account(
            ctx,
            x_gov::MODULE_NAME,
            &depositor,
            &Coins::from_proto(&d.amount)?,
        )
        .map_err(|e| sdk::error::go_panic(e.log))?;
        ctx.delete(x_gov::STORE_KEY, &key)?;
    }
    Ok(())
}

fn delete_and_burn_deposits(ctx: &mut Context<'_>, id: u64) -> Result<()> {
    for (key, d) in deposits(ctx, id)? {
        x_bank::burn_coins(ctx, x_gov::MODULE_NAME, &Coins::from_proto(&d.amount)?)
            .map_err(|e| sdk::error::go_panic(e.log))?;
        ctx.delete(x_gov::STORE_KEY, &key)?;
    }
    Ok(())
}

/// `Keeper.Tally` (v0.46): passes, burn deposits, the tally result.
fn tally(ctx: &mut Context<'_>, p: &v1::Proposal) -> Result<(bool, bool, v1::TallyResult)> {
    let counted = x_gov::tally::count_votes(ctx, p.id)?;
    let [quorum, threshold, veto] = tally_params(ctx)?;
    let result = counted.result();
    let math = |e: math::MathError| sdk::error::go_panic(e.to_string());
    let total_bonded = x_staking::validator::total_bonded_tokens(ctx)?;
    if total_bonded.is_zero() {
        return Ok((false, false, result));
    }
    let percent = counted
        .total
        .checked_quo(&total_bonded.to_legacy_dec())
        .map_err(math)?;
    if percent < quorum {
        return Ok((false, false, result));
    }
    let non_abstain = counted
        .total
        .checked_sub(&counted.results[2])
        .map_err(math)?;
    if non_abstain.is_zero() {
        return Ok((false, false, result));
    }
    if counted.results[4]
        .checked_quo(&counted.total)
        .map_err(math)?
        > veto
    {
        return Ok((false, true, result));
    }
    if counted.results[1].checked_quo(&non_abstain).map_err(math)? > threshold {
        return Ok((true, false, result));
    }
    Ok((false, false, result))
}

/// cosmos-sdk v0.46 gov `EndBlocker`.
pub fn end_block(ctx: &mut Context<'_>) -> Result<()> {
    let now: Timestamp = ctx.header.time;
    for (_, id) in x_gov::expired_queue_entries(ctx, x_gov::INACTIVE_QUEUE_PREFIX, &now)? {
        let p = must_get_proposal(ctx, id)?;
        // `DeleteProposal`.
        if let Some(t) = &p.deposit_end_time {
            ctx.delete(
                x_gov::STORE_KEY,
                &x_gov::queue_key(x_gov::INACTIVE_QUEUE_PREFIX, t, id),
            )?;
        }
        if let Some(t) = &p.voting_end_time {
            ctx.delete(
                x_gov::STORE_KEY,
                &x_gov::queue_key(x_gov::ACTIVE_QUEUE_PREFIX, t, id),
            )?;
        }
        ctx.delete(x_gov::STORE_KEY, &proposal_key(id))?;
        refund_and_delete_deposits(ctx, id)?;
        ctx.events.emit(event(
            "inactive_proposal",
            [
                ("proposal_id", id.to_string()),
                ("proposal_result", "proposal_dropped".to_owned()),
            ],
        ));
    }
    for (_, id) in x_gov::expired_queue_entries(ctx, x_gov::ACTIVE_QUEUE_PREFIX, &now)? {
        let mut p = must_get_proposal(ctx, id)?;
        let (passes, burn, result) = tally(ctx, &p)?;
        if burn {
            delete_and_burn_deposits(ctx, id)?;
        } else {
            refund_and_delete_deposits(ctx, id)?;
        }
        let tag = if passes {
            let cp = ctx.branch();
            let outer = std::mem::take(&mut ctx.events);
            let mut events = Vec::new();
            let mut failed = false;
            for a in &p.messages {
                if a.type_url != v1::MsgExecLegacyContent::type_url() {
                    ctx.events = outer;
                    ctx.discard(cp);
                    return Err(not_ported(format!("gov proposal message {}", a.type_url)));
                }
                let m: v1::MsgExecLegacyContent = decode(a)?;
                match exec_legacy_content(ctx, &m) {
                    Ok(ev) => events.extend(ev),
                    Err(e) if e.is_panic() => {
                        ctx.events = outer;
                        ctx.discard(cp);
                        return Err(not_ported("panic in a gov proposal handler"));
                    }
                    Err(_) => {
                        failed = true;
                        break;
                    }
                }
            }
            ctx.events = outer;
            if failed {
                ctx.discard(cp);
                p.status = v1::ProposalStatus::Failed as i32;
                "proposal_failed"
            } else {
                ctx.write(cp);
                ctx.events.emit_all(events);
                p.status = v1::ProposalStatus::Passed as i32;
                "proposal_passed"
            }
        } else {
            p.status = v1::ProposalStatus::Rejected as i32;
            "proposal_rejected"
        };
        p.final_tally_result = Some(result);
        set_proposal(ctx, &p)?;
        let vend = p.voting_end_time.unwrap_or_default();
        ctx.delete(
            x_gov::STORE_KEY,
            &x_gov::queue_key(x_gov::ACTIVE_QUEUE_PREFIX, &vend, id),
        )?;
        ctx.events.emit(event(
            "active_proposal",
            [
                ("proposal_id", id.to_string()),
                ("proposal_result", tag.to_owned()),
            ],
        ));
    }
    Ok(())
}
