//! `consensus/propagation/types`: the wire messages (compact block,
//! have/want parts, recovery parts), `CombinedPartSet` and `TxsToParts`.

use std::sync::Mutex;

use ctypes::merkle_proof::{leaf_hash, proofs_from_leaf_hashes};
use ctypes::proto::propagation as pp;
use ctypes::{BLOCK_PART_SIZE_BYTES, BitArray, Part, PartSet, PartSetHeader, Proof, Proposal};
use prost::Message as _;

use crate::erasure;

/// `ParityRatio`.
pub const PARITY_RATIO: u32 = 2;

/// `tmhash.Size`.
const HASH_SIZE: usize = 32;

/// `TxMetaData`: a tx's hash and its `[start, end)` range in the encoded
/// block.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TxMetaData {
    /// Tx hash (`TxKey`).
    pub hash: Vec<u8>,
    /// Start offset.
    pub start: u32,
    /// End offset (exclusive).
    pub end: u32,
}

impl TxMetaData {
    /// `ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pp::TxMetaData {
        pp::TxMetaData {
            hash: self.hash.clone(),
            start: self.start,
            end: self.end,
        }
    }

    /// `TxMetaDataFromProto`.
    #[must_use]
    pub fn from_proto(p: &pp::TxMetaData) -> Self {
        Self {
            hash: p.hash.clone(),
            start: p.start,
            end: p.end,
        }
    }

    /// `ValidateBasic`.
    pub fn validate_basic(&self) -> Result<(), String> {
        if self.start >= self.end {
            return Err(format!(
                "TxMetaData: start {} >= end {}",
                self.start, self.end
            ));
        }
        if self.hash.len() != HASH_SIZE {
            return Err(format!(
                "TxMetaData: hash size is invalid {}",
                hex::encode_upper(&self.hash)
            ));
        }
        Ok(())
    }
}

/// `CompactBlock`: the signed proposal plus the commitments needed to
/// verify original and parity parts.
#[derive(Debug, Default)]
pub struct CompactBlock {
    /// Merkle root of the parity parts.
    pub bp_hash: Vec<u8>,
    /// Tx locations (for recovering parts from the mempool).
    pub blobs: Vec<TxMetaData>,
    /// Proposer signature over [`Self::sign_bytes`].
    pub signature: Vec<u8>,
    /// The proposal.
    pub proposal: Proposal,
    /// Length of the last original part.
    pub last_len: u32,
    /// Leaf hashes of the original then the parity parts.
    pub parts_hashes: Vec<Vec<u8>>,
    pub(crate) proofs_cache: Mutex<Option<Vec<Proof>>>,
}

impl Clone for CompactBlock {
    fn clone(&self) -> Self {
        Self {
            bp_hash: self.bp_hash.clone(),
            blobs: self.blobs.clone(),
            signature: self.signature.clone(),
            proposal: self.proposal.clone(),
            last_len: self.last_len,
            parts_hashes: self.parts_hashes.clone(),
            proofs_cache: Mutex::new(self.cached_proofs()),
        }
    }
}

