//! The cosmos-sdk v0.46 message router of celestia-app v1
//! (`MsgServiceRouter` with v0.46 msg servers): `ValidateBasic` and the
//! handlers of the ported messages. A message accepted at app version 1
//! whose handler is not ported fails with an explicit error.

use celestia_proto::Any;
use celestia_proto::cosmos::bank::v1beta1 as bank;
use celestia_proto::cosmos::distribution::v1beta1 as distr;
use celestia_proto::cosmos::staking::v1beta1 as staking;
use prost::{Message, Name};
use sdk::coins::Coins;
use sdk::error::sdk as errs;
use sdk::event::event;
use sdk::{Context, Result};

fn decode<M: Message + Default>(msg: &Any) -> Result<M> {
    M::decode(msg.value.as_slice()).map_err(|e| errs::tx_decode(e.to_string()))
}

/// `sdk.AccAddressFromBech32`, failing with `invalid <what> address`.
pub(super) fn acc_address(s: &str, what: &str) -> Result<Vec<u8>> {
    sdk::address::acc_from_bech32_go(s)
        .map_err(|e| errs::invalid_address(format!("invalid {what} address: {e}")))
}

/// `Coins.IsValid() && Coins.IsAllPositive()` of decoded coins, failing
/// with `ErrInvalidCoins` wrapping their string.
fn valid_positive_coins(coins: &[celestia_proto::cosmos::base::v1beta1::Coin]) -> Result<Coins> {
    let invalid = || errs::invalid_coins("").wrap(sdk::coins::raw_coins_string(coins));
    if !sdk::coins::raw_coins_is_valid(coins)? {
        return Err(invalid());
    }
    let c = Coins::from_proto(coins)?;
    // `IsAllPositive` is false for empty coins.
    if c.is_empty() || !c.iter().all(|c| c.amount.is_positive()) {
        return Err(invalid());
    }
    Ok(c)
}

/// v0.46 `Coins.IsEqual`: different lengths are unequal; otherwise the
/// sorted coins are compared pairwise and a denom mismatch is a Go panic.
fn coins_is_equal(a: &Coins, b: &Coins) -> Result<bool> {
    if a.0.len() != b.0.len() {
        return Ok(false);
    }
    for (x, y) in a.iter().zip(b.iter()) {
        if x.denom != y.denom {
            return Err(sdk::error::go_panic(format!(
                "invalid coin denominations; {x}, {y}"
            )));
        }
        if x.amount != y.amount {
            return Ok(false);
        }
    }
    Ok(true)
}

/// bank `ValidateInputsOutputs` (v0.46: any number of inputs).
fn validate_inputs_outputs(inputs: &[bank::Input], outputs: &[bank::Output]) -> Result<()> {
    let mut total_in = Coins::new();
    for i in inputs {
        acc_address(&i.address, "input")?;
        total_in = total_in.add(&valid_positive_coins(&i.coins)?)?;
    }
    let mut total_out = Coins::new();
    for o in outputs {
        acc_address(&o.address, "output")?;
        total_out = total_out.add(&valid_positive_coins(&o.coins)?)?;
    }
    if !coins_is_equal(&total_in, &total_out)? {
        return Err(x_bank::msgs::errors::input_output_mismatch());
    }
    Ok(())
}

/// `sdk.ValAddressFromBech32`, failing with `invalid <what> address`.
fn val_address(s: &str, what: &str) -> Result<Vec<u8>> {
    x_staking::addr::val_from_string(s)
        .map_err(|e| errs::invalid_address(format!("invalid {what} address: {}", e.log)))
}

/// `!coin.IsValid() || !coin.Amount.IsPositive()` of a staking amount.
fn valid_positive_coin(
    c: Option<&celestia_proto::cosmos::base::v1beta1::Coin>,
) -> Result<sdk::coins::Coin> {
    let (coin, valid) = x_staking::msgs::parse_coin(c);
    if !valid || !coin.amount.is_positive() {
        return Err(errs::invalid_request("invalid delegation amount"));
    }
    Ok(coin)
}

/// `!msg.Amount.IsValid() || !msg.Amount.Amount.IsPositive()` of an
/// undelegation or redelegation.
fn valid_shares_amount(
    c: Option<&celestia_proto::cosmos::base::v1beta1::Coin>,
) -> Result<sdk::coins::Coin> {
    let (coin, valid) = x_staking::msgs::parse_coin(c);
    if !valid || !coin.amount.is_positive() {
        return Err(errs::invalid_request("invalid shares amount"));
    }
    Ok(coin)
}

