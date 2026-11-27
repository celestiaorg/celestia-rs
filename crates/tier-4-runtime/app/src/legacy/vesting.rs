//! cosmos-sdk v0.46 `x/auth/vesting` of celestia-app v1 to v3:
//! `MsgCreateVestingAccount`, `MsgCreatePermanentLockedAccount` and
//! `MsgCreatePeriodicVestingAccount` (`types/msgs.go` `ValidateBasic`,
//! `msg_server.go`).

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

fn decode_as<M: Message + Default>(msg: &Any) -> Result<M> {
    M::decode(msg.value.as_slice()).map_err(|e| errs::tx_decode(e.to_string()))
}

/// `MsgCreatePermanentLockedAccount.ValidateBasic`: returns the amount.
fn validate_permanent(m: &pb::MsgCreatePermanentLockedAccount) -> Result<Coins> {
    msgs::acc_address(&m.from_address, "sender")?;
    msgs::acc_address(&m.to_address, "recipient")?;
    msgs::valid_positive_coins(&m.amount)
}

/// `MsgCreatePeriodicVestingAccount.ValidateBasic`. Its address and
/// time errors are plain Go errors (ABCI's internal error).
fn validate_periodic(m: &pb::MsgCreatePeriodicVestingAccount) -> Result<()> {
    sdk::address::acc_from_bech32_go(&m.from_address).map_err(sdk::error::internal)?;
    sdk::address::acc_from_bech32_go(&m.to_address).map_err(sdk::error::internal)?;
    if m.start_time < 1 {
        return Err(sdk::error::internal(format!(
            "invalid start time of {}, length must be greater than 0",
            m.start_time
        )));
    }
    for (i, p) in m.vesting_periods.iter().enumerate() {
        msgs::valid_positive_coins(&p.amount)?;
        if p.length < 1 {
            return Err(sdk::error::internal(format!(
                "invalid period length of {} in period {i}, length must be greater than 0",
                p.length
            )));
        }
    }
    Ok(())
}

/// `ak.NewAccount(NewBaseAccountWithAddress(to))`: the next account
/// number.
fn new_base_account(ctx: &mut Context<'_>, to: &[u8]) -> Result<BaseAccount> {
    Ok(BaseAccount {
        address: x_auth::bech32(to)?,
        pub_key: None,
        account_number: x_auth::next_account_number(ctx)?,
        sequence: 0,
    })
}

/// `msgServer.CreatePermanentLockedAccount` (v0.46).
fn create_permanent_locked_account(
    ctx: &mut Context<'_>,
    msg: &pb::MsgCreatePermanentLockedAccount,
) -> Result<pb::MsgCreatePermanentLockedAccountResponse> {
    let amount = validate_permanent(msg)?;
    msgs::is_send_enabled_coins(ctx, &amount)?;
    let from = sdk::address::acc_from_bech32_go(&msg.from_address).map_err(sdk::error::internal)?;
    let to = sdk::address::acc_from_bech32_go(&msg.to_address).map_err(sdk::error::internal)?;
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
    let base = new_base_account(ctx, &to)?;
    let acc = Account::PermanentLocked(pb::PermanentLockedAccount {
        base_vesting_account: Some(pb::BaseVestingAccount {
            base_account: Some(base),
            original_vesting: amount.to_proto(),
            delegated_free: Vec::new(),
            delegated_vesting: Vec::new(),
            end_time: 0,
        }),
    });
    x_auth::set_account(ctx, &acc)?;
    x_bank::send_coins(ctx, &from, &to, &amount)?;
    ctx.events.emit(event("message", [("module", "vesting")]));
    Ok(pb::MsgCreatePermanentLockedAccountResponse {})
}

/// `msgServer.CreatePeriodicVestingAccount` (v0.46): no blocked-address
/// check, and the account lookup comes before the send-enabled check.
fn create_periodic_vesting_account(
    ctx: &mut Context<'_>,
    msg: &pb::MsgCreatePeriodicVestingAccount,
) -> Result<pb::MsgCreatePeriodicVestingAccountResponse> {
    validate_periodic(msg)?;
    let from = sdk::address::acc_from_bech32_go(&msg.from_address).map_err(sdk::error::internal)?;
    let to = sdk::address::acc_from_bech32_go(&msg.to_address).map_err(sdk::error::internal)?;
    if x_auth::get_account(ctx, &to)?.is_some() {
        return Err(errs::invalid_request(format!(
            "account {} already exists",
            msg.to_address
        )));
    }
    let mut total = Coins::default();
    for p in &msg.vesting_periods {
        total = total.add(&Coins::from_proto(&p.amount)?)?;
    }
    msgs::is_send_enabled_coins(ctx, &total)?;
    let base = new_base_account(ctx, &to)?;
    let end_time = msg
        .vesting_periods
        .iter()
        .fold(msg.start_time, |t, p| t.wrapping_add(p.length));
    let acc = Account::PeriodicVesting(pb::PeriodicVestingAccount {
        base_vesting_account: Some(pb::BaseVestingAccount {
            base_account: Some(base),
            original_vesting: total.to_proto(),
            delegated_free: Vec::new(),
            delegated_vesting: Vec::new(),
            end_time,
        }),
        start_time: msg.start_time,
        vesting_periods: msg.vesting_periods.clone(),
    });
    x_auth::set_account(ctx, &acc)?;
    x_bank::send_coins(ctx, &from, &to, &total)?;
    ctx.events.emit(event("message", [("module", "vesting")]));
    Ok(pb::MsgCreatePeriodicVestingAccountResponse {})
}

/// `ValidateBasic`; `None` for any other message.
#[must_use]
pub fn validate_basic(msg: &Any) -> Option<Result<()>> {
    let u = msg.type_url.as_str();
    if u == pb::MsgCreateVestingAccount::type_url() {
        Some(decode(msg).and_then(|m| validate(&m).map(|_| ())))
    } else if u == pb::MsgCreatePermanentLockedAccount::type_url() {
        Some(decode_as(msg).and_then(|m| validate_permanent(&m).map(|_| ())))
    } else if u == pb::MsgCreatePeriodicVestingAccount::type_url() {
        Some(decode_as(msg).and_then(|m| validate_periodic(&m)))
    } else {
        None
    }
}

/// Executes the vesting messages; `None` for any other message.
pub fn route(ctx: &mut Context<'_>, msg: &Any) -> Option<Result<Any>> {
    let u = msg.type_url.as_str();
    if u == pb::MsgCreateVestingAccount::type_url() {
        Some(decode(msg).and_then(|m| create_vesting_account(ctx, &m).map(|r| Any::pack(&r))))
    } else if u == pb::MsgCreatePermanentLockedAccount::type_url() {
        Some(
            decode_as(msg)
                .and_then(|m| create_permanent_locked_account(ctx, &m).map(|r| Any::pack(&r))),
        )
    } else if u == pb::MsgCreatePeriodicVestingAccount::type_url() {
        Some(
            decode_as(msg)
                .and_then(|m| create_periodic_vesting_account(ctx, &m).map(|r| Any::pack(&r))),
        )
    } else {
        None
    }
}
