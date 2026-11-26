//! cosmos-sdk v0.46 `x/auth/vesting` `MsgCreateVestingAccount` of
//! celestia-app v1 (`types/msgs.go` `ValidateBasic`, `msg_server.go`
//! `CreateVestingAccount`). The periodic and permanent-locked messages
//! are not ported.

use celestia_proto::Any;
use celestia_proto::cosmos::auth::v1beta1::BaseAccount;
use celestia_proto::cosmos::vesting::v1beta1 as pb;
use prost::{Message, Name};
use sdk::coins::Coins;
use sdk::error::sdk as errs;
use sdk::event::event;
use sdk::{Context, Result};
use x_auth::Account;

use super::msgs;

fn decode(msg: &Any) -> Result<pb::MsgCreateVestingAccount> {
    pb::MsgCreateVestingAccount::decode(msg.value.as_slice())
        .map_err(|e| errs::tx_decode(e.to_string()))
}

/// `MsgCreateVestingAccount.ValidateBasic`: returns the amount.
fn validate(m: &pb::MsgCreateVestingAccount) -> Result<Coins> {
    msgs::acc_address(&m.from_address, "'from'")?;
    msgs::acc_address(&m.to_address, "'to'")?;
    let amount = msgs::valid_positive_coins(&m.amount)?;
    if m.end_time <= 0 {
        return Err(errs::invalid_request("invalid end time"));
    }
    Ok(amount)
}

/// `msgServer.CreateVestingAccount` (v0.46): a start time of 0 is the
/// block time; the account takes the next account number before the
/// coins move.
fn create_vesting_account(
    ctx: &mut Context<'_>,
    msg: &pb::MsgCreateVestingAccount,
) -> Result<pb::MsgCreateVestingAccountResponse> {
    let amount = validate(msg)?;
    msgs::is_send_enabled_coins(ctx, &amount)?;
    let from =
        sdk::address::acc_from_bech32_go(&msg.from_address).map_err(errs::invalid_address)?;
    let to = sdk::address::acc_from_bech32_go(&msg.to_address).map_err(errs::invalid_address)?;
    if msg.end_time <= msg.start_time {
        return Err(errs::invalid_request(
            "invalid start and end time (must be start < end)",
        ));
    }
    if x_bank::is_blocked(&to) {
        return Err(errs::unauthorized(format!(
            "{} is not allowed to receive funds",
            msg.to_address
        )));
    }
    if x_auth::get_account(ctx, &to)?.is_some() {
        return Err(errs::invalid_request(format!(
            "account {} already exists",
            msg.to_address
        )));
    }
    let number = x_auth::next_account_number(ctx)?;
    let bva = pb::BaseVestingAccount {
        base_account: Some(BaseAccount {
            address: x_auth::bech32(&to)?,
            pub_key: None,
            account_number: number,
            sequence: 0,
        }),
        // `msg.Amount.Sort()`: valid coins are already sorted.
        original_vesting: amount.to_proto(),
        delegated_free: Vec::new(),
        delegated_vesting: Vec::new(),
        end_time: msg.end_time,
    };
    let acc = if msg.delayed {
        Account::DelayedVesting(pb::DelayedVestingAccount {
            base_vesting_account: Some(bva),
        })
    } else {
        let start_time = if msg.start_time == 0 {
            ctx.header.time.seconds
        } else {
            msg.start_time
        };
        Account::ContinuousVesting(pb::ContinuousVestingAccount {
            base_vesting_account: Some(bva),
            start_time,
        })
    };
    x_auth::set_account(ctx, &acc)?;
    x_bank::send_coins(ctx, &from, &to, &amount)?;
    ctx.events.emit(event("message", [("module", "vesting")]));
    Ok(pb::MsgCreateVestingAccountResponse {})
}

/// `ValidateBasic`; `None` for any other message.
#[must_use]
pub fn validate_basic(msg: &Any) -> Option<Result<()>> {
    (msg.type_url == pb::MsgCreateVestingAccount::type_url())
        .then(|| decode(msg).and_then(|m| validate(&m).map(|_| ())))
}

/// Executes `MsgCreateVestingAccount`; `None` for any other message.
pub fn route(ctx: &mut Context<'_>, msg: &Any) -> Option<Result<Any>> {
    (msg.type_url == pb::MsgCreateVestingAccount::type_url()).then(|| {
        let m = decode(msg)?;
        create_vesting_account(ctx, &m).map(|r| Any::pack(&r))
    })
}
