//! cosmos-sdk v0.46 `Msg.GetSigners` of every message in celestia-app
//! v3.13.0's v1 interface registry, for the callers that run it on a
//! message its `ValidateBasic` may not have checked: authz
//! `DispatchActions`, the interchain accounts host's `authenticateTx` and
//! gov `SubmitProposal`. Most modules drop an address error (an empty
//! signer); the ibc-go core messages, celestia's signal and blobstream
//! messages and `MsgCreatePeriodicVestingAccount` panic.

use celestia_proto::Any;
use celestia_proto::cosmos::vesting::v1beta1 as vesting;
use celestia_proto::ibc::core::{channel::v1 as ch, client::v1 as cl, connection::v1 as conn};
use prost::{Message, Name};
use sdk::Result;

fn decode<M: Message + Default>(msg: &Any) -> Result<M> {
    M::decode(msg.value.as_slice()).map_err(|e| sdk::error::sdk::tx_decode(e.to_string()))
}

/// `sdk.AccAddressFromBech32` whose error the module drops.
fn acc(s: &str) -> Vec<u8> {
    sdk::address::acc_from_bech32_go(s).unwrap_or_default()
}

/// `sdk.AccAddressFromBech32` whose error the module panics with.
fn must_acc(s: &str) -> Result<Vec<u8>> {
    sdk::address::acc_from_bech32_go(s).map_err(sdk::error::go_panic)
}

/// `sdk.ValAddressFromBech32` whose error the module panics with.
fn must_val(s: &str) -> Result<Vec<u8>> {
    x_staking::addr::val_from_string(s).map_err(|e| sdk::error::go_panic(e.log))
}

/// The `signer` field of an ibc-go v6 core message (all of them panic on
/// an address that does not parse).
fn ibc_signer(m: &Any) -> Result<Option<String>> {
    let u = m.type_url.as_str();
    macro_rules! signer_of {
        ($($t:ty),* $(,)?) => {
            $(if u == <$t>::type_url() {
                return Ok(Some(decode::<$t>(m)?.signer));
            })*
        };
    }
    signer_of!(
        cl::MsgCreateClient,
        cl::MsgUpdateClient,
        cl::MsgUpgradeClient,
        cl::MsgSubmitMisbehaviour,
        conn::MsgConnectionOpenInit,
        conn::MsgConnectionOpenTry,
        conn::MsgConnectionOpenAck,
        conn::MsgConnectionOpenConfirm,
        ch::MsgChannelOpenInit,
        ch::MsgChannelOpenTry,
        ch::MsgChannelOpenAck,
        ch::MsgChannelOpenConfirm,
        ch::MsgChannelCloseInit,
        ch::MsgChannelCloseConfirm,
        ch::MsgRecvPacket,
        ch::MsgTimeout,
        ch::MsgTimeoutOnClose,
        ch::MsgAcknowledgement,
    );
    Ok(None)
}

