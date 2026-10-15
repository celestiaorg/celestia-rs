//! Msg server (`keeper/msg_server.go`).

use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;
use celestia_proto::cosmos::distribution::v1beta1 as pb;
use sdk::coins::Coins;
use sdk::dec_coins::DecCoins;
use sdk::error::sdk as errs;
use sdk::{Context, Result};

use crate::keeper;
use crate::store::{self, Params};

fn invalid_addr(what: &str, e: &sdk::SdkError) -> sdk::SdkError {
    errs::invalid_address(format!("invalid {what} address: {}", e.log))
}

fn acc(s: &str) -> Result<Vec<u8>> {
    x_staking::addr::acc_from_string(s)
}

/// `validateAmount`.
fn validate_amount(amount: &[PbCoin]) -> Result<Coins> {
    if amount.is_empty() {
        return Err(errs::invalid_coins("amount cannot be nil"));
    }
    let coins = Coins::from_proto(amount).map_err(|_| {
        errs::invalid_coins(
            amount
                .iter()
                .map(|c| format!("{}{}", c.amount, c.denom))
                .collect::<Vec<_>>()
                .join(","),
        )
    })?;
    if !coins.is_valid() {
        return Err(errs::invalid_coins(coins.to_string()));
    }
    Ok(coins)
}

fn validate_authority(authority: &str) -> Result<()> {
    acc(authority)
        .map_err(|e| errs::invalid_address(format!("invalid authority address: {}", e.log)))?;
    let expected = x_staking::msgs::gov_authority()?;
    if expected != authority {
        return Err(x_staking::errors::gov_invalid_signer().wrap(format!(
            "invalid authority; expected {expected}, got {authority}"
        )));
    }
    Ok(())
}

/// `MsgSetWithdrawAddress`.
pub fn handle_set_withdraw_address(
    ctx: &mut Context<'_>,
    msg: &pb::MsgSetWithdrawAddress,
) -> Result<pb::MsgSetWithdrawAddressResponse> {
    let del = acc(&msg.delegator_address).map_err(|e| invalid_addr("delegator", &e))?;
    let account = x_auth::get_account(ctx, &del)?;
    if matches!(
        account,
        Some(
            x_auth::Account::ContinuousVesting(_)
                | x_auth::Account::DelayedVesting(_)
                | x_auth::Account::PeriodicVesting(_)
                | x_auth::Account::PermanentLocked(_)
        )
    ) {
        return Err(errs::invalid_request(format!(
            // Go formats the address bytes (`[]byte`) with `%s`.
            "cannot set withdraw address for vesting account {}",
            gostr::from_go_bytes(del.as_ref())
        )));
    }
    let to = acc(&msg.withdraw_address).map_err(|e| invalid_addr("withdraw", &e))?;
    keeper::set_withdraw_addr(ctx, &del, &to)?;
    Ok(pb::MsgSetWithdrawAddressResponse {})
}

/// `MsgWithdrawDelegatorReward`.
pub fn handle_withdraw_delegator_reward(
    ctx: &mut Context<'_>,
    msg: &pb::MsgWithdrawDelegatorReward,
) -> Result<pb::MsgWithdrawDelegatorRewardResponse> {
    let val = x_staking::addr::val_from_string(&msg.validator_address)
        .map_err(|e| invalid_addr("validator", &e))?;
    let del = acc(&msg.delegator_address).map_err(|e| invalid_addr("delegator", &e))?;
    let amount = keeper::withdraw_delegation_rewards_for(ctx, &del, &val)?;
    Ok(pb::MsgWithdrawDelegatorRewardResponse {
        amount: amount.to_proto(),
    })
}

/// `MsgWithdrawValidatorCommission`.
pub fn handle_withdraw_validator_commission(
    ctx: &mut Context<'_>,
    msg: &pb::MsgWithdrawValidatorCommission,
) -> Result<pb::MsgWithdrawValidatorCommissionResponse> {
    let val = x_staking::addr::val_from_string(&msg.validator_address)
        .map_err(|e| invalid_addr("validator", &e))?;
    let amount = keeper::withdraw_validator_commission(ctx, &val)?;
    Ok(pb::MsgWithdrawValidatorCommissionResponse {
        amount: amount.to_proto(),
    })
}

