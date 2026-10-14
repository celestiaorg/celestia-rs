//! `Tx`, `Txs`, `TxProof` (`types/tx.go`), with celestia's wrapper-aware
//! hashing: an `IndexWrapper` or `BlobTx` hashes (and keys) as its inner tx.
//!
//! Wrapper detection reproduces the gogoproto generated `Unmarshal` of
//! `IndexWrapper`, `BlobTx` and `Blob` byte for byte (prost differs on edge
//! cases such as truncated packed varints, invalid UTF-8 and 64-bit tags).

use crate::error::{Error, Result};
use crate::merkle_proof::{Proof, hash_from_byte_slices, proofs_from_byte_slices};
use crate::proto::types as pb;

/// `TxKeySize`.
pub const TX_KEY_SIZE: usize = 32;
/// `go-square` `ProtoIndexWrapperTypeID`.
pub const PROTO_INDEX_WRAPPER_TYPE_ID: &str = "INDX";
/// `go-square` `ProtoBlobTxTypeID`.
pub const PROTO_BLOB_TX_TYPE_ID: &str = "BLOB";
/// `share.NamespaceIDSize`.
pub const NAMESPACE_ID_SIZE: usize = 28;

/// A transaction.
#[derive(Debug, Clone, PartialEq, Eq, Default, Hash)]
pub struct Tx(pub Vec<u8>);

impl Tx {
    /// `Tx.Hash`: SHA-256 of the inner tx of an index wrapper / blob tx,
    /// else of the tx.
    #[must_use]
    pub fn hash(&self) -> Vec<u8> {
        if let Some(iw) = unmarshal_index_wrapper(&self.0) {
            return hash::sha256(&iw.tx).to_vec();
        }
        if let Some(btx) = unmarshal_blob_tx(&self.0) {
            return hash::sha256(&btx.tx).to_vec();
        }
        hash::sha256(&self.0).to_vec()
    }

    /// `Tx.Key` (blob tx checked before index wrapper).
    #[must_use]
    pub fn key(&self) -> [u8; TX_KEY_SIZE] {
        if let Some(btx) = unmarshal_blob_tx(&self.0) {
            return hash::sha256(&btx.tx);
        }
        if let Some(iw) = unmarshal_index_wrapper(&self.0) {
            return hash::sha256(&iw.tx);
        }
        hash::sha256(&self.0)
    }
}

impl From<Vec<u8>> for Tx {
    fn from(v: Vec<u8>) -> Self {
        Self(v)
    }
}

/// `Txs.Hash`: Merkle root of the tx hashes.
#[must_use]
pub fn txs_hash(txs: &[Tx]) -> Vec<u8> {
    hash_from_byte_slices(&hash_list(txs))
}

fn hash_list(txs: &[Tx]) -> Vec<Vec<u8>> {
    txs.iter().map(Tx::hash).collect()
}

/// `Txs.Proof(i)`.
#[must_use]
pub fn txs_proof(txs: &[Tx], i: usize) -> Option<TxProof> {
    let (root, proofs) = proofs_from_byte_slices(&hash_list(txs));
    Some(TxProof {
        root_hash: root,
        data: txs.get(i)?.clone(),
        proof: proofs.into_iter().nth(i)?,
    })
}

/// `ComputeProtoSizeForTxs`: encoded size of `Data{Txs: txs}`.
#[must_use]
pub fn compute_proto_size_for_txs(txs: &[Tx]) -> i64 {
    let n: usize = txs
        .iter()
        .map(|t| 1 + prost::encoding::encoded_len_varint(t.0.len() as u64) + t.0.len())
        .sum();
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// `Txs.Validate(maxSizeBytes)`.
pub fn validate_txs(txs: &[Tx], max_size_bytes: i64) -> Result<()> {
    let mut size = 0i64;
    for tx in txs {
        size += compute_proto_size_for_txs(core::slice::from_ref(tx));
        if size > max_size_bytes {
            return Err(Error::msg(format!(
                "transaction data size exceeds maximum {max_size_bytes}"
            )));
        }
    }
    Ok(())
}

/// A Merkle proof that a tx is in a block's tx list.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TxProof {
    /// Root (`Txs.Hash`).
    pub root_hash: Vec<u8>,
    /// The tx.
    pub data: Tx,
    /// Proof of the tx hash.
    pub proof: Proof,
}

impl TxProof {
    /// `TxProof.Leaf`.
    #[must_use]
    pub fn leaf(&self) -> Vec<u8> {
        self.data.hash()
    }

    /// `TxProof.Validate(dataHash)`.
    pub fn validate(&self, data_hash: &[u8]) -> Result<()> {
        if data_hash != self.root_hash.as_slice() {
            return Err(Error::msg("proof matches different data hash"));
        }
        if self.proof.index < 0 {
            return Err(Error::msg("proof index cannot be negative"));
        }
        if self.proof.total <= 0 {
            return Err(Error::msg("proof total must be positive"));
        }
        let root = (!self.root_hash.is_empty()).then_some(self.root_hash.as_slice());
        if self.proof.verify(root, &self.leaf()).is_err() {
            return Err(Error::msg("proof is not internally consistent"));
        }
        Ok(())
    }

