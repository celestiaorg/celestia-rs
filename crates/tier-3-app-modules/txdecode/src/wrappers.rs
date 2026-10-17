//! celestia wrapper formats: go-square v4 `BlobTx` (decoded with
//! google.golang.org/protobuf) and celestia-core's gogoproto `IndexWrapper`.

use sdk::tx::Blob;

use crate::gogo;
use crate::protowire::{self, append_bytes_field, append_tag, append_varint};

/// A protobuf-go decoding of `proto.blob.v4.BlobProto`.
#[derive(Debug, Default)]
struct BlobProto {
    namespace_id: Vec<u8>,
    data: Vec<u8>,
    share_version: u32,
    namespace_version: u32,
    /// `nil` when absent or empty (`consumeBytesNoZero`).
    signer: Vec<u8>,
    has_unknown: bool,
}

/// A protobuf-go decoding of `proto.blob.v4.BlobTx`.
#[derive(Debug, Default)]
struct BlobTxProto {
    tx: Vec<u8>,
    blobs: Vec<BlobProto>,
    type_id: String,
    has_unknown: bool,
}

/// One field as protobuf-go's fast-path decoder sees it.
pub enum PgoField<'a> {
    /// A known length-delimited field: number and payload.
    Bytes(u32, &'a [u8]),
    /// A known varint field: number and value.
    Varint(u32, u64),
    /// Any other (well-formed) field.
    Unknown,
}

/// Iterates fields with protobuf-go `unmarshalPointer` semantics.
/// `bytes_fields` / `varint_fields` say which field numbers are
/// length-delimited (wire type 2) and which are varints; a known field with
/// another wire type is an unknown field. `Err` where `proto.Unmarshal`
/// fails (malformed wire data, field number 0, a stray end group).
#[allow(clippy::result_unit_err)]
pub fn pgo_fields<'a>(
    mut b: &'a [u8],
    bytes_fields: &[u32],
    varint_fields: &[u32],
    mut visit: impl FnMut(PgoField<'a>) -> Result<(), ()>,
) -> Result<(), ()> {
    while !b.is_empty() {
        let (tag, n) = protowire::consume_varint(b);
        if n < 0 {
            return Err(());
        }
        b = &b[n as usize..];
        let num = tag >> 3;
        if !(1..=(1 << 29) - 1).contains(&num) {
            return Err(());
        }
        let num = num as u32;
        let wt = (tag & 7) as u8;
        if wt == protowire::END_GROUP {
            return Err(());
        }
        if wt == protowire::BYTES && bytes_fields.contains(&num) {
            let (v, n) = protowire::consume_bytes(b);
            if n < 0 {
                return Err(());
            }
            visit(PgoField::Bytes(num, v))?;
            b = &b[n as usize..];
        } else if wt == protowire::VARINT && varint_fields.contains(&num) {
            let (v, n) = protowire::consume_varint(b);
            if n < 0 {
                return Err(());
            }
            visit(PgoField::Varint(num, v))?;
            b = &b[n as usize..];
        } else {
            let n = protowire::consume_field_value(i64::from(num), wt, b);
            if n < 0 {
                return Err(());
            }
            visit(PgoField::Unknown)?;
            b = &b[n as usize..];
        }
    }
    Ok(())
}

fn parse_blob_proto(b: &[u8]) -> Result<BlobProto, ()> {
    let mut p = BlobProto::default();
    pgo_fields(b, &[1, 2, 5], &[3, 4], |f| {
        match f {
            PgoField::Bytes(1, v) => p.namespace_id = v.to_vec(),
            PgoField::Bytes(2, v) => p.data = v.to_vec(),
            PgoField::Bytes(_, v) => p.signer = v.to_vec(),
            PgoField::Varint(3, v) => p.share_version = v as u32,
            PgoField::Varint(_, v) => p.namespace_version = v as u32,
            PgoField::Unknown => p.has_unknown = true,
        }
        Ok(())
    })?;
    Ok(p)
}

/// `proto.Unmarshal(tx, &v4.BlobTx{})`.
fn parse_blob_tx(b: &[u8]) -> Result<BlobTxProto, ()> {
    let mut p = BlobTxProto::default();
    pgo_fields(b, &[1, 2, 3], &[], |f| {
        match f {
            PgoField::Bytes(1, v) => p.tx = v.to_vec(),
            PgoField::Bytes(2, v) => p.blobs.push(parse_blob_proto(v)?),
            PgoField::Bytes(_, v) => {
                // proto3 strings are validated as UTF-8.
                std::str::from_utf8(v)
                    .map_err(|_| ())?
                    .clone_into(&mut p.type_id);
            }
            PgoField::Varint(..) => {}
            PgoField::Unknown => p.has_unknown = true,
        }
        Ok(())
    })?;
    Ok(p)
}