/// `MsgFundCommunityPool`.
pub fn handle_fund_community_pool(
    ctx: &mut Context<'_>,
    msg: &pb::MsgFundCommunityPool,
) -> Result<pb::MsgFundCommunityPoolResponse> {
    let depositor = acc(&msg.depositor).map_err(|e| invalid_addr("depositor", &e))?;
    let amount = validate_amount(&msg.amount)?;
    keeper::fund_community_pool(ctx, &amount, &depositor)?;
    Ok(pb::MsgFundCommunityPoolResponse {})
}

/// `MsgUpdateParams`.
pub fn handle_update_params(
    ctx: &mut Context<'_>,
    msg: &pb::MsgUpdateParams,
) -> Result<pb::MsgUpdateParamsResponse> {
    validate_authority(&msg.authority)?;
    let p = Params::from_wire(&msg.params.clone().unwrap_or_default())?;
    let nonzero = |d: &Option<math::LegacyDec>| d.as_ref().is_some_and(|d| !d.is_zero());
    if nonzero(&p.base_proposer_reward) || nonzero(&p.bonus_proposer_reward) {
        return Err(errs::invalid_request(
            "cannot update base or bonus proposer reward because these are deprecated fields",
        ));
    }
    if p.community_tax.is_negative() {
        return Err(sdk::error::internal(format!(
            "community tax must be positive: {}",
            p.community_tax
        )));
    }
    if p.community_tax > math::LegacyDec::one() {
        return Err(sdk::error::internal(format!(
            "community tax too large: {}",
            p.community_tax
        )));
    }
    store::set_params(ctx, &p)?;
    Ok(pb::MsgUpdateParamsResponse {})
}

/// `MsgCommunityPoolSpend`.
pub fn handle_community_pool_spend(
    ctx: &mut Context<'_>,
    msg: &pb::MsgCommunityPoolSpend,
) -> Result<pb::MsgCommunityPoolSpendResponse> {
    validate_authority(&msg.authority)?;
    let amount = validate_amount(&msg.amount)?;
    let to = acc(&msg.recipient)?;
    if x_bank::is_blocked(&to) {
        return Err(errs::unauthorized(format!(
            "{} is not allowed to receive external funds",
            msg.recipient
        )));
    }
    keeper::distribute_from_fee_pool(ctx, &amount, &to)?;
    Ok(pb::MsgCommunityPoolSpendResponse {})
}

/// `MsgDepositValidatorRewardsPool`.
pub fn handle_deposit_validator_rewards_pool(
    ctx: &mut Context<'_>,
    msg: &pb::MsgDepositValidatorRewardsPool,
) -> Result<pb::MsgDepositValidatorRewardsPoolResponse> {
    let depositor = acc(&msg.depositor)?;
    let amount = Coins::from_proto(&msg.amount)?;
    x_bank::send_from_account_to_module(ctx, &depositor, crate::MODULE_NAME, &amount)?;
    let val = x_staking::addr::val_from_string(&msg.validator_address)?;
    let v = x_staking::store::validator(ctx, &val)?;
    let reward = DecCoins::from_coins(&amount)?;
    keeper::allocate_tokens_to_validator(ctx, &v, &reward)?;
    if !v.tokens.is_zero() {
        let cur = store::get_current(ctx, &val)?;
        let hist = store::get_historical(ctx, &val, cur.period.wrapping_sub(1))?;
        if !hist.cumulative_reward_ratio.is_zero() {
            hist.cumulative_reward_ratio
                .add(&cur.rewards)
                .map_err(|e| {
                    sdk::error::internal(format!(
                        "unable to deposit coins: deposit is too large: {}",
                        e.log
                    ))
                })?;
        }
    }
    Ok(pb::MsgDepositValidatorRewardsPoolResponse {})
}
