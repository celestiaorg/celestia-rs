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
    // cosmos-sdk v0.52.8 (app version 9) adds without `recover`: the
    // overflow stays a panic (a halt in BeginBlock); v0.52.12's
    // `addDecCoins` recovers it into an error.
    let new_ratio = if sdk::version::is_v9(sdk::version::app_version()) {
        historical.cumulative_reward_ratio.add(&current)?
    } else {
        sdk::error::recover(|| historical.cumulative_reward_ratio.add(&current))
            .map_err(|e| sdk::error::internal(format!("reward ratio overflow: {}", e.log)))?
    };
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
        // Go's handler runs inside the iteration: its reads come before
        // the `Next` that charges the event's gas (an out-of-gas happens
        // at the same point).
        store::for_each_slash_event_between(
            ctx,
            &val,
            info.height,
            height,
            |ctx, _, period, fraction| {
                if period > start_period {
                    let r = rewards_between(ctx, &val, start_period, period, &stake)?;
                    rewards = rewards.add(&r)?;
                    stake =
                        m(stake
                            .checked_mul_truncate(&m(LegacyDec::one().checked_sub(&fraction))?))?;
                    start_period = period;
                }
                Ok(false)
            },
        )?;
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

/// The zero coin of the event of an empty withdrawal: the registered
/// base denom, else `sdk.DefaultBondDenom` (`stake` in cosmos-sdk v0.46,
/// whose celestia-app registers no base denom).
fn base_denom_zero(ctx: &Context<'_>) -> Coins {
    let denom = if ctx.is_sdk_v046() {
        "stake"
    } else {
        x_staking::DEFAULT_BOND_DENOM
    };
    Coins(vec![Coin::with_amount(denom, Int::zero())])
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

    if !final_rewards.is_empty() && ctx.is_sdk_v046() {
        // cosmos-sdk v0.46 (app versions 1 to 3): every withdrawal, also
        // the hook's, sends the rewards to the withdraw address at once.
        let withdraw_addr = store::get_withdraw_addr(ctx, &del)?;
        x_bank::send_from_module_to_account(ctx, MODULE_NAME, &withdraw_addr, &final_rewards)?;
    } else if !final_rewards.is_empty() {
        let user = store::get_user_outstanding(ctx, &del, &val)?.unwrap_or_default();
        if withdraw_now {
            let withdraw_addr = store::get_withdraw_addr(ctx, &del)?;
            final_rewards = final_rewards.add(&user)?;
            let acc = x_auth::get_account(ctx, &del)?;
            if let Some(mut acc) = acc.filter(|a| is_vesting(Some(a))) {
                // celestia: rewards of a vesting account go to the account
                // itself and extend its vesting schedule.
                let now = ctx.header.time;
                x_auth::vesting::update_schedule(&mut acc, &now, &final_rewards)?;
                x_auth::set_account(ctx, &acc)?;
                x_bank::send_from_module_to_account(ctx, MODULE_NAME, &del, &final_rewards)?;
            } else {
                x_bank::send_from_module_to_account(
                    ctx,
                    MODULE_NAME,
                    &withdraw_addr,
                    &final_rewards,
                )?;
            }
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

    // cosmos-sdk v0.46 returns the (empty) rewards and emits the zero coin.
    let returned = if ctx.is_sdk_v046() {
        Some(final_rewards.clone())
    } else {
        None
    };
    if final_rewards.is_empty() {
        final_rewards = base_denom_zero(ctx);
    }
    ctx.events.emit(event(
        "withdraw_rewards",
        [
            ("amount", final_rewards.to_string()),
            ("validator", v.operator_address.clone()),
            ("delegator", d.delegator_address.clone()),
        ],
    ));
    Ok(returned.unwrap_or(final_rewards))
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
        if let Some(mut acc) = acc.filter(|a| is_vesting(Some(a))) {
            let now = ctx.header.time;
            x_auth::vesting::update_schedule(&mut acc, &now, &rewards)?;
            x_auth::set_account(ctx, &acc)?;
            x_bank::send_from_module_to_account(ctx, MODULE_NAME, del, &rewards)?;
        } else {
            x_bank::send_from_module_to_account(ctx, MODULE_NAME, &withdraw_addr, &rewards)?;
        }
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

/// cosmos-sdk v0.46 `AllocateTokens`: the previous proposer first gets
/// `base + bonus * signed fraction` of the fees, then every voter its
/// power share of `1 - proposer share - community tax`, and the
/// remainder goes to the community pool.
fn allocate_tokens_v046(
    ctx: &mut Context<'_>,
    sum_precommit_power: i64,
    total_power: i64,
    proposer: &[u8],
    votes: &[VoteInfo],
) -> Result<()> {
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
    let fraction =
        m(LegacyDec::new(sum_precommit_power).checked_quo(&LegacyDec::new(total_power)))?;
    // Go reads each parameter where it uses it (`GetBaseProposerReward`,
    // `GetBonusProposerReward`, later `GetCommunityTax`): the reads cost
    // gas on the block context's meter in this order.
    let base = x_params::v046::get_dec(ctx, "distribution", "baseproposerreward")?;
    let bonus = x_params::v046::get_dec(ctx, "distribution", "bonusproposerreward")?;
    let proposer_mult = m(bonus
        .checked_mul_truncate(&fraction)
        .and_then(|b| base.checked_add(&b)))?;
    let proposer_reward = fees.mul_dec_truncate(&proposer_mult)?;
    let mut remaining = fees.clone();
    // A missing proposer is only logged in Go.
    if let Some(v) = x_staking::store::get_validator_by_cons_addr(ctx, proposer)? {
        ctx.events.emit(event(
            "proposer_reward",
            [
                ("amount", proposer_reward.to_string()),
                ("validator", v.operator_address.clone()),
            ],
        ));
        allocate_tokens_to_validator(ctx, &v, &proposer_reward)?;
        remaining = remaining.sub(&proposer_reward)?;
    }
    let community_tax = x_params::v046::get_dec(ctx, "distribution", "communitytax")?;
    let vote_mult = m(LegacyDec::one()
        .checked_sub(&proposer_mult)
        .and_then(|d| d.checked_sub(&community_tax)))?;
    let fee_mult = fees.mul_dec_truncate(&vote_mult)?;
    for vote in votes {
        let (addr, power) = vote
            .validator
            .as_ref()
            .map_or((Vec::new(), 0), |v| (v.address.clone(), v.power));
        // Go calls a method on the nil validator and panics.
        let v = x_staking::store::get_validator_by_cons_addr(ctx, &addr)?
            .ok_or_else(x_staking::errors::no_validator_found)?;
        let share = m(LegacyDec::new(power).checked_quo_truncate(&LegacyDec::new(total_power)))?;
        let reward = fee_mult.mul_dec_truncate(&share)?;
        allocate_tokens_to_validator(ctx, &v, &reward)?;
        remaining = remaining.sub(&reward)?;
    }
    pool = pool.add(&remaining)?;
    store::set_fee_pool(ctx, &pool)
}

/// cosmos-sdk v0.46 `BeginBlocker`: from height 2, allocate the collected
/// fees to the previous proposer and the voters of the last commit (a
/// vote counts as signed unless its flag is absent), then record this
/// block's proposer.
fn begin_block_v046(ctx: &mut Context<'_>) -> Result<()> {
    let votes = ctx.vote_infos.clone();
    let (mut total, mut signed) = (0i64, 0i64);
    for v in &votes {
        let power = v.validator.as_ref().map_or(0, |v| v.power);
        total = total.wrapping_add(power);
        if v.block_id_flag != BLOCK_ID_FLAG_ABSENT {
            signed = signed.wrapping_add(power);
        }
    }
    if ctx.header.height > 1 {
        let previous = store::get_previous_proposer(ctx)?;
        allocate_tokens_v046(ctx, signed, total, &previous, &votes)?;
    }
    let proposer = ctx.header.proposer_address.clone();
    store::set_previous_proposer(ctx, &proposer)
}

/// `BlockIDFlagAbsent`.
const BLOCK_ID_FLAG_ABSENT: i32 = 1;

/// `BeginBlocker`.
pub fn begin_block(ctx: &mut Context<'_>) -> Result<()> {
    if ctx.is_sdk_v046() {
        return begin_block_v046(ctx);
    }
    let votes = ctx.vote_infos.clone();
    let total: i64 = votes
        .iter()
        .map(|v| v.validator.as_ref().map_or(0, |v| v.power))
        .sum();
    // SDK v0.53 leaves carried fees for the next block when there is no
    // previous commit, including when starting from an exported height.
    let allocate_without_votes = !sdk::version::is_v12(sdk::version::app_version());
    if ctx.header.height > 1 && (allocate_without_votes || !votes.is_empty()) {
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
