//! `Block`, `Data`, `EvidenceData`, block size limits and
//! `MarshalBlockWithTxPositions` (`types/block.go`, `types/marshalling.go`).

use crate::commit::{Commit, max_commit_bytes};
use crate::error::{Error, Result};
use crate::evidence::{Evidence, evidence_list_hash};
use crate::gofmt::hex_upper;
use crate::header::{Header, MAX_HEADER_BYTES};
use crate::part_set::{PartSet, TxPosition};
use crate::proto::types as pb;
use crate::tx::{Tx, txs_hash};
use prost::Message;

/// `MaxOverheadForBlock`.
pub const MAX_OVERHEAD_FOR_BLOCK: i64 = 11;

/// Block data: transactions plus celestia's square size and data root.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Data {
    /// Transactions.
    pub txs: Vec<Tx>,
    /// Square size of the block's data square.
    pub square_size: u64,
    /// Go's volatile `hash`: the data root when set from `PrepareProposal`
    /// or the encoded block, otherwise memoized `Txs.Hash()`. It is encoded
    /// in `Data.hash`. `Some(vec![])` is Go's non-nil empty slice.
    pub hash: Option<Vec<u8>>,
}

impl Data {
    /// `NewData`.
    #[must_use]
    pub fn new(txs: Vec<Tx>, square_size: u64, hash: Option<Vec<u8>>) -> Self {
        Self {
            txs,
            square_size,
            hash,
        }
    }

    /// `Data.Hash`: the stored hash, memoizing `Txs.Hash()` when unset.
    pub fn hash(&mut self) -> Vec<u8> {
        self.hash.get_or_insert_with(|| txs_hash(&self.txs)).clone()
    }

    /// `Data.ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pb::Data {
        pb::Data {
            txs: self.txs.iter().map(|t| t.0.clone()).collect(),
            square_size: self.square_size,
            hash: self.hash.clone().unwrap_or_default(),
        }
    }

    /// `DataFromProto`. Protobuf cannot tell an absent `hash` from an empty
    /// one; [`Block::decode`] recovers that from the wire bytes.
    #[must_use]
    pub fn from_proto(p: &pb::Data) -> Self {
        Self {
            txs: p.txs.iter().cloned().map(Tx).collect(),
            square_size: p.square_size,
            hash: (!p.hash.is_empty()).then(|| p.hash.clone()),
        }
    }
}

/// Evidence included in a block.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EvidenceData {
    /// Evidence list.
    pub evidence: Vec<Evidence>,
}

impl EvidenceData {
    /// `EvidenceData.Hash`.
    #[must_use]
    pub fn hash(&self) -> Vec<u8> {
        evidence_list_hash(&self.evidence)
    }

    /// `EvidenceData.ByteSize`: encoded size of the evidence list.
    #[must_use]
    pub fn byte_size(&self) -> i64 {
        if self.evidence.is_empty() {
            return 0;
        }
        self.to_proto()
            .map_or(0, |p| i64::try_from(p.encoded_len()).unwrap_or(i64::MAX))
    }

    /// `EvidenceData.ToProto`.
    pub fn to_proto(&self) -> Result<pb::EvidenceList> {
        Ok(pb::EvidenceList {
            evidence: self
                .evidence
                .iter()
                .map(Evidence::to_proto)
                .collect::<Result<_>>()?,
        })
    }

    /// `EvidenceData.FromProto`.
    pub fn from_proto(p: &pb::EvidenceList) -> Result<Self> {
        Ok(Self {
            evidence: p
                .evidence
                .iter()
                .map(Evidence::from_proto)
                .collect::<Result<_>>()?,
        })
    }
}

/// A block.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Block {
    /// Header.
    pub header: Header,
    /// Data.
    pub data: Data,
    /// Evidence.
    pub evidence: EvidenceData,
    /// Commit for the previous block (Go pointer; `None` is nil).
    pub last_commit: Option<Commit>,
    verified_hash: Option<Vec<u8>>,
}

