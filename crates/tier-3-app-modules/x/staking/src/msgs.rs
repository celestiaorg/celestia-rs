//! Msg server (`keeper/msg_server.go`).
//!
//! Messages arrive in their binary (wire) form: `LegacyDec` fields hold
//! `Marshal()` text. Use [`json_msg_to_wire`] before decoding a message from
//! proto3 JSON (e.g. genesis transactions).

use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;
use celestia_proto::cosmos::staking::v1beta1 as pb;
use math::{Int, LegacyDec};
use sdk::coins::{Coin, is_valid_denom};
use sdk::error::sdk as errs;
use sdk::event::event;
use sdk::{Context, Result};

use crate::addr::{PubKey, acc_from_string, val_from_string};
use crate::delegation;
use crate::errors;
use crate::store::{self, get_params};
use crate::types::{self, BondStatus, Commission, Params, Repr, Validator, math_err};
use crate::validator::update_validator_commission;

/// `types.MaxCommissionRate` (60%, celestia fork).
#[must_use]
pub fn max_commission_rate() -> LegacyDec {
    LegacyDec::new_with_prec(60, 2).unwrap_or_default()
}

fn invalid_addr(what: &str, e: &sdk::SdkError) -> sdk::SdkError {
    errs::invalid_address(format!("invalid {what} address: {}", e.log))
}

/// A proto coin as `(coin, valid)` following `Coin.IsValid`.
fn parse_coin(c: Option<&PbCoin>) -> (Coin, bool) {
    let Some(c) = c else {
        return (Coin::with_amount("", Int::zero()), false);
    };
    match Int::parse(&c.amount) {
        Ok(a) => {
            let valid = is_valid_denom(&c.denom) && !a.is_negative();
            (Coin::with_amount(c.denom.clone(), a), valid)
        }
        Err(_) => (Coin::with_amount(c.denom.clone(), Int::zero()), false),
    }
}

/// `CommissionRates.Validate` of the decoded rates. An empty customtype
/// field is a nil `LegacyDec` in Go, which `Validate` dereferences (panic)
/// as soon as it reaches it, after any earlier failing check.
fn commission_rates(
    rates: &pb::CommissionRates,
    update_time: pbjson_types::Timestamp,
) -> Result<Commission> {
    let nil_dec = |s: &str| -> Result<LegacyDec> {
        if s.is_empty() {
            Ok(LegacyDec::zero())
        } else {
            wire_dec(s)
        }
    };
    let commission = Commission {
        rate: nil_dec(&rates.rate)?,
        max_rate: nil_dec(&rates.max_rate)?,
        max_change_rate: nil_dec(&rates.max_change_rate)?,
        update_time,
    };
    let one = LegacyDec::one();
    if rates.max_rate.is_empty() {
        return Err(nil_deref());
    }
    if !(commission.max_rate.is_negative() || commission.max_rate > one) && rates.rate.is_empty() {
        return Err(nil_deref());
    }
    if !(commission.max_rate.is_negative()
        || commission.max_rate > one
        || commission.rate.is_negative()
        || commission.rate > commission.max_rate)
        && rates.max_change_rate.is_empty()
    {
        return Err(nil_deref());
    }
    commission.validate_rates()?;
    Ok(commission)
}

/// Go's recovered nil-pointer panic (a method on a nil `math.Int` /
/// `LegacyDec`): `sdkerrors.ErrPanic`.
fn nil_deref() -> sdk::SdkError {
    sdk::error::go_panic("runtime error: invalid memory address or nil pointer dereference")
}

fn wire_dec(s: &str) -> Result<LegacyDec> {
    types::dec(s, Repr::Wire)
}

fn wire_int(s: &str) -> Result<Int> {
    types::int(s, Repr::Wire)
}

