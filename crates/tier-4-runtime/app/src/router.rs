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
    (
        $( $ty:ty => $handler:path ),* $(,)?
        ; wire: $( $wty:ty => $whandler:path ),* $(,)?
    ) => {
        /// Whether a handler is registered for `type_url`.
        #[must_use]
        pub fn has_handler(type_url: &str) -> bool {
            $( if type_url == <$ty>::type_url() { return true; } )*
            $( if type_url == <$wty>::type_url() { return true; } )*
            crate::hyperlane::has_handler(type_url) || crate::ibc::has_handler(type_url)
        }

        /// Executes `msg`, returning its packed response.
        pub fn route(ctx: &mut Context<'_>, msg: &Any) -> Result<Any> {
            $(
                if msg.type_url == <$ty>::type_url() {
                    let m: $ty = decode(msg)?;
                    // baseapp's registered handler: ValidateBasic, then the
                    // circuit breaker (x/circuit), then the msg server.
                    validate_basic(msg)?;
                    if !x_circuit::is_allowed(ctx, &msg.type_url)? {
                        return Err(sdk::address::undefined(format!(
                            "circuit breaker disables execution of this message: {}",
                            msg.type_url
                        )));
                    }
                    let resp = $handler(ctx, &m)?;
                    return Ok(Any::pack(&resp));
                }
            )*
            // Handlers that also read the message's wire form (gogoproto
            // state prost does not keep, e.g. nil customtype pointers).
            $(
                if msg.type_url == <$wty>::type_url() {
                    let m: $wty = decode(msg)?;
                    validate_basic(msg)?;
                    if !x_circuit::is_allowed(ctx, &msg.type_url)? {
                        return Err(sdk::address::undefined(format!(
                            "circuit breaker disables execution of this message: {}",
                            msg.type_url
                        )));
                    }
                    let resp = $whandler(ctx, &m, &msg.value)?;
                    return Ok(Any::pack(&resp));
                }
            )*
            if crate::hyperlane::has_handler(&msg.type_url) {
                // Same baseapp wrapper: hyperlane msgs have no ValidateBasic,
                // so the circuit breaker check comes first.
                if !x_circuit::is_allowed(ctx, &msg.type_url)? {
                    return Err(sdk::address::undefined(format!(
                        "circuit breaker disables execution of this message: {}",
                        msg.type_url
                    )));
                }
            }
            if let Some(r) = crate::hyperlane::route(ctx, msg) {
                return r;
            }
            if let Some(r) = crate::ibc::route(ctx, msg) {
                return r;
            }
            Err(errs::unknown_request(format!("no message handler found for {}", msg.type_url)))
        }
    };
}

use celestia_proto::celestia::blob::v1 as blob;
use celestia_proto::celestia::fibre::v1 as fibre;
use celestia_proto::celestia::forwarding::v1 as forwarding;
use celestia_proto::celestia::minfee::v1 as minfee;
use celestia_proto::celestia::signal::v1 as signal;
use celestia_proto::celestia::valaddr::v1 as valaddr;
use celestia_proto::celestia::zkism::v1 as zkism;
use celestia_proto::cosmos::authz::v1beta1 as authz;
use celestia_proto::cosmos::bank::v1beta1 as bank;
use celestia_proto::cosmos::circuit::v1 as circuit;
use celestia_proto::cosmos::consensus::v1 as consensus;
use celestia_proto::cosmos::distribution::v1beta1 as distr;
use celestia_proto::cosmos::evidence::v1beta1 as evidence;
use celestia_proto::cosmos::feegrant::v1beta1 as feegrant;
use celestia_proto::cosmos::gov::v1 as gov;
use celestia_proto::cosmos::gov::v1beta1 as govb;
use celestia_proto::cosmos::slashing::v1beta1 as slashing;
use celestia_proto::cosmos::staking::v1beta1 as staking;
use celestia_proto::cosmos::upgrade::v1beta1 as upgrade;
use celestia_proto::cosmos::vesting::v1beta1 as vesting;

