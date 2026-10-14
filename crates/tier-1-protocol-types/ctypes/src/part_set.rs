//! `PartSetHeader`, `Part` and `PartSet` (`types/part_set.go`).
//!
//! The Reed-Solomon `Encode`/`Decode` helpers used by celestia's block
//! propagation reactor live with that reactor, not here.

use crate::block_id::validate_hash;
use crate::error::{Error, Result};
use crate::gofmt::{fingerprint_hex, hex_upper};
use crate::merkle_proof::{Proof, proofs_from_byte_slices};
use crate::proto::types as pb;

/// `DefaultMaxBlockSizeBytes` / `MaxBlockSizeBytes` (128 MiB).
pub const MAX_BLOCK_SIZE_BYTES: i64 = 128 * 1024 * 1024;
/// `ReducedMaxBlockSizeBytes` (8 MiB).
pub const REDUCED_MAX_BLOCK_SIZE_BYTES: i64 = 8 * 1024 * 1024;
/// `BlockPartSizeBytes` (64 KiB).
pub const BLOCK_PART_SIZE_BYTES: u32 = 65_536;
/// `MaxBlockPartsCount`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub const MAX_BLOCK_PARTS_COUNT: u32 = (MAX_BLOCK_SIZE_BYTES as u32 / BLOCK_PART_SIZE_BYTES) + 1;

/// Header identifying a part set: number of parts and Merkle root.
#[derive(Debug, Clone, PartialEq, Eq, Default, Hash)]
pub struct PartSetHeader {
    /// Number of parts.
    pub total: u32,
    /// Merkle root of the parts.
    pub hash: Vec<u8>,
}

impl PartSetHeader {
    /// `PartSetHeader.IsZero`.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.total == 0 && self.hash.is_empty()
    }

    /// `PartSetHeader.Equals`.
    #[must_use]
    pub fn equals(&self, other: &Self) -> bool {
        self == other
    }

    /// `PartSetHeader.ValidateBasic`.
    pub fn validate_basic(&self) -> Result<()> {
        if self.total > MAX_BLOCK_PARTS_COUNT {
            return Err(Error::msg(format!(
                "total {} exceeds MaxBlockPartsCount {MAX_BLOCK_PARTS_COUNT}",
                self.total
            )));
        }
        validate_hash(&self.hash).map_err(|e| e.wrap("wrong Hash: "))
    }

    /// `PartSetHeader.ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pb::PartSetHeader {
        pb::PartSetHeader {
            total: self.total,
            hash: self.hash.clone(),
        }
    }

    /// Converts without validation.
    #[must_use]
    pub fn from_proto_unvalidated(p: &pb::PartSetHeader) -> Self {
        Self {
            total: p.total,
            hash: p.hash.clone(),
        }
    }

    /// `PartSetHeaderFromProto` (validates).
    pub fn from_proto(p: &pb::PartSetHeader) -> Result<Self> {
        let h = Self::from_proto_unvalidated(p);
        h.validate_basic()?;
        Ok(h)
    }
}

impl core::fmt::Display for PartSetHeader {
    /// `PartSetHeader.String`: `total:FINGERPRINT`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}:{}", self.total, fingerprint_hex(&self.hash))
    }
}

/// A block part with its inclusion proof.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Part {
    /// Part index.
    pub index: u32,
    /// Part bytes.
    pub bytes: Vec<u8>,
    /// Inclusion proof against the part-set root.
    pub proof: Proof,
}

impl Part {
    /// `Part.ValidateBasic` (with `BlockPartSizeBytes`).
    pub fn validate_basic(&self) -> Result<()> {
        self.validate_basic_with_part_size(BLOCK_PART_SIZE_BYTES as usize)
    }

    fn validate_basic_with_part_size(&self, part_size: usize) -> Result<()> {
        if self.bytes.len() > part_size {
            return Err(Error::PartTooBig);
        }
        if i64::from(self.index) < self.proof.total.wrapping_sub(1) && self.bytes.len() != part_size
        {
            return Err(Error::PartInvalidSize);
        }
        if i64::from(self.index) != self.proof.index {
            return Err(Error::msg(format!(
                "invalid part: part index {} != proof index {}",
                self.index, self.proof.index
            )));
        }
        self.proof
            .validate_basic()
            .map_err(|e| e.wrap("invalid part: wrong Proof: "))
    }

