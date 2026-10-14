//! `InitGenesis` (`keeper/genesis.go`).

use celestia_proto::celestia_core_proto::abci::ValidatorUpdate;
use celestia_proto::cosmos::staking::v1beta1 as pb;
use math::Int;
use sdk::coins::{Coin, Coins};
use sdk::error::sdk as errs;
use sdk::{Context, Result};

use crate::store;
use crate::types::{
    BondStatus, Delegation, Params, Redelegation, Repr, UnbondingDelegation, Validator, math_err,
};
use crate::validator::apply_and_return_validator_set_updates;
use crate::{BONDED_POOL, NOT_BONDED_POOL, hooks};

/// Rewrites `bytes` fields that Go encodes as `math.Int` decimal strings in
/// JSON into the base64 form proto3 JSON expects.
fn normalize_int_bytes(v: &mut serde_json::Value, field: &str) -> Result<()> {
    if let Some(s) = v.get(field).and_then(serde_json::Value::as_str) {
        let b =
            math::int_json_to_proto_bytes(s).map_err(|e| errs::json_unmarshal(e.to_string()))?;
        v[field] = serde_json::Value::String(encoding::base64::encode_standard(&b));
    }
    Ok(())
}

fn pool_balance(ctx: &mut Context<'_>, name: &str) -> Result<(x_auth::Account, Coins)> {
    let acc = x_auth::get_module_account(ctx, name)?
        .ok_or_else(|| errs::logic(format!("{name} module account has not been set")))?;
    let acc = x_auth::Account::Module(acc);
    let bal = x_bank::get_all_balances(ctx, &acc.address()?)?;
    Ok((acc, bal))
}

fn decode_all<M: prost::Message + Default>(ctx: &mut Context<'_>, prefix: u8) -> Result<Vec<M>> {
    store::collect_prefix(ctx, &[prefix])?
        .iter()
        .map(|(_, v)| M::decode(v.as_slice()).map_err(|e| errs::json_unmarshal(e.to_string())))
        .collect()
}

/// `ExportGenesis` (`exported = true`): params, last total power, last
/// validator powers, validators, delegations, unbonding delegations and
/// redelegations, each in store order.
pub fn export_genesis(ctx: &mut Context<'_>) -> Result<pb::GenesisState> {
    let params = store::get_params(ctx)?.to_proto();
    let last_total_power = store::get_last_total_power(ctx)?.to_string().into_bytes();
    let mut last_validator_powers = Vec::new();
    for (k, v) in store::collect_prefix(ctx, &[crate::keys::LAST_VALIDATOR_POWER])? {
        let power = <pbjson_types::Int64Value as prost::Message>::decode(v.as_slice())
            .map_err(|e| errs::json_unmarshal(e.to_string()))?
            .value;
        last_validator_powers.push(pb::LastValidatorPower {
            address: crate::addr::val_to_string(&crate::keys::address_from_key(&k))?,
            power,
        });
    }
    Ok(pb::GenesisState {
        params: Some(params),
        last_total_power,
        last_validator_powers,
        validators: decode_all(ctx, crate::keys::VALIDATORS)?,
        delegations: decode_all(ctx, crate::keys::DELEGATION)?,
        unbonding_delegations: decode_all(ctx, crate::keys::UNBONDING_DELEGATION)?,
        redelegations: decode_all(ctx, crate::keys::REDELEGATION)?,
        exported: true,
    })
}

/// `staking.WriteValidators`: the last validator set (operator order) as
/// `(consensus pubkey Any, power, moniker)`.
pub fn write_validators(ctx: &mut Context<'_>) -> Result<Vec<(celestia_proto::Any, i64, String)>> {
    let mut out = Vec::new();
    for v in store::get_last_validators(ctx)? {
        let p = v.to_proto();
        out.push((
            p.consensus_pubkey.clone().unwrap_or_default(),
            v.consensus_power()?,
            p.description.map(|d| d.moniker).unwrap_or_default(),
        ));
    }
    Ok(out)
}

/// `InitGenesis`: returns the initial validator set.
pub fn init_genesis(
    ctx: &mut Context<'_>,
    genesis: &serde_json::Value,
) -> Result<Vec<ValidatorUpdate>> {
    let mut g = genesis.clone();
    normalize_int_bytes(&mut g, "last_total_power")?;
    let gs: pb::GenesisState =
        serde_json::from_value(g).map_err(|e| errs::json_unmarshal(e.to_string()))?;

    // InitGenesis runs at height 1 - ValidatorUpdateDelay.
    let saved_height = ctx.header.height;
    ctx.header.height = 0;
    let res = init_genesis_inner(ctx, &gs);
    ctx.header.height = saved_height;
    res
}

