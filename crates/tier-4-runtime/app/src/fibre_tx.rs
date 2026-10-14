//! Pay-for-fibre transaction handling shared by CheckTx and the proposal
//! handlers: `validatePayForFibreTxShape`, `payForFibreMsg`,
//! `containsFibreStateMsg` (celestia-app v10 `app/check_tx.go`) and
//! `x/fibre/types.TryParseFibreTx` / `ClassifyTxs`.

use celestia_proto::Any;
use celestia_proto::celestia::fibre::v1::{
    MsgDepositToEscrow, MsgPayForFibre, MsgPaymentPromiseTimeout, PaymentPromise,
};
use celestia_proto::cosmos::tx::v1beta1::{TxBody, TxRaw};
use prost::{Message, Name};
use sdk::SdkError;
use sdk::tx::Tx;
use shares::Namespace;

/// `MsgPayForFibreTypeURL`.
#[must_use]
pub fn pff_type_url() -> String {
    MsgPayForFibre::type_url()
}

/// `apperr.ErrInvalidPayForFibreTx`.
fn invalid_pff(log: impl Into<String>) -> SdkError {
    SdkError::new("app", 11_146, log)
}

/// Seconds of Go's zero `time.Time` as a protobuf timestamp.
const GO_ZERO_TIME_SECONDS: i64 = -62_135_596_800;

/// `share.Namespace.ValidateForBlob` on raw bytes.
fn valid_blob_namespace(bz: &[u8]) -> bool {
    Namespace::from_slice(bz).is_ok_and(|ns| !ns.is_reserved() && ns.version() == 0)
}

/// `PaymentPromise.ValidateBasic` (the error kind is irrelevant to callers:
/// it is wrapped in `ErrInvalidPayForFibreTx`).
fn validate_promise(p: &PaymentPromise) -> Result<(), String> {
    if p.chain_id.is_empty() {
        return Err("chain ID cannot be empty".into());
    }
    if p.namespace.is_empty() {
        return Err("namespace cannot be empty".into());
    }
    if p.namespace.len() != shares::NAMESPACE_SIZE {
        return Err("namespace must be 29 bytes".into());
    }
    if !valid_blob_namespace(&p.namespace) {
        return Err("invalid blob namespace".into());
    }
    if p.blob_size == 0 {
        return Err("blob size must be positive".into());
    }
    if p.commitment.len() != 32 {
        return Err("commitment must be 32 bytes".into());
    }
    if p.blob_version != 0 {
        return Err(format!("unsupported blob version: {}", p.blob_version));
    }
    if p.height <= 0 {
        return Err("height must be positive".into());
    }
    let zero = p
        .creation_timestamp
        .as_ref()
        .is_none_or(|t| t.seconds == GO_ZERO_TIME_SECONDS && t.nanos == 0);
    if zero {
        return Err("creation timestamp cannot be zero".into());
    }
    if p.signer_public_key.as_ref().map_or(0, |k| k.key.len()) != 33 {
        return Err("signer public key must be 33 bytes".into());
    }
    if p.signature.is_empty() {
        return Err("signature cannot be empty".into());
    }
    Ok(())
}

/// Number of `MsgPayForFibre` messages (`countMsgPayForFibre`).
#[must_use]
pub fn count_pff(msgs: &[Any]) -> usize {
    let url = pff_type_url();
    msgs.iter().filter(|m| m.type_url == url).count()
}

/// `validatePayForFibreTxShape`.
pub fn validate_shape(tx: &Tx) -> Result<(), SdkError> {
    let url = pff_type_url();
    let msgs = tx.msgs();
    for m in msgs {
        if m.type_url != url {
            continue;
        }
        if msgs.len() > 1 {
            return Err(invalid_pff(format!(
                "tx contains a MsgPayForFibre and {} total messages: invalid MsgPayForFibre transaction",
                msgs.len()
            )));
        }
        let msg =
            MsgPayForFibre::decode(m.value.as_slice()).map_err(|e| invalid_pff(e.to_string()))?;
        validate_promise(&msg.payment_promise.unwrap_or_default()).map_err(|e| {
            invalid_pff(format!(
                "invalid payment promise: {e}: invalid MsgPayForFibre transaction"
            ))
        })?;
    }
    Ok(())
}

/// `payForFibreMsg`: whether the tx is a single `MsgPayForFibre`.
#[must_use]
pub fn is_pff(tx: &Tx) -> bool {
    matches!(tx.msgs(), [m] if m.type_url == pff_type_url())
}

/// `containsFibreStateMsg`.
#[must_use]
pub fn contains_fibre_state_msg(tx: &Tx) -> bool {
    let a = MsgPaymentPromiseTimeout::type_url();
    let b = MsgDepositToEscrow::type_url();
    tx.msgs().iter().any(|m| m.type_url == a || m.type_url == b)
}

/// `TryParseFibreTx`: `Ok(None)` for txs that are not pay-for-fibre txs
/// (including every `BlobTx`), `Err` for a malformed `MsgPayForFibre`.
pub fn try_parse(raw: &[u8]) -> Result<Option<square::v4::FibreTx>, String> {
    if txdecode::unmarshal_blob_tx(raw).is_some() {
        return Ok(None);
    }
    let Ok(tx_raw) = TxRaw::decode(raw) else {
        return Ok(None);
    };
    let Ok(body) = TxBody::decode(tx_raw.body_bytes.as_slice()) else {
        return Ok(None);
    };
    let [m] = body.messages.as_slice() else {
        return Ok(None);
    };
    if m.type_url != pff_type_url() {
        return Ok(None);
    }
    let msg = MsgPayForFibre::decode(m.value.as_slice())
        .map_err(|e| format!("unmarshalling MsgPayForFibre: {e}"))?;
    let promise = msg.payment_promise.unwrap_or_default();
    let ns = Namespace::from_slice(&promise.namespace)
        .map_err(|e| format!("invalid namespace in MsgPayForFibre: {e}"))?;
    let (_, signer) = encoding::bech32::decode(&msg.signer)
        .map_err(|e| format!("decoding signer address in MsgPayForFibre: {e}"))?;
    let signer: [u8; shares::SIGNER_SIZE] = signer
        .try_into()
        .map_err(|_| "creating system blob for MsgPayForFibre: signer size".to_owned())?;
    if ns.version() != 0 {
        return Err("creating system blob for MsgPayForFibre: namespace version".into());
    }
    let blob = shares::Blob::new_v2(ns, promise.blob_version, &promise.commitment, signer)
        .map_err(|e| format!("creating system blob for MsgPayForFibre: {e}"))?;
    Ok(Some(square::v4::FibreTx {
        tx: raw.to_vec(),
        system_blob: blob,
    }))
}
