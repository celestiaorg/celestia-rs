//! Reward accounting (`keeper/{keeper,validator,delegation,allocation,
//! fee_pool,hooks}.go`).

use celestia_proto::celestia_core_proto::abci::VoteInfo;
use math::{Int, LegacyDec};
use sdk::coins::{Coin, Coins};
use sdk::dec_coins::DecCoins;
use sdk::error::sdk as errs;
use sdk::event::event;
use sdk::{Context, Result};
use x_staking::types::{Delegation, Validator};

use crate::store::{self, CurrentRewards, HistoricalRewards, StartingInfo};
use crate::{MODULE_NAME, errors};

fn m<T>(r: math::Result<T>) -> Result<T> {
    r.map_err(|e| sdk::error::go_panic(e.to_string()))
}

fn is_vesting(acc: Option<&x_auth::Account>) -> bool {
    matches!(
        acc,
        Some(
            x_auth::Account::ContinuousVesting(_)
                | x_auth::Account::DelayedVesting(_)
                | x_auth::Account::PeriodicVesting(_)
                | x_auth::Account::PermanentLocked(_)
        )
    )
}

/// `stakingKeeper.Validator` (error when not found).
fn staking_validator(ctx: &mut Context<'_>, val: &[u8]) -> Result<Validator> {
    x_staking::store::validator(ctx, val)
}

/// `stakingKeeper.Delegation` (error when not found).
fn staking_delegation(ctx: &mut Context<'_>, del: &[u8], val: &[u8]) -> Result<Delegation> {
    x_staking::store::get_delegation(ctx, del, val)?.ok_or_else(x_staking::errors::no_delegation)
}

/// `initializeValidator`.
pub fn initialize_validator(ctx: &mut Context<'_>, val: &[u8]) -> Result<()> {
    store::set_historical(
        ctx,
        val,
        0,
        &HistoricalRewards {
            cumulative_reward_ratio: DecCoins::new(),
            reference_count: 1,
        },
    )?;
    store::set_current(
        ctx,
        val,
        &CurrentRewards {
            rewards: DecCoins::new(),
            period: 1,
        },
    )?;
    store::set_commission(ctx, val, &DecCoins::new())?;
    store::set_outstanding(ctx, val, &DecCoins::new())
}

/// `IncrementValidatorPeriod`: returns the period that just ended.
pub fn increment_validator_period(ctx: &mut Context<'_>, v: &Validator) -> Result<u64> {
    let val = v.operator()?;
    let rewards = store::get_current(ctx, &val)?;
    let current = if v.tokens.is_zero() {
        let mut pool = store::get_fee_pool(ctx)?;
        let mut outstanding = store::get_outstanding(ctx, &val)?;
        pool = pool.add(&rewards.rewards)?;
        outstanding = outstanding.sub(&rewards.rewards)?;
        store::set_fee_pool(ctx, &pool)?;
        store::set_outstanding(ctx, &val, &outstanding)?;
        DecCoins::new()
    } else {
        rewards
            .rewards
            .quo_dec_truncate(&LegacyDec::from_int(&v.tokens))?
    };
    let prev = rewards.period.wrapping_sub(1);
    let historical = store::get_historical(ctx, &val, prev)?;
    let new_ratio = historical
        .cumulative_reward_ratio
        .add(&current)
        .map_err(|e| sdk::error::internal(format!("reward ratio overflow: {}", e.log)))?;
    decrement_reference_count(ctx, &val, prev)?;
    store::set_historical(
        ctx,
        &val,
        rewards.period,
        &HistoricalRewards {
            cumulative_reward_ratio: new_ratio,
            reference_count: 1,
        },
    )?;
    store::set_current(
        ctx,
        &val,
        &CurrentRewards {
            rewards: DecCoins::new(),
            period: rewards.period + 1,
        },
    )?;
    Ok(rewards.period)
}

/// `incrementReferenceCount`.
pub fn increment_reference_count(ctx: &mut Context<'_>, val: &[u8], period: u64) -> Result<()> {
    let mut h = store::get_historical(ctx, val, period)?;
    if h.reference_count > 2 {
        return Err(sdk::error::go_panic(
            "reference count should never exceed 2",
        ));
    }
    h.reference_count += 1;
    store::set_historical(ctx, val, period, &h)
}

