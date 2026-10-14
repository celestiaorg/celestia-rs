//! `InitGenesis` (`keeper/genesis.go`).

use celestia_proto::cosmos::distribution::v1beta1 as pb;
use math::LegacyDec;
use sdk::dec_coins::DecCoins;
use sdk::error::sdk as errs;
use sdk::{Context, Result};

use crate::MODULE_NAME;
use crate::store::{self, CurrentRewards, HistoricalRewards, Params};

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
