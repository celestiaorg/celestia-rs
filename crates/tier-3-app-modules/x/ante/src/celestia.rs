//! celestia-app decorators (`app/ante`, `x/blob/ante`, `x/fibre/ante`) and
//! ibc-go's `RedundantRelayDecorator`.

use celestia_proto::Any;
use celestia_proto::celestia::blob::v1::MsgPayForBlobs;
use celestia_proto::celestia::fibre::v1::MsgPayForFibre;
use kvstore::GasMeter;
use prost::{Message, Name};
use sdk::error::sdk as errs;
use sdk::tx::Tx;
use sdk::{Context, ExecMode, SdkError};

use crate::deps::{AnteDeps, IbcPacketMsg};
use crate::{Fail, R, consume_gas, idx, is_check_tx, is_recheck_tx, txview};

/// `MsgPayForBlobs` type URL.
pub const PFB_TYPE_URL: &str = "/celestia.blob.v1.MsgPayForBlobs";
/// `MsgPayForFibre` type URL.
pub const PFF_TYPE_URL: &str = "/celestia.fibre.v1.MsgPayForFibre";
/// `MsgExec` type URL.
pub const MSG_EXEC_TYPE_URL: &str = "/cosmos.authz.v1beta1.MsgExec";
/// gov v1 `MsgSubmitProposal` type URL.
pub const SUBMIT_PROPOSAL_TYPE_URL: &str = "/cosmos.gov.v1.MsgSubmitProposal";

/// `appconsts.GasPerBlobByte`.
pub const GAS_PER_BLOB_BYTE: u64 = 8;
/// `appconsts.SquareSizeUpperBound`.
pub const SQUARE_SIZE_UPPER_BOUND: u64 = 512;
/// `appconsts.PFFibreTxGasFixedCost`.
pub const PFF_TX_GAS_FIXED_COST: u64 = 1_378;
/// `appconsts.PFFibreGasPerValidatorSignature`.
pub const PFF_GAS_PER_VALIDATOR_SIGNATURE: u64 = 1_000;

const SHARE_SIZE: u64 = 512;
const NAMESPACE_SIZE: u32 = 29;
const FIRST_COMPACT_SHARE_CONTENT_SIZE: u32 = 512 - NAMESPACE_SIZE - 1 - 4 - 4;
const CONTINUATION_COMPACT_SHARE_CONTENT_SIZE: u32 = 512 - NAMESPACE_SIZE - 1 - 4;
const FIRST_SPARSE_SHARE_CONTENT_SIZE: u32 = 512 - NAMESPACE_SIZE - 1 - 4;
const FIRST_SPARSE_SHARE_CONTENT_SIZE_WITH_SIGNER: u32 = 512 - NAMESPACE_SIZE - 1 - 4 - 20;
const CONTINUATION_SPARSE_SHARE_CONTENT_SIZE: u32 = 512 - NAMESPACE_SIZE - 1;

/// `share.CompactSharesNeeded`.
#[must_use]
pub fn compact_shares_needed(len: u32) -> u64 {
    if len == 0 {
        return 0;
    }
    if len < FIRST_COMPACT_SHARE_CONTENT_SIZE {
        return 1;
    }
    let rem = len - FIRST_COMPACT_SHARE_CONTENT_SIZE;
    let mut n = rem / CONTINUATION_COMPACT_SHARE_CONTENT_SIZE;
    if !rem.is_multiple_of(CONTINUATION_COMPACT_SHARE_CONTENT_SIZE) {
        n += 1;
    }
    1 + u64::from(n)
}

/// `share.SparseSharesNeeded`.
#[must_use]
pub fn sparse_shares_needed(len: u32, contains_signer: bool) -> u64 {
    if len == 0 {
        return 0;
    }
    let first = if contains_signer {
        FIRST_SPARSE_SHARE_CONTENT_SIZE_WITH_SIGNER
    } else {
        FIRST_SPARSE_SHARE_CONTENT_SIZE
    };
    if len <= first {
        return 1;
    }
    let rem = len - first;
    let mut n = rem / CONTINUATION_SPARSE_SHARE_CONTENT_SIZE;
    if !rem.is_multiple_of(CONTINUATION_SPARSE_SHARE_CONTENT_SIZE) {
        n += 1;
    }
    1 + u64::from(n)
}

