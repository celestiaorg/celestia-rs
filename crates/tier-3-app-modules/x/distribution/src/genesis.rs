//! `InitGenesis` (`keeper/genesis.go`).

use celestia_proto::cosmos::distribution::v1beta1 as pb;
use math::LegacyDec;
use sdk::dec_coins::DecCoins;
use sdk::error::sdk as errs;
use sdk::{Context, Result};

use crate::MODULE_NAME;
use crate::store::{self, CurrentRewards, HistoricalRewards, Params};

fn raw(ctx: &mut Context<'_>, prefix: u8) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let mut it = ctx.prefix_iter(crate::STORE_KEY, &[prefix])?;
    let mut out = Vec::new();
    while let Some(kv) = it.next(ctx)? {
        out.push(kv);
    }
    Ok(out)
}

fn dec<M: prost::Message + Default>(b: &[u8]) -> Result<M> {
    M::decode(b).map_err(|e| errs::json_unmarshal(e.to_string()))
}

/// `(len || addr, rest)` of a key after its one-byte prefix.
fn split_lp(k: &[u8]) -> Result<(Vec<u8>, &[u8])> {
    let n = usize::from(*k.get(1).ok_or_else(|| errs::logic("short key"))?);
    let a = k.get(2..2 + n).ok_or_else(|| errs::logic("short key"))?;
    Ok((a.to_vec(), &k[2 + n..]))
}

fn val_str(a: &[u8]) -> Result<String> {
    Ok(sdk::address::to_bech32(sdk::address::VALOPER_PREFIX, a))
}

/// `ExportGenesis` (`keeper/genesis.go`).
#[allow(clippy::too_many_lines)]
pub fn export_genesis(ctx: &mut Context<'_>) -> Result<pb::GenesisState> {
    let params = ctx
        .get(crate::STORE_KEY, store::PARAMS)?
        .map(|b| dec::<pb::Params>(&b))
        .transpose()?;
    let fee_pool = ctx
        .get(crate::STORE_KEY, store::FEE_POOL)?
        .map(|b| dec::<pb::FeePool>(&b))
        .transpose()?;
    let mut delegator_withdraw_infos = Vec::new();
    for (k, v) in raw(ctx, store::WITHDRAW_ADDR)? {
        let (del, _) = split_lp(&k)?;
        delegator_withdraw_infos.push(pb::DelegatorWithdrawInfo {
            delegator_address: x_auth::bech32(&del)?,
            withdraw_address: x_auth::bech32(&v)?,
        });
    }
    let previous_proposer = match ctx.get(crate::STORE_KEY, store::PROPOSER)? {
        Some(b) => {
            let v = dec::<pbjson_types::BytesValue>(&b)?.value;
            sdk::address::to_bech32(sdk::address::VALCONS_PREFIX, &v)
        }
        None => String::new(),
    };
    let mut outstanding_rewards = Vec::new();
    for (k, v) in raw(ctx, store::OUTSTANDING_REWARDS)? {
        let (val, _) = split_lp(&k)?;
        outstanding_rewards.push(pb::ValidatorOutstandingRewardsRecord {
            validator_address: val_str(&val)?,
            outstanding_rewards: dec::<pb::ValidatorOutstandingRewards>(&v)?.rewards,
        });
    }
    let mut validator_accumulated_commissions = Vec::new();
    for (k, v) in raw(ctx, store::ACCUMULATED_COMMISSION)? {
        let (val, _) = split_lp(&k)?;
        validator_accumulated_commissions.push(pb::ValidatorAccumulatedCommissionRecord {
            validator_address: val_str(&val)?,
            accumulated: Some(dec::<pb::ValidatorAccumulatedCommission>(&v)?),
        });
    }
    let mut validator_historical_rewards = Vec::new();
    for (k, v) in raw(ctx, store::HISTORICAL_REWARDS)? {
        let (val, rest) = split_lp(&k)?;
        let period = u64::from_le_bytes(
            rest.try_into()
                .map_err(|_| errs::logic("historical rewards key"))?,
        );
        validator_historical_rewards.push(pb::ValidatorHistoricalRewardsRecord {
            validator_address: val_str(&val)?,
            period,
            rewards: Some(dec::<pb::ValidatorHistoricalRewards>(&v)?),
        });
    }
    let mut validator_current_rewards = Vec::new();
    for (k, v) in raw(ctx, store::CURRENT_REWARDS)? {
        let (val, _) = split_lp(&k)?;
        validator_current_rewards.push(pb::ValidatorCurrentRewardsRecord {
            validator_address: val_str(&val)?,
            rewards: Some(dec::<pb::ValidatorCurrentRewards>(&v)?),
        });
    }
    let mut delegator_starting_infos = Vec::new();
    for (k, v) in raw(ctx, store::STARTING_INFO)? {
        let (val, rest) = split_lp(&k)?;
        let n = usize::from(
            *rest
                .first()
                .ok_or_else(|| errs::logic("starting info key"))?,
        );
        let del = rest
            .get(1..1 + n)
            .ok_or_else(|| errs::logic("starting info key"))?;
        delegator_starting_infos.push(pb::DelegatorStartingInfoRecord {
            delegator_address: x_auth::bech32(del)?,
            validator_address: val_str(&val)?,
            starting_info: Some(dec::<pb::DelegatorStartingInfo>(&v)?),
        });
    }
    let mut validator_slash_events = Vec::new();
    for (k, v) in raw(ctx, store::SLASH_EVENT)? {
        let (val, rest) = split_lp(&k)?;
        let height = u64::from_be_bytes(
            rest.get(..8)
                .and_then(|b| b.try_into().ok())
                .ok_or_else(|| errs::logic("slash event key"))?,
        );
        let ev = dec::<pb::ValidatorSlashEvent>(&v)?;
        validator_slash_events.push(pb::ValidatorSlashEventRecord {
            validator_address: val_str(&val)?,
            height,
            period: ev.validator_period,
            validator_slash_event: Some(ev),
        });
    }
    Ok(pb::GenesisState {
        params,
        fee_pool,
        delegator_withdraw_infos,
        previous_proposer,
        outstanding_rewards,
        validator_accumulated_commissions,
        validator_historical_rewards,
        validator_current_rewards,
        delegator_starting_infos,
        validator_slash_events,
    })
}

