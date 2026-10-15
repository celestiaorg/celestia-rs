//! The concrete decoder, ante handler and staking hooks plugged into
//! [`crate::tx`]. Temporary stand-ins are marked and fail loudly.

use celestia_proto::celestia_core_proto::abci::ValidatorUpdate;
use sdk::tx::Tx;
use sdk::{Context, Result};

/// Decodes block tx bytes: celestia-app's `TxConfig.TxDecoder()`.
pub fn decode_tx(bytes: &[u8]) -> Result<Tx> {
    txdecode::decode(bytes)
}

/// celestia-app v9.0.8's decoder (app version 9, ADR 0004).
pub fn decode_tx_v9(bytes: &[u8]) -> Result<Tx> {
    txdecode::decode_v9(bytes)
}

/// The decoder of `app_version`.
#[must_use]
pub fn decoder(app_version: u64) -> crate::tx::DecodeFn {
    if crate::stores::is_v9(app_version) {
        decode_tx_v9
    } else {
        decode_tx
    }
}

/// The ante handler (`x_ante`) over store-backed keeper dependencies.
/// The node min gas price only matters in CheckTx and is unset here.
pub fn ante(
    ctx: &mut Context<'_>,
    tx: &Tx,
    simulate: bool,
) -> core::result::Result<u64, (sdk::SdkError, u64)> {
    let deps = x_ante::StoreDeps::new(None);
    x_ante::ante_handle(ctx, tx, simulate, &deps)
        .map(|o| o.gas_wanted)
        .map_err(|e| (e.error, e.gas_wanted))
}

/// `stakingKeeper.ApplyAndReturnValidatorSetUpdates`.
pub fn apply_validator_set(ctx: &mut Context<'_>) -> Result<Vec<ValidatorUpdate>> {
    x_staking::apply_and_return_validator_set_updates(ctx)
}