routes! {
    bank::MsgSend => x_bank::msgs::handle_send,
    bank::MsgMultiSend => x_bank::msgs::handle_multi_send,
    bank::MsgUpdateParams => x_bank::msgs::handle_update_params,
    bank::MsgSetSendEnabled => x_bank::msgs::handle_set_send_enabled,
    staking::MsgCreateValidator => x_staking::handle_create_validator,
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
    blob::MsgPayForBlobs => x_blob::pay_for_blobs,
    blob::MsgUpdateBlobParams => x_blob::handle_update_blob_params,
    minfee::MsgUpdateMinfeeParams => x_minfee::handle_update_minfee_params,
    circuit::MsgAuthorizeCircuitBreaker => x_circuit::handle_authorize_circuit_breaker,
    circuit::MsgTripCircuitBreaker => x_circuit::handle_trip_circuit_breaker,
    circuit::MsgResetCircuitBreaker => x_circuit::handle_reset_circuit_breaker,
    signal::MsgSignalVersion => crate::modules::handle_signal_version,
    signal::MsgTryUpgrade => crate::modules::handle_try_upgrade,
    gov::MsgSubmitProposal => handle_submit_proposal,
    gov::MsgExecLegacyContent => x_gov::msgs::exec_legacy_content,
    gov::MsgVote => x_gov::msgs::vote,
    gov::MsgVoteWeighted => x_gov::msgs::vote_weighted,
    gov::MsgDeposit => x_gov::msgs::deposit,
    gov::MsgUpdateParams => x_gov::msgs::update_params,
    gov::MsgCancelProposal => x_gov::msgs::cancel_proposal,
    govb::MsgSubmitProposal => handle_legacy_submit_proposal,
    govb::MsgVote => x_gov::msgs::legacy_vote,
    govb::MsgVoteWeighted => x_gov::msgs::legacy_vote_weighted,
    govb::MsgDeposit => x_gov::msgs::legacy_deposit,
    upgrade::MsgSoftwareUpgrade => x_upgrade::handle_software_upgrade,
    upgrade::MsgCancelUpgrade => x_upgrade::handle_cancel_upgrade,
    authz::MsgGrant => handle_authz_grant,
    authz::MsgExec => handle_authz_exec,
    authz::MsgRevoke => x_authz::handle_revoke,
    authz::MsgPruneExpiredGrants => x_authz::handle_prune_expired_grants,
    feegrant::MsgGrantAllowance => x_feegrant::handle_grant_allowance,
    feegrant::MsgRevokeAllowance => x_feegrant::handle_revoke_allowance,
    feegrant::MsgPruneAllowances => x_feegrant::handle_prune_allowances,
    vesting::MsgCreateVestingAccount => x_bank::vesting_msgs::handle_create_vesting_account,
    vesting::MsgCreatePermanentLockedAccount => x_bank::vesting_msgs::handle_create_permanent_locked_account,
    vesting::MsgCreatePeriodicVestingAccount => x_bank::vesting_msgs::handle_create_periodic_vesting_account,
    evidence::MsgSubmitEvidence => x_evidence::handle_submit_evidence,
    consensus::MsgUpdateParams => x_consensus::handle_update_params,
    fibre::MsgDepositToEscrow => x_fibre::msgs::deposit_to_escrow,
    fibre::MsgRequestWithdrawal => x_fibre::msgs::request_withdrawal,
    fibre::MsgPayForFibre => x_fibre::msgs::pay_for_fibre,
    fibre::MsgPaymentPromiseTimeout => x_fibre::msgs::payment_promise_timeout,
    fibre::MsgUpdateFibreParams => x_fibre::msgs::update_fibre_params,
    valaddr::MsgSetFibreProviderInfo => handle_set_fibre_provider_info,
    zkism::MsgCreateInterchainSecurityModule => crate::hyperlane::zkism_create,
    zkism::MsgUpdateInterchainSecurityModule => x_zkism::msgs::update_ism,
    zkism::MsgSubmitMessages => x_zkism::msgs::submit_messages,
    forwarding::MsgForward => crate::hyperlane::forwarding_forward;
    wire:
    staking::MsgEditValidator => x_staking::handle_edit_validator_wire,
}

/// The app's message router as gov sees it (`baseapp.MessageRouter`).
#[derive(Debug, Clone, Copy, Default)]
pub struct AppRouter;

impl sdk::router::MsgRouter for AppRouter {
    fn has_handler(&self, type_url: &str) -> bool {
        has_handler(type_url)
    }

    fn validate_basic(&self, msg: &Any) -> Result<()> {
        validate_basic(msg)
    }

    fn handle(&self, ctx: &mut Context<'_>, msg: &Any) -> Result<Any> {
        route(ctx, msg)
    }

