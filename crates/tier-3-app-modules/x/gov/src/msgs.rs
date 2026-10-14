//! Gov message server (`x/gov/keeper/msg_server.go`), v1 and the v1beta1
//! legacy wrappers.

use celestia_proto::Any;
use celestia_proto::cosmos::gov::{v1, v1beta1};
use prost::{Message, Name};
use sdk::address::{acc_from_bech32, undefined};
use sdk::coins::Coins;
use sdk::error::sdk as sdkerr;
use sdk::event::event;
use sdk::{Context, Result};

use crate::errors;
use crate::keeper::{self, MsgRouter};

fn invalid_address(what: &str, e: &sdk::SdkError) -> sdk::SdkError {
    sdkerr::invalid_address(format!("invalid {what} address: {}", e.log))
}

/// `sdk.NewCoins(coins...)`: sanitizes; panics (ErrPanic) on invalid coins.
fn new_coins(coins: &[celestia_proto::cosmos::base::v1beta1::Coin]) -> Result<Coins> {
    let c = Coins::from_proto(coins).map_err(|e| sdk::error::go_panic(e.log))?;
    let mut v = c.0;
    v.retain(|c| !c.is_zero());
    v.sort_by(|a, b| a.denom.cmp(&b.denom));
    let out = Coins(v);
    let dup = out.0.windows(2).any(|w| w[0].denom == w[1].denom);
    if dup || !out.is_valid() {
        return Err(sdk::error::go_panic(format!("invalid coins: {out}")));
    }
    Ok(out)
}

/// `ProposalMetadata` JSON check: `Ok(Some((title, summary)))` when Go's
/// `json.Unmarshal` into the struct succeeds.
fn metadata_title_summary(metadata: &str) -> Option<(String, String)> {
    let v: serde_json::Value = serde_json::from_str(metadata).ok()?;
    let mut title = String::new();
    let mut summary = String::new();
    match v {
        serde_json::Value::Null => {}
        serde_json::Value::Object(map) => {
            let mut ok = true;
            for (k, val) in map {
                let lk = k.to_ascii_lowercase();
                match lk.as_str() {
                    "title"
                    | "summary"
                    | "details"
                    | "proposal_forum_url"
                    | "vote_option_context" => match val {
                        serde_json::Value::String(s) => {
                            if lk == "title" {
                                title = s;
                            } else if lk == "summary" {
                                summary = s;
                            }
                        }
                        serde_json::Value::Null => {}
                        _ => ok = false,
                    },
                    "authors" => match val {
                        serde_json::Value::Array(a) => {
                            if !a.iter().all(|x| x.is_string() || x.is_null()) {
                                ok = false;
                            }
                        }
                        serde_json::Value::Null => {}
                        _ => ok = false,
                    },
                    _ => {}
                }
            }
            if !ok {
                return None;
            }
        }
        _ => return None,
    }
    Some((title, summary))
}

