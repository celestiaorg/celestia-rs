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
    let msgs = tx.gogo_msgs();
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

/// go-square v4.0.0-rc4 `tx.TryParseFibreTx` (celestia-app v9.0.8): rc4's
/// `square.Construct` runs it on every tx that is not a `BlobTx`, so it
/// decides which txs of an app-version-9 block are laid out as pay-for-fibre
/// txs with a system blob.
///
/// Unlike v10's [`try_parse`] it decodes go-square's trimmed
/// `cosmos.tx.v1beta1.Tx {TxBody body = 1}` with protobuf-go: repeated
/// `body` fields are *merged*, so their messages are concatenated, and only
/// the first message is checked. A tx whose `TxRaw` repeats `body_bytes`
/// (the SDK decoder keeps the last one) can therefore carry a
/// `MsgPayForFibre` first message that the v9 decoder never sees.
/// `Ok(None)`: not a fibre tx; `Err`: a malformed `MsgPayForFibre` (rc4's
/// `Construct` fails).
// protobuf-go truncates a uint32 varint like Go's `uint32(v)`.
#[allow(clippy::cast_possible_truncation)]
pub fn try_parse_v9(raw: &[u8]) -> Result<Option<square::v4::FibreTx>, String> {
    use txdecode::{PgoField, pgo_fields};
    // proto.Unmarshal(txBytes, &cosmostx.Tx{}): any error means "not a
    // fibre tx". `Body` is set by any occurrence of field 1.
    let mut has_body = false;
    let mut first: Option<(Vec<u8>, Vec<u8>)> = None;
    let parsed = pgo_fields(raw, &[1], &[], |f| {
        let PgoField::Bytes(_, body) = f else {
            return Ok(());
        };
        has_body = true;
        pgo_fields(body, &[1], &[], |f| {
            let PgoField::Bytes(_, any) = f else {
                return Ok(());
            };
            // google.protobuf.Any: proto3 string type_url (UTF-8 checked),
            // bytes value; the last occurrence of each wins.
            let (mut url, mut value) = (Vec::new(), Vec::new());
            pgo_fields(any, &[1, 2], &[], |f| {
                match f {
                    PgoField::Bytes(1, v) => {
                        std::str::from_utf8(v).map_err(|_| ())?;
                        url = v.to_vec();
                    }
                    PgoField::Bytes(_, v) => value = v.to_vec(),
                    _ => {}
                }
                Ok(())
            })?;
            if first.is_none() {
                first = Some((url, value));
            }
            Ok(())
        })
    });
    if parsed.is_err() || !has_body {
        return Ok(None);
    }
    let Some((url, value)) = first else {
        return Ok(None);
    };
    if url != pff_type_url().as_bytes() {
        return Ok(None);
    }
    // proto.Unmarshal(anyMsg.Value, &fibrev1.MsgPayForFibre{}) with rc4's
    // trimmed messages: signer = 1 (string), payment_promise = 2 (merged);
    // PaymentPromise: namespace = 3, blob_version = 5 (uint32),
    // commitment = 6.
    let mut signer = Vec::new();
    let mut promise: Option<(Vec<u8>, u32, Vec<u8>)> = None;
    pgo_fields(&value, &[1, 2], &[], |f| {
        match f {
            PgoField::Bytes(1, v) => {
                std::str::from_utf8(v).map_err(|_| ())?;
                signer = v.to_vec();
            }
            PgoField::Bytes(_, v) => {
                let p = promise.get_or_insert_with(Default::default);
                pgo_fields(v, &[3, 6], &[5], |f| {
                    match f {
                        PgoField::Bytes(3, v) => p.0 = v.to_vec(),
                        PgoField::Bytes(_, v) => p.2 = v.to_vec(),
                        PgoField::Varint(_, v) => p.1 = v as u32,
                        PgoField::Unknown => {}
                    }
                    Ok(())
                })?;
            }
            _ => {}
        }
        Ok(())
    })
    .map_err(|()| "unmarshalling MsgPayForFibre: proto: cannot parse invalid wire-format data")?;
    let Some((namespace, blob_version, commitment)) = promise else {
        return Err("MsgPayForFibre is missing payment_promise field".into());
    };
    let ns = Namespace::from_slice(&namespace)
        .map_err(|e| format!("invalid namespace in MsgPayForFibre: {e}"))?;
    let signer = String::from_utf8(signer).unwrap_or_default();
    let (_, signer) = encoding::bech32::decode_to_base256(&signer)
        .map_err(|e| format!("decoding signer address in MsgPayForFibre: {e}"))?;
    let signer: [u8; shares::SIGNER_SIZE] = signer
        .try_into()
        .map_err(|_| "creating system blob for MsgPayForFibre: signer size".to_owned())?;
    if ns.version() != 0 {
        return Err("creating system blob for MsgPayForFibre: namespace version".into());
    }
    let blob = shares::Blob::new_v2(ns, blob_version, &commitment, signer)
        .map_err(|e| format!("creating system blob for MsgPayForFibre: {e}"))?;
    Ok(Some(square::v4::FibreTx {
        tx: raw.to_vec(),
        system_blob: blob,
    }))
}

/// `TryParseFibreTx`: `Ok(None)` for txs that are not pay-for-fibre txs
/// (including every `BlobTx`), `Err` for a malformed `MsgPayForFibre`.
pub fn try_parse(raw: &[u8]) -> Result<Option<square::v4::FibreTx>, String> {
    if txdecode::unmarshal_blob_tx(raw).is_some() {
        return Ok(None);
    }
    // gogoproto `TxRaw.Unmarshal` / `TxBody.Unmarshal` /
    // `MsgPayForFibre.Unmarshal`: no UTF-8 validation (prost would reject
    // invalid UTF-8 in e.g. the memo and misclassify the tx), stdtime and
    // last-wins semantics of Go.
    let gogo = |name: &str, b: &[u8]| txdecode::gogo_unmarshal(name, b);
    let Ok(tx_raw) = gogo("cosmos.tx.v1beta1.TxRaw", raw) else {
        return Ok(None);
    };
    let Ok(tx_raw) = TxRaw::decode(tx_raw.as_slice()) else {
        return Ok(None);
    };
    let Ok(body) = gogo("cosmos.tx.v1beta1.TxBody", &tx_raw.body_bytes) else {
        return Ok(None);
    };
    let Ok(body) = TxBody::decode(body.as_slice()) else {
        return Ok(None);
    };
    let [m] = body.messages.as_slice() else {
        return Ok(None);
    };
    if m.type_url != pff_type_url() {
        return Ok(None);
    }
    let msg = gogo("celestia.fibre.v1.MsgPayForFibre", &m.value)
        .map_err(|e| format!("unmarshalling MsgPayForFibre: {e}"))?;
    let msg = MsgPayForFibre::decode(msg.as_slice())
        .map_err(|e| format!("unmarshalling MsgPayForFibre: {e}"))?;
    let promise = msg.payment_promise.unwrap_or_default();
    let ns = Namespace::from_slice(&promise.namespace)
        .map_err(|e| format!("invalid namespace in MsgPayForFibre: {e}"))?;
    // btcutil `bech32.DecodeToBase256` (90-character limit, any prefix).
    let (_, signer) = encoding::bech32::decode_to_base256(&msg.signer)
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