/// `msg.GetSigners()`; `None` for a type outside the v1 registry.
#[allow(deprecated)] // `MsgCreateValidator.delegator_address` (v0.46 reads it).
pub(super) fn signers(m: &Any) -> Result<Option<Vec<Vec<u8>>>> {
    use celestia_proto::celestia::signal::v1 as signal;
    use celestia_proto::cosmos::authz::v1beta1 as authz;
    use celestia_proto::cosmos::bank::v1beta1 as bank;
    use celestia_proto::cosmos::crisis::v1beta1 as crisis;
    use celestia_proto::cosmos::evidence::v1beta1 as evidence;
    use celestia_proto::cosmos::gov;
    use celestia_proto::cosmos::slashing::v1beta1 as slashing;
    use celestia_proto::cosmos::staking::v1beta1 as staking;
    if let Some(s) = super::authz::exec_signer(m)? {
        return Ok(Some(vec![s]));
    }
    if let Some(s) = ibc_signer(m)? {
        return Ok(Some(vec![must_acc(&s)?]));
    }
    let val = |s: &str| x_staking::addr::val_from_string(s).unwrap_or_default();
    let u = m.type_url.as_str();
    Ok(Some(if u == bank::MsgMultiSend::type_url() {
        let msg: bank::MsgMultiSend = decode(m)?;
        msg.inputs.iter().map(|i| acc(&i.address)).collect()
    } else if u == slashing::MsgUnjail::type_url() {
        vec![val(&decode::<slashing::MsgUnjail>(m)?.validator_addr)]
    } else if u == staking::MsgCreateValidator::type_url() {
        let msg: staking::MsgCreateValidator = decode(m)?;
        let (del, v) = (acc(&msg.delegator_address), val(&msg.validator_address));
        if del == v { vec![del] } else { vec![del, v] }
    } else if u == staking::MsgEditValidator::type_url() {
        vec![val(
            &decode::<staking::MsgEditValidator>(m)?.validator_address
        )]
    } else if u == gov::v1beta1::MsgDeposit::type_url() {
        vec![acc(&decode::<gov::v1beta1::MsgDeposit>(m)?.depositor)]
    } else if u == gov::v1::MsgDeposit::type_url() {
        vec![acc(&decode::<gov::v1::MsgDeposit>(m)?.depositor)]
    } else if u == gov::v1beta1::MsgVoteWeighted::type_url() {
        vec![acc(&decode::<gov::v1beta1::MsgVoteWeighted>(m)?.voter)]
    } else if u == gov::v1::MsgVoteWeighted::type_url() {
        vec![acc(&decode::<gov::v1::MsgVoteWeighted>(m)?.voter)]
    } else if u == gov::v1::MsgExecLegacyContent::type_url() {
        vec![acc(&decode::<gov::v1::MsgExecLegacyContent>(m)?.authority)]
    } else if u == authz::MsgExec::type_url() {
        vec![acc(&decode::<authz::MsgExec>(m)?.grantee)]
    } else if u == vesting::MsgCreateVestingAccount::type_url() {
        vec![acc(
            &decode::<vesting::MsgCreateVestingAccount>(m)?.from_address
        )]
    } else if u == vesting::MsgCreatePermanentLockedAccount::type_url() {
        vec![acc(
            &decode::<vesting::MsgCreatePermanentLockedAccount>(m)?.from_address
        )]
    } else if u == vesting::MsgCreatePeriodicVestingAccount::type_url() {
        vec![must_acc(
            &decode::<vesting::MsgCreatePeriodicVestingAccount>(m)?.from_address,
        )?]
    } else if u == crisis::MsgVerifyInvariant::type_url() {
        vec![acc(&decode::<crisis::MsgVerifyInvariant>(m)?.sender)]
    } else if u == evidence::MsgSubmitEvidence::type_url() {
        vec![acc(&decode::<evidence::MsgSubmitEvidence>(m)?.submitter)]
    } else if u == signal::MsgSignalVersion::type_url() {
        vec![must_val(
            &decode::<signal::MsgSignalVersion>(m)?.validator_address,
        )?]
    } else if u == signal::MsgTryUpgrade::type_url() {
        vec![must_acc(&decode::<signal::MsgTryUpgrade>(m)?.signer)?]
    } else if u
        == "/ibc.applications.interchain_accounts.controller.v1.MsgRegisterInterchainAccount"
    {
        let msg: celestia_proto::ibc::applications::interchain_accounts::controller::v1::MsgRegisterInterchainAccount =
            decode(m)?;
        vec![must_acc(&msg.owner)?]
    } else if u == "/ibc.applications.interchain_accounts.controller.v1.MsgSendTx" {
        let msg: celestia_proto::ibc::applications::interchain_accounts::controller::v1::MsgSendTx =
            decode(m)?;
        vec![must_acc(&msg.owner)?]
    } else if u == super::qgb::REGISTER_EVM_ADDRESS {
        vec![must_val(
            &decode::<x_qgb::types::MsgRegisterEvmAddress>(m)?.validator_address,
        )?]
    } else {
        return Ok(None);
    }))
}