impl PartialEq for CompactBlock {
    fn eq(&self, o: &Self) -> bool {
        self.bp_hash == o.bp_hash
            && self.blobs == o.blobs
            && self.signature == o.signature
            && self.proposal == o.proposal
            && self.last_len == o.last_len
            && self.parts_hashes == o.parts_hashes
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl CompactBlock {
    fn cached_proofs(&self) -> Option<Vec<Proof>> {
        lock(&self.proofs_cache).clone()
    }

    /// `SignBytes`: the length-delimited protobuf without the signature.
    #[must_use]
    pub fn sign_bytes(&self) -> Vec<u8> {
        let p = pp::CompactBlock {
            bp_hash: self.bp_hash.clone(),
            blobs: self.blobs.iter().map(TxMetaData::to_proto).collect(),
            signature: Vec::new(),
            proposal: Some(self.proposal.to_proto()),
            last_length: self.last_len,
            parts_hashes: self.parts_hashes.clone(),
        };
        p.encode_length_delimited_to_vec()
    }

    /// `ValidateBasic`.
    pub fn validate_basic(&self) -> Result<(), String> {
        self.proposal.validate_basic().map_err(|e| e.to_string())?;
        ctypes::validate_hash(&self.bp_hash).map_err(|e| e.to_string())?;
        for b in &self.blobs {
            b.validate_basic()?;
        }
        if self.signature.len() > ctypes::commit::MAX_SIGNATURE_SIZE {
            return Err("CompactBlock: Signature is too big".into());
        }
        let expected =
            PARITY_RATIO as usize * self.proposal.block_id.part_set_header.total as usize;
        if self.parts_hashes.len() != expected {
            return Err(format!(
                "invalid number of partset hashes: expected {expected} actual {}",
                self.parts_hashes.len()
            ));
        }
        for (i, h) in self.parts_hashes.iter().enumerate() {
            ctypes::validate_hash(h).map_err(|e| {
                format!(
                    "invalid part hash height {} round {} index {i}: {e}",
                    self.proposal.height, self.proposal.round
                )
            })?;
        }
        has_overlapping_ranges(&self.blobs)
    }

    /// `ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pp::CompactBlock {
        pp::CompactBlock {
            bp_hash: self.bp_hash.clone(),
            blobs: self.blobs.iter().map(TxMetaData::to_proto).collect(),
            signature: self.signature.clone(),
            proposal: Some(self.proposal.to_proto()),
            last_length: self.last_len,
            parts_hashes: self.parts_hashes.clone(),
        }
    }

    /// `CompactBlockFromProto` (validates).
    pub fn from_proto(c: &pp::CompactBlock) -> Result<Self, String> {
        let proposal = Proposal::from_proto(c.proposal.as_ref().ok_or("nil proposal")?)
            .map_err(|e| e.to_string())?;
        let cb = Self {
            bp_hash: c.bp_hash.clone(),
            blobs: c.blobs.iter().map(TxMetaData::from_proto).collect(),
            signature: c.signature.clone(),
            proposal,
            last_len: c.last_length,
            parts_hashes: c.parts_hashes.clone(),
            proofs_cache: Mutex::new(None),
        };
        cb.validate_basic()?;
        Ok(cb)
    }

    /// `Proofs`: the proofs of every part, generated from
    /// [`Self::parts_hashes`] on first use and checked against the part-set
    /// root and [`Self::bp_hash`].
    pub fn proofs(&self) -> Result<Vec<Proof>, String> {
        let mut cache = lock(&self.proofs_cache);
        if let Some(p) = cache.as_ref() {
            return Ok(p.clone());
        }
        let total = self.proposal.block_id.part_set_header.total as usize;
        if self.parts_hashes.len() != PARITY_RATIO as usize * total {
            return Err("invalid number of partset hashes".into());
        }
        let (root, mut proofs) = proofs_from_leaf_hashes(&self.parts_hashes[..total]);
        if root != self.proposal.block_id.part_set_header.hash {
            return Err("incorrect PartsHash: original root".into());
        }
        let (proot, eproofs) = proofs_from_leaf_hashes(&self.parts_hashes[total..]);
        if proot != self.bp_hash {
            return Err("incorrect PartsHash: parity root".into());
        }
        proofs.extend(eproofs);
        *cache = Some(proofs.clone());
        Ok(proofs)
    }

    /// `GetProof` (only once the proofs are cached).
    #[must_use]
    pub fn get_proof(&self, i: u32) -> Option<Proof> {
        lock(&self.proofs_cache)
            .as_ref()
            .and_then(|p| p.get(i as usize).cloned())
    }

    /// `SetProofCache`.
    pub fn set_proof_cache(&self, proofs: Vec<Proof>) {
        *lock(&self.proofs_cache) = Some(proofs);
    }
}

/// `hasOverlappingRanges`.
fn has_overlapping_ranges(blobs: &[TxMetaData]) -> Result<(), String> {
    let mut b = blobs.to_vec();
    b.sort_by_key(|m| m.start);
    for i in 1..b.len() {
        let (prev, curr) = (&b[i - 1], &b[i]);
        if curr.start < prev.end {
            return Err(format!(
                "overlapping tx metadata ranges: {}:[{}-{}) and {i}:[{}-{})",
                i - 1,
                prev.start,
                prev.end,
                curr.start,
                curr.end
            ));
        }
    }
    Ok(())
}

/// `PartMetaData`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PartMetaData {
    /// Index in the combined (original + parity) set.
    pub index: u32,
    /// Leaf hash.
    pub hash: Vec<u8>,
}