/// staking `MsgCreateValidator.ValidateBasic` (v0.46): returns the
/// validated commission rates (at a zero update time) and minimum self
/// delegation. (v0.46 still uses the delegator address.)
#[allow(deprecated)]
fn validate_create_validator(
    m: &staking::MsgCreateValidator,
) -> Result<(x_staking::types::Commission, math::Int)> {
    let del = acc_address(&m.delegator_address, "delegator")?;
    let val = val_address(&m.validator_address, "validator")?;
    if val != del {
        return Err(errs::invalid_request("validator address is invalid"));
    }
    if m.pubkey.is_none() {
        return Err(x_staking::errors::empty_validator_pub_key());
    }
    let value = valid_positive_coin(m.value.as_ref())?;
    if m.description.clone().unwrap_or_default() == staking::Description::default() {
        return Err(errs::invalid_request("empty description"));
    }
    let rates = m.commission.clone().unwrap_or_default();
    if rates == staking::CommissionRates::default() {
        return Err(errs::invalid_request("empty commission"));
    }
    let commission = x_staking::msgs::commission_rates(&rates, pbjson_types::Timestamp::default())?;
    // A nil `MinSelfDelegation` panics in `IsPositive`.
    if m.min_self_delegation.is_empty() {
        return Err(x_staking::msgs::nil_deref());
    }
    let min_self = x_staking::msgs::wire_int(&m.min_self_delegation)?;
    if !min_self.is_positive() {
        return Err(errs::invalid_request(
            "minimum self delegation must be a positive integer",
        ));
    }
    if value.amount < min_self {
        return Err(x_staking::errors::self_delegation_below_minimum());
    }
    Ok((commission, min_self))
}

/// The optional fields of a `MsgEditValidator` after its v0.46
/// `ValidateBasic`.
struct EditFields {
    min_self: Option<math::Int>,
    rate: Option<math::LegacyDec>,
}

/// staking `MsgEditValidator.ValidateBasic` (v0.46). `raw` is the
/// message's wire form: a present but empty `min_self_delegation` or
/// `commission_rate` is a non-nil pointer to a nil value, whose method
/// call panics (recovered by `runTx`).
fn validate_edit_validator(m: &staking::MsgEditValidator, raw: &[u8]) -> Result<EditFields> {
    val_address(&m.validator_address, "validator")?;
    if m.description.clone().unwrap_or_default() == staking::Description::default() {
        return Err(errs::invalid_request("empty description"));
    }
    if x_staking::msgs::empty_field(raw, 4) {
        return Err(x_staking::msgs::nil_deref());
    }
    let min_self = if m.min_self_delegation.is_empty() {
        None
    } else {
        Some(x_staking::msgs::wire_int(&m.min_self_delegation)?)
    };
    if min_self.as_ref().is_some_and(|v| !v.is_positive()) {
        return Err(errs::invalid_request(
            "minimum self delegation must be a positive integer",
        ));
    }
    if x_staking::msgs::empty_field(raw, 3) {
        return Err(x_staking::msgs::nil_deref());
    }
    let rate = if m.commission_rate.is_empty() {
        None
    } else {
        Some(x_staking::msgs::wire_dec(&m.commission_rate)?)
    };
    if rate
        .as_ref()
        .is_some_and(|r| *r > math::LegacyDec::one() || r.is_negative())
    {
        return Err(errs::invalid_request(
            "commission rate must be between 0 and 1 (inclusive)",
        ));
    }
    Ok(EditFields { min_self, rate })
}

/// v0.46 `Commission.String()`: YAML of its JSON form (keys sorted, the
/// decimals and the RFC 3339 time quoted).
#[must_use]
pub fn commission_yaml(c: &x_staking::types::Commission) -> String {
    format!(
        "commission_rates:\n  max_change_rate: \"{}\"\n  max_rate: \"{}\"\n  rate: \"{}\"\nupdate_time: \"{}\"\n",
        c.max_change_rate,
        c.max_rate,
        c.rate,
        sdk::time::format_rfc3339_nano(&c.update_time)
    )
}

