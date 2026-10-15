//! Consensus-exact port of celestia-app v10.4.0's transaction decoder.
//!
//! [`decode`] is `encoding.MakeConfig(...).TxConfig.TxDecoder()`, the decoder
//! `BaseApp` runs on every block tx in FinalizeBlock (and CheckTx /
//! ProcessProposal run on unwrapped bytes). It is a chain of wrappers:
//!
//! 1. `rejectEmptyTxDecoder`: empty bytes are an (unregistered) error.
//! 2. `blobTxDecoder`: if the bytes parse as a go-square v4 `BlobTx` with type
//!    id `BLOB`, it must be canonical and valid ([`unmarshal_blob_tx`]); the
//!    inner tx continues down the chain.
//! 3. `indexWrapperDecoder`: if the bytes parse (gogoproto) as an
//!    `IndexWrapper` with type id `INDX`, the inner tx is decoded and must
//!    hold exactly one `MsgPayForBlobs`.
//! 4. cosmos-sdk `DefaultTxDecoder` ([`default_tx_decode`]): ADR-027 check on
//!    `TxRaw`, `unknownproto` on `TxRaw` / `TxBody` (non-critical fields
//!    allowed) / `AuthInfo`, gogoproto unmarshal and `UnpackInterfaces`.
//!
//! No other wrapper exists in v10: go-square's `FibreTx` is only used
//! during square construction and never reaches the decoder.
//!
//! Errors carry the ABCI codespace/code Go reports for the error
//! (`errorsmod.ABCIInfo`): `sdk`/2 for decode failures, `undefined`/1 for
//! unregistered errors (empty tx, invalid `BlobTx`), `app`/11144–11145 for
//! invalid `IndexWrapper` contents, and `undefined`/111222 (`ErrPanic`) where
//! Go panics and `BaseApp` recovers.

// Integer conversions mirror Go's (truncating / reinterpreting) casts.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::many_single_char_names
)]

mod convert;
mod custom;
mod gogo;
mod protowire;
mod registry;
mod unknownproto;
mod unpack;
mod wrappers;

use sdk::SdkError;
use sdk::tx::Tx;

pub use wrappers::{
    BlobTxError, PgoField, UnwrappedBlobTx, marshal_blob_tx, pgo_fields, unmarshal_blob_tx,
    unmarshal_blob_tx_v9, unmarshal_index_wrapper,
};

use crate::gogo::{Dyn, GoError, Mode};
use crate::unpack::UnpackError;

/// Codespace of errors Go does not register (`errorsmod.UndefinedCodespace`).
pub const UNDEFINED_CODESPACE: &str = "undefined";

/// `MsgPayForBlobs` type URL.
const MSG_PAY_FOR_BLOBS: &str = "/celestia.blob.v1.MsgPayForBlobs";

/// An unregistered Go error (`ABCIInfo` → `undefined`/1).
fn undefined(log: impl Into<String>) -> SdkError {
    SdkError::new(UNDEFINED_CODESPACE, 1, log)
}

/// `errorsmod.ErrPanic` (`BaseApp` recovers decoder panics into it); it is
/// registered in the `undefined` codespace.
fn panic_err(log: impl AsRef<str>) -> SdkError {
    SdkError::new(
        UNDEFINED_CODESPACE,
        111_222,
        format!("{}: panic", log.as_ref()),
    )
}

/// Inputs longer than this are decoded on a thread with a large stack: Go's
/// recursion limits (10 000 nested messages) need more than a default thread
/// stack in the recursive Rust walkers.
const LARGE_INPUT: usize = 8 * 1024;
const LARGE_STACK: usize = 256 * 1024 * 1024;

fn with_stack(len: usize, f: impl FnOnce() -> sdk::Result<Tx> + Send) -> sdk::Result<Tx> {
    if len <= LARGE_INPUT {
        return f();
    }
    std::thread::scope(|s| {
        match std::thread::Builder::new()
            .stack_size(LARGE_STACK)
            .spawn_scoped(s, f)
        {
            Ok(h) => match h.join() {
                Ok(v) => v,
                Err(e) => std::panic::resume_unwind(e),
            },
            Err(e) => Err(undefined(format!("spawning decoder thread: {e}"))),
        }
    })
}