/// `HaveParts`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HaveParts {
    /// Height.
    pub height: i64,
    /// Round.
    pub round: i32,
    /// Parts the sender has (or is getting).
    pub parts: Vec<PartMetaData>,
}

impl HaveParts {
    /// `BitArray(size)`.
    #[must_use]
    pub fn bit_array(&self, size: usize) -> BitArray {
        let mut ba = BitArray::new(size);
        for p in &self.parts {
            ba.set(p.index as usize, true);
        }
        ba
    }

    /// `ValidateBasic`.
    pub fn validate_basic(&self) -> Result<(), String> {
        if self.parts.is_empty() {
            return Err("HaveParts: Parts cannot be nil or empty".into());
        }
        if self.height < 0 || self.round < 0 {
            return Err("HaveParts: Height and Round cannot be negative".into());
        }
        for p in &self.parts {
            ctypes::validate_hash(&p.hash).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// `ValidatePartHashes`.
    pub fn validate_part_hashes(&self, expected: &[Vec<u8>]) -> Result<(), String> {
        if expected.is_empty() {
            return Err(format!(
                "empty expected hashes for height {} round {}",
                self.height, self.round
            ));
        }
        for p in &self.parts {
            let Some(h) = expected.get(p.index as usize) else {
                return Err(format!("non existing part hash index {}", p.index));
            };
            if *h != p.hash {
                return Err(format!("invalid part hash at index {}", p.index));
            }
        }
        Ok(())
    }

    /// `ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pp::HaveParts {
        pp::HaveParts {
            height: self.height,
            round: self.round,
            parts: self
                .parts
                .iter()
                .map(|p| pp::PartMetaData {
                    index: p.index,
                    hash: p.hash.clone(),
                })
                .collect(),
        }
    }

    /// `HavePartFromProto` (validates).
    pub fn from_proto(h: &pp::HaveParts) -> Result<Self, String> {
        let hp = Self {
            height: h.height,
            round: h.round,
            parts: h
                .parts
                .iter()
                .map(|p| PartMetaData {
                    index: p.index,
                    hash: p.hash.clone(),
                })
                .collect(),
        };
        hp.validate_basic()?;
        Ok(hp)
    }
}

/// `WantParts`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WantParts {
    /// Requested parts (combined indices).
    pub parts: BitArray,
    /// Height.
    pub height: i64,
    /// Round.
    pub round: i32,
    /// Whether proofs must be included (catch-up).
    pub prove: bool,
    /// Parts the requester still needs to decode.
    pub missing_parts_count: i32,
}

impl WantParts {
    /// `ValidateBasic`.
    pub fn validate_basic(&self) -> Result<(), String> {
        if self.parts.is_nil() {
            return Err("WantParts: Parts cannot be nil".into());
        }
        let max_bits = ctypes::part_set::MAX_BLOCK_PARTS_COUNT as usize * PARITY_RATIO as usize;
        if self.parts.size() > max_bits {
            return Err(format!(
                "WantParts: parts bit array size {} exceeds maximum {max_bits}",
                self.parts.size()
            ));
        }
        if self.missing_parts_count <= 0 {
            return Err("WantParts: MissingPartsCount cannot be negative or zero".into());
        }
        let missing = self.missing_parts_count as usize;
        if missing > self.parts.size() {
            return Err(format!(
                "WantParts: MissingPartsCount {} exceeds parts bit array size {}",
                self.missing_parts_count,
                self.parts.size()
            ));
        }
        if missing > ctypes::part_set::MAX_BLOCK_PARTS_COUNT as usize {
            return Err(format!(
                "WantParts: MissingPartsCount {} exceeds MaxBlockPartsCount {}",
                self.missing_parts_count,
                ctypes::part_set::MAX_BLOCK_PARTS_COUNT
            ));
        }
        Ok(())
    }

