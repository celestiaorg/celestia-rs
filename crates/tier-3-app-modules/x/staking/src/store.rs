//! Store accessors (`keeper/{validator,delegation,params,historical_info,
//! unbonding}.go` getters and setters).

use celestia_proto::celestia_core_proto::abci::ValidatorUpdate;
use celestia_proto::cosmos::base::v1beta1::IntProto;
use celestia_proto::cosmos::staking::v1beta1 as pb;
use math::Int;
use pbjson_types::Timestamp;
use prost::Message;
use sdk::error::sdk as errs;
use sdk::{Context, Result};

use crate::types::{
    Delegation, Params, Redelegation, Repr, UnbondingDelegation, Validator, math_err,
};
use crate::{STORE_KEY, errors, hooks, keys};

fn decode<M: Message + Default>(b: &[u8]) -> Result<M> {
    M::decode(b).map_err(|e| errs::json_unmarshal(e.to_string()))
}

/// Iterates the full items under `prefix` charging gas like Go's
/// `for ; it.Valid(); it.Next()` loop that runs to the end.
pub(crate) fn collect_prefix(
    ctx: &mut Context<'_>,
    prefix: &[u8],
) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let mut it = ctx.prefix_iter(STORE_KEY, prefix)?;
    let mut out = Vec::new();
    while let Some(kv) = it.next(ctx)? {
        out.push(kv);
    }
    Ok(out)
}

// ---- params ----

/// `GetParams` (zero params when unset).
pub fn get_params(ctx: &mut Context<'_>) -> Result<Params> {
    match ctx.get(STORE_KEY, keys::PARAMS)? {
        Some(b) => Params::from_proto(&decode::<pb::Params>(&b)?, Repr::Wire),
        None => Ok(Params::default()),
    }
}

/// `SetParams`.
pub fn set_params(ctx: &mut Context<'_>, p: &Params) -> Result<()> {
    ctx.set(
        STORE_KEY,
        keys::PARAMS.to_vec(),
        p.to_proto().encode_to_vec(),
    )
}

/// `BondDenom` (falls back to `sdk.DefaultBondDenom`, `utia` in celestia).
pub fn bond_denom(ctx: &mut Context<'_>) -> Result<String> {
    let p = get_params(ctx)?;
    Ok(if p.bond_denom.is_empty() {
        crate::DEFAULT_BOND_DENOM.to_owned()
    } else {
        p.bond_denom
    })
}

// ---- last total power / last validator power ----

/// `GetLastTotalPower`.
pub fn get_last_total_power(ctx: &mut Context<'_>) -> Result<Int> {
    match ctx.get(STORE_KEY, keys::LAST_TOTAL_POWER)? {
        Some(b) => Int::unmarshal(decode::<IntProto>(&b)?.int.as_bytes()).map_err(math_err),
        None => Ok(Int::zero()),
    }
}

/// `SetLastTotalPower`.
pub fn set_last_total_power(ctx: &mut Context<'_>, power: &Int) -> Result<()> {
    let v = IntProto {
        int: power.to_string(),
    }
    .encode_to_vec();
    ctx.set(STORE_KEY, keys::LAST_TOTAL_POWER.to_vec(), v)
}

/// `GetLastValidatorPower`.
pub fn get_last_validator_power(ctx: &mut Context<'_>, op: &[u8]) -> Result<i64> {
    Ok(ctx
        .get(STORE_KEY, &keys::last_validator_power(op))?
        .map(|b| decode::<pbjson_types::Int64Value>(&b))
        .transpose()?
        .map_or(0, |v| v.value))
}

/// Encodes `gogotypes.Int64Value`.
#[must_use]
pub fn int64_value(v: i64) -> Vec<u8> {
    pbjson_types::Int64Value { value: v }.encode_to_vec()
}

/// `SetLastValidatorPower`.
pub fn set_last_validator_power(ctx: &mut Context<'_>, op: &[u8], power: i64) -> Result<()> {
    ctx.set(
        STORE_KEY,
        keys::last_validator_power(op),
        int64_value(power),
    )
}

/// `DeleteLastValidatorPower`.
pub fn delete_last_validator_power(ctx: &mut Context<'_>, op: &[u8]) -> Result<()> {
    ctx.delete(STORE_KEY, &keys::last_validator_power(op))
}

// ---- validators ----

/// `GetValidator` (`None` for `ErrNoValidatorFound`).
pub fn get_validator(ctx: &mut Context<'_>, op: &[u8]) -> Result<Option<Validator>> {
    ctx.get(STORE_KEY, &keys::validator(op))?
        .map(|b| Validator::decode(&b))
        .transpose()
}

