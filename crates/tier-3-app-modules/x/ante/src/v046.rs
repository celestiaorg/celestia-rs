//! Decorators of celestia-app v1 to v3 (cosmos-sdk v0.46,
//! `app/ante/ante.go` at v3.13.0) that differ from their later versions
//! in gas, store access or rules. Only [`crate::ante_handle_v046`] runs
//! them.

use celestia_proto::celestia::blob::v1::MsgPayForBlobs;
use sdk::Context;
use sdk::error::sdk as errs;
use sdk::tx::Tx;

use crate::celestia::{MSG_EXEC_TYPE_URL, PFB_TYPE_URL, SUBMIT_PROPOSAL_TYPE_URL};
use crate::pubkey::PubKey;
use crate::{R, consume_gas, is_check_tx, is_recheck_tx, txview};

fn app_version(ctx: &Context<'_>) -> u64 {
    ctx.consensus_params.version.as_ref().map_or(0, |v| v.app)
}

/// celestia-app v3 `appconsts.MaxTxSize` (2 MiB).
pub const MAX_TX_SIZE_V3: usize = 2_097_152;

/// `MaxTxSizeDecorator`: from app version 3, a transaction larger than
/// `MaxTxSize` fails (also in DeliverTx), with a plain Go error.
pub fn max_tx_size(ctx: &Context<'_>) -> R<()> {
    if app_version(ctx) < 3 {
        return Ok(());
    }
    let size = ctx.tx_bytes.len();
    if size > MAX_TX_SIZE_V3 {
        return Err(sdk::error::internal(format!(
            "tx size {size} bytes is larger than the application's configured threshold of {MAX_TX_SIZE_V3} bytes. Please reduce the size by {} bytes",
            size - MAX_TX_SIZE_V3
        ))
        .into());
    }
    Ok(())
}

/// `ValidateMemoDecorator`: reads the auth params for every tx.
pub fn validate_memo(ctx: &mut Context<'_>, tx: &Tx) -> R<()> {
    let params = x_auth::get_params(ctx)?;
    let len = gostr::go_len(&tx.body.memo) as u64;
    if len > params.max_memo_characters {
        return Err(errs::memo_too_large(format!(
            "maximum number of characters is {} but received {len} characters",
            params.max_memo_characters
        ))
        .into());
    }
    Ok(())
}

/// celestia `consumeGasForTxSize`: the auth param `TxSizeCostPerByte` up
/// to app version 2, the versioned constant (10) from app version 3.
fn consume_gas_for_tx_size(ctx: &mut Context<'_>, bytes: u64, per_byte_param: u64) -> R<()> {
    let per_byte = if app_version(ctx) <= 2 {
        per_byte_param
    } else {
        crate::basic::TX_SIZE_COST_PER_BYTE
    };
    consume_gas(ctx, per_byte.wrapping_mul(bytes), "txSize")
}

/// celestia `ConsumeTxSizeGasDecorator` (v3.13.0): reads the auth params
/// once; in simulation each signer without a complete signature is
/// charged an amino `StdSignature` with its account's key (or the
/// placeholder).
pub fn consume_tx_size_gas(ctx: &mut Context<'_>, tx: &Tx, simulate: bool) -> R<()> {
    let params = x_auth::get_params(ctx)?;
    let size = ctx.tx_bytes.len() as u64;
    consume_gas_for_tx_size(ctx, size, params.tx_size_cost_per_byte)?;
    if !simulate {
        return Ok(());
    }
    let sigs = txview::signatures_v2(tx)?;
    let signers = txview::signers(tx)?;
    for (i, signer) in signers.iter().enumerate() {
        if sigs.get(i).is_some_and(|s| !is_incomplete(s.data.as_ref())) {
            continue;
        }
        let acc = x_auth::get_account(ctx, signer)?;
        let pk = match acc.as_ref().and_then(|a| a.pub_key()) {
            Some(any) => PubKey::from_any(any).or_else(|e| crate::panic(e.log))?,
            None => PubKey::sim(),
        };
        let key = pk.amino_bytes()?;
        let sig_len = 1 + crate::pubkey::uvarint(key.len() as u64).len() + key.len() + 1 + 1 + 64;
        let mut bytes = sig_len as u64 + 6;
        if matches!(pk, PubKey::Multisig { .. }) {
            bytes = bytes.wrapping_mul(params.tx_sig_limit);
        }
        consume_gas_for_tx_size(ctx, bytes, params.tx_size_cost_per_byte)?;
    }
    Ok(())
}