/// `msg.ValidateBasic()` of a v0.46 message.
pub fn validate_basic(msg: &Any) -> Result<()> {
    if let Some(r) = super::ibc_channel::validate_basic(msg) {
        return r;
    }
    if let Some(r) = super::ibc_transfer::validate_basic(msg) {
        return r;
    }
    if let Some(r) = super::ibc::validate_basic(msg) {
        return r;
    }
    if let Some(r) = super::authz::validate_basic(msg) {
        return r;
    }
    if msg.type_url == bank::MsgSend::type_url() {
        let m: bank::MsgSend = decode(msg)?;
        acc_address(&m.from_address, "from")?;
        acc_address(&m.to_address, "to")?;
        valid_positive_coins(&m.amount)?;
    } else if msg.type_url == bank::MsgMultiSend::type_url() {
        let m: bank::MsgMultiSend = decode(msg)?;
        if m.inputs.is_empty() {
            return Err(x_bank::msgs::errors::no_inputs());
        }
        if m.outputs.is_empty() {
            return Err(x_bank::msgs::errors::no_outputs());
        }
        validate_inputs_outputs(&m.inputs, &m.outputs)?;
    } else if msg.type_url == staking::MsgDelegate::type_url() {
        let m: staking::MsgDelegate = decode(msg)?;
        acc_address(&m.delegator_address, "delegator")?;
        val_address(&m.validator_address, "validator")?;
        valid_positive_coin(m.amount.as_ref())?;
    } else if msg.type_url == staking::MsgCreateValidator::type_url() {
        validate_create_validator(&decode(msg)?)?;
    } else if msg.type_url == staking::MsgEditValidator::type_url() {
        validate_edit_validator(&decode(msg)?, &msg.value)?;
    } else if msg.type_url == distr::MsgSetWithdrawAddress::type_url() {
        let m: distr::MsgSetWithdrawAddress = decode(msg)?;
        acc_address(&m.delegator_address, "delegator")?;
        acc_address(&m.withdraw_address, "withdraw")?;
    } else if msg.type_url == distr::MsgWithdrawDelegatorReward::type_url() {
        let m: distr::MsgWithdrawDelegatorReward = decode(msg)?;
        acc_address(&m.delegator_address, "delegator")?;
        val_address(&m.validator_address, "validator")?;
    } else if msg.type_url == distr::MsgWithdrawValidatorCommission::type_url() {
        let m: distr::MsgWithdrawValidatorCommission = decode(msg)?;
        val_address(&m.validator_address, "validator")?;
    } else if msg.type_url == staking::MsgUndelegate::type_url() {
        let m: staking::MsgUndelegate = decode(msg)?;
        acc_address(&m.delegator_address, "delegator")?;
        val_address(&m.validator_address, "validator")?;
        valid_shares_amount(m.amount.as_ref())?;
    } else if msg.type_url == staking::MsgBeginRedelegate::type_url() {
        let m: staking::MsgBeginRedelegate = decode(msg)?;
        acc_address(&m.delegator_address, "delegator")?;
        val_address(&m.validator_src_address, "source validator")?;
        val_address(&m.validator_dst_address, "destination validator")?;
        valid_shares_amount(m.amount.as_ref())?;
    }
    Ok(())
}

/// `validateBasicTxMsgs`.
pub fn validate_basic_tx_msgs(msgs: &[Any]) -> Result<()> {
    if msgs.is_empty() {
        return Err(errs::invalid_request("must contain at least one message"));
    }
    msgs.iter().try_for_each(validate_basic)
}