fn init_genesis_inner(
    ctx: &mut Context<'_>,
    gs: &pb::GenesisState,
) -> Result<Vec<ValidatorUpdate>> {
    let params = Params::from_proto(&gs.params.clone().unwrap_or_default(), Repr::Json)?;
    let mut bonded = Int::zero();
    let mut not_bonded = Int::zero();
    store::set_params(ctx, &params)?;
    let ltp = Int::unmarshal(&gs.last_total_power).map_err(math_err)?;
    store::set_last_total_power(ctx, &ltp)?;
    let h = hooks::hooks();

    for pv in &gs.validators {
        let v = Validator::from_proto(pv, Repr::Json)?;
        store::set_validator(ctx, &v)?;
        store::set_validator_by_cons_addr(ctx, &v)?;
        store::set_validator_by_power_index(ctx, &v)?;
        if !gs.exported {
            h.after_validator_created(ctx, &v.operator()?)?;
        }
        if v.is_unbonding() {
            store::insert_unbonding_validator_queue(ctx, &v)?;
        }
        match v.status {
            BondStatus::Bonded => bonded = bonded.checked_add(&v.tokens).map_err(math_err)?,
            BondStatus::Unbonding | BondStatus::Unbonded => {
                not_bonded = not_bonded.checked_add(&v.tokens).map_err(math_err)?;
            }
            BondStatus::Unspecified => return Err(errs::logic("invalid validator status")),
        }
    }

    for pd in &gs.delegations {
        let d = Delegation::from_proto(pd, Repr::Json)?;
        let del = crate::addr::acc_from_string(&d.delegator_address)?;
        let val = crate::addr::val_from_string(&d.validator_address)?;
        if !gs.exported {
            h.before_delegation_created(ctx, &del, &val)?;
        }
        store::set_delegation(ctx, &d)?;
        if !gs.exported {
            h.after_delegation_modified(ctx, &del, &val)?;
        }
    }

    for pu in &gs.unbonding_delegations {
        let u = UnbondingDelegation::from_proto(pu, Repr::Json)?;
        store::set_ubd(ctx, &u)?;
        for e in &u.entries {
            store::insert_ubd_queue(ctx, &u, &e.completion_time)?;
            not_bonded = not_bonded.checked_add(&e.balance).map_err(math_err)?;
        }
    }

    for pr in &gs.redelegations {
        let r = Redelegation::from_proto(pr, Repr::Json)?;
        store::set_red(ctx, &r)?;
        for e in &r.entries {
            store::insert_red_queue(ctx, &r, &e.completion_time)?;
        }
    }

    let bonded_coins =
        Coins::sanitized(vec![Coin::with_amount(params.bond_denom.clone(), bonded)])?;
    let not_bonded_coins = Coins::sanitized(vec![Coin::with_amount(
        params.bond_denom.clone(),
        not_bonded,
    )])?;

    let (acc, bal) = pool_balance(ctx, BONDED_POOL)?;
    if bal.is_empty() {
        x_auth::set_account(ctx, &acc)?;
    }
    if bal != bonded_coins {
        return Err(errs::logic(format!(
            "bonded pool balance is different from bonded coins: {bal} <-> {bonded_coins}"
        )));
    }
    let (acc, bal) = pool_balance(ctx, NOT_BONDED_POOL)?;
    if bal.is_empty() {
        x_auth::set_account(ctx, &acc)?;
    }
    if bal != not_bonded_coins {
        return Err(errs::logic(format!(
            "not bonded pool balance is different from not bonded coins: {bal} <-> {not_bonded_coins}"
        )));
    }

    if gs.exported {
        let mut res = Vec::new();
        for lv in &gs.last_validator_powers {
            let op = crate::addr::val_from_string(&lv.address)?;
            store::set_last_validator_power(ctx, &op, lv.power)?;
            let v = store::get_validator(ctx, &op)?
                .ok_or_else(|| errs::logic(format!("validator {} not found", lv.address)))?;
            let mut u = v.abci_update()?;
            u.power = lv.power;
            res.push(u);
        }
        Ok(res)
    } else {
        apply_and_return_validator_set_updates(ctx)
    }
}