    /// `TxProof.ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pb::TxProof {
        pb::TxProof {
            root_hash: self.root_hash.clone(),
            data: self.data.0.clone(),
            proof: Some(self.proof.to_proto()),
        }
    }

    /// `TxProofFromProto`.
    pub fn from_proto(p: &pb::TxProof) -> Result<Self> {
        let proof = Proof::from_proto(p.proof.as_ref(), false)?.unwrap_or_default();
        Ok(Self {
            root_hash: p.root_hash.clone(),
            data: Tx(p.data.clone()),
            proof,
        })
    }
}

// ---------------------------------------------------------------------------
// gogoproto-faithful decoding of the wrapper messages.

/// Decoded `IndexWrapper`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexWrapper {
    /// Inner tx.
    pub tx: Vec<u8>,
    /// Share indexes.
    pub share_indexes: Vec<u32>,
    /// Type id (raw bytes; gogoproto does not validate UTF-8).
    pub type_id: Vec<u8>,
}

/// Decoded `Blob`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Blob {
    /// Namespace id.
    pub namespace_id: Vec<u8>,
    /// Data.
    pub data: Vec<u8>,
    /// Share version.
    pub share_version: u32,
    /// Namespace version.
    pub namespace_version: u32,
}

/// Decoded `BlobTx`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlobTx {
    /// Inner tx.
    pub tx: Vec<u8>,
    /// Blobs.
    pub blobs: Vec<Blob>,
    /// Type id.
    pub type_id: Vec<u8>,
}

struct GogoErr;

/// A varint as gogo's generated loops read it (error past 64 bits of shift
/// or at end of buffer). Returns the accumulated u64.
fn read_varint(data: &[u8], idx: &mut usize) -> Result<u64, GogoErr> {
    let mut v = 0u64;
    let mut shift = 0u32;
    loop {
        if shift >= 64 {
            return Err(GogoErr);
        }
        let b = *data.get(*idx).ok_or(GogoErr)?;
        *idx += 1;
        v |= u64::from(b & 0x7f) << shift;
        if b < 0x80 {
            return Ok(v);
        }
        shift += 7;
    }
}

/// A varint accumulated into a Go `uint32` (`v |= uint32(b&0x7F) << shift`;
/// shifts of 32 or more contribute nothing).
fn read_varint_u32(data: &[u8], idx: &mut usize) -> Result<u32, GogoErr> {
    let mut v = 0u32;
    let mut shift = 0u32;
    loop {
        if shift >= 64 {
            return Err(GogoErr);
        }
        let b = *data.get(*idx).ok_or(GogoErr)?;
        *idx += 1;
        if shift < 32 {
            v |= u32::from(b & 0x7f) << shift;
        }
        if b < 0x80 {
            return Ok(v);
        }
        shift += 7;
    }
}

/// A length read into a Go `int` (wrapping), checked like the generated
/// code (`< 0` and `postIndex < 0` are invalid, `> l` truncated).
#[allow(clippy::cast_possible_wrap)]
fn read_len(data: &[u8], idx: &mut usize) -> Result<usize, GogoErr> {
    let v = read_varint(data, idx)? as i64;
    if v < 0 {
        return Err(GogoErr);
    }
    let post = (*idx as i64).checked_add(v).ok_or(GogoErr)?;
    if post < 0 || post > i64::try_from(data.len()).unwrap_or(i64::MAX) {
        return Err(GogoErr);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(post as usize)
}

/// gogo `skip<File>`: returns the number of bytes of the field at `data`.
#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation
)]
fn skip(data: &[u8]) -> Result<usize, GogoErr> {
    let l = data.len() as i64;
    let mut idx: i64 = 0;
    let mut depth = 0i64;
    while idx < l {
        let mut i = idx as usize;
        let wire = read_varint(data, &mut i)?;
        idx = i as i64;
        match wire & 7 {
            0 => {
                let mut shift = 0;
                loop {
                    if shift >= 64 {
                        return Err(GogoErr);
                    }
                    if idx >= l {
                        return Err(GogoErr);
                    }
                    idx += 1;
                    if data[(idx - 1) as usize] < 0x80 {
                        break;
                    }
                    shift += 7;
                }
            }
            1 => idx += 8,
            2 => {
                let mut i = idx as usize;
                let length = read_varint(data, &mut i)? as i64;
                idx = i as i64;
                if length < 0 {
                    return Err(GogoErr);
                }
                idx = idx.wrapping_add(length);
            }
            3 => depth += 1,
            4 => {
                if depth == 0 {
                    return Err(GogoErr);
                }
                depth -= 1;
            }
            5 => idx += 4,
            _ => return Err(GogoErr),
        }
        if idx < 0 {
            return Err(GogoErr);
        }
        if depth == 0 {
            return Ok(idx as usize);
        }
    }
    Err(GogoErr)
}

