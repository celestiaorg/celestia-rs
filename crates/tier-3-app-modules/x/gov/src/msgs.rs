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

/// `json.Unmarshal(metadata, &govtypes.ProposalMetadata{})` on the
/// metadata's bytes: `Some((title, summary))` (Go forms) when it succeeds.
///
/// Go validates the whole input first (at most 10 000 nested levels, any
/// length: `SubmitProposal` parses before the metadata length check), then
/// decodes the members in order: a key matches a field exactly or by
/// `EqualFold`, a later key overwrites an earlier one, `null` leaves a
/// string as it was, and a type error fails the call even when a later key
/// fixes the field. Unknown keys are skipped whatever they hold.
#[must_use]
pub fn metadata_title_summary(metadata: &[u8]) -> Option<(String, String)> {
    use gojson::Kind;
    /// `ProposalMetadata`'s JSON names; `authors` is the `[]string`.
    const FIELDS: [&str; 6] = [
        "title",
        "authors",
        "summary",
        "details",
        "proposal_forum_url",
        "vote_option_context",
    ];
    let v = gojson::parse(metadata).ok()?;
    let members = match &v.kind {
        Kind::Null => return Some(Default::default()),
        Kind::Object(m) => m,
        _ => return None,
    };
    let (mut title, mut summary, mut ok) = (String::new(), String::new(), true);
    for (k, val) in members {
        let Some(f) = FIELDS.iter().position(|f| f == k).or_else(|| {
            let k = gostr::from_go_bytes(k.as_bytes());
            FIELDS.iter().position(|f| gostr::equal_fold(&k, f))
        }) else {
            continue;
        };
        match (FIELDS[f], &val.kind) {
            (_, Kind::Null) => {}
            ("authors", Kind::Array(items)) => {
                ok &= items
                    .iter()
                    .all(|x| matches!(x.kind, Kind::String(_) | Kind::Null));
            }
            ("title", Kind::String(s)) => title = gostr::from_go_bytes(s.as_bytes()).into_owned(),
            ("summary", Kind::String(s)) => {
                summary = gostr::from_go_bytes(s.as_bytes()).into_owned();
            }
            (name, Kind::String(_)) if name != "authors" => {}
            _ => ok = false,
        }
    }
    ok.then_some((title, summary))
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
    if !msg.metadata.is_empty()
        && let Some((title, summary)) = metadata_title_summary(&gostr::to_go_bytes(&msg.metadata))
    {
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
    // `GetInitialDeposit()` is the decoded slice as is; `validateInitialDeposit`
    // checks it after reading the params.
    let params = crate::get_params(ctx)
        .map_err(|e| undefined(format!("failed to get governance parameters: {}", e.log)))?;
    if !sdk::coins::raw_coins_is_valid(&msg.initial_deposit)? {
        return Err(sdkerr::invalid_coins(sdk::coins::raw_coins_string(
            &msg.initial_deposit,
        )));
    }
    let initial = Coins::from_proto(&msg.initial_deposit)?;
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
    // `validateDeposit`: `IsValid` and `IsAllPositive` (false for no coins)
    // of the coins as decoded, `ErrInvalidCoins.Wrap(amount.String())`.
    if !sdk::coins::raw_coins_is_valid(&msg.amount)? || msg.amount.is_empty() {
        return Err(sdkerr::invalid_coins("").wrap(sdk::coins::raw_coins_string(&msg.amount)));
    }
    let amount = Coins::from_proto(&msg.amount)?;
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
        // `NewLegacyContent`: `NewAnyWithValue(content)` re-packs it.
        content: Some(txdecode::gogo_repack(content)),
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
    if gostr::go_len(&tp.title) > 140 {
        return Err(errors::invalid_proposal_content()
            .wrap("proposal title is longer than max length of 140"));
    }
    if tp.description.is_empty() {
        return Err(errors::invalid_proposal_content().wrap("proposal description cannot be blank"));
    }
    if gostr::go_len(&tp.description) > 10000 {
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

#[cfg(test)]
mod tests {
    use super::metadata_title_summary as meta;

    /// Outcomes of cosmos-sdk v0.52.12's `json.Unmarshal` into
    /// `ProposalMetadata` (the v10.4.0 oracle's `json_gov` op).
    #[test]
    fn metadata_decoding_is_go_s() {
        let ts = |t: &str, s: &str| Some((t.to_owned(), s.to_owned()));
        let deep = |d: usize| format!("{}0{}", "[".repeat(d), "]".repeat(d));
        for d in [127, 128, 1_000, 9_999] {
            let m = format!(r#"{{"title":"T","summary":"S","x":{}}}"#, deep(d));
            assert_eq!(meta(m.as_bytes()), ts("T", "S"), "{d}");
        }
        let m = format!(r#"{{"title":"T","summary":"S","x":{}}}"#, deep(10_000));
        assert_eq!(meta(m.as_bytes()), None);
        assert_eq!(meta(br#"{"title":1,"title":"T","summary":"S"}"#), None);
        assert_eq!(
            meta(br#"{"title":"b","Title":"a","summary":"S"}"#),
            ts("a", "S")
        );
        assert_eq!(
            meta(br#"{"title":"a","title":null,"summary":"S"}"#),
            ts("a", "S")
        );
        assert_eq!(
            meta("{\"\u{17f}ummary\":\"S\",\"title\":\"T\"}".as_bytes()),
            ts("T", "S")
        );
        assert_eq!(
            meta(br#"{"title":"T","summary":"S","x":1e400}"#),
            ts("T", "S")
        );
        assert_eq!(meta(br#"{"title":"T","authors":[1]}"#), None);
        assert_eq!(meta(b"null"), ts("", ""));
        assert_eq!(meta(b"[]"), None);
    }
}