/// `decrementReferenceCount`.
pub fn decrement_reference_count(ctx: &mut Context<'_>, val: &[u8], period: u64) -> Result<()> {
    let mut h = store::get_historical(ctx, val, period)?;
    if h.reference_count == 0 {
        return Err(sdk::error::go_panic("cannot set negative reference count"));
    }
    h.reference_count -= 1;
    if h.reference_count == 0 {
        return ctx.delete(crate::STORE_KEY, &store::historical_key(val, period));
    }
    store::set_historical(ctx, val, period, &h)
}

/// `updateValidatorSlashFraction`.
pub fn update_validator_slash_fraction(
    ctx: &mut Context<'_>,
    val: &[u8],
    fraction: &LegacyDec,
) -> Result<()> {
    if *fraction > LegacyDec::one() || fraction.is_negative() {
        return Err(sdk::error::go_panic(format!(
            "fraction must be >=0 and <=1, current fraction: {fraction}"
        )));
    }
    let v = staking_validator(ctx, val)?;
    let new_period = increment_validator_period(ctx, &v)?;
    // Go ignores this error.
    let _ = increment_reference_count(ctx, val, new_period);
    #[allow(clippy::cast_sign_loss)]
    let height = ctx.header.height as u64;
    store::set_slash_event(ctx, val, height, new_period, fraction)
}

/// `initializeDelegation`.
pub fn initialize_delegation(ctx: &mut Context<'_>, val: &[u8], del: &[u8]) -> Result<()> {
    let cur = store::get_current(ctx, val)?;
    let previous = cur.period.wrapping_sub(1);
    increment_reference_count(ctx, val, previous)?;
    let v = staking_validator(ctx, val)?;
    let d = staking_delegation(ctx, del, val)?;
    let stake = v.tokens_from_shares_truncated(&d.shares)?;
    #[allow(clippy::cast_sign_loss)]
    let height = ctx.header.height as u64;
    store::set_starting_info(
        ctx,
        val,
        del,
        &StartingInfo {
            previous_period: previous,
            stake,
            height,
        },
    )
}

fn rewards_between(
    ctx: &mut Context<'_>,
    val: &[u8],
    start: u64,
    end: u64,
    stake: &LegacyDec,
) -> Result<DecCoins> {
    if start > end {
        return Err(sdk::error::go_panic(
            "startingPeriod cannot be greater than endingPeriod",
        ));
    }
    if stake.is_negative() {
        return Err(sdk::error::go_panic("stake should not be negative"));
    }
    let starting = store::get_historical(ctx, val, start)?;
    let ending = store::get_historical(ctx, val, end)?;
    let diff = ending
        .cumulative_reward_ratio
        .safe_sub(&starting.cumulative_reward_ratio)?
        .0;
    if diff.is_any_negative() {
        return Err(sdk::error::go_panic(
            "negative rewards should not be possible",
        ));
    }
    diff.mul_dec_truncate(stake)
}

/// `CalculateDelegationRewards`.
pub fn calculate_delegation_rewards(
    ctx: &mut Context<'_>,
    v: &Validator,
    d: &Delegation,
    ending_period: u64,
) -> Result<DecCoins> {
    let del = x_auth::parse_bech32(&d.delegator_address)?;
    let val = x_staking::addr::val_from_string(&d.validator_address)?;
    let info = store::get_starting_info(ctx, &val, &del)?;
    #[allow(clippy::cast_sign_loss)]
    let height = ctx.header.height as u64;
    let mut rewards = DecCoins::new();
    if info.height == height {
        return Ok(rewards);
    }
    let mut start_period = info.previous_period;
    let mut stake = info.stake;
    if height > info.height {
        for (_, period, fraction) in store::slash_events_between(ctx, &val, info.height, height)? {
            if period > start_period {
                let r = rewards_between(ctx, &val, start_period, period, &stake)?;
                rewards = rewards.add(&r)?;
                stake =
                    m(stake.checked_mul_truncate(&m(LegacyDec::one().checked_sub(&fraction))?))?;
                start_period = period;
            }
        }
    }
    let current_stake = v.tokens_from_shares(&d.shares)?;
    if stake > current_stake {
        let margin = m(LegacyDec::smallest().checked_mul_int64(3))?;
        if stake <= m(current_stake.checked_add(&margin))? {
            stake = current_stake;
        } else {
            return Err(sdk::error::go_panic(format!(
                "calculated final stake for delegator {} greater than current stake",
                d.delegator_address
            )));
        }
    }
    let r = rewards_between(ctx, &val, start_period, ending_period, &stake)?;
    rewards.add(&r)
}