/// `MsgSubmitProposal`.
pub fn submit_proposal(
    ctx: &mut Context<'_>,
    router: &dyn MsgRouter,
    msg: &v1::MsgSubmitProposal,
) -> Result<v1::MsgSubmitProposalResponse> {
    if msg.title.is_empty() {
        return Err(sdkerr::invalid_request("proposal title cannot be empty"));
    }
    if msg.summary.is_empty() {
        return Err(sdkerr::invalid_request("proposal summary cannot be empty"));
    }
    let proposer = acc_from_bech32(&msg.proposer).map_err(|e| invalid_address("proposer", &e))?;
    if msg.messages.is_empty() && msg.metadata.is_empty() {
        return Err(
            errors::no_proposal_msgs().wrap("either metadata or Msgs length must be non-nil")
        );
    }
    if !msg.metadata.is_empty() {
        if let Some((title, summary)) = metadata_title_summary(&msg.metadata) {
            if title != msg.title {
                return Err(errors::invalid_proposal_content().wrap(format!(
                    "metadata title '{title}' must equal proposal title '{}'",
                    msg.title
                )));
            }
            if summary != msg.summary {
                return Err(errors::invalid_proposal_content().wrap(format!(
                    "metadata summary '{summary}' must equal proposal summary '{}'",
                    msg.summary
                )));
            }
        }
    }
    let initial = new_coins(&msg.initial_deposit)?;
    let params = crate::get_params(ctx)
        .map_err(|e| undefined(format!("failed to get governance parameters: {}", e.log)))?;
    keeper::validate_initial_deposit(&params, &initial, msg.expedited)?;
    keeper::validate_deposit_denom(&params, &initial)?;
    let proposal = keeper::submit_proposal(
        ctx,
        router,
        &msg.messages,
        &msg.metadata,
        &msg.title,
        &msg.summary,
        &proposer,
        msg.expedited,
    )?;
    let len = proposal.encoded_len() as u64;
    ctx.gas
        .consume(3 * 30 * len, "submit proposal")
        .map_err(|_| sdkerr::out_of_gas(""))?;
    let started = keeper::add_deposit(ctx, proposal.id, &proposer, &initial)?;
    if started {
        ctx.events.emit(event(
            "submit_proposal",
            [("voting_period_start", proposal.id.to_string())],
        ));
    }
    Ok(v1::MsgSubmitProposalResponse {
        proposal_id: proposal.id,
    })
}

/// `MsgCancelProposal`.
pub fn cancel_proposal(
    ctx: &mut Context<'_>,
    msg: &v1::MsgCancelProposal,
) -> Result<v1::MsgCancelProposalResponse> {
    acc_from_bech32(&msg.proposer).map_err(|e| invalid_address("proposer", &e))?;
    keeper::cancel_proposal(ctx, msg.proposal_id, &msg.proposer)?;
    ctx.events.emit(event(
        "cancel_proposal",
        [
            ("sender", msg.proposer.clone()),
            ("proposal_id", msg.proposal_id.to_string()),
        ],
    ));
    Ok(v1::MsgCancelProposalResponse {
        proposal_id: msg.proposal_id,
        canceled_time: Some(ctx.header.time),
        canceled_height: u64::try_from(ctx.header.height).unwrap_or_default(),
    })
}

/// `MsgExecLegacyContent`. celestia's legacy router has no routes, so any
/// content that unpacks fails with `ErrNoProposalHandlerExists`.
pub fn exec_legacy_content(
    ctx: &mut Context<'_>,
    msg: &v1::MsgExecLegacyContent,
) -> Result<v1::MsgExecLegacyContentResponse> {
    let gov = x_auth::bech32(&keeper::governance_account(ctx)?)?;
    if gov != msg.authority {
        return Err(errors::invalid_signer().wrap(format!("expected {gov} got {}", msg.authority)));
    }
    let Some(content) = &msg.content else {
        return Err(errors::invalid_proposal_content()
            .wrap("invalid type: expected v1beta1.Content, got <nil>: invalid type"));
    };
    let route = legacy_content_route(content).ok_or_else(|| {
        errors::invalid_proposal_content().wrap(format!(
            "expected v1beta1.Content, got {}: invalid type",
            content.type_url
        ))
    })?;
    Err(errors::no_proposal_handler_exists().wrap(route))
}

/// `Content.ProposalRoute()` of the registered legacy content types.
#[must_use]
pub fn legacy_content_route(content: &Any) -> Option<&'static str> {
    match content.type_url.as_str() {
        "/cosmos.gov.v1beta1.TextProposal" => Some("gov"),
        "/cosmos.params.v1beta1.ParameterChangeProposal" => Some("params"),
        "/cosmos.upgrade.v1beta1.SoftwareUpgradeProposal"
        | "/cosmos.upgrade.v1beta1.CancelSoftwareUpgradeProposal" => Some("upgrade"),
        "/ibc.core.client.v1.ClientUpdateProposal" | "/ibc.core.client.v1.UpgradeProposal" => {
            Some("ibc")
        }
        _ => None,
    }
}

/// `MsgExecLegacyContent.ValidateBasic`.
pub fn validate_basic_exec_legacy_content(msg: &v1::MsgExecLegacyContent) -> Result<()> {
    acc_from_bech32(&msg.authority).map(|_| ())
}