    /// `ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pp::WantParts {
        pp::WantParts {
            parts: Some(self.parts.to_proto()),
            height: self.height,
            round: self.round,
            prove: self.prove,
            missing_parts_count: self.missing_parts_count,
        }
    }

    /// `WantPartsFromProto` (validates).
    pub fn from_proto(w: &pp::WantParts) -> Result<Self, String> {
        let pb = w.parts.clone().unwrap_or_default();
        BitArray::validate_proto(&pb).map_err(|e| format!("WantParts: {e}"))?;
        let wp = Self {
            parts: BitArray::from_proto(&pb),
            height: w.height,
            round: w.round,
            prove: w.prove,
            missing_parts_count: w.missing_parts_count,
        };
        wp.validate_basic()?;
        Ok(wp)
    }
}

/// `RecoveryPart`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecoveryPart {
    /// Height.
    pub height: i64,
    /// Round.
    pub round: i32,
    /// Combined index.
    pub index: u32,
    /// Part bytes.
    pub data: Vec<u8>,
    /// Proof (catch-up only).
    pub proof: Option<Proof>,
}

impl RecoveryPart {
    /// `ValidateBasic`.
    pub fn validate_basic(&self) -> Result<(), String> {
        if self.height < 0 || self.round < 0 {
            return Err("RecoveryPart: Height and Round cannot be negative".into());
        }
        if self.data.is_empty() {
            return Err("RecoveryPart: Data cannot be nil or empty".into());
        }
        if let Some(p) = &self.proof {
            p.validate_basic()
                .map_err(|e| format!("RecoveryPart: invalid proof: {e}"))?;
            if leaf_hash(&self.data) != p.leaf_hash {
                return Err("RecoveryPart: invalid proof leaf hash".into());
            }
        }
        Ok(())
    }

    /// `ToProto` (the proof field is non-nullable in Go).
    #[must_use]
    pub fn to_proto(&self) -> pp::RecoveryPart {
        pp::RecoveryPart {
            height: self.height,
            round: self.round,
            index: self.index,
            data: self.data.clone(),
            proof: Some(self.proof.as_ref().map(Proof::to_proto).unwrap_or_default()),
        }
    }

    /// `RecoveryPartFromProto` (validates).
    pub fn from_proto(r: &pp::RecoveryPart) -> Result<Self, String> {
        let proof = Proof::from_proto(r.proof.as_ref(), true).map_err(|e| e.to_string())?;
        let rp = Self {
            height: r.height,
            round: r.round,
            index: r.index,
            data: r.data.clone(),
            proof,
        };
        rp.validate_basic()?;
        Ok(rp)
    }
}

/// A propagation reactor message (`propagation.Message`).
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// `CompactBlock`.
    CompactBlock(Box<CompactBlock>),
    /// `HaveParts`.
    HaveParts(HaveParts),
    /// `WantParts`.
    WantParts(WantParts),
    /// `RecoveryPart`.
    RecoveryPart(RecoveryPart),
}

impl Message {
    /// Wraps into `propagation.Message`.
    #[must_use]
    pub fn to_proto(&self) -> pp::Message {
        use pp::message::Sum;
        let sum = match self {
            Self::CompactBlock(c) => Sum::CompactBlock(c.to_proto()),
            Self::HaveParts(h) => Sum::HaveParts(h.to_proto()),
            Self::WantParts(w) => Sum::WantParts(w.to_proto()),
            Self::RecoveryPart(r) => Sum::RecoveryPart(r.to_proto()),
        };
        pp::Message { sum: Some(sum) }
    }

    /// Encoded wire bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        self.to_proto().encode_to_vec()
    }

    /// `MsgFromProto` (+ `ValidateBasic`).
    pub fn decode(bz: &[u8]) -> Result<Self, String> {
        use pp::message::Sum;
        let m = pp::Message::decode(bz).map_err(|e| e.to_string())?;
        Ok(match m.sum.ok_or("propagation: nil message")? {
            Sum::CompactBlock(c) => Self::CompactBlock(Box::new(CompactBlock::from_proto(&c)?)),
            Sum::HaveParts(h) => Self::HaveParts(HaveParts::from_proto(&h)?),
            Sum::WantParts(w) => Self::WantParts(WantParts::from_proto(&w)?),
            Sum::RecoveryPart(r) => Self::RecoveryPart(RecoveryPart::from_proto(&r)?),
        })
    }
}