/// `MsgCreateValidator`.
pub fn handle_create_validator(
    ctx: &mut Context<'_>,
    msg: &pb::MsgCreateValidator,
) -> Result<pb::MsgCreateValidatorResponse> {
    let val = val_from_string(&msg.validator_address).map_err(|e| invalid_addr("validator", &e))?;

    // msg.Validate
    let Some(pk_any) = msg.pubkey.as_ref() else {
        return Err(errors::empty_validator_pub_key());
    };
    let (value, value_valid) = parse_coin(msg.value.as_ref());
    if !value_valid || !value.amount.is_positive() {
        return Err(errs::invalid_request("invalid delegation amount"));
    }
    let description = msg.description.clone().unwrap_or_default();
    if description == pb::Description::default() {
        return Err(errs::invalid_request("empty description"));
    }
    let rates = msg.commission.clone().unwrap_or_default();
    if rates == pb::CommissionRates::default() {
        return Err(errs::invalid_request("empty commission"));
    }
    let mut commission = commission_rates(&rates, ctx.header.time)?;
    if msg.min_self_delegation.is_empty() {
        return Err(nil_deref());
    }
    let min_self = wire_int(&msg.min_self_delegation)?;
    if !min_self.is_positive() {
        return Err(errs::invalid_request(
            "minimum self delegation must be a positive integer",
        ));
    }
    if value.amount < min_self {
        return Err(errors::self_delegation_below_minimum());
    }

    let min_rate = get_params(ctx)?.min_commission_rate;
    if commission.rate < min_rate {
        return Err(errors::commission_lt_min_rate().wrap(format!(
            "cannot set validator commission to less than minimum rate of {min_rate}"
        )));
    }
    let max_rate = max_commission_rate();
    if commission.rate > max_rate {
        return Err(errors::commission_gt_max_rate().wrap(format!(
            "cannot set validator commission rate to greater than maximum rate of {max_rate}, proposed {}",
            commission.max_rate
        )));
    }
    if store::get_validator(ctx, &val)?.is_some() {
        return Err(errors::validator_owner_exists());
    }
    let pk = PubKey::from_any(pk_any)?;
    if store::get_validator_by_cons_addr(ctx, &pk.address()?)?.is_some() {
        return Err(errors::validator_pub_key_exists());
    }
    let denom = store::bond_denom(ctx)?;
    if value.denom != denom {
        return Err(errs::invalid_request(format!(
            "invalid coin denomination: got {}, expected {denom}",
            value.denom
        )));
    }
    types::ensure_description_length(&description)?;
    if let Some(vp) = ctx.consensus_params.validator.as_ref()
        && !vp.pub_key_types.iter().any(|t| t == pk.type_str())
    {
        return Err(errors::validator_pub_key_type_not_supported().wrap(format!(
            "got: {}, expected: {:?}",
            pk.type_str(),
            vp.pub_key_types
        )));
    }

    let mut v = Validator::new(msg.validator_address.clone(), pk_any.clone(), description);
    commission.update_time = ctx.header.time;
    v.commission = commission;
    v.min_self_delegation = min_self;
    store::set_validator(ctx, &v)?;
    store::set_validator_by_cons_addr(ctx, &v)?;
    store::set_new_validator_by_power_index(ctx, &v)?;
    crate::hooks::hooks().after_validator_created(ctx, &val)?;
    delegation::delegate(ctx, &val, &value.amount, BondStatus::Unbonded, &v, true)?;
    ctx.events.emit(event(
        "create_validator",
        [
            ("validator", msg.validator_address.clone()),
            ("amount", value.to_string()),
        ],
    ));
    Ok(pb::MsgCreateValidatorResponse {})
}

/// `MsgEditValidator`.
pub fn handle_edit_validator(
    ctx: &mut Context<'_>,
    msg: &pb::MsgEditValidator,
) -> Result<pb::MsgEditValidatorResponse> {
    edit_validator(ctx, msg, (false, false))
}

/// Whether the last occurrence of field `num` in the message `raw` is
/// empty (length 0): a nullable customtype gogoproto leaves as a non-nil
/// pointer to a nil value.
fn empty_field(raw: &[u8], num: u64) -> bool {
    fn varint(b: &mut &[u8]) -> Option<u64> {
        let mut v = 0u64;
        for i in 0..10 {
            let x = *b.get(i)?;
            v |= u64::from(x & 0x7f) << (7 * i);
            if x < 0x80 {
                *b = &b[i + 1..];
                return Some(v);
            }
        }
        None
    }
    let mut b = raw;
    let mut empty = false;
    while !b.is_empty() {
        let Some(tag) = varint(&mut b) else {
            return false;
        };
        let n = match tag & 7 {
            0 => {
                if varint(&mut b).is_none() {
                    return false;
                }
                0
            }
            1 => 8,
            5 => 4,
            2 => match varint(&mut b).and_then(|l| usize::try_from(l).ok()) {
                Some(l) => l,
                None => return false,
            },
            _ => return false,
        };
        if b.len() < n {
            return false;
        }
        if tag >> 3 == num {
            empty = tag & 7 == 2 && n == 0;
        }
        b = &b[n..];
    }
    empty
}