/// The message as Go's decorators see it (gogoproto's decoding; see
/// `txdecode::prost_compatible`).
fn decode<M: Message + Default>(any: &Any) -> R<M> {
    let fixed = txdecode::prost_compatible(any);
    let any = fixed.as_ref().unwrap_or(any);
    M::decode(any.value.as_slice()).map_err(|e| Fail::Err(errs::tx_decode(e.to_string())))
}

fn is_type<M: Name>(any: &Any) -> bool {
    any.type_url == M::type_url()
}

/// `rejectWrapped`.
fn reject_wrapped(msgs: &[Any], container: &str) -> R<()> {
    for m in msgs {
        let name = match m.type_url.as_str() {
            PFB_TYPE_URL => "MsgPayForBlobs",
            PFF_TYPE_URL => "MsgPayForFibre",
            "/celestia.fibre.v1.MsgPaymentPromiseTimeout" => "MsgPaymentPromiseTimeout",
            "/celestia.fibre.v1.MsgDepositToEscrow" => "MsgDepositToEscrow",
            "/celestia.fibre.v1.MsgRequestWithdrawal" => "MsgRequestWithdrawal",
            MSG_EXEC_TYPE_URL => "MsgExec",
            SUBMIT_PROPOSAL_TYPE_URL => "MsgSubmitProposal",
            _ => continue,
        };
        return Err(
            errs::not_supported(format!("{name} inside {container} is not supported")).into(),
        );
    }
    Ok(())
}