impl Block {
    /// `MakeBlock`: a block with the self-derived header fields filled in.
    #[must_use]
    pub fn make(
        height: i64,
        data: Data,
        last_commit: Option<Commit>,
        evidence: Vec<Evidence>,
    ) -> Self {
        let mut b = Self {
            header: Header {
                version: crate::header::Consensus {
                    block: crate::header::BLOCK_PROTOCOL,
                    app: 0,
                },
                height,
                ..Header::default()
            },
            data,
            evidence: EvidenceData { evidence },
            last_commit,
            verified_hash: None,
        };
        b.fill_header();
        b
    }

    /// Builds a block from parts (no hashing or validation).
    #[must_use]
    pub fn new(
        header: Header,
        data: Data,
        evidence: EvidenceData,
        last_commit: Option<Commit>,
    ) -> Self {
        Self {
            header,
            data,
            evidence,
            last_commit,
            verified_hash: None,
        }
    }

    /// `Block.ValidateBasic` (memoizes `Data.hash` like Go).
    pub fn validate_basic(&mut self) -> Result<()> {
        self.header
            .validate_basic()
            .map_err(|e| e.wrap("invalid header: "))?;
        let Some(lc) = &self.last_commit else {
            return Err(Error::msg("nil LastCommit"));
        };
        lc.validate_basic()
            .map_err(|e| e.wrap("wrong LastCommit: "))?;
        let lch = lc.hash();
        if self.header.last_commit_hash != lch {
            return Err(Error::msg(format!(
                "wrong Header.LastCommitHash. Expected {}, got {}",
                hex_upper(&lch),
                hex_upper(&self.header.last_commit_hash)
            )));
        }
        let dh = self.data.hash();
        if self.header.data_hash != dh {
            return Err(Error::msg(format!(
                "wrong Header.DataHash. Expected {}, got {}",
                hex_upper(&dh),
                hex_upper(&self.header.data_hash)
            )));
        }
        for (i, ev) in self.evidence.evidence.iter().enumerate() {
            ev.validate_basic()
                .map_err(|e| e.wrap(&format!("invalid evidence (#{i}): ")))?;
        }
        let eh = self.evidence.hash();
        if self.header.evidence_hash != eh {
            return Err(Error::msg(format!(
                "wrong Header.EvidenceHash. Expected {}, got {}",
                hex_upper(&self.header.evidence_hash),
                hex_upper(&eh)
            )));
        }
        Ok(())
    }

    /// `fillHeader`: fills unset (empty) self-derived hashes.
    fn fill_header(&mut self) {
        if self.header.last_commit_hash.is_empty() {
            self.header.last_commit_hash = self
                .last_commit
                .as_ref()
                .map(Commit::hash)
                .unwrap_or_default();
        }
        if self.header.data_hash.is_empty() {
            self.header.data_hash = self.data.hash();
        }
        if self.header.evidence_hash.is_empty() {
            self.header.evidence_hash = self.evidence.hash();
        }
    }

    /// `Block.Hash`: `None` without a last commit or validators hash.
    /// Fills the header like Go and memoizes the result.
    pub fn hash(&mut self) -> Option<Vec<u8>> {
        self.last_commit.as_ref()?;
        if let Some(h) = &self.verified_hash {
            return Some(h.clone());
        }
        self.fill_header();
        let h = self.header.hash()?;
        self.verified_hash = Some(h.clone());
        Some(h)
    }

    /// `Block.HashesTo`.
    pub fn hashes_to(&mut self, hash: &[u8]) -> bool {
        if hash.is_empty() {
            return false;
        }
        self.hash().is_some_and(|h| h == hash)
    }

    /// `Block.MakePartSet(partSize)`.
    pub fn make_part_set(&self, part_size: u32) -> Result<PartSet> {
        let pbb = self.to_proto()?;
        let (bz, pos) = marshal_block_with_tx_positions(&pbb, self.data.txs.len())?;
        let mut ps = PartSet::from_data(&bz, part_size)?;
        ps.tx_pos = pos;
        Ok(ps)
    }