/// `MsgEditValidator` from its wire form `raw` as the tx decoder
/// normalized it (gogoproto's struct): a present but empty
/// `commission_rate` / `min_self_delegation` is a non-nil pointer to a nil
/// `LegacyDec` / `Int`, which the msg server dereferences (panic) once
/// past the address and description checks.
pub fn handle_edit_validator_wire(
    ctx: &mut Context<'_>,
    msg: &pb::MsgEditValidator,
    raw: &[u8],
) -> Result<pb::MsgEditValidatorResponse> {
    edit_validator(ctx, msg, (empty_field(raw, 4), empty_field(raw, 3)))
}

fn edit_validator(
    ctx: &mut Context<'_>,
    msg: &pb::MsgEditValidator,
    (nil_min_self, nil_rate): (bool, bool),
) -> Result<pb::MsgEditValidatorResponse> {
    let val = val_from_string(&msg.validator_address).map_err(|e| invalid_addr("validator", &e))?;
    let description = msg.description.clone().unwrap_or_default();
    if description == pb::Description::default() {
        return Err(errs::invalid_request("empty description"));
    }
    // `!msg.MinSelfDelegation.IsPositive()` / `msg.CommissionRate.GT(..)`
    // on a nil value.
    if nil_min_self || nil_rate {
        return Err(nil_deref());
    }
    let min_self = if msg.min_self_delegation.is_empty() {
        None
    } else {
        Some(wire_int(&msg.min_self_delegation)?)
    };
    if let Some(m) = &min_self
        && !m.is_positive()
    {
        return Err(errs::invalid_request(
            "minimum self delegation must be a positive integer",
        ));
    }
    let rate = if msg.commission_rate.is_empty() {
        None
    } else {
        Some(wire_dec(&msg.commission_rate)?)
    };
    if let Some(r) = &rate {
        if *r > LegacyDec::one() || r.is_negative() {
            return Err(errs::invalid_request(
                "commission rate must be between 0 and 1 (inclusive)",
            ));
        }
        let min = get_params(ctx)?.min_commission_rate;
        if *r < min {
            return Err(errs::invalid_request(format!(
                "commission rate cannot be less than the min commission rate {min}"
            )));
        }
        let max = max_commission_rate();
        if *r > max {
            return Err(errs::invalid_request(format!(
                "commission rate cannot be greater than the max commission rate {max}, proposed: {r}"
            )));
        }
    }

    let mut v = store::validator(ctx, &val)?;
    v.description = types::update_description(&v.description, &description)?;
    if let Some(r) = &rate {
        let c = update_validator_commission(ctx, &v, r)?;
        crate::hooks::hooks().before_validator_modified(ctx, &val)?;
        v.commission = c;
    }
    if let Some(m) = min_self {
        if m <= v.min_self_delegation {
            return Err(errors::min_self_delegation_decreased());
        }
        if m > v.tokens {
            return Err(errors::self_delegation_below_minimum());
        }
        v.min_self_delegation = m;
    }
    store::set_validator(ctx, &v)?;
    ctx.events.emit(event(
        "edit_validator",
        [
            ("commission_rate", v.commission.compact_text()),
            ("min_self_delegation", v.min_self_delegation.to_string()),
        ],
    ));
    Ok(pb::MsgEditValidatorResponse {})
}

fn check_amount(c: Option<&PbCoin>, what: &str) -> Result<Coin> {
    let (coin, valid) = parse_coin(c);
    if !valid || !coin.amount.is_positive() {
        return Err(errs::invalid_request(what));
    }
    Ok(coin)
}

fn check_denom(ctx: &mut Context<'_>, coin: &Coin) -> Result<()> {
    let denom = store::bond_denom(ctx)?;
    if coin.denom != denom {
        return Err(errs::invalid_request(format!(
            "invalid coin denomination: got {}, expected {denom}",
            coin.denom
        )));
    }
    Ok(())
}

/// `MsgDelegate`.
pub fn handle_delegate(
    ctx: &mut Context<'_>,
    msg: &pb::MsgDelegate,
) -> Result<pb::MsgDelegateResponse> {
    let val = val_from_string(&msg.validator_address).map_err(|e| invalid_addr("validator", &e))?;
    let del = acc_from_string(&msg.delegator_address).map_err(|e| invalid_addr("delegator", &e))?;
    let amount = check_amount(msg.amount.as_ref(), "invalid delegation amount")?;
    let v = store::validator(ctx, &val)?;
    check_denom(ctx, &amount)?;
    let shares = delegation::delegate(ctx, &del, &amount.amount, BondStatus::Unbonded, &v, true)?;
    ctx.events.emit(event(
        "delegate",
        [
            ("validator", msg.validator_address.clone()),
            ("delegator", msg.delegator_address.clone()),
            ("amount", amount.to_string()),
            ("new_shares", shares.to_string()),
        ],
    ));
    Ok(pb::MsgDelegateResponse {})
}

