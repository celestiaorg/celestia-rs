//! `x/bank` message server.

use celestia_proto::cosmos::bank::v1beta1::{
    Input, MsgMultiSend, MsgMultiSendResponse, MsgSend, MsgSendResponse, Output,
};
use sdk::coins::Coins;
use sdk::error::sdk as errs;
use sdk::event::event;
use sdk::{Context, Result, SdkError};

use crate::{STORE_KEY, add_coins, get_params, is_blocked, send_coins, sub_unlocked_coins};

/// `x/bank` codespace errors.
pub mod errors {
    use sdk::SdkError;
    /// `ErrNoInputs`.
    #[must_use]
    pub fn no_inputs() -> SdkError {
        SdkError::new("bank", 2, "no inputs to send transaction")
    }
    /// `ErrNoOutputs`.
    #[must_use]
    pub fn no_outputs() -> SdkError {
        SdkError::new("bank", 3, "no outputs to send transaction")
    }
    /// `ErrInputOutputMismatch`.
    #[must_use]
    pub fn input_output_mismatch() -> SdkError {
        SdkError::new("bank", 4, "sum inputs != sum outputs")
    }
    /// `ErrSendDisabled`.
    #[must_use]
    pub fn send_disabled(denom: &str) -> SdkError {
        SdkError::new(
            "bank",
            5,
            format!(
                "{denom} transfers are currently disabled: send transfers are currently disabled"
            ),
        )
    }
    /// `ErrMultipleSenders`.
    #[must_use]
    pub fn multiple_senders() -> SdkError {
        SdkError::new("bank", 9, "multiple senders not allowed")
    }
}

fn parse_addr(s: &str, what: &str) -> Result<Vec<u8>> {
    x_auth::parse_bech32(s).map_err(|e| e.wrap(format!("invalid {what} address")))
}

fn all_positive(c: &Coins) -> bool {
    !c.is_empty() && c.iter().all(|c| c.amount.is_positive())
}

/// `IsSendEnabledCoins`.
pub fn is_send_enabled_coins(ctx: &mut Context<'_>, coins: &Coins) -> Result<()> {
    if coins.is_empty() {
        return Ok(());
    }
    let default = get_params(ctx)?.default_send_enabled;
    for c in coins {
        let key = sdk::codec::key(&[&[0x04], c.denom.as_bytes()]);
        let enabled = if ctx.has(STORE_KEY, &key)? {
            // BoolValue proto: empty = false.
            ctx.get(STORE_KEY, &key)?
                .is_some_and(|v| v.last() == Some(&1))
        } else {
            default
        };
        if !enabled {
            return Err(errors::send_disabled(&c.denom));
        }
    }
    Ok(())
}

/// `MsgSend`.
pub fn handle_send(ctx: &mut Context<'_>, msg: &MsgSend) -> Result<MsgSendResponse> {
    let from = parse_addr(&msg.from_address, "from")?;
    let to = parse_addr(&msg.to_address, "to")?;
    let amount = Coins::from_proto(&msg.amount)?;
    if !amount.is_valid() || !all_positive(&amount) {
        return Err(errs::invalid_coins(amount.to_string()));
    }
    is_send_enabled_coins(ctx, &amount)?;
    if is_blocked(&to) {
        return Err(errs::unauthorized(format!(
            "{} is not allowed to receive funds",
            msg.to_address
        )));
    }
    send_coins(ctx, &from, &to, &amount)?;
    Ok(MsgSendResponse {})
}

fn validate_input(i: &Input) -> Result<Coins> {
    x_auth::parse_bech32(&i.address)?;
    let c = Coins::from_proto(&i.coins)?;
    if !c.is_valid() || !all_positive(&c) {
        return Err(errs::invalid_coins(c.to_string()));
    }
    Ok(c)
}

fn validate_output(o: &Output) -> Result<Coins> {
    x_auth::parse_bech32(&o.address)?;
    let c = Coins::from_proto(&o.coins)?;
    if !c.is_valid() || !all_positive(&c) {
        return Err(errs::invalid_coins(c.to_string()));
    }
    Ok(c)
}

/// `types.ValidateInputOutputs`.
pub fn validate_input_outputs(input: &Input, outputs: &[Output]) -> Result<()> {
    let total_in = validate_input(input)?;
    let mut total_out = Coins::new();
    for o in outputs {
        total_out = total_out.add(&validate_output(o)?)?;
    }
    if total_in != total_out {
        return Err(errors::input_output_mismatch());
    }
    Ok(())
}

/// `InputOutputCoins`.
pub fn input_output_coins(ctx: &mut Context<'_>, input: &Input, outputs: &[Output]) -> Result<()> {
    validate_input_outputs(input, outputs)?;
    let in_addr = x_auth::parse_bech32(&input.address)?;
    sub_unlocked_coins(ctx, &in_addr, &Coins::from_proto(&input.coins)?)?;
    ctx.events
        .emit(event("message", [("sender", input.address.clone())]));
    for out in outputs {
        let out_addr = x_auth::parse_bech32(&out.address)?;
        let coins = Coins::from_proto(&out.coins)?;
        add_coins(ctx, &out_addr, &coins)?;
        ctx.events.emit(event(
            "transfer",
            [
                ("recipient", x_auth::bech32(&out_addr)?),
                ("sender", input.address.clone()),
                ("amount", coins.to_string()),
            ],
        ));
        if !x_auth::has_account(ctx, &out_addr)? {
            let acc = x_auth::new_account_with_address(ctx, &out_addr)?;
            x_auth::set_account(ctx, &acc)?;
        }
    }
    Ok(())
}

/// `MsgMultiSend`.
pub fn handle_multi_send(
    ctx: &mut Context<'_>,
    msg: &MsgMultiSend,
) -> Result<MsgMultiSendResponse> {
    if msg.inputs.is_empty() {
        return Err(errors::no_inputs());
    }
    if msg.inputs.len() != 1 {
        return Err(errors::multiple_senders());
    }
    if msg.outputs.is_empty() {
        return Err(errors::no_outputs());
    }
    validate_input_outputs(&msg.inputs[0], &msg.outputs)?;
    for i in &msg.inputs {
        is_send_enabled_coins(ctx, &Coins::from_proto(&i.coins)?)?;
    }
    for o in &msg.outputs {
        let a = x_auth::parse_bech32(&o.address)?;
        if is_blocked(&a) {
            return Err(errs::unauthorized(format!(
                "{} is not allowed to receive funds",
                o.address
            )));
        }
    }
    input_output_coins(ctx, &msg.inputs[0], &msg.outputs)?;
    Ok(MsgMultiSendResponse {})
}

/// Keeps `SdkError` in the public signature set used by the router.
pub type HandlerResult<T> = core::result::Result<T, SdkError>;
