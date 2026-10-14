//! Validator state changes (`keeper/validator.go`, `keeper/val_state_change.go`,
//! `keeper/pool.go`).

use std::collections::BTreeMap;

use celestia_proto::celestia_core_proto::abci::ValidatorUpdate;
use math::{Int, LegacyDec};
use sdk::coins::{Coin, Coins};
use sdk::{Context, Result};

use crate::store::{self, get_params};
use crate::types::{BondStatus, Validator, math_err};
use crate::{BONDED_POOL, NOT_BONDED_POOL, STORE_KEY, errors, hooks, keys};

fn bond_coins(ctx: &mut Context<'_>, amt: &Int) -> Result<Coins> {
    let denom = store::bond_denom(ctx)?;
    if amt.is_negative() {
        return Err(sdk::error::go_panic(format!("negative coin amount: {amt}")));
    }
    Coins::sanitized(vec![Coin::with_amount(denom, amt.clone())])
}

/// `bondedTokensToNotBonded`.
pub fn bonded_tokens_to_not_bonded(ctx: &mut Context<'_>, tokens: &Int) -> Result<()> {
    let coins = bond_coins(ctx, tokens)?;
    x_bank::send_from_module_to_module(ctx, BONDED_POOL, NOT_BONDED_POOL, &coins)
}

/// `notBondedTokensToBonded`.
pub fn not_bonded_tokens_to_bonded(ctx: &mut Context<'_>, tokens: &Int) -> Result<()> {
    let coins = bond_coins(ctx, tokens)?;
    x_bank::send_from_module_to_module(ctx, NOT_BONDED_POOL, BONDED_POOL, &coins)
}

/// `burnBondedTokens`.
pub fn burn_bonded_tokens(ctx: &mut Context<'_>, amt: &Int) -> Result<()> {
    if !amt.is_positive() {
        return Ok(());
    }
    let coins = bond_coins(ctx, amt)?;
    x_bank::burn_coins(ctx, BONDED_POOL, &coins)
}

/// `burnNotBondedTokens`.
pub fn burn_not_bonded_tokens(ctx: &mut Context<'_>, amt: &Int) -> Result<()> {
    if !amt.is_positive() {
        return Ok(());
    }
    let coins = bond_coins(ctx, amt)?;
    x_bank::burn_coins(ctx, NOT_BONDED_POOL, &coins)
}

/// `TotalBondedTokens`.
pub fn total_bonded_tokens(ctx: &mut Context<'_>) -> Result<Int> {
    let pool = x_auth::get_module_account(ctx, BONDED_POOL)?
        .ok_or_else(|| sdk::error::go_panic("bonded pool not set"))?;
    let addr = x_auth::Account::Module(pool).address()?;
    let denom = store::bond_denom(ctx)?;
    Ok(x_bank::get_balance(ctx, &addr, &denom)?.amount)
}

/// `AddValidatorTokensAndShares`.
pub fn add_validator_tokens_and_shares(
    ctx: &mut Context<'_>,
    mut v: Validator,
    tokens: &Int,
) -> Result<(Validator, LegacyDec)> {
    store::delete_validator_by_power_index(ctx, &v)?;
    let shares = v.add_tokens_from_del(tokens)?;
    store::set_validator(ctx, &v)?;
    store::set_validator_by_power_index(ctx, &v)?;
    Ok((v, shares))
}

/// `RemoveValidatorTokensAndShares`.
pub fn remove_validator_tokens_and_shares(
    ctx: &mut Context<'_>,
    mut v: Validator,
    shares: &LegacyDec,
) -> Result<(Validator, Int)> {
    store::delete_validator_by_power_index(ctx, &v)?;
    let removed = v.remove_del_shares(shares)?;
    store::set_validator(ctx, &v)?;
    store::set_validator_by_power_index(ctx, &v)?;
    Ok((v, removed))
}

/// `RemoveValidatorTokens`.
pub fn remove_validator_tokens(
    ctx: &mut Context<'_>,
    mut v: Validator,
    tokens: &Int,
) -> Result<Validator> {
    store::delete_validator_by_power_index(ctx, &v)?;
    v.remove_tokens(tokens)?;
    store::set_validator(ctx, &v)?;
    store::set_validator_by_power_index(ctx, &v)?;
    Ok(v)
}

/// `UpdateValidatorCommission`.
pub fn update_validator_commission(
    ctx: &mut Context<'_>,
    v: &Validator,
    new_rate: &LegacyDec,
) -> Result<crate::types::Commission> {
    let mut c = v.commission.clone();
    let block_time = ctx.header.time;
    c.validate_new_rate(new_rate, &block_time)?;
    let min = get_params(ctx)?.min_commission_rate;
    if *new_rate < min {
        return Err(sdk::error::internal(format!(
            "cannot set validator commission to less than minimum rate of {min}"
        )));
    }
    c.rate = new_rate.clone();
    c.update_time = block_time;
    Ok(c)
}