    /// `Block.Size`: encoded size (0 on encoding error).
    #[must_use]
    pub fn size(&self) -> usize {
        self.to_proto().map_or(0, |p| p.encoded_len())
    }

    /// `Block.ToProto`.
    pub fn to_proto(&self) -> Result<pb::Block> {
        Ok(pb::Block {
            header: Some(self.header.to_proto()),
            data: Some(self.data.to_proto()),
            evidence: Some(self.evidence.to_proto()?),
            last_commit: self.last_commit.as_ref().map(Commit::to_proto),
        })
    }

    /// Converts without `ValidateBasic` (each part still validates as in
    /// `BlockFromProto`).
    fn from_proto_parts(bp: &pb::Block) -> Result<Self> {
        let header = Header::from_proto(&bp.header.clone().unwrap_or_default())?;
        let data = Data::from_proto(&bp.data.clone().unwrap_or_default());
        let evidence = EvidenceData::from_proto(&bp.evidence.clone().unwrap_or_default())?;
        let last_commit = bp
            .last_commit
            .as_ref()
            .map(Commit::from_proto)
            .transpose()?;
        Ok(Self::new(header, data, evidence, last_commit))
    }

    /// `BlockFromProto`: converts and runs `ValidateBasic`.
    pub fn from_proto(bp: &pb::Block) -> Result<Self> {
        let mut b = Self::from_proto_parts(bp)?;
        b.validate_basic()?;
        Ok(b)
    }

    /// Decodes an encoded block and runs `BlockFromProto`, preserving
    /// whether `Data.hash` was present but empty.
    pub fn decode(bz: &[u8]) -> Result<Self> {
        let bp = pb::Block::decode(bz).map_err(|e| Error::msg(e.to_string()))?;
        let mut b = Self::from_proto_parts(&bp)?;
        if b.data.hash.is_none() && data_hash_field_present(bz) {
            b.data.hash = Some(Vec::new());
        }
        b.validate_basic()?;
        Ok(b)
    }
}

/// `MaxDataBytes(maxBytes, evidenceBytes, valsCount)`; Go panics where this
/// errors.
pub fn max_data_bytes(max_bytes: i64, evidence_bytes: i64, vals_count: usize) -> Result<i64> {
    let m = max_bytes
        .wrapping_sub(MAX_OVERHEAD_FOR_BLOCK)
        .wrapping_sub(MAX_HEADER_BYTES)
        .wrapping_sub(max_commit_bytes(vals_count))
        .wrapping_sub(evidence_bytes);
    if m < 0 {
        return Err(Error::msg(format!(
            "Negative MaxDataBytes. Block.MaxBytes={max_bytes} is too small to accommodate header&lastCommit&evidence={}",
            m.wrapping_sub(max_bytes).wrapping_neg()
        )));
    }
    Ok(m)
}

/// `MaxDataBytesNoEvidence(maxBytes, valsCount)`; Go panics where this
/// errors.
pub fn max_data_bytes_no_evidence(max_bytes: i64, vals_count: usize) -> Result<i64> {
    let m = max_bytes
        .wrapping_sub(MAX_OVERHEAD_FOR_BLOCK)
        .wrapping_sub(MAX_HEADER_BYTES)
        .wrapping_sub(max_commit_bytes(vals_count));
    if m < 0 {
        return Err(Error::msg(format!(
            "Negative MaxDataBytesNoEvidence. Block.MaxBytes={max_bytes} is too small to accommodate header&lastCommit&evidence={}",
            m.wrapping_sub(max_bytes).wrapping_neg()
        )));
    }
    Ok(m)
}

fn read_uvarint(b: &[u8]) -> Result<(u64, usize)> {
    let mut x = 0u64;
    let mut s = 0u32;
    for (i, &byte) in b.iter().enumerate() {
        if i == 10 {
            return Err(Error::msg("failed to read varint"));
        }
        if byte < 0x80 {
            if i == 9 && byte > 1 {
                return Err(Error::msg("failed to read varint"));
            }
            return Ok((x | (u64::from(byte) << s), i + 1));
        }
        x |= u64::from(byte & 0x7f) << s;
        s += 7;
    }
    Err(Error::msg("failed to read varint"))
}