/// `isIncompleteSignature`.
fn is_incomplete(data: Option<&txview::SigData>) -> bool {
    use txview::SigData;
    match data {
        None => true,
        Some(SigData::Single { signature, .. }) => signature.is_empty(),
        Some(SigData::Multi { signatures, .. }) => {
            signatures.is_empty() || signatures.iter().any(|s| is_incomplete(s.as_ref()))
        }
    }
}

/// celestia `MsgExecDecorator` (CheckTx only): no `MsgExec` or
/// `MsgPayForBlobs` inside a `MsgExec`.
pub fn msg_exec(ctx: &Context<'_>, tx: &Tx) -> R<()> {
    if !is_check_tx(ctx.mode) {
        return Ok(());
    }
    for m in tx.msgs() {
        if let Some(nested) = txview::msg_exec_msgs(m)? {
            for n in &nested {
                let name = match n.type_url.as_str() {
                    MSG_EXEC_TYPE_URL => "MsgExec",
                    PFB_TYPE_URL => "MsgPayForBlobs",
                    _ => continue,
                };
                return Err(
                    errs::not_supported(format!("{name} inside MsgExec is not supported")).into(),
                );
            }
        }
    }
    Ok(())
}

/// blob `MinGasPFBDecorator` (v3.13.0): top-level `MsgPayForBlobs` only;
/// up to app version 2 the gas per blob byte is the blob param (read once,
/// at the first PFB), from app version 3 the versioned constant.
pub fn min_gas_pfb(ctx: &mut Context<'_>, tx: &Tx) -> R<()> {
    if is_recheck_tx(ctx.mode) {
        return Ok(());
    }
    let tx_gas = ctx.gas.remaining().unwrap_or(u64::MAX);
    let mut gas_per_byte: u64 = 0;
    for m in tx.msgs() {
        if m.type_url != PFB_TYPE_URL {
            continue;
        }
        let pfb: MsgPayForBlobs = crate::celestia::decode(m)?;
        if gas_per_byte == 0 {
            gas_per_byte = if app_version(ctx) <= 2 {
                u64::from(x_params::v046::get_u32(ctx, "blob", "GasPerBlobByte")?)
            } else {
                crate::celestia::GAS_PER_BLOB_BYTE
            };
        }
        let shares = pfb.blob_sizes.iter().fold(0u64, |acc, size| {
            acc.wrapping_add(crate::celestia::sparse_shares_needed(*size, false))
        });
        let need = shares
            .wrapping_mul(crate::celestia::SHARE_SIZE)
            .wrapping_mul(gas_per_byte);
        if need > tx_gas {
            return Err(errs::insufficient_fee(format!(
                "not enough gas to pay for blobs (minimum: {need}, got: {tx_gas})"
            ))
            .into());
        }
    }
    Ok(())
}

/// blob `BlobShareDecorator` (CheckTx only, from app version 2).
pub fn blob_share(ctx: &mut Context<'_>, tx: &Tx, deps: &impl crate::AnteDeps) -> R<()> {
    if app_version(ctx) == 1 {
        return Ok(());
    }
    crate::celestia::blob_share(ctx, tx, deps)
}

/// celestia `GovProposalDecorator`: a gov v1 `MsgSubmitProposal` needs at
/// least one message.
pub fn gov_proposal(tx: &Tx) -> R<()> {
    for m in tx.msgs() {
        if m.type_url != SUBMIT_PROPOSAL_TYPE_URL {
            continue;
        }
        if let Some(msgs) = txview::submit_proposal_msgs(m)?
            && msgs.is_empty()
        {
            return Err(sdk::SdkError::new(
                "gov",
                11,
                "must include at least one message in proposal: no messages proposed",
            )
            .into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use celestia_proto::celestia_core_proto::tendermint::types::{ConsensusParams, VersionParams};
    use sdk::{ExecMode, Header, MultiStore};

    #[test]
    fn max_tx_size_applies_from_app_version_3() {
        let mut ms = MultiStore::new();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        for (app, size, ok) in [
            (2, MAX_TX_SIZE_V3 + 1, true),
            (3, MAX_TX_SIZE_V3, true),
            (3, MAX_TX_SIZE_V3 + 1, false),
        ] {
            ctx.consensus_params = ConsensusParams {
                version: Some(VersionParams { app }),
                ..ConsensusParams::default()
            };
            ctx.tx_bytes = vec![0; size];
            assert_eq!(max_tx_size(&ctx).is_ok(), ok, "app {app} size {size}");
        }
    }
}