/// `GetValidator` with `ErrNoValidatorFound` as an error.
pub fn validator(ctx: &mut Context<'_>, op: &[u8]) -> Result<Validator> {
    get_validator(ctx, op)?.ok_or_else(errors::no_validator_found)
}

/// `mustGetValidator`.
pub(crate) fn must_get_validator(ctx: &mut Context<'_>, op: &[u8]) -> Result<Validator> {
    get_validator(ctx, op)?.ok_or_else(|| {
        sdk::error::go_panic(format!(
            "validator record not found for address: {}",
            encoding::hex::encode(op).to_uppercase()
        ))
    })
}

/// `GetValidatorByConsAddr` (`None` for `ErrNoValidatorFound`).
pub fn get_validator_by_cons_addr(ctx: &mut Context<'_>, cons: &[u8]) -> Result<Option<Validator>> {
    match ctx.get(STORE_KEY, &keys::validator_by_cons_addr(cons))? {
        Some(op) => get_validator(ctx, &op),
        None => Ok(None),
    }
}

/// `mustGetValidatorByConsAddr`.
pub(crate) fn must_get_validator_by_cons_addr(
    ctx: &mut Context<'_>,
    cons: &[u8],
) -> Result<Validator> {
    get_validator_by_cons_addr(ctx, cons)?
        .ok_or_else(|| sdk::error::go_panic("validator with consensus-Address not found"))
}

/// `SetValidator`.
pub fn set_validator(ctx: &mut Context<'_>, v: &Validator) -> Result<()> {
    let bz = v.encode();
    let op = v.operator()?;
    ctx.set(STORE_KEY, keys::validator(&op), bz)
}

/// `SetValidatorByConsAddr`.
pub fn set_validator_by_cons_addr(ctx: &mut Context<'_>, v: &Validator) -> Result<()> {
    let cons = v.cons_addr()?;
    let op = v.operator()?;
    ctx.set(STORE_KEY, keys::validator_by_cons_addr(&cons), op)
}

fn power_key(v: &Validator) -> Result<Vec<u8>> {
    Ok(keys::power_index(
        v.potential_consensus_power()?,
        &v.operator()?,
    ))
}

/// `SetValidatorByPowerIndex` (skipped for jailed validators).
pub fn set_validator_by_power_index(ctx: &mut Context<'_>, v: &Validator) -> Result<()> {
    if v.jailed {
        return Ok(());
    }
    let op = v.operator()?;
    ctx.set(STORE_KEY, power_key(v)?, op)
}

/// `DeleteValidatorByPowerIndex`.
pub fn delete_validator_by_power_index(ctx: &mut Context<'_>, v: &Validator) -> Result<()> {
    ctx.delete(STORE_KEY, &power_key(v)?)
}

/// `SetNewValidatorByPowerIndex`.
pub fn set_new_validator_by_power_index(ctx: &mut Context<'_>, v: &Validator) -> Result<()> {
    let op = v.operator()?;
    ctx.set(STORE_KEY, power_key(v)?, op)
}

/// `GetAllValidators`.
pub fn get_all_validators(ctx: &mut Context<'_>) -> Result<Vec<Validator>> {
    collect_prefix(ctx, &[keys::VALIDATORS])?
        .iter()
        .map(|(_, v)| Validator::decode(v))
        .collect()
}

/// `GetLastValidators` (in operator-address order).
pub fn get_last_validators(ctx: &mut Context<'_>) -> Result<Vec<Validator>> {
    let max = get_params(ctx)?.max_validators as usize;
    let mut it = ctx.prefix_iter(STORE_KEY, &[keys::LAST_VALIDATOR_POWER])?;
    let mut out = Vec::new();
    while let Some((k, _)) = it.next(ctx)? {
        if out.len() >= max {
            return Err(sdk::error::go_panic(
                "more validators than maxValidators found",
            ));
        }
        out.push(validator(ctx, &keys::address_from_key(&k))?);
    }
    Ok(out)
}

