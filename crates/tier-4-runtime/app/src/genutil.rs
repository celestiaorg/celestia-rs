//! `x/genutil` InitGenesis: delivers genesis transactions (`DeliverGenTxs`)
//! through the regular (non-phased) `deliverTx` path, then applies the
//! staking validator set.

use celestia_proto::celestia_core_proto::abci::ValidatorUpdate;
use celestia_proto::cosmos::tx::v1beta1::{Tx as TxJson, TxRaw};
use prost::Message;
use sdk::error::sdk as errs;
use sdk::{Context, Result};

use crate::tx::{AnteFn, DecodeFn, run_ante, run_msgs};

/// JSON gentx → canonical tx bytes (`TxJSONDecoder` + `TxEncoder`).
pub fn gentx_bytes(json: &serde_json::Value) -> Result<Vec<u8>> {
    let tx: TxJson =
        serde_json::from_value(json.clone()).map_err(|e| errs::json_unmarshal(e.to_string()))?;
    let raw = TxRaw {
        body_bytes: tx.body.unwrap_or_default().encode_to_vec(),
        auth_info_bytes: tx.auth_info.unwrap_or_default().encode_to_vec(),
        signatures: tx.signatures,
    };
    Ok(raw.encode_to_vec())
}

/// `genutil.InitGenesis`. Returns validator updates from staking when
/// gentxs were delivered.
pub fn init_genesis(
    ctx: &mut Context<'_>,
    genesis: &serde_json::Value,
    decode: DecodeFn,
    ante: AnteFn,
    apply_validator_set: fn(&mut Context<'_>) -> Result<Vec<ValidatorUpdate>>,
) -> Result<Vec<ValidatorUpdate>> {
    let gen_txs = genesis
        .get("gen_txs")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    if gen_txs.is_empty() {
        return Ok(Vec::new());
    }
    for json in &gen_txs {
        let bytes = gentx_bytes(json)?;
        let (mut p, res) = run_ante(ctx, &bytes, decode, ante, false);
        res.map_err(|e| e.wrap("failed to deliver gentx (ante)"))?;
        run_msgs(ctx, &mut p, true).map_err(|e| e.wrap("failed to deliver gentx"))?;
    }
    apply_validator_set(ctx)
}
