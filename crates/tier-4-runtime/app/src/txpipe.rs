//! The concrete decoder, ante handler and staking hooks plugged into
//! [`crate::tx`]. Temporary stand-ins are marked and fail loudly.

use celestia_proto::celestia_core_proto::abci::ValidatorUpdate;
use celestia_proto::cosmos::tx::v1beta1::{AuthInfo, TxBody, TxRaw};
use prost::Message;
use sdk::error::sdk as errs;
use sdk::tx::Tx;
use sdk::{Context, Result};

/// Decodes block tx bytes. Stand-in until the `txdecode` crate lands:
/// plain prost decoding without unknown-field rejection or blob wrappers.
pub fn decode_tx(bytes: &[u8]) -> Result<Tx> {
    let raw = TxRaw::decode(bytes).map_err(|e| errs::tx_decode(e.to_string()))?;
    let body =
        TxBody::decode(raw.body_bytes.as_slice()).map_err(|e| errs::tx_decode(e.to_string()))?;
    let auth_info = AuthInfo::decode(raw.auth_info_bytes.as_slice())
        .map_err(|e| errs::tx_decode(e.to_string()))?;
    Ok(Tx {
        tx_bytes: bytes.to_vec(),
        body_bytes: raw.body_bytes,
        auth_info_bytes: raw.auth_info_bytes,
        signatures: raw.signatures,
        body,
        auth_info,
        ..Tx::default()
    })
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