/// Executes `msg` through its v0.46 msg server, returning its packed
/// response. Events go to `ctx.events` (a fresh manager per message).
pub fn route(ctx: &mut Context<'_>, msg: &Any) -> Result<Any> {
    if msg.type_url == bank::MsgSend::type_url() {
        return Ok(Any::pack(&send(ctx, &decode(msg)?)?));
    }
    if msg.type_url == bank::MsgMultiSend::type_url() {
        return Ok(Any::pack(&multi_send(ctx, &decode(msg)?)?));
    }
    if msg.type_url == staking::MsgDelegate::type_url() {
        return Ok(Any::pack(&delegate(ctx, &decode(msg)?)?));
    }
    if msg.type_url == staking::MsgCreateValidator::type_url() {
        return Ok(Any::pack(&create_validator(ctx, &decode(msg)?)?));
    }
    if msg.type_url == staking::MsgEditValidator::type_url() {
        return Ok(Any::pack(&edit_validator(ctx, &decode(msg)?, &msg.value)?));
    }
    if msg.type_url == staking::MsgUndelegate::type_url() {
        return Ok(Any::pack(&undelegate(ctx, &decode(msg)?)?));
    }
    if msg.type_url == distr::MsgSetWithdrawAddress::type_url() {
        return Ok(Any::pack(&set_withdraw_address(ctx, &decode(msg)?)?));
    }
    if msg.type_url == distr::MsgWithdrawDelegatorReward::type_url() {
        return Ok(Any::pack(&withdraw_delegator_reward(ctx, &decode(msg)?)?));
    }
    if msg.type_url == distr::MsgWithdrawValidatorCommission::type_url() {
        return Ok(Any::pack(&withdraw_validator_commission(
            ctx,
            &decode(msg)?,
        )?));
    }
    if msg.type_url == staking::MsgBeginRedelegate::type_url() {
        return Ok(Any::pack(&begin_redelegate(ctx, &decode(msg)?)?));
    }
    if let Some(r) = super::ibc::route(ctx, msg) {
        return r;
    }
    if let Some(r) = super::ibc_channel::route(ctx, msg) {
        return r;
    }
    if let Some(r) = super::ibc_transfer::route(ctx, msg) {
        return r;
    }
    if let Some(r) = super::authz::route(ctx, msg) {
        return r;
    }
    Err(super::not_ported(errs::unknown_request(format!(
        "the cosmos-sdk v0.46 handler of {} is not ported",
        msg.type_url
    ))))
}

/// v0.46 bank `IsSendEnabledCoins`: each coin reads the params
/// (`SendEnabledDenom`: the denom's entry, else the default).
pub(super) fn is_send_enabled_coins(ctx: &mut Context<'_>, coins: &Coins) -> Result<()> {
    for c in coins {
        #[allow(deprecated)]
        let p = x_bank::get_params(ctx)?;
        #[allow(deprecated)]
        let enabled = p
            .send_enabled
            .iter()
            .find(|e| e.denom == c.denom)
            .map_or(p.default_send_enabled, |e| e.enabled);
        if !enabled {
            return Err(x_bank::msgs::errors::send_disabled(&c.denom));
        }
    }
    Ok(())
}

fn module_event(ctx: &mut Context<'_>, module: &str) {
    ctx.events.emit(event("message", [("module", module)]));
}

/// bank `msgServer.Send`.
fn send(ctx: &mut Context<'_>, msg: &bank::MsgSend) -> Result<bank::MsgSendResponse> {
    let amount = Coins::from_proto(&msg.amount)?;
    is_send_enabled_coins(ctx, &amount)?;
    let from = acc_address(&msg.from_address, "from")?;
    let to = acc_address(&msg.to_address, "to")?;
    if x_bank::is_blocked(&to) {
        return Err(errs::unauthorized(format!(
            "{} is not allowed to receive funds",
            msg.to_address
        )));
    }
    x_bank::send_coins(ctx, &from, &to, &amount)?;
    module_event(ctx, "bank");
    Ok(bank::MsgSendResponse {})
}

/// bank `msgServer.MultiSend` and v0.46 `InputOutputCoins`.
fn multi_send(
    ctx: &mut Context<'_>,
    msg: &bank::MsgMultiSend,
) -> Result<bank::MsgMultiSendResponse> {
    for i in &msg.inputs {
        is_send_enabled_coins(ctx, &Coins::from_proto(&i.coins)?)?;
    }
    for o in &msg.outputs {
        let a = acc_address(&o.address, "output")?;
        if x_bank::is_blocked(&a) {
            return Err(errs::unauthorized(format!(
                "{} is not allowed to receive transactions",
                o.address
            )));
        }
    }
    validate_inputs_outputs(&msg.inputs, &msg.outputs)?;
    for i in &msg.inputs {
        let addr = acc_address(&i.address, "input")?;
        x_bank::sub_unlocked_coins(ctx, &addr, &Coins::from_proto(&i.coins)?)?;
        ctx.events
            .emit(event("message", [("sender", i.address.clone())]));
    }
    for o in &msg.outputs {
        let addr = acc_address(&o.address, "output")?;
        let coins = Coins::from_proto(&o.coins)?;
        x_bank::add_coins(ctx, &addr, &coins)?;
        ctx.events.emit(event(
            "transfer",
            [
                ("recipient", o.address.clone()),
                ("amount", coins.to_string()),
            ],
        ));
        if !x_auth::has_account(ctx, &addr)? {
            let acc = x_auth::new_account_with_address(ctx, &addr)?;
            x_auth::set_account(ctx, &acc)?;
        }
    }
    module_event(ctx, "bank");
    Ok(bank::MsgMultiSendResponse {})
}