    fn msg_text(&self, msg: &Any) -> String {
        x_ante::gogotext::compact_text(&msg.type_url, &msg.value, sdk::version::app_version())
            .unwrap_or_else(|| msg.type_url.clone())
    }
}

fn handle_submit_proposal(
    ctx: &mut Context<'_>,
    msg: &gov::MsgSubmitProposal,
) -> Result<gov::MsgSubmitProposalResponse> {
    x_gov::msgs::submit_proposal(ctx, &AppRouter, msg)
}

fn handle_set_fibre_provider_info(
    ctx: &mut Context<'_>,
    msg: &valaddr::MsgSetFibreProviderInfo,
) -> Result<valaddr::MsgSetFibreProviderInfoResponse> {
    x_valaddr::set_fibre_provider_info_msg(ctx, &mut crate::modules::ValaddrStaking, msg)
}

fn handle_authz_grant(
    ctx: &mut Context<'_>,
    msg: &authz::MsgGrant,
) -> Result<authz::MsgGrantResponse> {
    x_authz::handle_grant(ctx, &AppRouter, msg)
}

fn handle_authz_exec(
    ctx: &mut Context<'_>,
    msg: &authz::MsgExec,
) -> Result<authz::MsgExecResponse> {
    x_authz::handle_exec(ctx, &AppRouter, msg)
}

fn handle_legacy_submit_proposal(
    ctx: &mut Context<'_>,
    msg: &govb::MsgSubmitProposal,
) -> Result<govb::MsgSubmitProposalResponse> {
    x_gov::msgs::legacy_submit_proposal(ctx, &AppRouter, msg)
}

/// Legacy `ValidateBasic` for messages that still implement it in v0.52.12.
pub fn validate_basic(msg: &Any) -> Result<()> {
    if let Some(r) = crate::ibc::validate_basic(msg) {
        return r;
    }
    if msg.type_url == blob::MsgPayForBlobs::type_url() {
        return x_blob::validate_basic(&decode(msg)?);
    }
    if msg.type_url == signal::MsgSignalVersion::type_url() {
        return x_signal::validate_basic_signal_version(&decode(msg)?);
    }
    if msg.type_url == signal::MsgTryUpgrade::type_url() {
        return x_signal::validate_basic_try_upgrade(&decode(msg)?);
    }
    if msg.type_url == fibre::MsgDepositToEscrow::type_url() {
        let m: fibre::MsgDepositToEscrow = decode(msg)?;
        return x_fibre::msgs::validate_basic_escrow(&m.signer, m.amount.as_ref());
    }
    if msg.type_url == fibre::MsgRequestWithdrawal::type_url() {
        let m: fibre::MsgRequestWithdrawal = decode(msg)?;
        return x_fibre::msgs::validate_basic_escrow(&m.signer, m.amount.as_ref());
    }
    if msg.type_url == fibre::MsgPayForFibre::type_url() {
        return x_fibre::msgs::validate_basic_pay_for_fibre(&decode(msg)?);
    }
    if msg.type_url == fibre::MsgPaymentPromiseTimeout::type_url() {
        return x_fibre::msgs::validate_basic_timeout(&decode(msg)?);
    }
    if msg.type_url == fibre::MsgUpdateFibreParams::type_url() {
        return x_fibre::msgs::validate_basic_update_params(&decode(msg)?);
    }
    if msg.type_url == zkism::MsgCreateInterchainSecurityModule::type_url() {
        return x_zkism::msgs::validate_basic_create(&decode(msg)?);
    }
    if msg.type_url == zkism::MsgUpdateInterchainSecurityModule::type_url() {
        return x_zkism::msgs::validate_basic_update(&decode(msg)?);
    }
    if msg.type_url == zkism::MsgSubmitMessages::type_url() {
        return x_zkism::msgs::validate_basic_submit(&decode(msg)?);
    }
    if msg.type_url == forwarding::MsgForward::type_url() {
        return x_forwarding::msgs::validate_basic(&decode(msg)?);
    }
    if msg.type_url == valaddr::MsgSetFibreProviderInfo::type_url() {
        return x_valaddr::validate_basic(&decode(msg)?);
    }
    if msg.type_url == gov::MsgExecLegacyContent::type_url() {
        return x_gov::msgs::validate_basic_exec_legacy_content(&decode(msg)?);
    }
    Ok(())
}
