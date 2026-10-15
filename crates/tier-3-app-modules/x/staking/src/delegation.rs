//! Delegation flows (`keeper/delegation.go`).

use math::{Int, LegacyDec};
use pbjson_types::Timestamp;
use sdk::coins::{Coin, Coins};
use sdk::error::sdk as errs;
use sdk::{Context, Result};

use crate::store::{self, get_params};
use crate::types::{
    BondStatus, Delegation, RedEntry, Redelegation, UbdEntry, UnbondingDelegation, Validator,
    math_err,
};
use crate::validator::{
    add_validator_tokens_and_shares, bonded_tokens_to_not_bonded, jail_validator,
    not_bonded_tokens_to_bonded, remove_validator, remove_validator_tokens_and_shares,
};
use crate::{BONDED_POOL, NOT_BONDED_POOL, errors, hooks};

/// `Delegate`: returns the newly issued shares.
pub fn delegate(
    ctx: &mut Context<'_>,
    del: &[u8],
    bond_amt: &Int,
    token_src: BondStatus,
    v: &Validator,
    subtract_account: bool,
) -> Result<LegacyDec> {
    if v.invalid_ex_rate() {
        return Err(errors::delegator_share_ex_rate_invalid());
    }
    let val = v.operator()?;
    let h = hooks::hooks();
    let mut delegation = if let Some(d) = store::get_delegation(ctx, del, &val)? {
        h.before_delegation_shares_modified(ctx, del, &val)?;
        d
    } else {
        let d = Delegation {
            delegator_address: crate::addr::acc_to_string(del)?,
            validator_address: v.operator_address.clone(),
            shares: LegacyDec::zero(),
        };
        h.before_delegation_created(ctx, del, &val)?;
        d
    };

    if subtract_account {
        if token_src == BondStatus::Bonded {
            return Err(sdk::error::go_panic(
                "delegation token source cannot be bonded",
            ));
        }
        let send_name = match v.status {
            BondStatus::Bonded => BONDED_POOL,
            BondStatus::Unbonding | BondStatus::Unbonded => NOT_BONDED_POOL,
            BondStatus::Unspecified => {
                return Err(sdk::error::go_panic("invalid validator status"));
            }
        };
        let denom = store::bond_denom(ctx)?;
        let coins = Coins::sanitized(vec![Coin::with_amount(denom, bond_amt.clone())])?;
        x_bank::delegate_coins_from_account_to_module(ctx, del, send_name, &coins)?;
    } else {
        let src_bonded = token_src == BondStatus::Bonded;
        match (src_bonded, v.is_bonded()) {
            (true, true) | (false, false) => {}
            (false, true) => not_bonded_tokens_to_bonded(ctx, bond_amt)?,
            (true, false) => bonded_tokens_to_not_bonded(ctx, bond_amt)?,
        }
    }

    let (_, new_shares) = add_validator_tokens_and_shares(ctx, v.clone(), bond_amt)?;
    delegation.shares = delegation
        .shares
        .checked_add(&new_shares)
        .map_err(math_err)?;
    store::set_delegation(ctx, &delegation)?;
    h.after_delegation_modified(ctx, del, &val)?;
    Ok(new_shares)
}

/// `Unbond`: returns the removed tokens.
pub fn unbond(ctx: &mut Context<'_>, del: &[u8], val: &[u8], shares: &LegacyDec) -> Result<Int> {
    let Some(mut delegation) = store::get_delegation(ctx, del, val)? else {
        return Err(errors::no_delegator_for_address());
    };
    let h = hooks::hooks();
    h.before_delegation_shares_modified(ctx, del, val)?;
    if delegation.shares < *shares {
        return Err(errors::not_enough_delegation_shares().wrap(delegation.shares.to_string()));
    }
    let mut v = store::validator(ctx, val)?;
    delegation.shares = delegation.shares.checked_sub(shares).map_err(math_err)?;
    let delegator = crate::addr::acc_from_string(&delegation.delegator_address)?;
    let valbz = v.operator()?;
    let is_operator = delegator == valbz;
    if is_operator
        && !v.jailed
        && v.tokens_from_shares(&delegation.shares)?.truncate_int() < v.min_self_delegation
    {
        jail_validator(ctx, v.clone())?;
        v = store::must_get_validator(ctx, &valbz)?;
    }
    if delegation.shares.is_zero() {
        store::remove_delegation(ctx, &delegation)?;
    } else {
        store::set_delegation(ctx, &delegation)?;
        let va = crate::addr::val_from_string(&delegation.validator_address)?;
        h.after_delegation_modified(ctx, &delegator, &va)?;
    }
    let (v, amount) = remove_validator_tokens_and_shares(ctx, v, shares)?;
    if v.delegator_shares.is_zero() && v.is_unbonded() {
        remove_validator(ctx, &valbz)?;
    }
    Ok(amount)
}

