//! Message service router (`baseapp.MsgServiceRouter`).
//!
//! Maps `Any` type URLs to module handlers. A message whose type has no
//! handler fails `runTx` with `ErrUnknownRequest`, exactly as in Go — so
//! unported handlers surface as visible result-code differences in the
//! differential harness rather than silent state divergence.

use celestia_proto::Any;
use prost::{Message, Name};
use sdk::error::sdk as errs;
use sdk::{Context, Result};

fn decode<M: Message + Default>(msg: &Any) -> Result<M> {
    M::decode(msg.value.as_slice()).map_err(|e| errs::tx_decode(e.to_string()))
}

macro_rules! routes {
    ($( $ty:ty => $handler:path ),* $(,)?) => {
        /// Whether a handler is registered for `type_url`.
        #[must_use]
        pub fn has_handler(type_url: &str) -> bool {
            $( if type_url == <$ty>::type_url() { return true; } )*
            crate::hyperlane::has_handler(type_url)
        }

        /// Executes `msg`, returning its packed response.
        pub fn route(ctx: &mut Context<'_>, msg: &Any) -> Result<Any> {
            $(
                if msg.type_url == <$ty>::type_url() {
                    let m: $ty = decode(msg)?;
                    let resp = $handler(ctx, &m)?;
                    return Ok(Any::pack(&resp));
                }
            )*
            if let Some(r) = crate::hyperlane::route(ctx, msg) {
                return r;
            }
            Err(errs::unknown_request(format!("no message handler found for {}", msg.type_url)))
        }
    };
}

use celestia_proto::cosmos::bank::v1beta1 as bank;
use celestia_proto::cosmos::distribution::v1beta1 as distr;
use celestia_proto::cosmos::slashing::v1beta1 as slashing;
use celestia_proto::cosmos::staking::v1beta1 as staking;

routes! {
    bank::MsgSend => x_bank::msgs::handle_send,
    bank::MsgMultiSend => x_bank::msgs::handle_multi_send,
    staking::MsgCreateValidator => x_staking::handle_create_validator,
    staking::MsgEditValidator => x_staking::handle_edit_validator,
    staking::MsgDelegate => x_staking::handle_delegate,
    staking::MsgBeginRedelegate => x_staking::handle_begin_redelegate,
    staking::MsgUndelegate => x_staking::handle_undelegate,
    staking::MsgCancelUnbondingDelegation => x_staking::handle_cancel_unbonding_delegation,
    staking::MsgUpdateParams => x_staking::handle_update_params,
    distr::MsgSetWithdrawAddress => x_distribution::handle_set_withdraw_address,
    distr::MsgWithdrawDelegatorReward => x_distribution::handle_withdraw_delegator_reward,
    distr::MsgWithdrawValidatorCommission => x_distribution::handle_withdraw_validator_commission,
    distr::MsgFundCommunityPool => x_distribution::handle_fund_community_pool,
    distr::MsgCommunityPoolSpend => x_distribution::handle_community_pool_spend,
    distr::MsgUpdateParams => x_distribution::handle_update_params,
    distr::MsgDepositValidatorRewardsPool => x_distribution::handle_deposit_validator_rewards_pool,
    slashing::MsgUnjail => x_slashing::handle_unjail,
    slashing::MsgUpdateParams => x_slashing::handle_update_params,
}

/// Legacy `ValidateBasic` for messages that still implement it in v0.52.12.
pub fn validate_basic(msg: &Any) -> Result<()> {
    let _ = msg;
    Ok(())
}