/// `MsgVote`.
pub fn vote(ctx: &mut Context<'_>, msg: &v1::MsgVote) -> Result<v1::MsgVoteResponse> {
    let voter = acc_from_bech32(&msg.voter).map_err(|e| invalid_address("voter", &e))?;
    if !keeper::valid_vote_option(msg.option) {
        return Err(errors::invalid_vote().wrap(keeper::vote_option_name(msg.option)));
    }
    let opts = [v1::WeightedVoteOption {
        option: msg.option,
        weight: math::LegacyDec::one().to_string(),
    }];
    keeper::add_vote(ctx, msg.proposal_id, &voter, &opts, &msg.metadata)?;
    Ok(v1::MsgVoteResponse {})
}

/// `MsgVoteWeighted`.
pub fn vote_weighted(
    ctx: &mut Context<'_>,
    msg: &v1::MsgVoteWeighted,
) -> Result<v1::MsgVoteWeightedResponse> {
    let voter = acc_from_bech32(&msg.voter).map_err(|e| invalid_address("voter", &e))?;
    if msg.options.is_empty() {
        return Err(sdkerr::invalid_request("[]"));
    }
    let mut total = math::LegacyDec::zero();
    let mut used = Vec::new();
    for o in &msg.options {
        if !keeper::valid_weighted_option(o) {
            return Err(errors::invalid_vote().wrap(keeper::weighted_option_string(o)));
        }
        let w = math::LegacyDec::from_str_go(&o.weight)
            .map_err(|e| errors::invalid_vote().wrap(format!("invalid weight: {e}")))?;
        total = total
            .checked_add(&w)
            .map_err(|e| sdk::error::go_panic(e.to_string()))?;
        if used.contains(&o.option) {
            return Err(errors::invalid_vote().wrap("duplicated vote option"));
        }
        used.push(o.option);
    }
    if total > math::LegacyDec::one() {
        return Err(errors::invalid_vote().wrap("total weight overflow 1.00"));
    }
    if total < math::LegacyDec::one() {
        return Err(errors::invalid_vote().wrap("total weight lower than 1.00"));
    }
    keeper::add_vote(ctx, msg.proposal_id, &voter, &msg.options, &msg.metadata)?;
    Ok(v1::MsgVoteWeightedResponse {})
}

/// `MsgDeposit`.
pub fn deposit(ctx: &mut Context<'_>, msg: &v1::MsgDeposit) -> Result<v1::MsgDepositResponse> {
    let depositor =
        acc_from_bech32(&msg.depositor).map_err(|e| invalid_address("depositor", &e))?;
    let amount = Coins::from_proto(&msg.amount)?;
    if !amount.is_valid() || amount.iter().any(|c| !c.amount.is_positive()) {
        return Err(sdkerr::invalid_coins(amount.to_string()));
    }
    if keeper::add_deposit(ctx, msg.proposal_id, &depositor, &amount)? {
        ctx.events.emit(event(
            "proposal_deposit",
            [("voting_period_start", msg.proposal_id.to_string())],
        ));
    }
    Ok(v1::MsgDepositResponse {})
}