fn base_denom_zero() -> Coins {
    Coins(vec![Coin::with_amount(
        x_staking::DEFAULT_BOND_DENOM,
        Int::zero(),
    )])
}

/// `withdrawDelegationRewards`.
pub fn withdraw_delegation_rewards(
    ctx: &mut Context<'_>,
    v: &Validator,
    d: &Delegation,
    withdraw_now: bool,
) -> Result<Coins> {
    let del = x_auth::parse_bech32(&d.delegator_address)?;
    let val = x_staking::addr::val_from_string(&d.validator_address)?;
    if !store::has_starting_info(ctx, &val, &del)? {
        return Err(errors::empty_delegation_dist_info());
    }
    let ending = increment_validator_period(ctx, v)?;
    let raw = calculate_delegation_rewards(ctx, v, d, ending)?;
    let outstanding = store::get_outstanding(ctx, &val)?;
    let rewards = raw.intersect(&outstanding);
    let (mut final_rewards, remainder) = rewards.truncate_decimal()?;

    if !final_rewards.is_empty() {
        let user = store::get_user_outstanding(ctx, &del, &val)?.unwrap_or_default();
        if withdraw_now {
            let withdraw_addr = store::get_withdraw_addr(ctx, &del)?;
            final_rewards = final_rewards.add(&user)?;
            let acc = x_auth::get_account(ctx, &del)?;
            if is_vesting(acc.as_ref()) {
                return Err(errs::not_supported(
                    "vesting account reward schedule update",
                ));
            }
            x_bank::send_from_module_to_account(ctx, MODULE_NAME, &withdraw_addr, &final_rewards)?;
            store::remove_user_outstanding(ctx, &del, &val)?;
        } else {
            let total = user.add(&final_rewards)?;
            store::set_user_outstanding(ctx, &del, &val, &total)?;
            final_rewards = Coins::new();
        }
    }

    let left = outstanding.sub(&rewards)?;
    store::set_outstanding(ctx, &val, &left)?;
    let pool = store::get_fee_pool(ctx)?.add(&remainder)?;
    store::set_fee_pool(ctx, &pool)?;
    let info = store::get_starting_info(ctx, &val, &del)?;
    decrement_reference_count(ctx, &val, info.previous_period)?;
    store::delete_starting_info(ctx, &val, &del)?;

    if final_rewards.is_empty() {
        final_rewards = base_denom_zero();
    }
    ctx.events.emit(event(
        "withdraw_rewards",
        [
            ("amount", final_rewards.to_string()),
            ("validator", v.operator_address.clone()),
            ("delegator", d.delegator_address.clone()),
        ],
    ));
    Ok(final_rewards)
}

/// `WithdrawDelegationRewards`.
pub fn withdraw_delegation_rewards_for(
    ctx: &mut Context<'_>,
    del: &[u8],
    val: &[u8],
) -> Result<Coins> {
    let v = staking_validator(ctx, val)?;
    let Some(d) = x_staking::store::get_delegation(ctx, del, val)? else {
        let Some(rewards) = store::get_user_outstanding(ctx, del, val)? else {
            return Err(errors::empty_delegation_dist_info());
        };
        let withdraw_addr = store::get_withdraw_addr(ctx, del)?;
        let acc = x_auth::get_account(ctx, del)?;
        if is_vesting(acc.as_ref()) {
            return Err(errs::not_supported(
                "vesting account reward schedule update",
            ));
        }
        x_bank::send_from_module_to_account(ctx, MODULE_NAME, &withdraw_addr, &rewards)?;
        store::remove_user_outstanding(ctx, del, val)?;
        ctx.events.emit(event(
            "withdraw_rewards",
            [
                ("amount", rewards.to_string()),
                ("validator", x_staking::addr::val_to_string(val)?),
                ("delegator", x_auth::bech32(del)?),
            ],
        ));
        return Ok(rewards);
    };
    let rewards = withdraw_delegation_rewards(ctx, &v, &d, true)?;
    initialize_delegation(ctx, val, del)?;
    Ok(rewards)
}