/// celestia-app v9.0.8 `MsgExecDecorator` (app version 9): a `MsgExec` may
/// not wrap a `MsgExec` or a `MsgPayForBlobs`; `MsgSubmitProposal` is not
/// inspected.
pub fn msg_exec_v9(tx: &Tx) -> R<()> {
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

/// `NestedMsgDecorator`.
pub fn nested_msg(tx: &Tx) -> R<()> {
    for m in tx.msgs() {
        if let Some(nested) = txview::msg_exec_msgs(m)? {
            reject_wrapped(&nested, "MsgExec")?;
        } else if let Some(nested) = txview::submit_proposal_msgs(m)? {
            reject_wrapped(&nested, "MsgSubmitProposal")?;
        }
    }
    Ok(())
}

/// `PayForFibreMessage`: the tx's only message, if it is a
/// `MsgPayForFibre`.
pub fn pay_for_fibre_message(tx: &Tx) -> R<Option<MsgPayForFibre>> {
    match tx.msgs() {
        [m] if m.type_url == PFF_TYPE_URL => Ok(Some(decode(m)?)),
        _ => Ok(None),
    }
}

/// `FibreSignatureGasDecorator`.
pub fn fibre_signature_gas(ctx: &mut Context<'_>, tx: &Tx) -> R<()> {
    if let Some(msg) = pay_for_fibre_message(tx)? {
        let n = msg.validator_signatures.len() as u64;
        consume_gas(
            ctx,
            PFF_TX_GAS_FIXED_COST.wrapping_add(n.wrapping_mul(PFF_GAS_PER_VALIDATOR_SIGNATURE)),
            "ante verify: pay for fibre validator signatures",
        )?;
    }
    Ok(())
}

/// `GasToConsume` (uint64 arithmetic wraps like Go).
fn pfb_gas(pfb: &MsgPayForBlobs) -> R<u64> {
    let mut shares: u64 = 0;
    for (i, size) in pfb.blob_sizes.iter().enumerate() {
        let v = *idx(&pfb.share_versions, i)?;
        shares = shares.wrapping_add(sparse_shares_needed(*size, v == 1));
    }
    Ok(shares
        .wrapping_mul(SHARE_SIZE)
        .wrapping_mul(GAS_PER_BLOB_BYTE))
}

fn validate_pfb_has_enough_gas(msgs: &[Any], tx_gas: u64) -> R<()> {
    for m in msgs {
        if let Some(nested) = txview::msg_exec_msgs(m)? {
            validate_pfb_has_enough_gas(&nested, tx_gas)?;
        }
        if is_type::<MsgPayForBlobs>(m) {
            let pfb: MsgPayForBlobs = decode(m)?;
            let need = pfb_gas(&pfb)?;
            if need > tx_gas {
                return Err(errs::insufficient_fee(format!(
                    "not enough gas to pay for blobs (minimum: {need}, got: {tx_gas})"
                ))
                .into());
            }
        }
    }
    Ok(())
}

/// `MinGasPFBDecorator`.
pub fn min_gas_pfb(ctx: &mut Context<'_>, tx: &Tx) -> R<()> {
    if is_recheck_tx(ctx.mode) || ctx.height() == 0 {
        return Ok(());
    }
    let remaining = ctx.gas.remaining().unwrap_or(u64::MAX);
    validate_pfb_has_enough_gas(tx.msgs(), remaining)
}

/// `blobtypes.ErrBlobsTooLarge`.
fn blobs_too_large(log: String) -> SdkError {
    SdkError::new("blob", 11139, format!("{log}: blob(s) too large"))
}

fn validate_blob_shares(msgs: &[Any], tx_size: u32, max_shares: u64) -> R<()> {
    for m in msgs {
        if let Some(nested) = txview::msg_exec_msgs(m)? {
            validate_blob_shares(&nested, tx_size, max_shares)?;
        }
        if is_type::<MsgPayForBlobs>(m) {
            let pfb: MsgPayForBlobs = decode(m)?;
            let mut need = compact_shares_needed(tx_size);
            for (i, size) in pfb.blob_sizes.iter().enumerate() {
                let v = *idx(&pfb.share_versions, i)?;
                need += sparse_shares_needed(*size, v == 1);
            }
            if need > max_shares {
                return Err(blobs_too_large(format!(
                    "the number of shares occupied by blobs in this MsgPayForBlobs {need} exceeds the max number of shares available for blob data {max_shares}"
                ))
                .into());
            }
        }
    }
    Ok(())
}

/// `BlobShareDecorator` (CheckTx only).
pub fn blob_share(ctx: &mut Context<'_>, tx: &Tx, deps: &impl AnteDeps) -> R<()> {
    if !is_check_tx(ctx.mode) {
        return Ok(());
    }
    let Ok(tx_size) = u32::try_from(ctx.tx_bytes.len()) else {
        return Err(blobs_too_large("the tx size exceeds the max uint32".into()).into());
    };
    let gov_max = deps.blob_gov_max_square_size(ctx)?;
    let square = gov_max.min(SQUARE_SIZE_UPPER_BOUND);
    validate_blob_shares(tx.msgs(), tx_size, square * square)
}

fn with_infinite_gas<T>(
    ctx: &mut Context<'_>,
    f: impl FnOnce(&mut Context<'_>) -> Result<T, SdkError>,
) -> Result<T, SdkError> {
    let saved = std::mem::replace(&mut ctx.gas, GasMeter::infinite());
    let r = f(ctx);
    ctx.gas = saved;
    r
}

/// `FibreStatefulValidationDecorator` (CheckTx and ReCheckTx only).
pub fn fibre_stateful_validation(
    ctx: &mut Context<'_>,
    tx: &Tx,
    simulate: bool,
    deps: &impl AnteDeps,
) -> R<()> {
    let Some(msg) = pay_for_fibre_message(tx)? else {
        return Ok(());
    };
    if simulate || !is_check_tx(ctx.mode) {
        return Ok(());
    }
    let promise = msg.payment_promise.clone().unwrap_or_default();
    with_infinite_gas(ctx, |c| deps.fibre_validate_payment_promise(c, &promise))?;
    Ok(())
}

/// `NewPffSigCacheKey`: sha256 of the message without its signer.
#[must_use]
pub fn pff_sig_cache_key(msg: &MsgPayForFibre) -> [u8; 32] {
    let cert = MsgPayForFibre {
        signer: String::new(),
        payment_promise: Some(msg.payment_promise.clone().unwrap_or_default()),
        validator_signatures: msg.validator_signatures.clone(),
    };
    hash::sha256(&cert.encode_to_vec())
}

/// `FibreSignatureVerificationDecorator` (skipped in simulation and
/// FinalizeBlock).
pub fn fibre_sig_verification(
    ctx: &mut Context<'_>,
    tx: &Tx,
    simulate: bool,
    deps: &impl AnteDeps,
) -> R<()> {
    let Some(msg) = pay_for_fibre_message(tx)? else {
        return Ok(());
    };
    if simulate || ctx.mode == ExecMode::Finalize {
        return Ok(());
    }
    let key = pff_sig_cache_key(&msg);
    if deps.pff_sig_cached(&key) {
        return Ok(());
    }
    with_infinite_gas(ctx, |c| deps.fibre_validate_signatures(c, &msg))?;
    deps.pff_sig_cache(key);
    Ok(())
}

/// `ParamFilterDecorator.validateMsgs`.
fn validate_param_msgs(msgs: &[Any], deps: &impl AnteDeps) -> R<()> {
    if msgs.is_empty() {
        return Err(errs::invalid_request("must include at least one message").into());
    }
    for m in msgs {
        if let Some(nested) = txview::msg_exec_msgs(m)? {
            validate_param_msgs(&nested, deps)?;
        } else if let Some(nested) = txview::submit_proposal_msgs(m)? {
            validate_param_msgs(&nested, deps)?;
        } else if let Some(r) = deps.param_filter(m) {
            r?;
        }
    }
    Ok(())
}

/// `ParamFilterDecorator`.
pub fn param_filter(tx: &Tx, deps: &impl AnteDeps) -> R<()> {
    for m in tx.msgs() {
        if let Some(nested) = txview::submit_proposal_msgs(m)? {
            validate_param_msgs(&nested, deps)?;
        }
        if let Some(nested) = txview::msg_exec_msgs(m)? {
            validate_param_msgs(&nested, deps)?;
        }
    }
    Ok(())
}

/// `channeltypes.ErrRedundantTx`.
fn redundant_tx() -> SdkError {
    SdkError::new("channel", 22, "packet messages are redundant")
}

/// ibc-go `RedundantRelayDecorator` (CheckTx/ReCheckTx, not simulation).
pub fn redundant_relay(
    ctx: &mut Context<'_>,
    tx: &Tx,
    simulate: bool,
    deps: &impl AnteDeps,
) -> R<()> {
    if !is_check_tx(ctx.mode) || simulate {
        return Ok(());
    }
    let recheck = is_recheck_tx(ctx.mode);
    let mut redundancies = 0usize;
    let mut packet_msgs = 0usize;
    for m in tx.gogo_msgs() {
        let kind = match m.type_url.as_str() {
            "/ibc.core.channel.v1.MsgRecvPacket" => IbcPacketMsg::RecvPacket,
            "/ibc.core.channel.v1.MsgAcknowledgement" => IbcPacketMsg::Acknowledgement,
            "/ibc.core.channel.v1.MsgTimeout" => IbcPacketMsg::Timeout,
            "/ibc.core.channel.v1.MsgTimeoutOnClose" => IbcPacketMsg::TimeoutOnClose,
            "/ibc.core.client.v1.MsgUpdateClient" => {
                deps.ibc_update_client_check(ctx, m, recheck)?;
                continue;
            }
            // Any other message: the tx is processed regardless.
            _ => return Ok(()),
        };
        if deps.ibc_packet_is_noop(ctx, kind, m, recheck)? {
            redundancies += 1;
        }
        packet_msgs += 1;
    }
    if redundancies == packet_msgs && packet_msgs > 0 {
        return Err(redundant_tx().into());
    }
    Ok(())
}