/// `InitGenesis`.
pub fn init_genesis(ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    let gs: pb::GenesisState =
        serde_json::from_value(genesis.clone()).map_err(|e| errs::json_unmarshal(e.to_string()))?;
    let mut holdings = DecCoins::new();
    let pool = DecCoins::from_json_proto(&gs.fee_pool.clone().unwrap_or_default().community_pool)?;
    store::set_fee_pool(ctx, &pool)?;
    store::set_params(
        ctx,
        &Params::from_json(&gs.params.clone().unwrap_or_default())?,
    )?;

    for dwi in &gs.delegator_withdraw_infos {
        let del = x_auth::parse_bech32(&dwi.delegator_address)?;
        let to = x_auth::parse_bech32(&dwi.withdraw_address)?;
        store::set_withdraw_addr(ctx, &del, &to)?;
    }

    let proposer = if gs.previous_proposer.is_empty() {
        Vec::new()
    } else {
        x_staking::addr::cons_from_string(&gs.previous_proposer)?
    };
    store::set_previous_proposer(ctx, &proposer)?;

    for r in &gs.outstanding_rewards {
        let val = x_staking::addr::val_from_string(&r.validator_address)?;
        let rewards = DecCoins::from_json_proto(&r.outstanding_rewards)?;
        store::set_outstanding(ctx, &val, &rewards)?;
        holdings = holdings.add(&rewards)?;
    }
    for a in &gs.validator_accumulated_commissions {
        let val = x_staking::addr::val_from_string(&a.validator_address)?;
        let c = DecCoins::from_json_proto(&a.accumulated.clone().unwrap_or_default().commission)?;
        store::set_commission(ctx, &val, &c)?;
    }
    for h in &gs.validator_historical_rewards {
        let val = x_staking::addr::val_from_string(&h.validator_address)?;
        let r = h.rewards.clone().unwrap_or_default();
        store::set_historical(
            ctx,
            &val,
            h.period,
            &HistoricalRewards {
                cumulative_reward_ratio: DecCoins::from_json_proto(&r.cumulative_reward_ratio)?,
                reference_count: r.reference_count,
            },
        )?;
    }
    for c in &gs.validator_current_rewards {
        let val = x_staking::addr::val_from_string(&c.validator_address)?;
        let r = c.rewards.clone().unwrap_or_default();
        store::set_current(
            ctx,
            &val,
            &CurrentRewards {
                rewards: DecCoins::from_json_proto(&r.rewards)?,
                period: r.period,
            },
        )?;
    }
    for d in &gs.delegator_starting_infos {
        let val = x_staking::addr::val_from_string(&d.validator_address)?;
        let del = x_auth::parse_bech32(&d.delegator_address)?;
        let info = store::starting_info_from_json(&d.starting_info.clone().unwrap_or_default())?;
        store::set_starting_info(ctx, &val, &del, &info)?;
    }
    for e in &gs.validator_slash_events {
        let val = x_staking::addr::val_from_string(&e.validator_address)?;
        let ev = e.validator_slash_event.clone().unwrap_or_default();
        let fraction = LegacyDec::from_str_go(&ev.fraction)
            .map_err(|e| errs::json_unmarshal(e.to_string()))?;
        store::set_slash_event(ctx, &val, e.height, e.period, &fraction)?;
    }

    holdings = holdings.add(&pool)?;
    let (holdings_int, _) = holdings.truncate_decimal()?;
    let acc = x_auth::get_module_account(ctx, MODULE_NAME)?
        .ok_or_else(|| errs::logic("distribution module account has not been set"))?;
    let acc = x_auth::Account::Module(acc);
    let balances = x_bank::get_all_balances(ctx, &acc.address()?)?;
    if balances.is_empty() {
        x_auth::set_account(ctx, &acc)?;
    }
    if balances != holdings_int {
        return Err(errs::logic(format!(
            "distribution module balance does not match the module holdings: {balances} <-> {holdings_int}"
        )));
    }
    Ok(())
}
