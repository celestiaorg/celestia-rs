//! `SetUpContextDecorator` (cosmos-sdk `x/auth/ante/setup.go`).

use kvstore::GasMeter;
use sdk::Context;
use sdk::error::sdk as errs;

use crate::{R, txview};

/// `SetGasMeter`: simulation and the genesis block run unmetered.
pub fn set_gas_meter(ctx: &mut Context<'_>, simulate: bool, gas_limit: u64) {
    ctx.gas = if simulate || ctx.height() == 0 {
        GasMeter::infinite()
    } else {
        GasMeter::new(gas_limit)
    };
}

/// `SetUpContextDecorator.AnteHandle`: installs the tx gas meter and
/// enforces the block max gas. Out-of-gas panics of later decorators are
/// recovered here in Go; in Rust they are ordinary `ErrOutOfGas` errors
/// carried with the tx meter (see [`crate::ante_handle`]).
pub fn set_up_context(ctx: &mut Context<'_>, tx: &sdk::tx::Tx, simulate: bool) -> R<()> {
    // `gasTx.GetGas()` dereferences `AuthInfo.Fee` (nil → panic).
    let gas = txview::gas(tx)?;
    set_gas_meter(ctx, simulate, gas);
    if let Some(block) = &ctx.consensus_params.block
        && block.max_gas > 0
        && gas > block.max_gas.unsigned_abs()
    {
        return Err(errs::invalid_gas_limit(format!(
            "tx gas limit {gas} exceeds block max gas {}",
            block.max_gas
        ))
        .into());
    }
    Ok(())
}