/// `CombinedPartSet`: the original parts (indices `0..n`) and parity parts
/// (indices `n..2n`) of a proposal.
#[derive(Debug, Clone)]
pub struct CombinedPartSet {
    total_map: BitArray,
    original: PartSet,
    parity: PartSet,
    last_len: u32,
    catchup: bool,
    /// `IsDecoding`.
    pub is_decoding: bool,
}

impl CombinedPartSet {
    /// `NewCombinedSetFromCompactBlock`.
    #[must_use]
    pub fn from_compact_block(cb: &CompactBlock) -> Self {
        let original =
            PartSet::from_header(&cb.proposal.block_id.part_set_header, BLOCK_PART_SIZE_BYTES);
        let parity = PartSet::from_header(
            &PartSetHeader {
                total: original.total(),
                hash: cb.bp_hash.clone(),
            },
            BLOCK_PART_SIZE_BYTES,
        );
        let total_map = BitArray::new(original.total() as usize * 2);
        Self {
            total_map,
            original,
            parity,
            last_len: cb.last_len,
            catchup: false,
            is_decoding: false,
        }
    }

    /// `NewCombinedPartSetFromOriginal`.
    #[must_use]
    pub fn from_original(original: PartSet, catchup: bool) -> Self {
        let mut total_map = BitArray::new(original.total() as usize * 2);
        for i in 0..original.total() as usize {
            if original.has_part(i) {
                total_map.set(i, true);
            }
        }
        Self {
            total_map,
            original,
            parity: PartSet::from_header(&PartSetHeader::default(), BLOCK_PART_SIZE_BYTES),
            last_len: 0,
            catchup,
            is_decoding: false,
        }
    }

    /// `SetProposalData`.
    pub fn set_proposal_data(&mut self, original: PartSet, parity: PartSet) {
        self.total_map = BitArray::new((original.total() + parity.total()) as usize);
        self.total_map.fill();
        self.original = original;
        self.parity = parity;
    }

    /// `Original`.
    #[must_use]
    pub fn original(&self) -> &PartSet {
        &self.original
    }

    /// `Parity`.
    #[must_use]
    pub fn parity(&self) -> &PartSet {
        &self.parity
    }

    /// `BitArray`.
    #[must_use]
    pub fn bit_array(&self) -> &BitArray {
        &self.total_map
    }

    /// `MissingOriginal`.
    #[must_use]
    pub fn missing_original(&self) -> BitArray {
        let n = self.original.total() as usize;
        let mut out = BitArray::new(n * 2);
        for i in 0..n {
            if !self.original.has_part(i) {
                out.set(i, true);
            }
        }
        out
    }

    /// `Total`.
    #[must_use]
    pub fn total(&self) -> u32 {
        u32::try_from(self.total_map.size()).unwrap_or(0)
    }