/// `WithdrawValidatorCommission`.
pub fn withdraw_validator_commission(ctx: &mut Context<'_>, val: &[u8]) -> Result<Coins> {
    let acc = store::get_commission(ctx, val)?;
    if acc.is_zero() {
        return Err(errors::no_validator_commission());
    }
    let (commission, remainder) = acc.truncate_decimal()?;
    store::set_commission(ctx, val, &remainder)?;
    let outstanding = store::get_outstanding(ctx, val)?;
    let left = outstanding.sub(&DecCoins::from_coins(&commission)?)?;
    store::set_outstanding(ctx, val, &left)?;
    if !commission.is_empty() {
        let withdraw_addr = store::get_withdraw_addr(ctx, val)?;
        x_bank::send_from_module_to_account(ctx, MODULE_NAME, &withdraw_addr, &commission)?;
    }
    ctx.events.emit(event(
        "withdraw_commission",
        [("amount", commission.to_string())],
    ));
    Ok(commission)
}

/// `FundCommunityPool`.
pub fn fund_community_pool(ctx: &mut Context<'_>, amount: &Coins, sender: &[u8]) -> Result<()> {
    x_bank::send_from_account_to_module(ctx, sender, MODULE_NAME, amount)?;
    let pool = store::get_fee_pool(ctx)?.add(&DecCoins::from_coins(amount)?)?;
    store::set_fee_pool(ctx, &pool)
}

/// `DistributeFromFeePool`.
pub fn distribute_from_fee_pool(ctx: &mut Context<'_>, amount: &Coins, to: &[u8]) -> Result<()> {
    let pool = store::get_fee_pool(ctx)?;
    let (new_pool, negative) = pool.safe_sub(&DecCoins::from_coins(amount)?)?;
    if negative {
        return Err(errors::bad_distribution());
    }
    x_bank::send_from_module_to_account(ctx, MODULE_NAME, to, amount)?;
    store::set_fee_pool(ctx, &new_pool)
}

/// `SetWithdrawAddr`.
pub fn set_withdraw_addr(ctx: &mut Context<'_>, del: &[u8], to: &[u8]) -> Result<()> {
    if x_bank::is_blocked(to) {
        return Err(errs::unauthorized(format!(
            "{} is not allowed to receive external funds",
            x_auth::bech32(to)?
        )));
    }
    if !store::get_params(ctx)?.withdraw_addr_enabled {
        return Err(errors::set_withdraw_addr_disabled());
    }
    ctx.events.emit(event(
        "set_withdraw_address",
        [("withdraw_address", x_auth::bech32(to)?)],
    ));
    store::set_withdraw_addr(ctx, del, to)
}

/// `AllocateTokensToValidator`.
pub fn allocate_tokens_to_validator(
    ctx: &mut Context<'_>,
    v: &Validator,
    tokens: &DecCoins,
) -> Result<()> {
    let commission = tokens.mul_dec(&v.commission.rate)?;
    let shared = tokens.sub(&commission)?;
    let val = v.operator()?;
    ctx.events.emit(event(
        "commission",
        [
            ("amount", commission.to_string()),
            ("validator", v.operator_address.clone()),
        ],
    ));
    let cur_commission = store::get_commission(ctx, &val)?.add(&commission)?;
    store::set_commission(ctx, &val, &cur_commission)?;
    let mut cur = store::get_current(ctx, &val)?;
    cur.rewards = cur.rewards.add(&shared)?;
    store::set_current(ctx, &val, &cur)?;
    ctx.events.emit(event(
        "rewards",
        [
            ("amount", tokens.to_string()),
            ("validator", v.operator_address.clone()),
        ],
    ));
    let outstanding = store::get_outstanding(ctx, &val)?.add(tokens)?;
    store::set_outstanding(ctx, &val, &outstanding)
}

