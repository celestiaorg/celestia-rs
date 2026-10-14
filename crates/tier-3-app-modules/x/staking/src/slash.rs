//! Slashing (`keeper/slash.go`).

use math::{Int, LegacyDec};
use sdk::{Context, Result};

use crate::delegation::unbond;
use crate::store;
use crate::types::{BondStatus, Redelegation, UnbondingDelegation, Validator, math_err};
use crate::validator::{burn_bonded_tokens, burn_not_bonded_tokens, remove_validator_tokens};
use crate::{hooks, types};

/// `Slash`: returns the burned amount.
pub fn slash(
    ctx: &mut Context<'_>,
    cons: &[u8],
    infraction_height: i64,
    power: i64,
    slash_factor: &LegacyDec,
) -> Result<Int> {
    if slash_factor.is_negative() {
        return Err(sdk::error::internal(format!(
            "attempted to slash with a negative slash factor: {slash_factor}"
        )));
    }
    let amount = types::tokens_from_consensus_power(power)?;
    let slash_amount = LegacyDec::from_int(&amount)
        .checked_mul(slash_factor)
        .map_err(math_err)?
        .truncate_int();

    let Some(validator) = store::get_validator_by_cons_addr(ctx, cons)? else {
        // Go logs a warning and ignores the slash.
        return Ok(Int::zero());
    };
    if validator.is_unbonded() {
        return Err(sdk::error::internal(format!(
            "should not be slashing unbonded validator: {}",
            validator.operator_address
        )));
    }
    let op = validator.operator()?;
    let h = hooks::hooks();
    let _ = h.before_validator_modified(ctx, &op);

    let mut remaining = slash_amount.clone();
    let height = ctx.header.height;
    if infraction_height > height {
        return Err(sdk::error::internal(format!(
            "impossible attempt to slash future infraction at height {infraction_height} but we are at height {height}"
        )));
    } else if infraction_height < height {
        for ubd in store::get_ubds_from_validator(ctx, &op)? {
            let slashed = slash_unbonding_delegation(ctx, ubd, infraction_height, slash_factor)?;
            if slashed.is_zero() {
                continue;
            }
            remaining = remaining.checked_sub(&slashed).map_err(math_err)?;
        }
        for red in store::get_reds_from_src_validator(ctx, &op)? {
            let slashed =
                slash_redelegation(ctx, &validator, red, infraction_height, slash_factor)?;
            if slashed.is_zero() {
                continue;
            }
            remaining = remaining.checked_sub(&slashed).map_err(math_err)?;
        }
    }

    let mut to_burn = if remaining < validator.tokens {
        remaining
    } else {
        validator.tokens.clone()
    };
    if to_burn.is_negative() {
        to_burn = Int::zero();
    }
    if to_burn.is_zero() {
        return Ok(Int::zero());
    }

    if validator.tokens.is_positive() {
        let mut fraction = LegacyDec::from_int(&to_burn)
            .checked_quo_round_up(&LegacyDec::from_int(&validator.tokens))
            .map_err(math_err)?;
        if fraction > LegacyDec::one() {
            fraction = LegacyDec::one();
        }
        let _ = h.before_validator_slashed(ctx, &op, &fraction);
    }

    let validator = remove_validator_tokens(ctx, validator, &to_burn)?;
    match validator.status {
        BondStatus::Bonded => burn_bonded_tokens(ctx, &to_burn)?,
        BondStatus::Unbonding | BondStatus::Unbonded => burn_not_bonded_tokens(ctx, &to_burn)?,
        BondStatus::Unspecified => return Err(sdk::error::go_panic("invalid validator status")),
    }
    Ok(to_burn)
}