    /// `Part.ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pb::Part {
        pb::Part {
            index: self.index,
            bytes: self.bytes.clone(),
            proof: Some(self.proof.to_proto()),
        }
    }

    /// `PartFromProto` (validates).
    pub fn from_proto(p: &pb::Part) -> Result<Self> {
        let proof = Proof::from_proto(p.proof.as_ref(), false)?.unwrap_or_default();
        let part = Self {
            index: p.index,
            bytes: p.bytes.clone(),
            proof,
        };
        part.validate_basic()?;
        Ok(part)
    }
}

/// Start/end (exclusive) of a tx's `Data.txs` field inside an encoded block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TxPosition {
    /// Start offset (the field tag).
    pub start: u32,
    /// End offset, exclusive.
    pub end: u32,
}

/// A (possibly partial) set of block parts backed by one buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartSet {
    total: u32,
    hash: Vec<u8>,
    buffer: Vec<u8>,
    part_size: usize,
    last_part_size: usize,
    proofs: Vec<Proof>,
    parts_bit_array: Vec<bool>,
    count: u32,
    byte_size: i64,
    /// Positions of each tx inside the encoded block (`MakePartSet`).
    pub tx_pos: Vec<TxPosition>,
}

impl PartSet {
    /// `NewPartSetFromData`: splits `data` into `part_size` chunks.
    pub fn from_data(data: &[u8], part_size: u32) -> Result<Self> {
        if part_size == 0 {
            return Err(Error::msg("part size must be greater than zero"));
        }
        #[allow(clippy::cast_possible_truncation)]
        let len32 = data.len() as u32;
        let total = len32.wrapping_add(part_size).wrapping_sub(1) / part_size;
        let mut chunks = Vec::with_capacity(total as usize);
        for i in 0..total {
            let start = i.wrapping_mul(part_size) as usize;
            let end = data
                .len()
                .min(i.wrapping_add(1).wrapping_mul(part_size) as usize);
            chunks.push(data.get(start..end).unwrap_or_default());
        }
        if chunks.is_empty() {
            // Go: `bits.NewBitArray(0)` is nil and `Fill` dereferences it.
            return Err(Error::msg(
                "runtime error: invalid memory address or nil pointer dereference",
            ));
        }
        let (root, proofs) = proofs_from_byte_slices(&chunks);
        let mut ops = Self::from_header(&PartSetHeader { total, hash: root }, part_size);
        let copied = ops.buffer.len().min(data.len());
        ops.buffer[..copied].copy_from_slice(&data[..copied]);
        if copied != data.len() {
            return Err(Error::msg(format!(
                "copy failed: {copied} < {}",
                data.len()
            )));
        }
        if let Some(last) = chunks.last() {
            ops.last_part_size = last.len();
        }
        ops.proofs = proofs;
        ops.parts_bit_array.fill(true);
        ops.count = total;
        ops.byte_size = i64::try_from(data.len()).unwrap_or(i64::MAX);
        Ok(ops)
    }

    /// `NewPartSetFromHeader`: an empty part set to be filled.
    #[must_use]
    pub fn from_header(header: &PartSetHeader, part_size: u32) -> Self {
        let total = header.total as usize;
        Self {
            total: header.total,
            hash: header.hash.clone(),
            buffer: vec![0; total * part_size as usize],
            part_size: part_size as usize,
            last_part_size: 0,
            proofs: vec![Proof::default(); total],
            parts_bit_array: vec![false; total],
            count: 0,
            byte_size: 0,
            tx_pos: Vec::new(),
        }
    }

    /// `PartSet.Header`.
    #[must_use]
    pub fn header(&self) -> PartSetHeader {
        PartSetHeader {
            total: self.total,
            hash: self.hash.clone(),
        }
    }

    /// `PartSet.HasHeader`.
    #[must_use]
    pub fn has_header(&self, header: &PartSetHeader) -> bool {
        self.header() == *header
    }

    /// `PartSet.BitArray` (copy).
    #[must_use]
    pub fn bit_array(&self) -> Vec<bool> {
        self.parts_bit_array.clone()
    }

    /// `PartSet.Hash`.
    #[must_use]
    pub fn hash(&self) -> &[u8] {
        &self.hash
    }

