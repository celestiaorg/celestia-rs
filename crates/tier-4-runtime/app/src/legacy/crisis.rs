//! cosmos-sdk v0.46 `x/crisis` `MsgVerifyInvariant` of celestia-app v1 to
//! v3 (`types/msgs.go` `ValidateBasic`, `keeper/msg_server.go`
//! `VerifyInvariant`), up to the invariant route lookup: the constant fee
//! moves to the fee collector, then an unknown route fails. Running a
//! registered invariant (whose reads cost the transaction's gas) is not
//! ported.

use celestia_proto::Any;
use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;
use celestia_proto::cosmos::crisis::v1beta1::{MsgVerifyInvariant, MsgVerifyInvariantResponse};
use prost::{Message, Name};
use sdk::coins::Coins;
use sdk::error::sdk as errs;
use sdk::{Context, Result, SdkError};

use super::msgs;

/// The invariant routes the module manager registers (bank,
/// distribution, gov and staking `RegisterInvariants`).
const ROUTES: &[&str] = &[
    "bank/nonnegative-outstanding",
    "bank/total-supply",
    "distribution/nonnegative-outstanding",
    "distribution/can-withdraw",
    "distribution/reference-count",
    "distribution/module-account",
    "gov/module-account",
    "staking/module-accounts",
    "staking/nonnegative-power",
    "staking/positive-delegation",
    "staking/delegator-shares",
];

fn decode(msg: &Any) -> Result<MsgVerifyInvariant> {
    MsgVerifyInvariant::decode(msg.value.as_slice()).map_err(|e| errs::tx_decode(e.to_string()))
}

/// `MsgVerifyInvariant.ValidateBasic`.
fn validate(m: &MsgVerifyInvariant) -> Result<()> {
    msgs::acc_address(&m.sender, "sender").map(|_| ())
}

/// `Keeper.VerifyInvariant` (v0.46); every route it can run fails.
fn verify_invariant(
    ctx: &mut Context<'_>,
    m: &MsgVerifyInvariant,
) -> Result<MsgVerifyInvariantResponse> {
    let (denom, amount) = x_params::v046::get_coin(ctx, "crisis", "ConstantFee")?;
    // `sdk.NewCoins` drops a zero coin.
    let fee = Coins::from_proto(&[PbCoin { denom, amount }])?;
    let sender = sdk::address::acc_from_bech32_go(&m.sender).map_err(sdk::error::internal)?;
    x_bank::send_from_account_to_module(ctx, &sender, x_auth::FEE_COLLECTOR, &fee)?;
    let route = format!("{}/{}", m.invariant_module_name, m.invariant_route);
    if ROUTES.contains(&route.as_str()) {
        return Err(super::not_ported(errs::unknown_request(format!(
            "the cosmos-sdk v0.46 crisis invariant {route}"
        ))));
    }
    // `ErrUnknownInvariant` (crisis code 3).
    Err(SdkError::new("crisis", 3, "unknown invariant"))
}

/// `ValidateBasic`; `None` for any other message.
#[must_use]
pub fn validate_basic(msg: &Any) -> Option<Result<()>> {
    (msg.type_url == MsgVerifyInvariant::type_url()).then(|| decode(msg).and_then(|m| validate(&m)))
}

/// Executes `MsgVerifyInvariant`; `None` for any other message.
pub fn route(ctx: &mut Context<'_>, msg: &Any) -> Option<Result<Any>> {
    (msg.type_url == MsgVerifyInvariant::type_url()).then(|| {
        let m = decode(msg)?;
        verify_invariant(ctx, &m).map(|r| Any::pack(&r))
    })
}