/// `RemoveValidator`.
pub fn remove_validator(ctx: &mut Context<'_>, op: &[u8]) -> Result<()> {
    let Some(v) = store::get_validator(ctx, op)? else {
        return Ok(());
    };
    if !v.is_unbonded() {
        return Err(errors::bad_remove_validator()
            .wrap("cannot call RemoveValidator on bonded or unbonding validators"));
    }
    if v.tokens.is_positive() {
        return Err(errors::bad_remove_validator()
            .wrap("attempting to remove a validator which still contains tokens"));
    }
    let cons = v.cons_addr()?;
    ctx.delete(STORE_KEY, &keys::validator(op))?;
    ctx.delete(STORE_KEY, &keys::validator_by_cons_addr(&cons))?;
    ctx.delete(
        STORE_KEY,
        &keys::power_index(v.potential_consensus_power()?, op),
    )?;
    let op = v.operator()?;
    // Go logs and ignores hook errors here.
    let _ = hooks::hooks().after_validator_removed(ctx, &cons, &op);
    Ok(())
}

/// `jailValidator`.
pub fn jail_validator(ctx: &mut Context<'_>, mut v: Validator) -> Result<()> {
    if v.jailed {
        return Err(errors::validator_jailed().wrap(format!(
            "cannot jail already jailed validator, validator: {}",
            v.operator_address
        )));
    }
    v.jailed = true;
    store::set_validator(ctx, &v)?;
    store::delete_validator_by_power_index(ctx, &v)
}

/// `unjailValidator`.
pub fn unjail_validator(ctx: &mut Context<'_>, mut v: Validator) -> Result<()> {
    if !v.jailed {
        return Err(sdk::error::internal(format!(
            "cannot unjail already unjailed validator, validator: {}",
            v.operator_address
        )));
    }
    v.jailed = false;
    store::set_validator(ctx, &v)?;
    store::set_validator_by_power_index(ctx, &v)
}

/// `Jail` (by consensus address).
pub fn jail(ctx: &mut Context<'_>, cons: &[u8]) -> Result<()> {
    let v = store::must_get_validator_by_cons_addr(ctx, cons)?;
    jail_validator(ctx, v)
}

/// `Unjail` (by consensus address).
pub fn unjail(ctx: &mut Context<'_>, cons: &[u8]) -> Result<()> {
    let v = store::must_get_validator_by_cons_addr(ctx, cons)?;
    unjail_validator(ctx, v)
}

/// `bondValidator`.
fn bond_validator(ctx: &mut Context<'_>, mut v: Validator) -> Result<Validator> {
    store::delete_validator_by_power_index(ctx, &v)?;
    v.status = BondStatus::Bonded;
    store::set_validator(ctx, &v)?;
    store::set_validator_by_power_index(ctx, &v)?;
    store::delete_validator_queue(ctx, &v)?;
    let cons = v.cons_addr()?;
    let op = v.operator()?;
    hooks::hooks().after_validator_bonded(ctx, &cons, &op)?;
    Ok(v)
}

/// `BeginUnbondingValidator`.
pub fn begin_unbonding_validator(ctx: &mut Context<'_>, mut v: Validator) -> Result<Validator> {
    let params = get_params(ctx)?;
    store::delete_validator_by_power_index(ctx, &v)?;
    if v.status != BondStatus::Bonded {
        return Err(sdk::error::go_panic(
            "should not already be unbonded or unbonding",
        ));
    }
    let id = store::increment_unbonding_id(ctx)?;
    v.status = BondStatus::Unbonding;
    v.unbonding_time = sdk::time::add_duration(&ctx.header.time, &params.unbonding_time);
    v.unbonding_height = ctx.header.height;
    v.unbonding_ids.push(id);
    store::set_validator(ctx, &v)?;
    store::set_validator_by_power_index(ctx, &v)?;
    store::insert_unbonding_validator_queue(ctx, &v)?;
    let cons = v.cons_addr()?;
    let op = v.operator()?;
    let h = hooks::hooks();
    h.after_validator_begin_unbonding(ctx, &cons, &op)?;
    store::set_validator_by_unbonding_id(ctx, &v, id)?;
    h.after_unbonding_initiated(ctx, id)?;
    Ok(v)
}