/// `HasMaxUnbondingDelegationEntries`.
fn has_max_ubd_entries(ctx: &mut Context<'_>, del: &[u8], val: &[u8]) -> Result<bool> {
    let n = store::get_ubd(ctx, del, val)?.map_or(0, |u| u.entries.len());
    let max = get_params(ctx)?.max_entries as usize;
    Ok(n >= max)
}

/// `SetUnbondingDelegationEntry`.
pub fn set_ubd_entry(
    ctx: &mut Context<'_>,
    del: &[u8],
    val: &[u8],
    creation_height: i64,
    min_time: Timestamp,
    balance: &Int,
) -> Result<UnbondingDelegation> {
    let id = store::increment_unbonding_id(ctx)?;
    let (ubd, is_new) = if let Some(mut u) = store::get_ubd(ctx, del, val)? {
        let n = u.add_entry(creation_height, min_time, balance, id)?;
        (u, n)
    } else {
        let u = UnbondingDelegation {
            delegator_address: crate::addr::acc_to_string(del)?,
            validator_address: crate::addr::val_to_string(val)?,
            entries: vec![UbdEntry {
                creation_height,
                completion_time: min_time,
                initial_balance: balance.clone(),
                balance: balance.clone(),
                unbonding_id: id,
                unbonding_on_hold_ref_count: 0,
            }],
        };
        (u, true)
    };
    store::set_ubd(ctx, &ubd)?;
    if is_new {
        store::set_ubd_by_unbonding_id(ctx, &ubd, id)?;
        // Go logs and ignores hook errors here.
        let _ = hooks::hooks().after_unbonding_initiated(ctx, id);
    }
    Ok(ubd)
}

/// `Undelegate`: returns the completion time and the undelegated amount.
pub fn undelegate(
    ctx: &mut Context<'_>,
    del: &[u8],
    val: &[u8],
    shares: &LegacyDec,
) -> Result<(Timestamp, Int)> {
    let v = store::validator(ctx, val)?;
    if has_max_ubd_entries(ctx, del, val)? {
        return Err(errors::max_unbonding_delegation_entries());
    }
    let amount = unbond(ctx, del, val, shares)?;
    if v.is_bonded() {
        bonded_tokens_to_not_bonded(ctx, &amount)?;
    }
    let unbonding_time = get_params(ctx)?.unbonding_time;
    let completion = sdk::time::add_duration(&ctx.header.time, &unbonding_time);
    let height = ctx.header.height;
    let ubd = set_ubd_entry(ctx, del, val, height, completion, &amount)?;
    store::insert_ubd_queue(ctx, &ubd, &completion)?;
    Ok((completion, amount))
}

/// `CompleteUnbonding`: returns the coins paid out.
pub fn complete_unbonding(ctx: &mut Context<'_>, del: &[u8], val: &[u8]) -> Result<Coins> {
    let mut ubd = store::get_ubd(ctx, del, val)?.ok_or_else(errors::no_unbonding_delegation)?;
    let denom = store::bond_denom(ctx)?;
    let mut balances = Coins::new();
    let now = ctx.header.time;
    let delegator = crate::addr::acc_from_string(&ubd.delegator_address)?;
    let mut i = 0;
    while i < ubd.entries.len() {
        let entry = ubd.entries[i].clone();
        if entry.is_mature(&now) && !entry.on_hold() {
            ubd.entries.remove(i);
            store::delete_unbonding_index(ctx, entry.unbonding_id)?;
            if !entry.balance.is_zero() {
                let amt = Coin::with_amount(denom.clone(), entry.balance.clone());
                x_bank::undelegate_coins_from_module_to_account(
                    ctx,
                    NOT_BONDED_POOL,
                    &delegator,
                    &Coins::sanitized(vec![amt.clone()])?,
                )?;
                balances = balances.add(&Coins(vec![amt]))?;
            }
        } else {
            i += 1;
        }
    }
    if ubd.entries.is_empty() {
        store::remove_ubd(ctx, &ubd)?;
    } else {
        store::set_ubd(ctx, &ubd)?;
    }
    Ok(balances)
}

/// `getBeginInfo`: `(completion time, height, complete now)`.
fn get_begin_info(ctx: &mut Context<'_>, src: &[u8]) -> Result<(Timestamp, i64, bool)> {
    let v = store::get_validator(ctx, src)?;
    let unbonding_time = get_params(ctx)?.unbonding_time;
    match v {
        None => Ok((
            sdk::time::add_duration(&ctx.header.time, &unbonding_time),
            ctx.header.height,
            false,
        )),
        Some(v) => match v.status {
            BondStatus::Bonded => Ok((
                sdk::time::add_duration(&ctx.header.time, &unbonding_time),
                ctx.header.height,
                false,
            )),
            // Go returns its zero `time.Time` (0001-01-01), which the
            // msg response and the `redelegate` event carry.
            BondStatus::Unbonded => Ok((sdk::time::go_zero(), 0, true)),
            BondStatus::Unbonding => Ok((v.unbonding_time, v.unbonding_height, false)),
            BondStatus::Unspecified => Err(sdk::error::go_panic("unknown validator status")),
        },
    }
}