/// `Params.ValidateBasic` (plain errors).
pub fn validate_params(p: &v1::Params) -> Result<()> {
    use math::LegacyDec;
    let one = LegacyDec::one();
    let dec = |s: &str, what: &str| {
        LegacyDec::from_str_go(s).map_err(|e| undefined(format!("invalid {what} string: {e}")))
    };
    let min = Coins::from_proto(&p.min_deposit)?;
    if min.is_empty() || !min.is_valid() {
        return Err(undefined(format!("invalid minimum deposit: {min}")));
    }
    let exp = Coins::from_proto(&p.expedited_min_deposit)?;
    if exp.is_empty() || !exp.is_valid() {
        return Err(undefined(format!(
            "invalid expedited minimum deposit: {exp}"
        )));
    }
    // Coins.IsAllLTE: exp <= min for every denom of exp.
    if exp.iter().all(|c| c.amount <= min.amount_of(&c.denom)) {
        return Err(undefined(format!(
            "expedited minimum deposit must be greater than minimum deposit: {exp}"
        )));
    }
    let positive = |d: &Option<pbjson_types::Duration>, what: &str| -> Result<i128> {
        let d = d
            .as_ref()
            .ok_or_else(|| undefined(format!("{what} must not be nil")))?;
        let n = sdk::time::duration_nanos(d);
        if n <= 0 {
            return Err(undefined(format!("{what} must be positive")));
        }
        Ok(n)
    };
    positive(&p.max_deposit_period, "maximum deposit period")?;
    let quorum = dec(&p.quorum, "quorum")?;
    if quorum.is_negative() || quorum > one {
        return Err(undefined("invalid quorum"));
    }
    let threshold = dec(&p.threshold, "threshold")?;
    if !threshold.is_positive() || threshold > one {
        return Err(undefined("invalid vote threshold"));
    }
    let et = dec(&p.expedited_threshold, "expedited threshold")?;
    if !et.is_positive() || et > one || et <= threshold {
        return Err(undefined("invalid expedited vote threshold"));
    }
    let veto = dec(&p.veto_threshold, "vetoThreshold")?;
    if !veto.is_positive() || veto > one {
        return Err(undefined("invalid veto threshold"));
    }
    let vp = positive(&p.voting_period, "voting period")?;
    let evp = positive(&p.expedited_voting_period, "expedited voting period")?;
    // Go compares Duration.Seconds() as float64.
    #[allow(clippy::cast_precision_loss)]
    if (evp as f64) / 1e9 >= (vp as f64) / 1e9 {
        return Err(undefined(
            "expedited voting period must be strictly less that the regular voting period",
        ));
    }
    let midr = dec(
        &p.min_initial_deposit_ratio,
        "mininum initial deposit ratio",
    )?;
    if midr.is_negative() || midr > one {
        return Err(undefined("invalid mininum initial deposit ratio"));
    }
    let pcr = dec(&p.proposal_cancel_ratio, "burn rate of cancel proposal")?;
    if pcr.is_negative() || pcr > one {
        return Err(undefined("invalid burn rate of cancel proposal"));
    }
    if !p.proposal_cancel_dest.is_empty() && acc_from_bech32(&p.proposal_cancel_dest).is_err() {
        return Err(undefined(format!(
            "deposits destination address is invalid: {}",
            p.proposal_cancel_dest
        )));
    }
    Ok(())
}

/// `MsgUpdateParams`.
pub fn update_params(
    ctx: &mut Context<'_>,
    msg: &v1::MsgUpdateParams,
) -> Result<v1::MsgUpdateParamsResponse> {
    let authority = x_auth::bech32(&keeper::module_address())?;
    if authority != msg.authority {
        return Err(errors::invalid_signer().wrap(format!(
            "invalid authority; expected {authority}, got {}",
            msg.authority
        )));
    }
    let p = msg.params.clone().unwrap_or_default();
    validate_params(&p)?;
    crate::set_params(ctx, &p)?;
    Ok(v1::MsgUpdateParamsResponse {})
}

/// v1beta1 `MsgSubmitProposal`: wraps the content in `MsgExecLegacyContent`.
pub fn legacy_submit_proposal(
    ctx: &mut Context<'_>,
    router: &dyn MsgRouter,
    msg: &v1beta1::MsgSubmitProposal,
) -> Result<v1beta1::MsgSubmitProposalResponse> {
    let Some(content) = &msg.content else {
        return Err(errors::invalid_proposal_content().wrap("missing content"));
    };
    let (title, description) = legacy_content_title_description(content)?;
    let gov = x_auth::bech32(&keeper::module_address())?;
    let exec = v1::MsgExecLegacyContent {
        content: Some(content.clone()),
        authority: gov,
    };
    let v1msg = v1::MsgSubmitProposal {
        messages: vec![Any {
            type_url: v1::MsgExecLegacyContent::type_url(),
            value: exec.encode_to_vec(),
        }],
        initial_deposit: msg.initial_deposit.clone(),
        proposer: msg.proposer.clone(),
        metadata: String::new(),
        title,
        summary: description,
        expedited: false,
    };
    let r = submit_proposal(ctx, router, &v1msg)?;
    Ok(v1beta1::MsgSubmitProposalResponse {
        proposal_id: r.proposal_id,
    })
}