/// `ApplyAndReturnValidatorSetUpdates`.
pub fn apply_and_return_validator_set_updates(
    ctx: &mut Context<'_>,
) -> Result<Vec<ValidatorUpdate>> {
    let params = get_params(ctx)?;
    let max = params.max_validators as usize;
    let mut total_power = Int::zero();
    let mut from_bonded = Int::zero();
    let mut to_bonded = Int::zero();

    // getLastValidatorsByAddr
    let mut last: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
    for (k, v) in store::collect_prefix(ctx, &[keys::LAST_VALIDATOR_POWER])? {
        last.insert(keys::address_from_key(&k), v);
    }

    let mut updates = Vec::new();
    let prefix = [keys::VALIDATORS_BY_POWER_INDEX];
    let end = sdk::context::prefix_end(&prefix);
    let mut it = ctx.reverse_iterator(STORE_KEY, Some(&prefix), end.as_deref())?;
    let mut count = 0usize;
    while count < max {
        let Some((_, op)) = it.next(ctx)? else { break };
        let mut v = store::must_get_validator(ctx, &op)?;
        if v.jailed {
            return Err(sdk::error::go_panic(
                "should never retrieve a jailed validator from the power store",
            ));
        }
        if v.potential_consensus_power()? == 0 {
            break;
        }
        match v.status {
            BondStatus::Unbonded | BondStatus::Unbonding => {
                v = bond_validator(ctx, v)?;
                to_bonded = to_bonded.checked_add(&v.tokens).map_err(math_err)?;
            }
            BondStatus::Bonded => {}
            BondStatus::Unspecified => {
                return Err(sdk::error::go_panic("unexpected validator status"));
            }
        }
        let new_power = v.consensus_power()?;
        let new_bytes = store::int64_value(new_power);
        if last.get(&op) != Some(&new_bytes) {
            updates.push(v.abci_update()?);
            store::set_last_validator_power(ctx, &op, new_power)?;
        }
        last.remove(&op);
        count += 1;
        total_power = total_power
            .checked_add(&Int::from_i64(new_power))
            .map_err(math_err)?;
    }

    // sortNoLongerBonded: BTreeMap keys are already in byte order.
    for op in last.keys() {
        let v = store::must_get_validator(ctx, op)?;
        if !v.is_bonded() {
            return Err(sdk::error::go_panic(
                "bad state transition bondedToUnbonding",
            ));
        }
        let v = begin_unbonding_validator(ctx, v)?;
        let op = v.operator()?;
        from_bonded = from_bonded.checked_add(&v.tokens).map_err(math_err)?;
        store::delete_last_validator_power(ctx, &op)?;
        updates.push(v.abci_update_zero()?);
    }

    if to_bonded > from_bonded {
        not_bonded_tokens_to_bonded(ctx, &to_bonded.checked_sub(&from_bonded).map_err(math_err)?)?;
    } else if to_bonded < from_bonded {
        bonded_tokens_to_not_bonded(ctx, &from_bonded.checked_sub(&to_bonded).map_err(math_err)?)?;
    }

    if !updates.is_empty() {
        store::set_last_total_power(ctx, &total_power)?;
    }
    store::set_validator_updates(ctx, &updates)?;
    Ok(updates)
}

/// `UnbondAllMatureValidators`.
pub fn unbond_all_mature_validators(ctx: &mut Context<'_>) -> Result<()> {
    let block_time = ctx.header.time;
    let block_height = ctx.header.height;
    let mut end = keys::validator_queue(&block_time, block_height);
    end.push(0);
    let mut it = ctx.iterator(STORE_KEY, Some(&[keys::VALIDATOR_QUEUE]), Some(&end))?;
    let block_time_bytes = sdk::time::format_time_bytes(&block_time);
    while let Some((k, val)) = it.next(ctx)? {
        let (key_time, key_height) = keys::parse_validator_queue(&k)
            .ok_or_else(|| sdk::error::internal("failed to parse unbonding key"))?;
        // FormatTimeBytes is order preserving, so byte comparison equals
        // `keyTime.Before(blockTime) || keyTime.Equal(blockTime)`.
        if !(key_height <= block_height && key_time <= block_time_bytes) {
            continue;
        }
        let addrs =
            <celestia_proto::cosmos::staking::v1beta1::ValAddresses as prost::Message>::decode(
                val.as_slice(),
            )
            .map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))?;
        for a in addrs.addresses {
            let op = crate::addr::val_from_string(&a)?;
            let mut v = store::get_validator(ctx, &op)?.ok_or_else(|| {
                errors::no_validator_found().wrap("validator in the unbonding queue was not found")
            })?;
            if !v.is_unbonding() {
                return Err(sdk::error::internal(
                    "unexpected validator in unbonding queue; status was not unbonding",
                ));
            }
            if v.unbonding_on_hold_ref_count == 0 {
                for id in v.unbonding_ids.clone() {
                    store::delete_unbonding_index(ctx, id)?;
                }
                // UnbondingToUnbonded -> completeUnbondingValidator
                v.status = BondStatus::Unbonded;
                store::set_validator(ctx, &v)?;
                if v.delegator_shares.is_zero() {
                    remove_validator(ctx, &v.operator()?)?;
                } else {
                    v.unbonding_ids.clear();
                }
                store::delete_validator_queue(ctx, &v)?;
            }
        }
    }
    Ok(())
}