    /// `IsComplete` (the original parts).
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.original.is_complete()
    }

    /// `CanDecode`.
    #[must_use]
    pub fn can_decode(&self) -> bool {
        self.original.count() + self.parity.count() >= self.original.total() && !self.catchup
    }

    /// `Decode`.
    pub fn decode(&mut self) -> Result<(), String> {
        erasure::decode(&mut self.original, &mut self.parity, self.last_len as usize)?;
        self.total_map.fill();
        Ok(())
    }

    /// `AddPart`: an already verified recovery part with its proof.
    pub fn add_part(&mut self, part: &RecoveryPart, proof: Proof) -> Result<bool, String> {
        let total = self.original.total();
        let mut p = Part {
            index: part.index,
            bytes: part.data.clone(),
            proof,
        };
        if part.index < total {
            let added = self.original.add_part(&p).map_err(|e| e.to_string())?;
            if added {
                self.total_map.set(part.index as usize, true);
            }
            return Ok(added);
        }
        p.index -= total;
        let added = self.parity.add_part(&p).map_err(|e| e.to_string())?;
        if added {
            self.total_map.set(part.index as usize, true);
        }
        Ok(added)
    }

    /// `AddOriginalPart`.
    pub fn add_original_part(&mut self, part: &Part) -> Result<bool, String> {
        let added = self.original.add_part(part).map_err(|e| e.to_string())?;
        if added {
            self.total_map.set(part.index as usize, true);
        }
        Ok(added)
    }

    /// `HasPart`.
    #[must_use]
    pub fn has_part(&self, index: usize) -> bool {
        self.total_map.get(index)
    }

    /// `GetPart` (combined index).
    #[must_use]
    pub fn get_part(&self, index: u32) -> Option<Part> {
        if !self.total_map.get(index as usize) {
            return None;
        }
        let total = self.original.total();
        if index < total {
            return self.original.get_part(index as usize);
        }
        self.parity.get_part((index - total) as usize)
    }
}

/// `UnmarshalledTx`: a mempool tx (wrapped as `mempool.Txs`) at its
/// location in the block.
#[derive(Debug, Clone)]
pub struct UnmarshalledTx {
    /// Location.
    pub meta: TxMetaData,
    /// `proto.Marshal(&mempool.Txs{Txs: [tx]})`.
    pub tx_bytes: Vec<u8>,
}

/// Go's `sort.Find(n, cmp)`: the smallest index with `cmp(i) <= 0`.
fn go_find(n: usize, cmp: impl Fn(usize) -> i64) -> usize {
    let (mut lo, mut hi) = (0usize, n);
    while lo < hi {
        let mid = usize::midpoint(lo, hi);
        if cmp(mid) > 0 {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

/// `TxsToParts`: the original parts fully covered by the given txs.
pub fn txs_to_parts(
    txs: &mut [UnmarshalledTx],
    part_count: u32,
    part_size: u32,
    last_part_len: u32,
) -> Result<Vec<Part>, String> {
    if txs.is_empty() {
        return Ok(Vec::new());
    }
    for (i, tx) in txs.iter().enumerate() {
        let expected = tx.meta.end.wrapping_sub(tx.meta.start);
        if tx.tx_bytes.len() != expected as usize {
            return Err(format!(
                "transaction {i} has inconsistent TxBytes length: expected {expected}, got {}",
                tx.tx_bytes.len()
            ));
        }
    }
    txs.sort_by_key(|t| t.meta.start);
    let mut result = Vec::new();
    for i in 0..part_count {
        let start_b = i * part_size;
        let mut end_b = start_b + part_size;
        if i == part_count - 1 && last_part_len > 0 && last_part_len < part_size {
            end_b = start_b + last_part_len;
        }
        let first = go_find(txs.len(), |n| {
            i64::from(start_b) - i64::from(txs[n].meta.end)
        });
        let last = go_find(txs.len(), |n| {
            i64::from(end_b) - i64::from(txs[n].meta.start)
        });
        let mut boundary = start_b;
        for tx in txs.iter().take(last).skip(first) {
            if tx.meta.start <= boundary {
                boundary = tx.meta.end;
            }
        }
        if boundary < end_b {
            continue;
        }
        let mut bytes = vec![0u8; (end_b - start_b) as usize];
        let mut skip = false;
        for tx in txs.iter().take(last).skip(first) {
            let os = tx.meta.start.max(start_b);
            let oe = tx.meta.end.min(end_b);
            if oe <= os {
                continue;
            }
            let tx_off = (os - tx.meta.start) as usize;
            let part_off = (os - start_b) as usize;
            let len = (oe - os) as usize;
            if tx_off + len > tx.tx_bytes.len() || part_off + len > bytes.len() {
                skip = true;
                break;
            }
            bytes[part_off..part_off + len].copy_from_slice(&tx.tx_bytes[tx_off..tx_off + len]);
        }
        if skip {
            continue;
        }
        result.push(Part {
            index: i,
            bytes,
            proof: Proof::default(),
        });
    }
    Ok(result)
}