/// go-square `ProtoBlobTxTypeID`.
const BLOB_TYPE_ID: &str = "BLOB";
/// go-square `ProtoIndexWrapperTypeID`.
const INDEX_WRAPPER_TYPE_ID: &[u8] = b"INDX";

/// Why a `BlobTx` was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobTxError {
    /// `ErrNonCanonicalBlobTx` (unknown fields, or not the canonical
    /// encoding of its contents).
    NonCanonical,
    /// `ErrNestedBlobTx`.
    Nested,
    /// `ErrNonCanonicalBlobTx` for unknown fields in blob `i`.
    NonCanonicalBlob(usize),
    /// Any other error (no blobs, invalid namespace, share version, signer,
    /// data size, …).
    Invalid(String),
}

impl std::fmt::Display for BlobTxError {
    /// Go's error texts.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonCanonical => f.write_str("non-canonical BlobTx encoding"),
            Self::Nested => f.write_str("nested BlobTx wrapper"),
            Self::NonCanonicalBlob(i) => write!(f, "non-canonical BlobTx encoding: blob {i}"),
            Self::Invalid(s) => f.write_str(s),
        }
    }
}

/// Go's `%v` of a `[]byte`: `[0 1 255]`.
fn go_byte_list(b: &[u8]) -> String {
    let items: Vec<String> = b.iter().map(u8::to_string).collect();
    format!("[{}]", items.join(" "))
}

/// A valid `BlobTx`: the inner SDK tx and its blobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnwrappedBlobTx {
    /// The inner tx bytes.
    pub tx: Vec<u8>,
    /// The blobs.
    pub blobs: Vec<Blob>,
}

/// go-square v4 `UnmarshalBlobTx`.
///
/// `None` means "not a blob tx" (the bytes do not parse as a `BlobTx` or the
/// type id is not `BLOB`); Go returns `isBlobTx = false` and callers decode
/// the bytes as a normal tx.
#[must_use]
pub fn unmarshal_blob_tx(tx: &[u8]) -> Option<Result<UnwrappedBlobTx, BlobTxError>> {
    let p = parse_blob_tx(tx).ok()?;
    if p.type_id != BLOB_TYPE_ID {
        return None;
    }
    Some(validate_blob_tx(tx, p))
}

/// go-square v4.0.0-rc4 `UnmarshalBlobTx` (celestia-app v9.0.8): like
/// [`unmarshal_blob_tx`] without the canonical-encoding, unknown-field and
/// nested-`BlobTx` checks.
#[must_use]
pub fn unmarshal_blob_tx_v9(tx: &[u8]) -> Option<Result<UnwrappedBlobTx, BlobTxError>> {
    let p = parse_blob_tx(tx).ok()?;
    if p.type_id != BLOB_TYPE_ID {
        return None;
    }
    if p.blobs.is_empty() {
        return Some(Err(BlobTxError::Invalid("no blobs provided".into())));
    }
    let mut blobs = Vec::with_capacity(p.blobs.len());
    for b in p.blobs {
        match new_blob_from_proto(b) {
            Ok(b) => blobs.push(b),
            Err(e) => return Some(Err(BlobTxError::Invalid(e))),
        }
    }
    Some(Ok(UnwrappedBlobTx { tx: p.tx, blobs }))
}

fn validate_blob_tx(tx: &[u8], p: BlobTxProto) -> Result<UnwrappedBlobTx, BlobTxError> {
    if p.has_unknown {
        return Err(BlobTxError::NonCanonical);
    }
    if p.blobs.is_empty() {
        return Err(BlobTxError::Invalid("no blobs provided".into()));
    }
    if parse_blob_tx(&p.tx).is_ok_and(|inner| inner.type_id == BLOB_TYPE_ID) {
        return Err(BlobTxError::Nested);
    }
    let mut blobs = Vec::with_capacity(p.blobs.len());
    for (i, b) in p.blobs.into_iter().enumerate() {
        if b.has_unknown {
            return Err(BlobTxError::NonCanonicalBlob(i));
        }
        blobs.push(new_blob_from_proto(b).map_err(BlobTxError::Invalid)?);
    }
    let canonical = marshal_blob_tx(&p.tx, &blobs);
    if canonical != tx {
        return Err(BlobTxError::NonCanonical);
    }
    Ok(UnwrappedBlobTx { tx: p.tx, blobs })
}