/// `MsgBeginRedelegate`.
pub fn handle_begin_redelegate(
    ctx: &mut Context<'_>,
    msg: &pb::MsgBeginRedelegate,
) -> Result<pb::MsgBeginRedelegateResponse> {
    let src = val_from_string(&msg.validator_src_address)
        .map_err(|e| invalid_addr("source validator", &e))?;
    let dst = val_from_string(&msg.validator_dst_address)
        .map_err(|e| invalid_addr("destination validator", &e))?;
    let del = acc_from_string(&msg.delegator_address).map_err(|e| invalid_addr("delegator", &e))?;
    let amount = check_amount(msg.amount.as_ref(), "invalid shares amount")?;
    let shares = delegation::validate_unbond_amount(ctx, &del, &src, &amount.amount)?;
    check_denom(ctx, &amount)?;
    let completion = delegation::begin_redelegation(ctx, &del, &src, &dst, &shares)?;
    ctx.events.emit(event(
        "redelegate",
        [
            ("source_validator", msg.validator_src_address.clone()),
            ("destination_validator", msg.validator_dst_address.clone()),
            ("amount", amount.to_string()),
            ("completion_time", sdk::time::format_rfc3339(&completion)),
        ],
    ));
    Ok(pb::MsgBeginRedelegateResponse {
        completion_time: Some(completion),
    })
}

/// `MsgUndelegate`.
pub fn handle_undelegate(
    ctx: &mut Context<'_>,
    msg: &pb::MsgUndelegate,
) -> Result<pb::MsgUndelegateResponse> {
    let val = val_from_string(&msg.validator_address).map_err(|e| invalid_addr("validator", &e))?;
    let del = acc_from_string(&msg.delegator_address).map_err(|e| invalid_addr("delegator", &e))?;
    let amount = check_amount(msg.amount.as_ref(), "invalid shares amount")?;
    let shares = delegation::validate_unbond_amount(ctx, &del, &val, &amount.amount)?;
    check_denom(ctx, &amount)?;
    let (completion, undelegated) = delegation::undelegate(ctx, &del, &val, &shares)?;
    let coin = Coin::with_amount(amount.denom.clone(), undelegated);
    ctx.events.emit(event(
        "unbond",
        [
            ("validator", msg.validator_address.clone()),
            ("delegator", msg.delegator_address.clone()),
            ("amount", coin.to_string()),
            ("completion_time", sdk::time::format_rfc3339(&completion)),
        ],
    ));
    Ok(pb::MsgUndelegateResponse {
        completion_time: Some(completion),
        amount: Some(coin.to_proto()),
    })
}

/// `MsgCancelUnbondingDelegation`.
pub fn handle_cancel_unbonding_delegation(
    ctx: &mut Context<'_>,
    msg: &pb::MsgCancelUnbondingDelegation,
) -> Result<pb::MsgCancelUnbondingDelegationResponse> {
    let val = val_from_string(&msg.validator_address).map_err(|e| invalid_addr("validator", &e))?;
    let del = acc_from_string(&msg.delegator_address).map_err(|e| invalid_addr("delegator", &e))?;
    let amount = check_amount(msg.amount.as_ref(), "invalid amount")?;
    if msg.creation_height <= 0 {
        return Err(errs::invalid_request("invalid height"));
    }
    check_denom(ctx, &amount)?;
    let v = store::validator(ctx, &val)?;
    if v.invalid_ex_rate() {
        return Err(errors::delegator_share_ex_rate_invalid());
    }
    if v.jailed {
        return Err(errors::validator_jailed());
    }
    let Some(mut ubd) = store::get_ubd(ctx, &del, &val)? else {
        return Err(sdk::error::internal(format!(
            "unbonding delegation with delegator {} not found for validator {}",
            msg.delegator_address, msg.validator_address
        )));
    };
    let Some(idx) = ubd
        .entries
        .iter()
        .position(|e| e.creation_height == msg.creation_height)
    else {
        return Err(errs::not_found(format!(
            "unbonding delegation entry is not found at block height {}",
            msg.creation_height
        )));
    };
    let mut entry = ubd.entries[idx].clone();
    if entry.balance < amount.amount {
        return Err(errs::invalid_request(
            "amount is greater than the unbonding delegation entry balance",
        ));
    }
    if sdk::time::before(&entry.completion_time, &ctx.header.time) {
        return Err(errs::invalid_request(
            "unbonding delegation is already processed",
        ));
    }
    delegation::delegate(ctx, &del, &amount.amount, BondStatus::Unbonding, &v, false)?;
    let rest = entry
        .balance
        .checked_sub(&amount.amount)
        .map_err(math_err)?;
    if rest.is_zero() {
        ubd.entries.remove(idx);
    } else {
        entry.balance = rest;
        entry.initial_balance = entry
            .initial_balance
            .checked_sub(&amount.amount)
            .map_err(math_err)?;
        ubd.entries[idx] = entry;
    }
    if ubd.entries.is_empty() {
        store::remove_ubd(ctx, &ubd)?;
    } else {
        store::set_ubd(ctx, &ubd)?;
    }
    ctx.events.emit(event(
        "cancel_unbonding_delegation",
        [
            ("amount", amount.to_string()),
            ("validator", msg.validator_address.clone()),
            ("delegator", msg.delegator_address.clone()),
            ("creation_height", msg.creation_height.to_string()),
        ],
    ));
    Ok(pb::MsgCancelUnbondingDelegationResponse {})
}