/// `GetBondedValidatorsByPower`.
pub fn get_bonded_validators_by_power(ctx: &mut Context<'_>) -> Result<Vec<Validator>> {
    let max = get_params(ctx)?.max_validators as usize;
    let prefix = [keys::VALIDATORS_BY_POWER_INDEX];
    let end = sdk::context::prefix_end(&prefix);
    let mut it = ctx.reverse_iterator(STORE_KEY, Some(&prefix), end.as_deref())?;
    let mut out = Vec::new();
    while out.len() < max {
        let Some((_, op)) = it.next(ctx)? else { break };
        let v = must_get_validator(ctx, &op)?;
        if v.is_bonded() {
            out.push(v);
        }
    }
    Ok(out)
}

// ---- validator queue ----

/// `GetUnbondingValidators`.
pub fn get_unbonding_validators(
    ctx: &mut Context<'_>,
    t: &Timestamp,
    h: i64,
) -> Result<Vec<String>> {
    Ok(ctx
        .get(STORE_KEY, &keys::validator_queue(t, h))?
        .map(|b| decode::<pb::ValAddresses>(&b))
        .transpose()?
        .map(|v| v.addresses)
        .unwrap_or_default())
}

/// `SetUnbondingValidatorsQueue`.
pub fn set_unbonding_validators_queue(
    ctx: &mut Context<'_>,
    t: &Timestamp,
    h: i64,
    addrs: Vec<String>,
) -> Result<()> {
    let v = pb::ValAddresses { addresses: addrs }.encode_to_vec();
    ctx.set(STORE_KEY, keys::validator_queue(t, h), v)
}

/// `InsertUnbondingValidatorQueue`.
pub fn insert_unbonding_validator_queue(ctx: &mut Context<'_>, v: &Validator) -> Result<()> {
    let mut addrs = get_unbonding_validators(ctx, &v.unbonding_time, v.unbonding_height)?;
    addrs.push(v.operator_address.clone());
    set_unbonding_validators_queue(ctx, &v.unbonding_time, v.unbonding_height, addrs)
}

/// `DeleteValidatorQueue`.
pub fn delete_validator_queue(ctx: &mut Context<'_>, v: &Validator) -> Result<()> {
    let addrs = get_unbonding_validators(ctx, &v.unbonding_time, v.unbonding_height)?;
    let deleting = v.operator()?;
    let mut keep = Vec::new();
    for a in addrs {
        if crate::addr::val_from_string(&a)? != deleting {
            keep.push(a);
        }
    }
    if keep.is_empty() {
        return ctx.delete(
            STORE_KEY,
            &keys::validator_queue(&v.unbonding_time, v.unbonding_height),
        );
    }
    set_unbonding_validators_queue(ctx, &v.unbonding_time, v.unbonding_height, keep)
}

// ---- delegations ----

/// `GetDelegation` (`None` for `ErrNoDelegation`).
pub fn get_delegation(ctx: &mut Context<'_>, del: &[u8], val: &[u8]) -> Result<Option<Delegation>> {
    ctx.get(STORE_KEY, &keys::delegation(del, val))?
        .map(|b| Delegation::decode(&b))
        .transpose()
}

/// `SetDelegation`.
pub fn set_delegation(ctx: &mut Context<'_>, d: &Delegation) -> Result<()> {
    let del = crate::addr::acc_from_string(&d.delegator_address)?;
    let val = crate::addr::val_from_string(&d.validator_address)?;
    ctx.set(STORE_KEY, keys::delegation(&del, &val), d.encode())?;
    ctx.set(STORE_KEY, keys::delegation_by_val(&val, &del), Vec::new())
}

/// `RemoveDelegation` (calls `BeforeDelegationRemoved`).
pub fn remove_delegation(ctx: &mut Context<'_>, d: &Delegation) -> Result<()> {
    let del = crate::addr::acc_from_string(&d.delegator_address)?;
    let val = crate::addr::val_from_string(&d.validator_address)?;
    hooks::hooks().before_delegation_removed(ctx, &del, &val)?;
    ctx.delete(STORE_KEY, &keys::delegation(&del, &val))?;
    ctx.delete(STORE_KEY, &keys::delegation_by_val(&val, &del))
}

/// `GetAllDelegations`.
pub fn get_all_delegations(ctx: &mut Context<'_>) -> Result<Vec<Delegation>> {
    collect_prefix(ctx, &[keys::DELEGATION])?
        .iter()
        .map(|(_, v)| Delegation::decode(v))
        .collect()
}