fn staking_message_event(ctx: &mut Context<'_>, sender: &str) {
    ctx.events.emit(event(
        "message",
        [("module", "staking"), ("sender", sender)],
    ));
}

/// The bond denom check of the staking msg servers.
fn check_bond_denom(ctx: &mut Context<'_>, denom: &str) -> Result<()> {
    let bond_denom = x_staking::store::bond_denom(ctx)?;
    if denom != bond_denom {
        return Err(errs::invalid_request(format!(
            "invalid coin denomination: got {denom}, expected {bond_denom}"
        )));
    }
    Ok(())
}

/// staking `msgServer.Delegate` (v0.46).
fn delegate(
    ctx: &mut Context<'_>,
    msg: &staking::MsgDelegate,
) -> Result<staking::MsgDelegateResponse> {
    let val = val_address(&msg.validator_address, "validator")?;
    let v = x_staking::store::get_validator(ctx, &val)?
        .ok_or_else(x_staking::errors::no_validator_found)?;
    let del = acc_address(&msg.delegator_address, "delegator")?;
    let amount = valid_positive_coin(msg.amount.as_ref())?;
    check_bond_denom(ctx, &amount.denom)?;
    let shares = x_staking::delegation::delegate(
        ctx,
        &del,
        &amount.amount,
        x_staking::types::BondStatus::Unbonded,
        &v,
        true,
    )?;
    ctx.events.emit(event(
        "delegate",
        [
            ("validator", msg.validator_address.clone()),
            ("amount", amount.to_string()),
            ("new_shares", shares.to_string()),
        ],
    ));
    staking_message_event(ctx, &msg.delegator_address);
    Ok(staking::MsgDelegateResponse {})
}

/// staking `msgServer.CreateValidator` (v0.46).
#[allow(deprecated)]
fn create_validator(
    ctx: &mut Context<'_>,
    msg: &staking::MsgCreateValidator,
) -> Result<staking::MsgCreateValidatorResponse> {
    use x_staking::store;
    let (mut commission, min_self) = validate_create_validator(msg)?;
    let val = val_address(&msg.validator_address, "validator")?;
    let min_rate = store::min_commission_rate(ctx)?;
    if commission.rate < min_rate {
        // The error message reads the parameter again.
        let min_rate = store::min_commission_rate(ctx)?;
        return Err(x_staking::errors::commission_lt_min_rate().wrap(format!(
            "cannot set validator commission to less than minimum rate of {min_rate}"
        )));
    }
    if store::get_validator(ctx, &val)?.is_some() {
        return Err(x_staking::errors::validator_owner_exists());
    }
    let pk_any = msg
        .pubkey
        .as_ref()
        .ok_or_else(x_staking::errors::empty_validator_pub_key)?;
    let pk = x_staking::addr::PubKey::from_any(pk_any)?;
    if store::get_validator_by_cons_addr(ctx, &pk.address()?)?.is_some() {
        return Err(x_staking::errors::validator_pub_key_exists());
    }
    let value = valid_positive_coin(msg.value.as_ref())?;
    check_bond_denom(ctx, &value.denom)?;
    let description = msg.description.clone().unwrap_or_default();
    x_staking::types::ensure_description_length(&description)?;
    if let Some(vp) = ctx.consensus_params.validator.as_ref()
        && !vp.pub_key_types.iter().any(|t| t == pk.type_str())
    {
        return Err(
            x_staking::errors::validator_pub_key_type_not_supported().wrap(format!(
                "got: {}, expected: [{}]",
                pk.type_str(),
                vp.pub_key_types.join(" ")
            )),
        );
    }
    let mut v = x_staking::types::Validator::new(
        x_staking::addr::val_to_string(&val)?,
        pk.to_any(),
        description,
    );
    commission.update_time = ctx.header.time;
    v.commission = commission;
    let del = acc_address(&msg.delegator_address, "delegator")?;
    v.min_self_delegation = min_self;
    store::set_validator(ctx, &v)?;
    store::set_validator_by_cons_addr(ctx, &v)?;
    store::set_new_validator_by_power_index(ctx, &v)?;
    x_staking::hooks::hooks().after_validator_created(ctx, &val)?;
    x_staking::delegation::delegate(
        ctx,
        &del,
        &value.amount,
        x_staking::types::BondStatus::Unbonded,
        &v,
        true,
    )?;
    ctx.events.emit(event(
        "create_validator",
        [
            ("validator", msg.validator_address.clone()),
            ("amount", value.to_string()),
        ],
    ));
    staking_message_event(ctx, &msg.delegator_address);
    Ok(staking::MsgCreateValidatorResponse {})
}