/// `SlashUnbondingDelegation`.
pub fn slash_unbonding_delegation(
    ctx: &mut Context<'_>,
    mut ubd: UnbondingDelegation,
    infraction_height: i64,
    slash_factor: &LegacyDec,
) -> Result<Int> {
    let now = ctx.header.time;
    let mut total = Int::zero();
    let mut burned = Int::zero();
    for i in 0..ubd.entries.len() {
        let entry = ubd.entries[i].clone();
        if entry.creation_height < infraction_height {
            continue;
        }
        if entry.is_mature(&now) && !entry.on_hold() {
            continue;
        }
        let slash_amount = slash_factor
            .checked_mul_int(&entry.initial_balance)
            .map_err(math_err)?
            .truncate_int();
        total = total.checked_add(&slash_amount).map_err(math_err)?;
        let unbonding_slash = if slash_amount < entry.balance {
            slash_amount
        } else {
            entry.balance.clone()
        };
        if unbonding_slash.is_zero() {
            continue;
        }
        burned = burned.checked_add(&unbonding_slash).map_err(math_err)?;
        ubd.entries[i].balance = entry
            .balance
            .checked_sub(&unbonding_slash)
            .map_err(math_err)?;
        store::set_ubd(ctx, &ubd)?;
    }
    burn_not_bonded_tokens(ctx, &burned)?;
    Ok(total)
}

/// `SlashRedelegation`.
pub fn slash_redelegation(
    ctx: &mut Context<'_>,
    _src: &Validator,
    red: Redelegation,
    infraction_height: i64,
    slash_factor: &LegacyDec,
) -> Result<Int> {
    let now = ctx.header.time;
    let mut total = Int::zero();
    let mut bonded_burned = Int::zero();
    let mut not_bonded_burned = Int::zero();
    let dst = crate::addr::val_from_string(&red.validator_dst_address)?;
    let del = crate::addr::acc_from_string(&red.delegator_address)?;
    for entry in &red.entries {
        if entry.creation_height < infraction_height {
            continue;
        }
        if entry.is_mature(&now) && !entry.on_hold() {
            continue;
        }
        let mut slash_amount = slash_factor
            .checked_mul_int(&entry.initial_balance)
            .map_err(math_err)?
            .truncate_int();
        total = total.checked_add(&slash_amount).map_err(math_err)?;

        if let Some(mut ubd) = store::get_ubd(ctx, &del, &dst)? {
            for i in 0..ubd.entries.len() {
                let e = ubd.entries[i].clone();
                let unbonding_slash = if slash_amount < e.balance {
                    slash_amount.clone()
                } else {
                    e.balance.clone()
                };
                if unbonding_slash.is_zero()
                    || e.creation_height < infraction_height
                    || (e.is_mature(&now) && !e.on_hold())
                {
                    continue;
                }
                slash_amount = slash_amount
                    .checked_sub(&unbonding_slash)
                    .map_err(math_err)?;
                not_bonded_burned = not_bonded_burned
                    .checked_add(&unbonding_slash)
                    .map_err(math_err)?;
                ubd.entries[i].balance =
                    e.balance.checked_sub(&unbonding_slash).map_err(math_err)?;
                store::set_ubd(ctx, &ubd)?;
            }
        }

        let mut shares_to_unbond = slash_factor
            .checked_mul(&entry.shares_dst)
            .map_err(math_err)?;
        if shares_to_unbond.is_zero() || slash_amount.is_zero() {
            continue;
        }
        let Some(d) = store::get_delegation(ctx, &del, &dst)? else {
            continue;
        };
        if shares_to_unbond > d.shares {
            shares_to_unbond = d.shares;
        }
        let tokens_to_burn = unbond(ctx, &del, &dst, &shares_to_unbond)?;
        let dst_v = store::validator(ctx, &dst)?;
        match dst_v.status {
            BondStatus::Bonded => {
                bonded_burned = bonded_burned
                    .checked_add(&tokens_to_burn)
                    .map_err(math_err)?;
            }
            BondStatus::Unbonded | BondStatus::Unbonding => {
                not_bonded_burned = not_bonded_burned
                    .checked_add(&tokens_to_burn)
                    .map_err(math_err)?;
            }
            BondStatus::Unspecified => {
                return Err(sdk::error::go_panic("unknown validator status"));
            }
        }
    }
    burn_bonded_tokens(ctx, &bonded_burned)?;
    burn_not_bonded_tokens(ctx, &not_bonded_burned)?;
    Ok(total)
}
