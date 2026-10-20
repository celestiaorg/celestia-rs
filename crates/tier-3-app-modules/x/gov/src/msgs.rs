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
    let proposer = sdk::address::acc_codec_to_bytes(&msg.proposer)
        .map_err(|e| invalid_address("proposer", &e))?;
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
    sdk::address::acc_codec_to_bytes(&msg.proposer).map_err(|e| invalid_address("proposer", &e))?;
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
    // `LegacyContentFromMessage`: `msg.Content.GetCachedValue()` on a nil
    // `*Any` dereferences it (also inside `SubmitProposal`'s check).
    let Some(content) = &msg.content else {
        return Err(sdk::error::go_panic(
            "runtime error: invalid memory address or nil pointer dereference",
        ));
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
    let voter =
        sdk::address::acc_codec_to_bytes(&msg.voter).map_err(|e| invalid_address("voter", &e))?;
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
    vote_weighted_from(ctx, msg, "null")
}

/// `VoteWeighted`; `no_options` is `json.Marshal` of an empty
/// `msg.Options`: `null` as decoded (nil), `[]` as the v1beta1 server
/// builds it (`make([]*v1.WeightedVoteOption, 0)`).
fn vote_weighted_from(
    ctx: &mut Context<'_>,
    msg: &v1::MsgVoteWeighted,
    no_options: &str,
) -> Result<v1::MsgVoteWeightedResponse> {
    let voter =
        sdk::address::acc_codec_to_bytes(&msg.voter).map_err(|e| invalid_address("voter", &e))?;
    if msg.options.is_empty() {
        return Err(sdkerr::invalid_request(no_options));
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
    let depositor = sdk::address::acc_codec_to_bytes(&msg.depositor)
        .map_err(|e| invalid_address("depositor", &e))?;
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

/// `Params.ValidateBasic` (plain errors, Go's texts: a passed proposal that
/// fails stores the text as its `failed_reason`).
#[allow(clippy::too_many_lines)]
pub fn validate_params(p: &v1::Params) -> Result<()> {
    use math::LegacyDec;
    use sdk::time::{duration_nanos, go_duration_string};
    let one = LegacyDec::one();
    let dec = |s: &str, prefix: &str| {
        LegacyDec::from_str_go(s).map_err(|e| undefined(format!("{prefix}: {e}")))
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
    // `%d` of a nil `*time.Duration` is `0`. Of a non-nil one it is the
    // pointer's address (nondeterministic in Go, docs/security.md); Rust
    // prints the nanoseconds.
    let Some(mdp) = &p.max_deposit_period else {
        return Err(undefined("maximum deposit period must not be nil: 0"));
    };
    let n = duration_nanos(mdp);
    if n <= 0 {
        return Err(undefined(format!(
            "maximum deposit period must be positive: {n}"
        )));
    }
    let quorum = dec(&p.quorum, "invalid quorum string")?;
    if quorum.is_negative() {
        return Err(undefined(format!("quorom cannot be negative: {quorum}")));
    }
    if quorum > one {
        return Err(undefined(format!("quorom too large: {}", p.quorum)));
    }
    let threshold = dec(&p.threshold, "invalid threshold string")?;
    if !threshold.is_positive() {
        return Err(undefined(format!(
            "vote threshold must be positive: {threshold}"
        )));
    }
    if threshold > one {
        return Err(undefined(format!("vote threshold too large: {threshold}")));
    }
    let et = dec(&p.expedited_threshold, "invalid expedited threshold string")?;
    if !et.is_positive() {
        return Err(undefined(format!(
            "expedited vote threshold must be positive: {et}"
        )));
    }
    if et > one {
        return Err(undefined(format!(
            "expedited vote threshold too large: {et}"
        )));
    }
    if et <= threshold {
        return Err(undefined(format!(
            "expedited vote threshold {et}, must be greater than the regular threshold {threshold}"
        )));
    }
    let veto = dec(&p.veto_threshold, "invalid vetoThreshold string")?;
    if !veto.is_positive() {
        return Err(undefined(format!(
            "veto threshold must be positive: {veto}"
        )));
    }
    if veto > one {
        return Err(undefined(format!("veto threshold too large: {veto}")));
    }
    // `%s` of a `*time.Duration` is its `String()`.
    let positive = |d: &Option<pbjson_types::Duration>, what: &str| -> Result<i128> {
        let d = d
            .as_ref()
            .ok_or_else(|| undefined(format!("{what} must not be nil: 0")))?;
        let n = duration_nanos(d);
        if n <= 0 {
            return Err(undefined(format!(
                "{what} must be positive: {}",
                go_duration_string(n)
            )));
        }
        Ok(n)
    };
    let vp = positive(&p.voting_period, "voting period")?;
    let evp = positive(&p.expedited_voting_period, "expedited voting period")?;
    // Go compares Duration.Seconds() as float64.
    #[allow(clippy::cast_precision_loss)]
    if (evp as f64) / 1e9 >= (vp as f64) / 1e9 {
        return Err(undefined(format!(
            "expedited voting period {} must be strictly less that the regular voting period {}",
            go_duration_string(evp),
            go_duration_string(vp)
        )));
    }
    let midr = dec(
        &p.min_initial_deposit_ratio,
        "invalid mininum initial deposit ratio of proposal",
    )?;
    if midr.is_negative() {
        return Err(undefined(format!(
            "mininum initial deposit ratio of proposal must be positive: {midr}"
        )));
    }
    if midr > one {
        return Err(undefined(format!(
            "mininum initial deposit ratio of proposal is too large: {midr}"
        )));
    }
    let pcr = dec(
        &p.proposal_cancel_ratio,
        "invalid burn rate of cancel proposal",
    )?;
    if pcr.is_negative() {
        return Err(undefined(format!(
            "burn rate of cancel proposal must be positive: {pcr}"
        )));
    }
    if pcr > one {
        return Err(undefined(format!(
            "burn rate of cancel proposal is too large: {pcr}"
        )));
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
    // `msg.GetContent()`: `GetCachedValue` on a nil `*Any` panics.
    let Some(content) = &msg.content else {
        return Err(sdk::error::go_panic(
            "runtime error: invalid memory address or nil pointer dereference",
        ));
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
            // `opt.Weight.String()`: an empty encoding leaves the
            // `LegacyDec` nil, which prints `<nil>`.
            let weight = if o.weight.is_empty() {
                "<nil>".to_owned()
            } else {
                math::LegacyDec::unmarshal(o.weight.as_bytes())
                    .map_err(|e| undefined(e.to_string()))?
                    .to_string()
            };
            Ok(v1::WeightedVoteOption {
                option: o.option,
                weight,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    vote_weighted_from(
        ctx,
        &v1::MsgVoteWeighted {
            proposal_id: msg.proposal_id,
            voter: msg.voter.clone(),
            options,
            metadata: String::new(),
        },
        "[]",
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

    /// cosmos-sdk v0.52.8 / v0.52.12 `Params.ValidateBasic` texts: a passed
    /// `MsgUpdateParams` proposal that fails stores them as `failed_reason`
    /// (deliver-diff `v9-gov` seed 4018 split the app hash on
    /// `voting period must be positive: 0s`).
    #[test]
    fn validate_params_errors_are_go_s() {
        use celestia_proto::cosmos::base::v1beta1::Coin;
        use celestia_proto::cosmos::gov::v1::Params;
        use pbjson_types::Duration;
        let secs = |s: i64| {
            Some(Duration {
                seconds: s,
                nanos: 0,
            })
        };
        let coin = |a: &str| {
            vec![Coin {
                denom: "utia".into(),
                amount: a.into(),
            }]
        };
        let ok = Params {
            min_deposit: coin("10"),
            expedited_min_deposit: coin("50"),
            max_deposit_period: secs(172_800),
            voting_period: secs(172_800),
            expedited_voting_period: secs(86_400),
            quorum: "0.334".into(),
            threshold: "0.5".into(),
            expedited_threshold: "0.667".into(),
            veto_threshold: "0.334".into(),
            min_initial_deposit_ratio: "0".into(),
            proposal_cancel_ratio: "0.5".into(),
            ..Default::default()
        };
        assert!(super::validate_params(&ok).is_ok());
        let err = |f: &dyn Fn(&mut Params)| {
            let mut p = ok.clone();
            f(&mut p);
            super::validate_params(&p).unwrap_err().log
        };
        let d18 = |s: &str| format!("{s}000000000000000");
        for (f, want) in [
            (
                &(|p: &mut Params| p.voting_period = secs(0)) as &dyn Fn(&mut Params),
                "voting period must be positive: 0s".to_owned(),
            ),
            (
                &|p: &mut Params| p.voting_period = None,
                "voting period must not be nil: 0".into(),
            ),
            (
                &|p: &mut Params| p.expedited_voting_period = secs(-3_600),
                "expedited voting period must be positive: -1h0m0s".into(),
            ),
            (
                &|p: &mut Params| p.expedited_voting_period = secs(172_800),
                "expedited voting period 48h0m0s must be strictly less that the regular voting period 48h0m0s".into(),
            ),
            (
                &|p: &mut Params| p.max_deposit_period = None,
                "maximum deposit period must not be nil: 0".into(),
            ),
            (
                &|p: &mut Params| p.quorum = "-0.1".into(),
                format!("quorom cannot be negative: -{}", d18("0.100")),
            ),
            (
                &|p: &mut Params| p.quorum = "1.5".into(),
                "quorom too large: 1.5".into(),
            ),
            (
                &|p: &mut Params| p.quorum = String::new(),
                "invalid quorum string: decimal string cannot be empty".into(),
            ),
            (
                &|p: &mut Params| p.threshold = "0".into(),
                format!("vote threshold must be positive: {}", d18("0.000")),
            ),
            (
                &|p: &mut Params| p.expedited_threshold = "0.5".into(),
                format!(
                    "expedited vote threshold {}, must be greater than the regular threshold {}",
                    d18("0.500"),
                    d18("0.500")
                ),
            ),
            (
                &|p: &mut Params| p.veto_threshold = "2".into(),
                format!("veto threshold too large: {}", d18("2.000")),
            ),
            (
                &|p: &mut Params| p.min_initial_deposit_ratio = "x".into(),
                "invalid mininum initial deposit ratio of proposal: failed to set decimal string with base 10: x000000000000000000".into(),
            ),
            (
                &|p: &mut Params| p.proposal_cancel_ratio = "-1".into(),
                format!("burn rate of cancel proposal must be positive: -{}", d18("1.000")),
            ),
        ] {
            assert_eq!(err(f), want);
        }
    }
}