/// staking `msgServer.EditValidator` (v0.46).
fn edit_validator(
    ctx: &mut Context<'_>,
    msg: &staking::MsgEditValidator,
    raw: &[u8],
) -> Result<staking::MsgEditValidatorResponse> {
    use x_staking::store;
    // The msg server runs after `ValidateBasic`, which already passed.
    let fields = validate_edit_validator(msg, raw)?;
    let val = val_address(&msg.validator_address, "validator")?;
    let mut v =
        store::get_validator(ctx, &val)?.ok_or_else(x_staking::errors::no_validator_found)?;
    let description = msg.description.clone().unwrap_or_default();
    v.description = x_staking::types::update_description(&v.description, &description)?;
    if let Some(rate) = &fields.rate {
        let commission = x_staking::validator::update_validator_commission(ctx, &v, rate)?;
        x_staking::hooks::hooks().before_validator_modified(ctx, &val)?;
        v.commission = commission;
    }
    if let Some(min_self) = fields.min_self {
        if min_self <= v.min_self_delegation {
            return Err(x_staking::errors::min_self_delegation_decreased());
        }
        if min_self > v.tokens {
            return Err(x_staking::errors::self_delegation_below_minimum());
        }
        v.min_self_delegation = min_self;
    }
    store::set_validator(ctx, &v)?;
    ctx.events.emit(event(
        "edit_validator",
        [
            ("commission_rate", commission_yaml(&v.commission)),
            ("min_self_delegation", v.min_self_delegation.to_string()),
        ],
    ));
    staking_message_event(ctx, &msg.validator_address);
    Ok(staking::MsgEditValidatorResponse {})
}

/// The msg servers' `sdk.ValAddressFromBech32` / `sdk.AccAddressFromBech32`
/// (plain Go errors after `ValidateBasic` passed).
fn server_val(s: &str) -> Result<Vec<u8>> {
    x_staking::addr::val_from_string(s)
}

fn server_acc(s: &str) -> Result<Vec<u8>> {
    sdk::address::acc_from_bech32_go(s).map_err(errs::invalid_address)
}

/// Go `time.Format(time.RFC3339)` of a UTC time.
fn rfc3339(t: &pbjson_types::Timestamp) -> String {
    sdk::time::format_rfc3339(t)
}

/// staking `msgServer.Undelegate` (v0.46).
fn undelegate(
    ctx: &mut Context<'_>,
    msg: &staking::MsgUndelegate,
) -> Result<staking::MsgUndelegateResponse> {
    let val = server_val(&msg.validator_address)?;
    let del = server_acc(&msg.delegator_address)?;
    let amount = valid_shares_amount(msg.amount.as_ref())?;
    let shares = x_staking::delegation::validate_unbond_amount(ctx, &del, &val, &amount.amount)?;
    check_bond_denom(ctx, &amount.denom)?;
    let (completion, _) = x_staking::delegation::undelegate(ctx, &del, &val, &shares)?;
    ctx.events.emit(event(
        "unbond",
        [
            ("validator", msg.validator_address.clone()),
            ("amount", amount.to_string()),
            ("completion_time", rfc3339(&completion)),
        ],
    ));
    staking_message_event(ctx, &msg.delegator_address);
    Ok(staking::MsgUndelegateResponse {
        completion_time: Some(completion),
        amount: None,
    })
}