    /// `PartSet.HashesTo`.
    #[must_use]
    pub fn hashes_to(&self, hash: &[u8]) -> bool {
        self.hash == hash
    }

    /// `PartSet.Count`.
    #[must_use]
    pub fn count(&self) -> u32 {
        self.count
    }

    /// `PartSet.ByteSize`.
    #[must_use]
    pub fn byte_size(&self) -> i64 {
        self.byte_size
    }

    /// `PartSet.Total`.
    #[must_use]
    pub fn total(&self) -> u32 {
        self.total
    }

    /// `PartSet.AddPart`: validates the part and its proof, then stores it.
    /// `Ok(false)` for a duplicate.
    pub fn add_part(&mut self, part: &Part) -> Result<bool> {
        part.validate_basic_with_part_size(self.part_size)?;
        if self.has_part(part.index as usize) {
            return Ok(false);
        }
        if part.proof.total != i64::from(self.total) {
            return Err(Error::msg(format!(
                "error part set invalid proof: wrong total:{} {}",
                part.proof.total, self.total
            )));
        }
        let root = (!self.hash.is_empty()).then_some(self.hash.as_slice());
        part.proof
            .verify(root, &part.bytes)
            .map_err(|e| e.wrap("error part set invalid proof: wrong hash:"))?;
        self.add_part_without_proof(part)
    }

    /// `PartSet.AddPartWithoutProof`.
    pub fn add_part_without_proof(&mut self, part: &Part) -> Result<bool> {
        if part.index >= self.total {
            return Err(Error::PartSetUnexpectedIndex);
        }
        let index = part.index as usize;
        if self.parts_bit_array[index] {
            return Ok(false);
        }
        let start = index * self.part_size;
        let end = start + part.bytes.len();
        if end > self.buffer.len() {
            return Err(Error::msg("part data exceeds buffer bounds"));
        }
        self.buffer[start..end].copy_from_slice(&part.bytes);
        self.proofs[index] = part.proof.clone();
        if part.index == self.total - 1 {
            self.last_part_size = part.bytes.len();
        }
        self.count += 1;
        self.byte_size += i64::try_from(part.bytes.len()).unwrap_or(i64::MAX);
        self.parts_bit_array[index] = true;
        Ok(true)
    }

    /// `PartSet.GetPartBytes`.
    #[must_use]
    pub fn get_part_bytes(&self, index: usize) -> Option<&[u8]> {
        if !self.has_part(index) {
            return None;
        }
        let start = index * self.part_size;
        let size = if index + 1 == self.total as usize {
            self.last_part_size
        } else {
            self.part_size
        };
        let end = start + size;
        if start >= self.buffer.len() || end > self.buffer.len() {
            return None;
        }
        Some(&self.buffer[start..end])
    }

    /// `PartSet.GetPart`.
    #[must_use]
    pub fn get_part(&self, index: usize) -> Option<Part> {
        let bytes = self.get_part_bytes(index)?;
        Some(Part {
            index: u32::try_from(index).ok()?,
            bytes: bytes.to_vec(),
            proof: self.proofs[index].clone(),
        })
    }

    /// `PartSet.HasPart`.
    #[must_use]
    pub fn has_part(&self, index: usize) -> bool {
        self.parts_bit_array.get(index).copied().unwrap_or(false)
    }

    /// `PartSet.IsComplete`.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.count == self.total
    }

    /// `PartSet.GetBytes`: the reassembled data (`None` where Go panics:
    /// incomplete or empty part sets).
    #[must_use]
    pub fn get_bytes(&self) -> Option<&[u8]> {
        if !self.is_complete() || self.total == 0 {
            return None;
        }
        let size = (self.total as usize - 1) * self.part_size + self.last_part_size;
        self.buffer.get(..size)
    }

    /// `PartSet.StringShort`.
    #[must_use]
    pub fn string_short(&self) -> String {
        format!("({} of {})", self.count, self.total)
    }
}

impl core::fmt::Display for Part {
    /// Abbreviated `Part.String`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "Part{{#{}\n  Bytes: {}...\n  Proof: Proof{{\n    Aunts: [{}]\n  }}\n}}",
            self.index,
            fingerprint_hex(&self.bytes),
            self.proof
                .aunts
                .iter()
                .map(|a| hex_upper(a))
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
}
