//! `x/auth/vesting` message server (celestiaorg/cosmos-sdk v0.52.12).
//!
//! Only `MsgCreateVestingAccount` is functional; the permanent-locked and
//! periodic variants are rejected by celestia's fork.

use celestia_proto::cosmos::auth::v1beta1::BaseAccount;
use celestia_proto::cosmos::vesting::v1beta1 as pb;
use sdk::error::sdk as errs;
use sdk::{Context, Result};
use x_auth::Account;

use crate::msgs::is_send_enabled_coins;
use crate::{is_blocked, send_coins};

/// `MsgCreateVestingAccount`.
pub fn handle_create_vesting_account(
    ctx: &mut Context<'_>,
    msg: &pb::MsgCreateVestingAccount,
) -> Result<pb::MsgCreateVestingAccountResponse> {
    let from = sdk::address::acc_from_bech32(&msg.from_address)
        .map_err(|e| errs::invalid_address(format!("invalid 'from' address: {}", e.log)))?;
    let to = sdk::address::acc_from_bech32(&msg.to_address)
        .map_err(|e| errs::invalid_address(format!("invalid 'to' address: {}", e.log)))?;
    // `validateAmount`: `IsValid` / `IsAllPositive` of the raw coins (a nil
    // amount `Validate` reaches panics).
    let amount = crate::msgs::valid_raw_coins(&msg.amount)?;
    if msg.start_time <= 0 {
        return Err(errs::invalid_request("invalid start time"));
    }
    if msg.end_time <= 0 {
        return Err(errs::invalid_request("invalid end time"));
    }
    if msg.end_time <= msg.start_time {
        return Err(errs::invalid_request(
            "invalid start and end time (must be start < end)",
        ));
    }
    is_send_enabled_coins(ctx, &amount)?;
    if is_blocked(&to) {
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
        Account::ContinuousVesting(pb::ContinuousVestingAccount {
            base_vesting_account: Some(bva),
            start_time: msg.start_time,
        })
    };
    x_auth::set_account(ctx, &acc)?;
    send_coins(ctx, &from, &to, &amount)?;
    Ok(pb::MsgCreateVestingAccountResponse {})
}

/// `MsgCreatePermanentLockedAccount`: not supported by celestia.
pub fn handle_create_permanent_locked_account(
    _ctx: &mut Context<'_>,
    _msg: &pb::MsgCreatePermanentLockedAccount,
) -> Result<pb::MsgCreatePermanentLockedAccountResponse> {
    Err(errs::invalid_request(
        "Permanent locked accounts are not supported",
    ))
}

/// `MsgCreatePeriodicVestingAccount`: not supported by celestia.
pub fn handle_create_periodic_vesting_account(
    _ctx: &mut Context<'_>,
    _msg: &pb::MsgCreatePeriodicVestingAccount,
) -> Result<pb::MsgCreatePeriodicVestingAccountResponse> {
    Err(errs::invalid_request(
        "Periodic vesting accounts are not supported",
    ))
}