std::thread_local! {
    /// Set while decoding with celestia-app v9.0.8's decoder ([`decode_v9`]).
    static V9: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether the current decode runs with v9.0.8's registry and schema.
pub(crate) fn v9() -> bool {
    V9.with(std::cell::Cell::get)
}

fn with_v9<T>(f: impl FnOnce() -> T) -> T {
    let prev = V9.with(|v| v.replace(true));
    let out = f();
    V9.with(|v| v.set(prev));
    out
}

/// celestia-app v9.0.8's `TxConfig.TxDecoder()` (app version 9, ADR 0004).
///
/// Differs from [`decode`] in: no empty-tx rejection; go-square
/// v4.0.0-rc4's `UnmarshalBlobTx` (no canonical / nested / unknown-field
/// checks); an `IndexWrapper` may wrap any tx; the interface registry has no
/// `celestia.fibre.v1` / `celestia.valaddr.v1` types and no
/// `MsgCreateNativeSyntheticToken` (hyperlane-cosmos v1.1.0); `MsgForward`
/// has no fields 7 / 8.
pub fn decode_v9(raw: &[u8]) -> sdk::Result<Tx> {
    with_stack(raw.len(), || {
        with_v9(|| {
            // blobTxDecoder (rc4)
            match unmarshal_blob_tx_v9(raw) {
                Some(Ok(UnwrappedBlobTx { tx: inner, blobs })) => {
                    let mut tx = index_wrapper_decode_v9(&inner)?;
                    tx.blobs = blobs;
                    Ok(tx)
                }
                Some(Err(e)) => Err(undefined(format!("{e:?}"))),
                None => index_wrapper_decode_v9(raw),
            }
        })
    })
}

/// v9 `indexWrapperDecoder`: decodes the wrapped tx, whatever it holds.
fn index_wrapper_decode_v9(raw: &[u8]) -> sdk::Result<Tx> {
    let Some((inner, share_indexes)) = unmarshal_index_wrapper(raw) else {
        return default_tx_decode_inner(raw);
    };
    let mut tx = default_tx_decode_inner(&inner)?;
    tx.share_indexes = share_indexes;
    Ok(tx)
}

/// celestia-app v9.0.8 `hasDuplicateTxRawField`: whether a top-level field
/// other than `signatures` (3) appears more than once (undecodable bytes:
/// `false`).
#[must_use]
pub fn has_duplicate_tx_raw_field(mut tx: &[u8]) -> bool {
    let mut seen = std::collections::BTreeSet::new();
    while !tx.is_empty() {
        let (num, _, n) = protowire::consume_field(tx);
        if n < 0 {
            return false;
        }
        if num != 3 && !seen.insert(num) {
            return true;
        }
        tx = &tx[n as usize..];
    }
    false
}

/// gogoproto `Unmarshal` of `data` into the registered message `name` (full
/// proto name, e.g. `cosmos.tx.v1beta1.TxRaw`) with Go's semantics (no UTF-8
/// checks, customtype and stdtime validation, last-wins scalars), re-encoded
/// so prost decodes the struct Go holds (invalid UTF-8 replaced). `Err` is
/// Go's unmarshal error. `Any` fields are not unpacked.
pub fn gogo_unmarshal(name: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    let d = gogo::unmarshal(name, data).map_err(|e| e.0)?;
    Ok(gogo::marshal(&d, Mode::Struct))
}

/// [`decode`] or [`decode_v9`] by app version.
pub fn decode_for(app_version: u64, raw: &[u8]) -> sdk::Result<Tx> {
    if app_version != 0 && app_version < 10 {
        decode_v9(raw)
    } else {
        decode(raw)
    }
}

/// celestia-app's `TxConfig.TxDecoder()`: decodes raw block tx bytes.
///
/// Go accepts messages nested up to `unknownproto`'s recursion limit (e.g.
/// ~5 000 levels of multisig `ModeInfo`), so the returned struct can be very
/// deep: recursive `Clone`/`Drop`/`PartialEq` on it may need several MB of
/// stack.
pub fn decode(raw: &[u8]) -> sdk::Result<Tx> {
    with_stack(raw.len(), || decode_inner(raw))
}

fn decode_inner(raw: &[u8]) -> sdk::Result<Tx> {
    // rejectEmptyTxDecoder
    if raw.is_empty() {
        return Err(undefined("tx bytes are empty"));
    }
    // blobTxDecoder
    match unmarshal_blob_tx(raw) {
        Some(Ok(UnwrappedBlobTx { tx: inner, blobs })) => {
            let mut tx = index_wrapper_decode(&inner)?;
            tx.blobs = blobs;
            Ok(tx)
        }
        Some(Err(e)) => Err(undefined(format!("{e:?}"))),
        None => index_wrapper_decode(raw),
    }
}

/// `indexWrapperDecoder` applied to `raw`.
fn index_wrapper_decode(raw: &[u8]) -> sdk::Result<Tx> {
    let Some((inner, share_indexes)) = unmarshal_index_wrapper(raw) else {
        return default_tx_decode_inner(raw);
    };
    let mut tx = default_tx_decode_inner(&inner)?;
    if tx.body.messages.len() != 1 {
        return Err(SdkError::new(
            "app",
            11_145,
            "index wrapper txs must contain exactly one message",
        ));
    }
    if tx.body.messages[0].type_url != MSG_PAY_FOR_BLOBS {
        return Err(SdkError::new(
            "app",
            11_144,
            "index wrapper txs must contain a MsgPayForBlobs",
        ));
    }
    tx.share_indexes = share_indexes;
    Ok(tx)
}

fn tx_decode_err(e: &GoError) -> SdkError {
    sdk::error::sdk::tx_decode(&e.0)
}

fn unpack_err(e: UnpackError) -> SdkError {
    match e {
        UnpackError::Err(e) => tx_decode_err(&e),
        UnpackError::Panic(p) => panic_err(p),
    }
}

/// cosmos-sdk `DefaultTxDecoder` (no celestia wrappers).
pub fn default_tx_decode(tx_bytes: &[u8]) -> sdk::Result<Tx> {
    with_stack(tx_bytes.len(), || default_tx_decode_inner(tx_bytes))
}

fn default_tx_decode_inner(tx_bytes: &[u8]) -> sdk::Result<Tx> {
    reject_non_adr027_tx_raw(tx_bytes).map_err(|e| tx_decode_err(&e))?;
    let tx_raw_info = gogo::info("cosmos.tx.v1beta1.TxRaw").map_err(|e| tx_decode_err(&e))?;
    unknownproto::reject_unknown_fields(tx_bytes, tx_raw_info, false)
        .1
        .map_err(|e| tx_decode_err(&e))?;
    // cdc.Unmarshal(txBytes, &raw): its error is returned unwrapped.
    let raw = gogo::unmarshal("cosmos.tx.v1beta1.TxRaw", tx_bytes).map_err(|e| undefined(e.0))?;
    let body_bytes = raw.bytes(1).to_vec();
    let auth_info_bytes = raw.bytes(2).to_vec();
    let signatures = raw.bytes_list(3);

    let body_info = gogo::info("cosmos.tx.v1beta1.TxBody").map_err(|e| tx_decode_err(&e))?;
    let (has_unknown_non_criticals, r) =
        unknownproto::reject_unknown_fields(&body_bytes, body_info, true);
    r.map_err(|e| tx_decode_err(&e))?;
    let mut body =
        gogo::unmarshal("cosmos.tx.v1beta1.TxBody", &body_bytes).map_err(|e| tx_decode_err(&e))?;
    unpack::unpack_tx_body(&mut body).map_err(unpack_err)?;

    let auth_info = gogo::info("cosmos.tx.v1beta1.AuthInfo").map_err(|e| tx_decode_err(&e))?;
    unknownproto::reject_unknown_fields(&auth_info_bytes, auth_info, false)
        .1
        .map_err(|e| tx_decode_err(&e))?;
    let mut auth = gogo::unmarshal("cosmos.tx.v1beta1.AuthInfo", &auth_info_bytes)
        .map_err(|e| tx_decode_err(&e))?;
    unpack::unpack_auth_info(&mut auth).map_err(unpack_err)?;

    build_tx(
        tx_bytes,
        body_bytes,
        auth_info_bytes,
        signatures,
        &body,
        &auth,
        has_unknown_non_criticals,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_tx(
    tx_bytes: &[u8],
    body_bytes: Vec<u8>,
    auth_info_bytes: Vec<u8>,
    signatures: Vec<Vec<u8>>,
    body: &Dyn,
    auth: &Dyn,
    body_has_unknown_non_criticals: bool,
) -> sdk::Result<Tx> {
    // `GetMsgs` panics if a message was not unpacked (empty type URL).
    let mut normalized_msgs = Vec::new();
    for any in body.msg_list(1) {
        let Some(cached) = &any.cached else {
            return Err(panic_err(
                "any cached value is nil, transaction messages must be correctly packed any values",
            ));
        };
        normalized_msgs.push(celestia_proto::Any {
            type_url: String::from_utf8_lossy(any.bytes(1)).into_owned(),
            value: gogo::marshal(cached, Mode::Struct),
        });
    }
    Ok(Tx {
        tx_bytes: tx_bytes.to_vec(),
        body_bytes,
        auth_info_bytes,
        signatures,
        body: convert::tx_body(body),
        auth_info: convert::auth_info(auth),
        body_has_unknown_non_criticals,
        normalized_msgs,
        ..Tx::default()
    })
}

/// `rejectNonADR027TxRaw`: ascending field numbers (repeats allowed), only
/// wire type 2, minimal length-prefix varints.
fn reject_non_adr027_tx_raw(mut tx: &[u8]) -> Result<(), GoError> {
    let mut prev: i64 = 0;
    while !tx.is_empty() {
        let (num, wt, m) = protowire::consume_tag(tx);
        if m < 0 {
            return Err(GoError::new(format!(
                "invalid length; {}",
                protowire::parse_error(m)
            )));
        }
        if wt != protowire::BYTES {
            return Err(GoError::new(format!("expected 2 wire type, got {wt}")));
        }
        if num < prev {
            return Err(GoError::new(format!(
                "txRaw must follow ADR-027, got tagNum {num} after tagNum {prev}"
            )));
        }
        prev = num;
        let (len, n) = protowire::consume_varint(&tx[m as usize..]);
        if n < 0 {
            return Err(GoError::new(format!(
                "invalid length; {}",
                protowire::parse_error(n)
            )));
        }
        if protowire::size_varint(len) != n as usize {
            return Err(GoError::new(format!(
                "length prefix varint for tagNum {num} is not as short as possible"
            )));
        }
        let (_, _, m) = protowire::consume_field(tx);
        if m < 0 {
            return Err(GoError::new(format!(
                "invalid length; {}",
                protowire::parse_error(m)
            )));
        }
        tx = &tx[m as usize..];
    }
    Ok(())
}

/// Test-only access to the gogoproto re-encoding of a decoded tx, used by the
/// conformance tests to compare against Go's `Marshal` output.
#[doc(hidden)]
pub mod testing {
    /// gogoproto `Marshal` output of a decoded tx.
    pub struct CanonicalTx {
        /// `TxBody`.
        pub body: Vec<u8>,
        /// `AuthInfo`.
        pub auth_info: Vec<u8>,
        /// Each unpacked message.
        pub msgs: Vec<Vec<u8>>,
    }

    /// gogoproto `Marshal` of the decoded `TxBody`, `AuthInfo` and of each
    /// unpacked message, as Go holds them after decoding.
    #[must_use]
    pub fn canonical_tx(body_bytes: &[u8], auth_info_bytes: &[u8]) -> Option<CanonicalTx> {
        use crate::gogo::{Mode, marshal, unmarshal};
        let mut body = unmarshal("cosmos.tx.v1beta1.TxBody", body_bytes).ok()?;
        crate::unpack::unpack_tx_body(&mut body).ok()?;
        let mut auth = unmarshal("cosmos.tx.v1beta1.AuthInfo", auth_info_bytes).ok()?;
        crate::unpack::unpack_auth_info(&mut auth).ok()?;
        let msgs = body
            .msg_list(1)
            .iter()
            .map(|a| a.cached.as_ref().map(|c| marshal(c, Mode::Marshal)))
            .collect::<Option<Vec<_>>>()?;
        Some(CanonicalTx {
            body: marshal(&body, Mode::Marshal),
            auth_info: marshal(&auth, Mode::Marshal),
            msgs,
        })
    }

    /// [`canonical_tx`] with the registry and schema of `app_version`
    /// (celestia-app v9.0.8's below 10).
    #[must_use]
    pub fn canonical_tx_for(
        app_version: u64,
        body_bytes: &[u8],
        auth_info_bytes: &[u8],
    ) -> Option<CanonicalTx> {
        if app_version != 0 && app_version < 10 {
            crate::with_v9(|| canonical_tx(body_bytes, auth_info_bytes))
        } else {
            canonical_tx(body_bytes, auth_info_bytes)
        }
    }

    /// Message types the registry dump says implement
    /// `UnpackInterfacesMessage` and that this crate ports.
    pub const UNPACK_IMPLEMENTED: &[&str] = crate::unpack::IMPLEMENTED;

    /// Message types whose Go struct retains unknown fields.
    pub const RETAINS_UNKNOWN: &[&str] = crate::gogo::RETAINS_UNKNOWN;

    /// Custom types the decoder implements.
    pub const CUSTOM_TYPES: &[&str] = crate::custom::SUPPORTED;

    /// The static registry: `(type URL, message, interface bits)`.
    #[must_use]
    pub fn type_urls() -> &'static [(&'static str, &'static str, u32)] {
        crate::registry::TYPE_URLS
    }
}