/// `AllocateTokens`.
pub fn allocate_tokens(ctx: &mut Context<'_>, total_power: i64, votes: &[VoteInfo]) -> Result<()> {
    let fee_collector = x_auth::get_module_account(ctx, x_auth::FEE_COLLECTOR)?
        .ok_or_else(|| sdk::error::go_panic("fee collector not set"))?;
    let fc_addr = x_auth::Account::Module(fee_collector).address()?;
    let fees_int = x_bank::get_all_balances(ctx, &fc_addr)?;
    let fees = DecCoins::from_coins(&fees_int)?;
    x_bank::send_from_module_to_module(ctx, x_auth::FEE_COLLECTOR, MODULE_NAME, &fees_int)?;
    let mut pool = store::get_fee_pool(ctx)?;
    if total_power == 0 {
        pool = pool.add(&fees)?;
        return store::set_fee_pool(ctx, &pool);
    }
    let mut remaining = fees.clone();
    let tax = store::get_params(ctx)?.community_tax;
    let vote_mult = m(LegacyDec::one().checked_sub(&tax))?;
    let fee_mult = fees.mul_dec_truncate(&vote_mult)?;
    for vote in votes {
        let addr = vote
            .validator
            .as_ref()
            .map(|v| v.address.clone())
            .unwrap_or_default();
        let power = vote.validator.as_ref().map_or(0, |v| v.power);
        let v = x_staking::store::get_validator_by_cons_addr(ctx, &addr)?
            .ok_or_else(x_staking::errors::no_validator_found)?;
        let fraction = m(LegacyDec::new(power).checked_quo_truncate(&LegacyDec::new(total_power)))?;
        let reward = fee_mult.mul_dec_truncate(&fraction)?;
        allocate_tokens_to_validator(ctx, &v, &reward)?;
        remaining = remaining.sub(&reward)?;
    }
    pool = pool.add(&remaining)?;
    store::set_fee_pool(ctx, &pool)
}

/// `BeginBlocker`.
pub fn begin_block(ctx: &mut Context<'_>) -> Result<()> {
    let votes = ctx.vote_infos.clone();
    let total: i64 = votes
        .iter()
        .map(|v| v.validator.as_ref().map_or(0, |v| v.power))
        .sum();
    if ctx.header.height > 1 {
        allocate_tokens(ctx, total, &votes)?;
    }
    let proposer = ctx.header.proposer_address.clone();
    store::set_previous_proposer(ctx, &proposer)
}

/// Distribution staking hooks.
#[derive(Debug, Clone, Copy, Default)]
pub struct Hooks;

impl x_staking::StakingHooks for Hooks {
    fn after_validator_created(&self, ctx: &mut Context<'_>, val: &[u8]) -> Result<()> {
        staking_validator(ctx, val)?;
        initialize_validator(ctx, val)
    }

    fn after_validator_removed(
        &self,
        ctx: &mut Context<'_>,
        _cons: &[u8],
        val: &[u8],
    ) -> Result<()> {
        let mut outstanding = store::get_outstanding(ctx, val)?;
        let commission = store::get_commission(ctx, val)?;
        if !commission.is_zero() {
            outstanding = outstanding.sub(&commission)?;
            let (coins, remainder) = commission.truncate_decimal()?;
            let pool = store::get_fee_pool(ctx)?.add(&remainder)?;
            store::set_fee_pool(ctx, &pool)?;
            if !coins.is_empty() {
                let withdraw_addr = store::get_withdraw_addr(ctx, val)?;
                x_bank::send_from_module_to_account(ctx, MODULE_NAME, &withdraw_addr, &coins)?;
            }
        }
        let pool = store::get_fee_pool(ctx)?.add(&outstanding)?;
        store::set_fee_pool(ctx, &pool)?;
        ctx.delete(crate::STORE_KEY, &store::outstanding_key(val))?;
        ctx.delete(crate::STORE_KEY, &store::commission_key(val))?;
        store::delete_prefix(ctx, &store::slash_event_prefix(val))?;
        store::delete_prefix(ctx, &store::historical_prefix(val))?;
        ctx.delete(crate::STORE_KEY, &store::current_key(val))
    }

    fn before_delegation_created(
        &self,
        ctx: &mut Context<'_>,
        _del: &[u8],
        val: &[u8],
    ) -> Result<()> {
        let v = staking_validator(ctx, val)?;
        increment_validator_period(ctx, &v).map(|_| ())
    }

    fn before_delegation_shares_modified(
        &self,
        ctx: &mut Context<'_>,
        del: &[u8],
        val: &[u8],
    ) -> Result<()> {
        let v = staking_validator(ctx, val)?;
        let d = staking_delegation(ctx, del, val)?;
        withdraw_delegation_rewards(ctx, &v, &d, false).map(|_| ())
    }

    fn after_delegation_modified(
        &self,
        ctx: &mut Context<'_>,
        del: &[u8],
        val: &[u8],
    ) -> Result<()> {
        initialize_delegation(ctx, val, del)
    }

    fn before_validator_slashed(
        &self,
        ctx: &mut Context<'_>,
        val: &[u8],
        fraction: &LegacyDec,
    ) -> Result<()> {
        update_validator_slash_fraction(ctx, val, fraction)
    }
}