/// Iterates fields like a generated gogo `Unmarshal`, calling `field` for
/// each `(field_num, wire_type)`; `field` returns `Ok(false)` to request the
/// default skip handling.
fn unmarshal(
    data: &[u8],
    mut field: impl FnMut(i32, u64, &mut usize) -> Result<bool, GogoErr>,
) -> Result<(), GogoErr> {
    let l = data.len();
    let mut idx = 0usize;
    while idx < l {
        let pre = idx;
        let wire = read_varint(data, &mut idx)?;
        #[allow(clippy::cast_possible_truncation)]
        let field_num = (wire >> 3) as i32;
        let wire_type = wire & 7;
        if wire_type == 4 || field_num <= 0 {
            return Err(GogoErr);
        }
        if !field(field_num, wire_type, &mut idx)? {
            idx = pre;
            let skippy = skip(&data[idx..])?;
            if idx + skippy > l {
                return Err(GogoErr);
            }
            idx += skippy;
        }
    }
    if idx > l {
        return Err(GogoErr);
    }
    Ok(())
}

fn bytes_field(data: &[u8], wire_type: u64, idx: &mut usize) -> Result<Vec<u8>, GogoErr> {
    if wire_type != 2 {
        return Err(GogoErr);
    }
    let post = read_len(data, idx)?;
    let v = data[*idx..post].to_vec();
    *idx = post;
    Ok(v)
}

fn u32_field(data: &[u8], wire_type: u64, idx: &mut usize) -> Result<u32, GogoErr> {
    if wire_type != 0 {
        return Err(GogoErr);
    }
    read_varint_u32(data, idx)
}

fn decode_index_wrapper(data: &[u8]) -> Result<IndexWrapper, GogoErr> {
    let mut m = IndexWrapper::default();
    unmarshal(data, |num, wt, idx| {
        match num {
            1 => m.tx = bytes_field(data, wt, idx)?,
            2 => match wt {
                0 => m.share_indexes.push(read_varint_u32(data, idx)?),
                2 => {
                    let post = read_len(data, idx)?;
                    // The element loop bounds-checks against the whole
                    // buffer, not `post`.
                    while *idx < post {
                        m.share_indexes.push(read_varint_u32(data, idx)?);
                    }
                }
                _ => return Err(GogoErr),
            },
            3 => m.type_id = bytes_field(data, wt, idx)?,
            _ => return Ok(false),
        }
        Ok(true)
    })?;
    Ok(m)
}

fn decode_blob(data: &[u8]) -> Result<Blob, GogoErr> {
    let mut m = Blob::default();
    unmarshal(data, |num, wt, idx| {
        match num {
            1 => m.namespace_id = bytes_field(data, wt, idx)?,
            2 => m.data = bytes_field(data, wt, idx)?,
            3 => m.share_version = u32_field(data, wt, idx)?,
            4 => m.namespace_version = u32_field(data, wt, idx)?,
            _ => return Ok(false),
        }
        Ok(true)
    })?;
    Ok(m)
}

fn decode_blob_tx(data: &[u8]) -> Result<BlobTx, GogoErr> {
    let mut m = BlobTx::default();
    unmarshal(data, |num, wt, idx| {
        match num {
            1 => m.tx = bytes_field(data, wt, idx)?,
            2 => {
                let sub = bytes_field(data, wt, idx)?;
                m.blobs.push(decode_blob(&sub)?);
            }
            3 => m.type_id = bytes_field(data, wt, idx)?,
            _ => return Ok(false),
        }
        Ok(true)
    })?;
    Ok(m)
}

/// `UnmarshalIndexWrapper`: `Some` iff the tx decodes as an
/// `IndexWrapper` with type id `INDX`.
#[must_use]
pub fn unmarshal_index_wrapper(tx: &[u8]) -> Option<IndexWrapper> {
    let iw = decode_index_wrapper(tx).ok()?;
    (iw.type_id == PROTO_INDEX_WRAPPER_TYPE_ID.as_bytes()).then_some(iw)
}

/// `UnmarshalBlobTx`: `Some` iff the tx decodes as a `BlobTx` with type id
/// `BLOB`, at least one blob, and every namespace id 28 bytes long.
#[must_use]
pub fn unmarshal_blob_tx(tx: &[u8]) -> Option<BlobTx> {
    let btx = decode_blob_tx(tx).ok()?;
    if btx.type_id != PROTO_BLOB_TX_TYPE_ID.as_bytes() || btx.blobs.is_empty() {
        return None;
    }
    if btx
        .blobs
        .iter()
        .any(|b| b.namespace_id.len() != NAMESPACE_ID_SIZE)
    {
        return None;
    }
    Some(btx)
}