/// The gov module account address (`authtypes.NewModuleAddress("gov")`).
pub fn gov_authority() -> Result<String> {
    x_auth::bech32(&x_auth::module_address("gov"))
}

/// `MsgUpdateParams`.
pub fn handle_update_params(
    ctx: &mut Context<'_>,
    msg: &pb::MsgUpdateParams,
) -> Result<pb::MsgUpdateParamsResponse> {
    let authority = gov_authority()?;
    if authority != msg.authority {
        return Err(errors::gov_invalid_signer().wrap(format!(
            "invalid authority; expected {authority}, got {}",
            msg.authority
        )));
    }
    let p = Params::from_proto(&msg.params.clone().unwrap_or_default(), Repr::Wire)?;
    p.validate()?;
    store::set_params(ctx, &p)?;
    Ok(pb::MsgUpdateParamsResponse {})
}

fn json_dec_to_wire(v: &mut serde_json::Value, path: &[&str]) -> Result<()> {
    let mut cur = v;
    for p in &path[..path.len() - 1] {
        match cur.get_mut(*p) {
            Some(n) => cur = n,
            None => return Ok(()),
        }
    }
    let last = path[path.len() - 1];
    if let Some(s) = cur.get(last).and_then(serde_json::Value::as_str)
        && !s.is_empty()
    {
        let b = math::legacy_dec_json_to_proto_bytes(s)
            .map_err(|e| errs::json_unmarshal(e.to_string()))?;
        cur[last] = serde_json::Value::String(String::from_utf8_lossy(&b).into_owned());
    }
    Ok(())
}

/// Rewrites the `LegacyDec` fields of a proto3-JSON staking message
/// (identified by its `@type`) into their wire text, so that decoding the
/// JSON with the generated serde types yields the binary-equivalent proto.
pub fn json_msg_to_wire(v: &mut serde_json::Value) -> Result<()> {
    let ty = v
        .get("@type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    match ty.as_str() {
        "/cosmos.staking.v1beta1.MsgCreateValidator" => {
            for f in ["rate", "max_rate", "max_change_rate"] {
                json_dec_to_wire(v, &["commission", f])?;
            }
        }
        "/cosmos.staking.v1beta1.MsgEditValidator" => json_dec_to_wire(v, &["commission_rate"])?,
        "/cosmos.staking.v1beta1.MsgUpdateParams" => {
            json_dec_to_wire(v, &["params", "min_commission_rate"])?;
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::empty_field;

    #[test]
    fn empty_field_last_occurrence() {
        // description (1) = {}, commission_rate (3) = "".
        assert!(empty_field(&[0x0a, 0x00, 0x1a, 0x00], 3));
        assert!(!empty_field(&[0x0a, 0x00, 0x1a, 0x00], 4));
        assert!(!empty_field(&[0x1a, 0x00, 0x1a, 0x01, b'1'], 3));
        assert!(empty_field(&[0x1a, 0x01, b'1', 0x1a, 0x00], 3));
        // Truncated input.
        assert!(!empty_field(&[0x1a, 0x00, 0x22, 0x05], 3));
    }
}