/// `SetRedelegationEntry`.
#[allow(clippy::too_many_arguments)]
fn set_red_entry(
    ctx: &mut Context<'_>,
    del: &[u8],
    src: &[u8],
    dst: &[u8],
    creation_height: i64,
    min_time: Timestamp,
    balance: &Int,
    shares_dst: &LegacyDec,
) -> Result<Redelegation> {
    let id = store::increment_unbonding_id(ctx)?;
    let entry = RedEntry {
        creation_height,
        completion_time: min_time,
        initial_balance: balance.clone(),
        shares_dst: shares_dst.clone(),
        unbonding_id: id,
        unbonding_on_hold_ref_count: 0,
    };
    let red = if let Some(mut r) = store::get_red(ctx, del, src, dst)? {
        r.entries.push(entry);
        r
    } else {
        Redelegation {
            delegator_address: crate::addr::acc_to_string(del)?,
            validator_src_address: crate::addr::val_to_string(src)?,
            validator_dst_address: crate::addr::val_to_string(dst)?,
            entries: vec![entry],
        }
    };
    store::set_red(ctx, &red)?;
    store::set_red_by_unbonding_id(ctx, &red, id)?;
    let _ = hooks::hooks().after_unbonding_initiated(ctx, id);
    Ok(red)
}

/// `BeginRedelegation`: returns the completion time.
pub fn begin_redelegation(
    ctx: &mut Context<'_>,
    del: &[u8],
    src: &[u8],
    dst: &[u8],
    shares: &LegacyDec,
) -> Result<Timestamp> {
    if src == dst {
        return Err(errors::self_redelegation());
    }
    let dst_v = store::get_validator(ctx, dst)?.ok_or_else(errors::bad_redelegation_dst)?;
    let src_v = store::get_validator(ctx, src)?.ok_or_else(errors::bad_redelegation_src)?;
    if store::has_receiving_redelegation(ctx, del, src)? {
        return Err(errors::transitive_redelegation());
    }
    // HasMaxRedelegationEntries
    if let Some(r) = store::get_red(ctx, del, src, dst)? {
        let max = get_params(ctx)?.max_entries as usize;
        if r.entries.len() >= max {
            return Err(errors::max_redelegation_entries());
        }
    }
    let amount = unbond(ctx, del, src, shares)?;
    if amount.is_zero() {
        return Err(errors::tiny_redelegation_amount());
    }
    let created = delegate(ctx, del, &amount, src_v.status, &dst_v, false)?;
    let (completion, height, now) = get_begin_info(ctx, src)?;
    if now {
        return Ok(completion);
    }
    let red = set_red_entry(ctx, del, src, dst, height, completion, &amount, &created)?;
    store::insert_red_queue(ctx, &red, &completion)?;
    Ok(completion)
}

/// `CompleteRedelegation`: returns the matured balances.
pub fn complete_redelegation(
    ctx: &mut Context<'_>,
    del: &[u8],
    src: &[u8],
    dst: &[u8],
) -> Result<Coins> {
    let mut red = store::get_red(ctx, del, src, dst)?.ok_or_else(errors::no_redelegation)?;
    let denom = store::bond_denom(ctx)?;
    let mut balances = Coins::new();
    let now = ctx.header.time;
    let mut i = 0;
    while i < red.entries.len() {
        let entry = red.entries[i].clone();
        if entry.is_mature(&now) && !entry.on_hold() {
            red.entries.remove(i);
            store::delete_unbonding_index(ctx, entry.unbonding_id)?;
            if !entry.initial_balance.is_zero() {
                balances = balances.add(&Coins(vec![Coin::with_amount(
                    denom.clone(),
                    entry.initial_balance.clone(),
                )]))?;
            }
        } else {
            i += 1;
        }
    }
    if red.entries.is_empty() {
        store::remove_red(ctx, &red)?;
    } else {
        store::set_red(ctx, &red)?;
    }
    Ok(balances)
}

/// `ValidateUnbondAmount`: the shares to unbond for `amt` tokens.
pub fn validate_unbond_amount(
    ctx: &mut Context<'_>,
    del: &[u8],
    val: &[u8],
    amt: &Int,
) -> Result<LegacyDec> {
    let v = store::validator(ctx, val)?;
    let d = store::get_delegation(ctx, del, val)?.ok_or_else(errors::no_delegation)?;
    let mut shares = v.shares_from_tokens(amt)?;
    let truncated = v.shares_from_tokens_truncated(amt)?;
    if truncated > d.shares {
        return Err(errs::invalid_request("invalid shares amount"));
    }
    if shares > d.shares {
        shares = d.shares;
    }
    Ok(shares)
}