/// `GetValidatorDelegations`.
pub fn get_validator_delegations(ctx: &mut Context<'_>, val: &[u8]) -> Result<Vec<Delegation>> {
    let mut it = ctx.prefix_iter(STORE_KEY, &keys::delegations_by_val(val))?;
    let mut out = Vec::new();
    while let Some((k, _)) = it.next(ctx)? {
        let (v, d) = keys::parse_delegation_by_val(&k)
            .ok_or_else(|| errs::logic("bad delegation index key"))?;
        let b = ctx
            .get(STORE_KEY, &keys::delegation(&d, &v))?
            .unwrap_or_default();
        out.push(Delegation::decode(&b)?);
    }
    Ok(out)
}

// ---- unbonding delegations ----

/// `GetUnbondingDelegation` (`None` for `ErrNoUnbondingDelegation`).
pub fn get_ubd(
    ctx: &mut Context<'_>,
    del: &[u8],
    val: &[u8],
) -> Result<Option<UnbondingDelegation>> {
    ctx.get(STORE_KEY, &keys::ubd(del, val))?
        .map(|b| UnbondingDelegation::decode(&b))
        .transpose()
}

/// `SetUnbondingDelegation`.
pub fn set_ubd(ctx: &mut Context<'_>, u: &UnbondingDelegation) -> Result<()> {
    let del = crate::addr::acc_from_string(&u.delegator_address)?;
    let bz = u.encode();
    let val = crate::addr::val_from_string(&u.validator_address)?;
    ctx.set(STORE_KEY, keys::ubd(&del, &val), bz)?;
    ctx.set(STORE_KEY, keys::ubd_by_val(&del, &val), Vec::new())
}

/// `RemoveUnbondingDelegation`.
pub fn remove_ubd(ctx: &mut Context<'_>, u: &UnbondingDelegation) -> Result<()> {
    let del = crate::addr::acc_from_string(&u.delegator_address)?;
    let val = crate::addr::val_from_string(&u.validator_address)?;
    ctx.delete(STORE_KEY, &keys::ubd(&del, &val))?;
    ctx.delete(STORE_KEY, &keys::ubd_by_val(&del, &val))
}

/// `GetUnbondingDelegationsFromValidator`.
pub fn get_ubds_from_validator(
    ctx: &mut Context<'_>,
    val: &[u8],
) -> Result<Vec<UnbondingDelegation>> {
    let mut it = ctx.prefix_iter(STORE_KEY, &keys::ubds_by_val(val))?;
    let mut out = Vec::new();
    while let Some((k, _)) = it.next(ctx)? {
        let key = keys::ubd_from_val_index(&k).ok_or_else(|| errs::logic("bad ubd index"))?;
        let b = ctx.get(STORE_KEY, &key)?.unwrap_or_default();
        out.push(UnbondingDelegation::decode(&b)?);
    }
    Ok(out)
}

/// `GetAllUnbondingDelegations` (genesis export order).
pub fn get_all_ubds(ctx: &mut Context<'_>) -> Result<Vec<UnbondingDelegation>> {
    collect_prefix(ctx, &[keys::UNBONDING_DELEGATION])?
        .iter()
        .map(|(_, v)| UnbondingDelegation::decode(v))
        .collect()
}

// ---- redelegations ----

/// `GetRedelegation` (`None` for `ErrNoRedelegation`).
pub fn get_red(
    ctx: &mut Context<'_>,
    del: &[u8],
    src: &[u8],
    dst: &[u8],
) -> Result<Option<Redelegation>> {
    ctx.get(STORE_KEY, &keys::red(del, src, dst))?
        .map(|b| Redelegation::decode(&b))
        .transpose()
}

fn red_addrs(r: &Redelegation) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    Ok((
        crate::addr::acc_from_string(&r.delegator_address)?,
        crate::addr::val_from_string(&r.validator_src_address)?,
        crate::addr::val_from_string(&r.validator_dst_address)?,
    ))
}

/// `SetRedelegation`.
pub fn set_red(ctx: &mut Context<'_>, r: &Redelegation) -> Result<()> {
    let bz = r.encode();
    let (del, src, dst) = red_addrs(r)?;
    ctx.set(STORE_KEY, keys::red(&del, &src, &dst), bz)?;
    ctx.set(
        STORE_KEY,
        keys::red_by_val_src(&del, &src, &dst),
        Vec::new(),
    )?;
    ctx.set(
        STORE_KEY,
        keys::red_by_val_dst(&del, &src, &dst),
        Vec::new(),
    )
}

/// `RemoveRedelegation`.
pub fn remove_red(ctx: &mut Context<'_>, r: &Redelegation) -> Result<()> {
    let (del, src, dst) = red_addrs(r)?;
    ctx.delete(STORE_KEY, &keys::red(&del, &src, &dst))?;
    ctx.delete(STORE_KEY, &keys::red_by_val_src(&del, &src, &dst))?;
    ctx.delete(STORE_KEY, &keys::red_by_val_dst(&del, &src, &dst))
}