/// go-square v4 `share.NewBlobFromProto`.
fn new_blob_from_proto(p: BlobProto) -> Result<Blob, String> {
    const NAMESPACE_ID_SIZE: usize = 28;
    const VERSION_ZERO_PREFIX: usize = 18;
    const SIGNER_SIZE: usize = 20;
    if p.namespace_version > 255 {
        return Err("namespace version can not be greater than MaxNamespaceVersion".into());
    }
    if p.share_version > 127 {
        return Err("share version can not be greater than MaxShareVersion 127".into());
    }
    // NewNamespace: version 0 or 255, 28-byte id, version 0 ids start with
    // 18 zero bytes.
    let ns_version = p.namespace_version as u8;
    if ns_version != 0 && ns_version != 255 {
        return Err(format!(
            "invalid namespace: unsupported namespace version {ns_version}"
        ));
    }
    if p.namespace_id.len() != NAMESPACE_ID_SIZE {
        return Err(format!(
            "invalid namespace: unsupported namespace id length: id {} must be {NAMESPACE_ID_SIZE} bytes but it was {} bytes",
            go_byte_list(&p.namespace_id),
            p.namespace_id.len()
        ));
    }
    if ns_version == 0
        && p.namespace_id[..VERSION_ZERO_PREFIX]
            .iter()
            .any(|&b| b != 0)
    {
        return Err(format!(
            "invalid namespace: unsupported namespace id with version 0. ID {} must start with {VERSION_ZERO_PREFIX} leading zeros",
            go_byte_list(&p.namespace_id)
        ));
    }
    // NewBlob.
    if p.data.is_empty() {
        return Err("data can not be empty".into());
    }
    if ns_version != 0 {
        return Err(format!("namespace version must be 0 got {ns_version}"));
    }
    match p.share_version {
        0 => {
            if !p.signer.is_empty() {
                return Err("share version 0 does not support signer".into());
            }
        }
        1 => {
            if p.signer.len() != SIGNER_SIZE {
                return Err("share version 1 requires signer of size 20 bytes".into());
            }
        }
        2 => {
            if p.signer.len() != SIGNER_SIZE {
                return Err("share version 2 requires signer of size 20 bytes".into());
            }
            if p.data.len() != 4 + 32 {
                return Err(format!(
                    "share version 2 requires data of size 36 bytes (fibre_blob_version + commitment), got {}",
                    p.data.len()
                ));
            }
        }
        v => {
            return Err(format!(
                "share version {v} not supported. Please use 0, 1, or 2"
            ));
        }
    }
    Ok(Blob {
        namespace_id: p.namespace_id,
        namespace_version: p.namespace_version,
        share_version: p.share_version,
        data: p.data,
        signer: p.signer,
    })
}

/// go-square v4 `MarshalBlobTx` (protobuf-go deterministic field order).
#[must_use]
pub fn marshal_blob_tx(tx: &[u8], blobs: &[Blob]) -> Vec<u8> {
    let mut out = Vec::new();
    if !tx.is_empty() {
        append_bytes_field(&mut out, 1, tx);
    }
    for b in blobs {
        let mut bp = Vec::new();
        if !b.namespace_id.is_empty() {
            append_bytes_field(&mut bp, 1, &b.namespace_id);
        }
        if !b.data.is_empty() {
            append_bytes_field(&mut bp, 2, &b.data);
        }
        if b.share_version != 0 {
            append_tag(&mut bp, 3, protowire::VARINT);
            append_varint(&mut bp, u64::from(b.share_version));
        }
        if b.namespace_version != 0 {
            append_tag(&mut bp, 4, protowire::VARINT);
            append_varint(&mut bp, u64::from(b.namespace_version));
        }
        if !b.signer.is_empty() {
            append_bytes_field(&mut bp, 5, &b.signer);
        }
        append_bytes_field(&mut out, 2, &bp);
    }
    append_bytes_field(&mut out, 3, BLOB_TYPE_ID.as_bytes());
    out
}

/// celestia-core `types.UnmarshalIndexWrapper` (gogoproto): the inner tx and
/// share indexes, or `None` if the bytes are not an `IndexWrapper`.
#[must_use]
pub fn unmarshal_index_wrapper(tx: &[u8]) -> Option<(Vec<u8>, Vec<u32>)> {
    let d = gogo::unmarshal("tendermint.types.IndexWrapper", tx).ok()?;
    if d.bytes(3) != INDEX_WRAPPER_TYPE_ID {
        return None;
    }
    let indexes = d.num_list(2).into_iter().map(|v| v as u32).collect();
    Some((d.bytes(1).to_vec(), indexes))
}