/// `findField`: `(content_start, content_end)` of the first
/// length-delimited field `target`.
fn find_field(b: &[u8], target: u64) -> Result<(usize, usize)> {
    let mut off = 0usize;
    while off < b.len() {
        let (tag, n) = read_uvarint(&b[off..])?;
        off += n;
        match tag & 7 {
            2 => {
                let (len, n) = read_uvarint(b.get(off..).unwrap_or_default())?;
                off += n;
                let start = off;
                let end = off.saturating_add(usize::try_from(len).unwrap_or(usize::MAX));
                if tag >> 3 == target {
                    return Ok((start, end));
                }
                off = end;
            }
            0 => {
                let (_, n) = read_uvarint(b.get(off..).unwrap_or_default())?;
                off += n;
            }
            1 => off += 8,
            5 => off += 4,
            w => return Err(Error::msg(format!("unsupported wire type {w}"))),
        }
    }
    Err(Error::msg("field not found"))
}

/// True when the encoded block's `Data` carries a `hash` field (possibly
/// empty).
fn data_hash_field_present(bz: &[u8]) -> bool {
    let Ok((s, e)) = find_field(bz, 2) else {
        return false;
    };
    let Some(data) = bz.get(s..e) else {
        return false;
    };
    let mut off = 0;
    let mut present = false;
    while off < data.len() {
        let Ok((tag, n)) = read_uvarint(&data[off..]) else {
            return present;
        };
        off += n;
        match tag & 7 {
            2 => {
                let Ok((len, n)) = read_uvarint(data.get(off..).unwrap_or_default()) else {
                    return present;
                };
                off += n + usize::try_from(len).unwrap_or(usize::MAX / 2);
                if tag >> 3 == 6 {
                    present = true;
                }
            }
            0 => {
                let Ok((_, n)) = read_uvarint(data.get(off..).unwrap_or_default()) else {
                    return present;
                };
                off += n;
            }
            1 => off += 8,
            5 => off += 4,
            _ => return present,
        }
    }
    present
}

/// `MarshalBlockWithTxPositions`: encodes the block and records where each
/// `Data.txs` entry (tag through payload) sits in the encoding.
pub fn marshal_block_with_tx_positions(
    block: &pb::Block,
    txs_count: usize,
) -> Result<(Vec<u8>, Vec<TxPosition>)> {
    let b = block.encode_to_vec();
    let (data_start, data_end) = find_field(&b, 2)?;
    let content = b
        .get(data_start..data_end)
        .ok_or_else(|| Error::msg("field out of range"))?;
    let mut positions = vec![TxPosition::default(); txs_count];
    let to_u32 = |x: usize| u32::try_from(x).map_err(|_| Error::msg("integer overflow"));
    let mut off = 0usize;
    let mut tx_index = 0usize;
    while off < content.len() {
        let (tag, n) = read_uvarint(&content[off..])?;
        let field_start = off;
        off += n;
        match tag & 7 {
            2 => {
                let (len, n) = read_uvarint(content.get(off..).unwrap_or_default())?;
                off += n;
                off += usize::try_from(len).map_err(|_| Error::msg("integer overflow"))?;
                if tag >> 3 == 1 {
                    let pos = positions
                        .get_mut(tx_index)
                        .ok_or_else(|| Error::msg("index out of range"))?;
                    *pos = TxPosition {
                        start: to_u32(data_start + field_start)?,
                        end: to_u32(data_start + off)?,
                    };
                    tx_index += 1;
                }
            }
            0 => {
                let (_, n) = read_uvarint(content.get(off..).unwrap_or_default())?;
                off += n;
            }
            1 => off += 8,
            5 => off += 4,
            w => return Err(Error::msg(format!("unsupported wire type {w}"))),
        }
    }
    Ok((b, positions))
}
