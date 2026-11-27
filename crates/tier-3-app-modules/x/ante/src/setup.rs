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

/// `SetUpContextDecorator.AnteHandle`: installs the tx gas meter and, from
/// cosmos-sdk v0.50, enforces the block max gas. Out-of-gas panics of later decorators are
/// recovered here in Go; in Rust they are ordinary `ErrOutOfGas` errors
/// carried with the tx meter (see [`crate::ante_handle`]).
pub fn set_up_context(ctx: &mut Context<'_>, tx: &sdk::tx::Tx, simulate: bool) -> R<()> {
    // `gasTx.GetGas()` dereferences `AuthInfo.Fee` (nil → panic).
    let gas = txview::gas(tx)?;
    set_gas_meter(ctx, simulate, gas);
    // cosmos-sdk v0.46 (app versions 1 to 3) has no block max gas check.
    if ctx.is_sdk_v046() {
        return Ok(());
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use celestia_proto::celestia_core_proto::tendermint::types::{
        BlockParams, ConsensusParams, VersionParams,
    };
    use celestia_proto::cosmos::tx::v1beta1::Fee;
    use sdk::{ExecMode, Header, MultiStore};

    /// The block max gas check is cosmos-sdk v0.50's: v0.46 (app versions 1
    /// to 3) has none.
    #[test]
    fn block_max_gas_applies_only_from_sdk_v050() {
        let mut tx = sdk::tx::Tx::default();
        tx.auth_info.fee = Some(Fee {
            gas_limit: 10,
            ..Fee::default()
        });
        for (app, v046, ok) in [(1, true, true), (3, true, true), (4, false, false)] {
            let _scope = v046.then(|| sdk::version::SdkV046Scope::enter(app));
            let mut ms = MultiStore::new();
            let mut ctx = Context::new(
                &mut ms,
                Header {
                    height: 2,
                    ..Header::default()
                },
                ExecMode::Finalize,
            );
            ctx.consensus_params = ConsensusParams {
                block: Some(BlockParams {
                    max_gas: 5,
                    ..BlockParams::default()
                }),
                version: Some(VersionParams { app }),
                ..ConsensusParams::default()
            };
            assert_eq!(
                set_up_context(&mut ctx, &tx, false).is_ok(),
                ok,
                "app {app}"
            );
        }
    }
}