/// `GetRedelegationsFromSrcValidator`.
pub fn get_reds_from_src_validator(ctx: &mut Context<'_>, val: &[u8]) -> Result<Vec<Redelegation>> {
    let mut it = ctx.prefix_iter(STORE_KEY, &keys::reds_from_val_src(val))?;
    let mut out = Vec::new();
    while let Some((k, _)) = it.next(ctx)? {
        let key = keys::red_from_val_src_index(&k).ok_or_else(|| errs::logic("bad red index"))?;
        let b = ctx.get(STORE_KEY, &key)?.unwrap_or_default();
        out.push(Redelegation::decode(&b)?);
    }
    Ok(out)
}

/// `HasReceivingRedelegation`.
pub fn has_receiving_redelegation(ctx: &mut Context<'_>, del: &[u8], dst: &[u8]) -> Result<bool> {
    let mut it = ctx.prefix_iter(STORE_KEY, &keys::reds_by_del_to_val_dst(del, dst))?;
    Ok(it.next(ctx)?.is_some())
}

/// `GetAllRedelegations` (genesis export order).
pub fn get_all_reds(ctx: &mut Context<'_>) -> Result<Vec<Redelegation>> {
    collect_prefix(ctx, &[keys::REDELEGATION])?
        .iter()
        .map(|(_, v)| Redelegation::decode(v))
        .collect()
}

// ---- queues ----

/// `GetUBDQueueTimeSlice`.
pub fn get_ubd_queue_time_slice(ctx: &mut Context<'_>, t: &Timestamp) -> Result<Vec<pb::DvPair>> {
    Ok(ctx
        .get(STORE_KEY, &keys::ubd_queue(t))?
        .map(|b| decode::<pb::DvPairs>(&b))
        .transpose()?
        .map(|p| p.pairs)
        .unwrap_or_default())
}

/// `InsertUBDQueue`.
pub fn insert_ubd_queue(
    ctx: &mut Context<'_>,
    u: &UnbondingDelegation,
    t: &Timestamp,
) -> Result<()> {
    let pair = pb::DvPair {
        delegator_address: u.delegator_address.clone(),
        validator_address: u.validator_address.clone(),
    };
    let mut slice = get_ubd_queue_time_slice(ctx, t)?;
    slice.push(pair);
    ctx.set(
        STORE_KEY,
        keys::ubd_queue(t),
        pb::DvPairs { pairs: slice }.encode_to_vec(),
    )
}

/// `GetRedelegationQueueTimeSlice`.
pub fn get_red_queue_time_slice(
    ctx: &mut Context<'_>,
    t: &Timestamp,
) -> Result<Vec<pb::DvvTriplet>> {
    Ok(ctx
        .get(STORE_KEY, &keys::red_queue(t))?
        .map(|b| decode::<pb::DvvTriplets>(&b))
        .transpose()?
        .map(|p| p.triplets)
        .unwrap_or_default())
}

/// `InsertRedelegationQueue`.
pub fn insert_red_queue(ctx: &mut Context<'_>, r: &Redelegation, t: &Timestamp) -> Result<()> {
    let mut slice = get_red_queue_time_slice(ctx, t)?;
    slice.push(pb::DvvTriplet {
        delegator_address: r.delegator_address.clone(),
        validator_src_address: r.validator_src_address.clone(),
        validator_dst_address: r.validator_dst_address.clone(),
    });
    ctx.set(
        STORE_KEY,
        keys::red_queue(t),
        pb::DvvTriplets { triplets: slice }.encode_to_vec(),
    )
}

/// Iterates `[prefix, inclusive_end]` in key order, deleting each entry
/// (`DequeueAllMatureUBDQueue` / `DequeueAllMatureRedelegationQueue`).
fn dequeue(ctx: &mut Context<'_>, prefix: u8, end_inclusive: &[u8]) -> Result<Vec<Vec<u8>>> {
    let mut end = end_inclusive.to_vec();
    end.push(0);
    let mut it = ctx.iterator(STORE_KEY, Some(&[prefix]), Some(&end))?;
    let mut vals = Vec::new();
    while let Some((k, v)) = it.next(ctx)? {
        vals.push(v);
        ctx.delete(STORE_KEY, &k)?;
    }
    Ok(vals)
}