/// Title and description of a legacy content after `IsValidProposalType`
/// and `ValidateAbstract`.
fn legacy_content_title_description(content: &Any) -> Result<(String, String)> {
    let registered = [
        "/cosmos.gov.v1beta1.TextProposal",
        "/cosmos.params.v1beta1.ParameterChangeProposal",
        "/cosmos.upgrade.v1beta1.SoftwareUpgradeProposal",
        "/cosmos.upgrade.v1beta1.CancelSoftwareUpgradeProposal",
        "/ibc.core.client.v1.ClientUpdateProposal",
        "/ibc.core.client.v1.UpgradeProposal",
    ];
    if !registered.contains(&content.type_url.as_str()) {
        return Err(errors::invalid_proposal_type().wrap(&content.type_url));
    }
    // Every registered content type has `title = 1` and `description = 2`
    // (type-specific ValidateBasic checks beyond ValidateAbstract are not
    // ported; such proposals fail with ErrNoProposalHandlerExists anyway).
    let tp = v1beta1::TextProposal::decode(content.value.as_slice())
        .map_err(|e| undefined(e.to_string()))?;
    // ValidateAbstract.
    if tp.title.trim().is_empty() {
        return Err(errors::invalid_proposal_content().wrap("proposal title cannot be blank"));
    }
    if tp.title.len() > 140 {
        return Err(errors::invalid_proposal_content()
            .wrap("proposal title is longer than max length of 140"));
    }
    if tp.description.is_empty() {
        return Err(errors::invalid_proposal_content().wrap("proposal description cannot be blank"));
    }
    if tp.description.len() > 10000 {
        return Err(errors::invalid_proposal_content()
            .wrap("proposal description is longer than max length of 10000"));
    }
    Ok((tp.title, tp.description))
}

/// v1beta1 `MsgVote`.
pub fn legacy_vote(
    ctx: &mut Context<'_>,
    msg: &v1beta1::MsgVote,
) -> Result<v1beta1::MsgVoteResponse> {
    vote(
        ctx,
        &v1::MsgVote {
            proposal_id: msg.proposal_id,
            voter: msg.voter.clone(),
            option: msg.option,
            metadata: String::new(),
        },
    )?;
    Ok(v1beta1::MsgVoteResponse {})
}

/// v1beta1 `MsgVoteWeighted` (weights are `LegacyDec` wire strings).
pub fn legacy_vote_weighted(
    ctx: &mut Context<'_>,
    msg: &v1beta1::MsgVoteWeighted,
) -> Result<v1beta1::MsgVoteWeightedResponse> {
    let options = msg
        .options
        .iter()
        .map(|o| {
            let w = math::LegacyDec::unmarshal(o.weight.as_bytes())
                .map_err(|e| undefined(e.to_string()))?;
            Ok(v1::WeightedVoteOption {
                option: o.option,
                weight: w.to_string(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    vote_weighted(
        ctx,
        &v1::MsgVoteWeighted {
            proposal_id: msg.proposal_id,
            voter: msg.voter.clone(),
            options,
            metadata: String::new(),
        },
    )?;
    Ok(v1beta1::MsgVoteWeightedResponse {})
}

/// v1beta1 `MsgDeposit`.
pub fn legacy_deposit(
    ctx: &mut Context<'_>,
    msg: &v1beta1::MsgDeposit,
) -> Result<v1beta1::MsgDepositResponse> {
    deposit(
        ctx,
        &v1::MsgDeposit {
            proposal_id: msg.proposal_id,
            depositor: msg.depositor.clone(),
            amount: msg.amount.clone(),
        },
    )?;
    Ok(v1beta1::MsgDepositResponse {})
}