/// staking `msgServer.BeginRedelegate` (v0.46).
fn begin_redelegate(
    ctx: &mut Context<'_>,
    msg: &staking::MsgBeginRedelegate,
) -> Result<staking::MsgBeginRedelegateResponse> {
    let src = server_val(&msg.validator_src_address)?;
    let del = server_acc(&msg.delegator_address)?;
    let amount = valid_shares_amount(msg.amount.as_ref())?;
    let shares = x_staking::delegation::validate_unbond_amount(ctx, &del, &src, &amount.amount)?;
    check_bond_denom(ctx, &amount.denom)?;
    let dst = server_val(&msg.validator_dst_address)?;
    let completion = x_staking::delegation::begin_redelegation(ctx, &del, &src, &dst, &shares)?;
    ctx.events.emit(event(
        "redelegate",
        [
            ("source_validator", msg.validator_src_address.clone()),
            ("destination_validator", msg.validator_dst_address.clone()),
            ("amount", amount.to_string()),
            ("completion_time", rfc3339(&completion)),
        ],
    ));
    staking_message_event(ctx, &msg.delegator_address);
    Ok(staking::MsgBeginRedelegateResponse {
        completion_time: Some(completion),
    })
}

fn distribution_message_event(ctx: &mut Context<'_>, sender: &str) {
    ctx.events.emit(event(
        "message",
        [("module", "distribution"), ("sender", sender)],
    ));
}

/// distribution `msgServer.SetWithdrawAddress` and keeper
/// `SetWithdrawAddr` (v0.46: reads only `WithdrawAddrEnabled`).
fn set_withdraw_address(
    ctx: &mut Context<'_>,
    msg: &distr::MsgSetWithdrawAddress,
) -> Result<distr::MsgSetWithdrawAddressResponse> {
    let del = server_acc(&msg.delegator_address)?;
    let to = server_acc(&msg.withdraw_address)?;
    if x_bank::is_blocked(&to) {
        return Err(errs::unauthorized(format!(
            "{} is not allowed to receive external funds",
            msg.withdraw_address
        )));
    }
    if !x_params::v046::get_bool(ctx, "distribution", "withdrawaddrenabled")? {
        return Err(x_distribution::errors::set_withdraw_addr_disabled());
    }
    ctx.events.emit(event(
        "set_withdraw_address",
        [("withdraw_address", x_auth::bech32(&to)?)],
    ));
    x_distribution::store::set_withdraw_addr(ctx, &del, &to)?;
    distribution_message_event(ctx, &msg.delegator_address);
    Ok(distr::MsgSetWithdrawAddressResponse {})
}

/// distribution `msgServer.WithdrawDelegatorReward` and keeper
/// `WithdrawDelegationRewards` (v0.46).
fn withdraw_delegator_reward(
    ctx: &mut Context<'_>,
    msg: &distr::MsgWithdrawDelegatorReward,
) -> Result<distr::MsgWithdrawDelegatorRewardResponse> {
    let val = server_val(&msg.validator_address)?;
    let del = server_acc(&msg.delegator_address)?;
    let v = x_staking::store::get_validator(ctx, &val)?
        .ok_or_else(x_distribution::errors::no_validator_dist_info)?;
    let d = x_staking::store::get_delegation(ctx, &del, &val)?
        .ok_or_else(x_distribution::errors::empty_delegation_dist_info)?;
    let rewards = x_distribution::keeper::withdraw_delegation_rewards(ctx, &v, &d, true)?;
    // Zero rewards are one coin of the unregistered base denom ("").
    let amount = if rewards.is_empty() {
        vec![celestia_proto::cosmos::base::v1beta1::Coin {
            denom: String::new(),
            amount: "0".to_owned(),
        }]
    } else {
        rewards.to_proto()
    };
    x_distribution::keeper::initialize_delegation(ctx, &val, &del)?;
    distribution_message_event(ctx, &msg.delegator_address);
    Ok(distr::MsgWithdrawDelegatorRewardResponse { amount })
}

/// distribution `msgServer.WithdrawValidatorCommission` (v0.46).
fn withdraw_validator_commission(
    ctx: &mut Context<'_>,
    msg: &distr::MsgWithdrawValidatorCommission,
) -> Result<distr::MsgWithdrawValidatorCommissionResponse> {
    let val = server_val(&msg.validator_address)?;
    let amount = x_distribution::keeper::withdraw_validator_commission(ctx, &val)?;
    distribution_message_event(ctx, &msg.validator_address);
    Ok(distr::MsgWithdrawValidatorCommissionResponse {
        amount: amount.to_proto(),
    })
}