/// `DequeueAllMatureUBDQueue`.
pub fn dequeue_all_mature_ubd_queue(
    ctx: &mut Context<'_>,
    now: &Timestamp,
) -> Result<Vec<pb::DvPair>> {
    let mut out = Vec::new();
    for v in dequeue(ctx, keys::UNBONDING_QUEUE, &keys::ubd_queue(now))? {
        out.extend(decode::<pb::DvPairs>(&v)?.pairs);
    }
    Ok(out)
}

/// `DequeueAllMatureRedelegationQueue`.
pub fn dequeue_all_mature_red_queue(
    ctx: &mut Context<'_>,
    now: &Timestamp,
) -> Result<Vec<pb::DvvTriplet>> {
    let mut out = Vec::new();
    for v in dequeue(ctx, keys::REDELEGATION_QUEUE, &keys::red_queue(now))? {
        out.extend(decode::<pb::DvvTriplets>(&v)?.triplets);
    }
    Ok(out)
}

// ---- unbonding IDs ----

/// `IncrementUnbondingID`.
pub fn increment_unbonding_id(ctx: &mut Context<'_>) -> Result<u64> {
    let cur = ctx
        .get(STORE_KEY, keys::UNBONDING_ID)?
        .and_then(|b| sdk::codec::decode_u64(&b))
        .unwrap_or(0);
    let id = cur + 1;
    ctx.set(
        STORE_KEY,
        keys::UNBONDING_ID.to_vec(),
        id.to_be_bytes().to_vec(),
    )?;
    Ok(id)
}

/// `DeleteUnbondingIndex`.
pub fn delete_unbonding_index(ctx: &mut Context<'_>, id: u64) -> Result<()> {
    ctx.delete(STORE_KEY, &keys::unbonding_index(id))
}

fn set_unbonding_type(ctx: &mut Context<'_>, id: u64, ty: u64) -> Result<()> {
    ctx.set(
        STORE_KEY,
        keys::unbonding_type(id),
        ty.to_be_bytes().to_vec(),
    )
}

/// `SetUnbondingDelegationByUnbondingID`.
pub fn set_ubd_by_unbonding_id(
    ctx: &mut Context<'_>,
    u: &UnbondingDelegation,
    id: u64,
) -> Result<()> {
    let del = crate::addr::acc_from_string(&u.delegator_address)?;
    let val = crate::addr::val_from_string(&u.validator_address)?;
    ctx.set(STORE_KEY, keys::unbonding_index(id), keys::ubd(&del, &val))?;
    set_unbonding_type(ctx, id, 1)
}

/// `SetRedelegationByUnbondingID`.
pub fn set_red_by_unbonding_id(ctx: &mut Context<'_>, r: &Redelegation, id: u64) -> Result<()> {
    let (del, src, dst) = red_addrs(r)?;
    ctx.set(
        STORE_KEY,
        keys::unbonding_index(id),
        keys::red(&del, &src, &dst),
    )?;
    set_unbonding_type(ctx, id, 2)
}

/// `SetValidatorByUnbondingID`.
pub fn set_validator_by_unbonding_id(ctx: &mut Context<'_>, v: &Validator, id: u64) -> Result<()> {
    let op = v.operator()?;
    ctx.set(STORE_KEY, keys::unbonding_index(id), keys::validator(&op))?;
    set_unbonding_type(ctx, id, 3)
}

// ---- historical info / validator updates ----

/// `GetHistoricalInfo`.
pub fn get_historical_info(
    ctx: &mut Context<'_>,
    height: i64,
) -> Result<Option<pb::HistoricalInfo>> {
    ctx.get(STORE_KEY, &keys::historical_info(height))?
        .map(|b| decode(&b))
        .transpose()
}

/// `SetValidatorUpdates`.
pub fn set_validator_updates(ctx: &mut Context<'_>, updates: &[ValidatorUpdate]) -> Result<()> {
    let v = pb::ValidatorUpdates {
        updates: updates.to_vec(),
    }
    .encode_to_vec();
    ctx.set(STORE_KEY, keys::VALIDATOR_UPDATES.to_vec(), v)
}

/// `GetValidatorUpdates`.
pub fn get_validator_updates(ctx: &mut Context<'_>) -> Result<Vec<ValidatorUpdate>> {
    let b = ctx
        .get(STORE_KEY, keys::VALIDATOR_UPDATES)?
        .unwrap_or_default();
    Ok(decode::<pb::ValidatorUpdates>(&b)?.updates)
}
