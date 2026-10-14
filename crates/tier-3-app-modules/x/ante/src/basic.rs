//! Stateless and parameter checks: circuit breaker, extension options,
//! `ValidateBasic`, timeout height, memo and tx-size gas.

use sdk::Context;
use sdk::error::sdk as errs;
use sdk::tx::Tx;

use crate::deps::AnteDeps;
use crate::pubkey::PubKey;
use crate::txview::{self, SigData};
use crate::{Fail, R, consume_gas, internal, is_recheck_tx};

/// `appconsts.TxSizeCostPerByte`.
pub const TX_SIZE_COST_PER_BYTE: u64 = 10;
/// `types.MaxGasWanted`.
pub const MAX_GAS_WANTED: u64 = (1 << 63) - 1;

/// `CircuitBreakerDecorator` (cosmossdk.io/x/circuit v0.1.1 `ante`).
pub fn circuit_breaker(ctx: &mut Context<'_>, tx: &Tx, deps: &impl AnteDeps) -> R<()> {
    for msg in tx.msgs() {
        if !deps.circuit_is_allowed(ctx, &msg.type_url)? {
            // cockroachdb errors.New: not an ABCI-registered error.
            return Err(internal("tx type not allowed").into());
        }
    }
    Ok(())
}

/// `RejectExtensionOptionsDecorator` with the default (reject-all) checker.
/// Only critical extension options are checked.
pub fn extension_options(tx: &Tx) -> R<()> {
    if tx.body.extension_options.is_empty() {
        Ok(())
    } else {
        Err(errs::unknown_extension_options("").into())
    }
}

/// A fee coin amount as gogoproto decodes it: an empty string leaves the
/// `math.Int` nil.
pub fn coin_amount(s: &str) -> R<Option<math::Int>> {
    if s.is_empty() {
        return Ok(None);
    }
    math::Int::parse(s)
        .map(Some)
        .map_err(|e| Fail::Err(errs::tx_decode(e.to_string())))
}

/// `wrapper.ValidateBasic` (skipped on ReCheckTx).
pub fn validate_basic(ctx: &Context<'_>, tx: &Tx) -> R<()> {
    if is_recheck_tx(ctx.mode) {
        return Ok(());
    }
    // tx.ValidateBasic
    let Some(fee) = &tx.auth_info.fee else {
        return Err(internal("missing fee").into());
    };
    if fee.gas_limit > MAX_GAS_WANTED {
        return Err(errs::invalid_request(format!(
            "invalid gas supplied; {} > {MAX_GAS_WANTED}",
            fee.gas_limit
        ))
        .into());
    }
    let mut amounts = Vec::with_capacity(fee.amount.len());
    for c in &fee.amount {
        amounts.push(coin_amount(&c.amount)?);
    }
    if amounts.iter().any(Option::is_none) {
        return Err(errs::insufficient_fee("invalid fee provided: null").into());
    }
    if amounts.iter().flatten().any(math::Int::is_negative) {
        return Err(errs::insufficient_fee("invalid fee provided").into());
    }
    if !fee.payer.is_empty()
        && let Err(e) = txview::acc_address_from_bech32(&fee.payer)
    {
        return Err(errs::invalid_address(format!("Invalid fee payer address ({})", e.log)).into());
    }
    if tx.signatures.is_empty() {
        return Err(errs::no_signatures("").into());
    }
    // wrapper: signer count must match signature count.
    let signers = txview::signers(tx)?;
    if tx.signatures.len() != signers.len() {
        return Err(errs::unauthorized(format!(
            "wrong number of signers; expected {}, got {}",
            signers.len(),
            tx.signatures.len()
        ))
        .into());
    }
    Ok(())
}

/// `TxTimeoutHeightDecorator`.
pub fn timeout_height(ctx: &Context<'_>, tx: &Tx) -> R<()> {
    let t = tx.body.timeout_height;
    // uint64(ctx.BlockHeight()) > timeoutHeight
    #[allow(clippy::cast_sign_loss)]
    let height = ctx.height() as u64;
    if t > 0 && height > t {
        return Err(errs::tx_timeout_height(format!(
            "block height: {}, timeout height: {t}",
            ctx.height()
        ))
        .into());
    }
    Ok(())
}

/// `ValidateMemoDecorator`: memo length in bytes against
/// `MaxMemoCharacters` (params are only read for a non-empty memo).
pub fn validate_memo(ctx: &mut Context<'_>, tx: &Tx) -> R<()> {
    let len = tx.body.memo.len() as u64;
    if len > 0 {
        let params = x_auth::get_params(ctx)?;
        if len > params.max_memo_characters {
            return Err(errs::memo_too_large(format!(
                "maximum number of characters is {} but received {len} characters",
                params.max_memo_characters
            ))
            .into());
        }
    }
    Ok(())
}

/// `isIncompleteSignature`.
fn is_incomplete(data: Option<&SigData>) -> bool {
    match data {
        None => true,
        Some(SigData::Single { signature, .. }) => signature.is_empty(),
        Some(SigData::Multi { signatures, .. }) => {
            signatures.is_empty() || signatures.iter().any(|s| is_incomplete(Some(s)))
        }
    }
}

/// celestia `ConsumeTxSizeGasDecorator` (`app/ante/tx_size_gas.go`):
/// `TxSizeCostPerByte` is the versioned constant, and in simulation each
/// incomplete signature is charged as an amino `StdSignature` with the
/// signer info's public key (or the placeholder).
pub fn consume_tx_size_gas(ctx: &mut Context<'_>, tx: &Tx, simulate: bool) -> R<()> {
    let size = ctx.tx_bytes.len() as u64;
    consume_gas(ctx, TX_SIZE_COST_PER_BYTE.wrapping_mul(size), "txSize")?;
    if simulate {
        let sigs = txview::signatures_v2(tx)?;
        for sig in &sigs {
            if !is_incomplete(sig.data.as_ref()) {
                continue;
            }
            let pk = sig.pub_key.clone().unwrap_or_else(PubKey::sim);
            let mut cost = std_signature_amino_len(&pk)? + 6;
            // celestia reads the params for every incomplete signature.
            let params = x_auth::get_params(ctx)?;
            if matches!(pk, PubKey::Multisig { .. }) {
                cost = cost.wrapping_mul(params.tx_sig_limit);
            }
            consume_gas(ctx, TX_SIZE_COST_PER_BYTE.wrapping_mul(cost), "txSize")?;
        }
    }
    Ok(())
}

/// Length of `legacy.Cdc.MustMarshal(StdSignature{PubKey: pk, Signature:
/// [64]byte{}})`: field 1 the amino-encoded key, field 2 the signature.
fn std_signature_amino_len(pk: &PubKey) -> R<u64> {
    let key = pk.amino_bytes()?;
    let mut len = 1 + crate::pubkey::uvarint(key.len() as u64).len() + key.len();
    len += 1 + 1 + 64;
    Ok(len as u64)
}
