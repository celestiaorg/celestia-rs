//! go-square v4.0.1 square construction (celestia-app v10).
//!
//! Differences from the v1 [`crate::Builder`]: transactions are classified
//! by the caller ([`ClassifiedTx`]), pay-for-fibre transactions get their
//! own compact namespace and a share-version-two system blob, PFB share
//! indexes are estimated with `512 * 512` (the v4 `worstCaseShareIndexes`),
//! blob share counts account for signers, compact share counts use
//! `CompactShareCounter`, and the last appended tx of each kind can be
//! reverted (`FilteredSquareBuilder`).

use shares::{
    Blob, CONTINUATION_COMPACT_SHARE_CONTENT_SIZE, CONTINUATION_SPARSE_SHARE_CONTENT_SIZE,
    CompactShareSplitter, FIRST_COMPACT_SHARE_CONTENT_SIZE, FIRST_SPARSE_SHARE_CONTENT_SIZE,
    FIRST_SPARSE_SHARE_CONTENT_SIZE_WITH_SIGNER, IndexWrapper, PAY_FOR_BLOB_NAMESPACE,
    PAY_FOR_FIBRE_NAMESPACE, SHARE_VERSION_TWO, SHARE_VERSION_ZERO, SparseShareSplitter,
    TX_NAMESPACE, reserved_padding_shares, tail_padding_shares,
};

use crate::inclusion::{blob_min_square_size, is_power_of_two, next_share_index, sub_tree_width};
use crate::square::{Square, empty_square};
use crate::{Error, Result};

/// go-square `squareSizeUpperBound`.
pub const SQUARE_SIZE_UPPER_BOUND: usize = 512;

/// A decoded go-square `BlobTx`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobTx {
    /// Inner SDK tx bytes.
    pub tx: Vec<u8>,
    /// Blobs.
    pub blobs: Vec<Blob>,
}

/// go-square `tx.FibreTx`: a `MsgPayForFibre` tx with its system blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FibreTx {
    /// Raw SDK tx bytes.
    pub tx: Vec<u8>,
    /// Share-version-two system blob.
    pub system_blob: Blob,
}

/// go-square `ClassifiedTx`, with the blob tx already unmarshalled by the
/// caller (go-square's `tx.UnmarshalBlobTx`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassifiedTx {
    /// A normal tx.
    Normal(Vec<u8>),
    /// A canonical `BlobTx`.
    Blob(BlobTx),
    /// A pay-for-fibre tx.
    Fibre(FibreTx),
}

impl ClassifiedTx {
    /// `ClassifiedTx.Validate` (the blob-tx check is the caller's).
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Normal(b) if b.is_empty() => {
                Err(Error::Decode("classified tx has no bytes".into()))
            }
            Self::Fibre(f) if f.tx.is_empty() => {
                Err(Error::Decode("classified tx has no bytes".into()))
            }
            Self::Fibre(f) if f.system_blob.share_version() != SHARE_VERSION_TWO => Err(
                Error::Decode("system blobs must use share version 2".into()),
            ),
            _ => Ok(()),
        }
    }
}

/// go-square `share.CompactShareCounter`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompactShareCounter {
    last_shares: usize,
    last_remainder: usize,
    shares: usize,
    remainder: usize,
}

fn delim_len(n: usize) -> usize {
    encoding::varint::encode_uvarint(n as u64).len()
}

impl CompactShareCounter {
    /// Adds a unit of `data_len` bytes; returns the share count increase.
    pub fn add(&mut self, data_len: usize) -> usize {
        let mut data_len = data_len + delim_len(data_len);
        self.last_remainder = self.remainder;
        self.last_shares = self.shares;
        if self.shares == 0 {
            if data_len >= FIRST_COMPACT_SHARE_CONTENT_SIZE - self.remainder {
                data_len -= FIRST_COMPACT_SHARE_CONTENT_SIZE - self.remainder;
                self.shares += 1;
                self.remainder = 0;
            } else {
                self.remainder += data_len;
                data_len = 0;
            }
        }
        if data_len >= CONTINUATION_COMPACT_SHARE_CONTENT_SIZE - self.remainder {
            data_len -= CONTINUATION_COMPACT_SHARE_CONTENT_SIZE - self.remainder;
            self.shares += 1;
            self.remainder = 0;
        } else {
            self.remainder += data_len;
            data_len = 0;
        }
        if data_len > 0 {
            self.shares += data_len / CONTINUATION_COMPACT_SHARE_CONTENT_SIZE;
            self.remainder = data_len % CONTINUATION_COMPACT_SHARE_CONTENT_SIZE;
        }
        let mut diff = self.shares - self.last_shares;
        if self.last_remainder == 0 && self.remainder > 0 {
            diff += 1;
        } else if self.last_remainder > 0 && self.remainder == 0 {
            diff -= 1;
        }
        diff
    }

    /// Reverts the last `add`.
    pub fn revert(&mut self) {
        self.shares = self.last_shares;
        self.remainder = self.last_remainder;
    }

    /// Shares counted so far.
    #[must_use]
    pub fn size(&self) -> usize {
        if self.remainder == 0 {
            self.shares
        } else {
            self.shares + 1
        }
    }

    /// Bytes used in the last share.
    #[must_use]
    pub fn remainder(&self) -> usize {
        self.remainder
    }
}

/// go-square `share.SparseSharesNeeded`.
#[must_use]
pub fn sparse_shares_needed(sequence_len: usize, contains_signer: bool) -> usize {
    if sequence_len == 0 {
        return 0;
    }
    let first = if contains_signer {
        FIRST_SPARSE_SHARE_CONTENT_SIZE_WITH_SIGNER
    } else {
        FIRST_SPARSE_SHARE_CONTENT_SIZE
    };
    if sequence_len <= first {
        return 1;
    }
    1 + (sequence_len - first).div_ceil(CONTINUATION_SPARSE_SHARE_CONTENT_SIZE)
}

/// A blob placed by the builder (`square.Element`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// The blob.
    pub blob: Blob,
    /// Index of the PFB paying for it (`None` for fibre system blobs).
    pub pfb_index: Option<usize>,
    /// Index of the blob within its PFB.
    pub blob_index: usize,
    /// Shares the blob occupies.
    pub num_shares: usize,
    /// Worst-case namespace padding before it.
    pub max_padding: usize,
}

impl Element {
    fn new(
        blob: Blob,
        pfb_index: Option<usize>,
        blob_index: usize,
        threshold: usize,
    ) -> Result<Self> {
        let num_shares = sparse_shares_needed(blob.data_len(), blob.has_signer());
        let width = sub_tree_width(num_shares, threshold)?;
        Ok(Self {
            blob,
            pfb_index,
            blob_index,
            num_shares,
            max_padding: width - 1,
        })
    }

    fn max_share_offset(&self) -> usize {
        self.num_shares + self.max_padding
    }
}

/// go-square v4 `square.Builder`.
#[derive(Debug, Clone)]
pub struct Builder {
    max_square_size: usize,
    current_size: usize,
    txs: Vec<Vec<u8>>,
    pfbs: Vec<IndexWrapper>,
    pff_txs: Vec<Vec<u8>>,
    blobs: Vec<Element>,
    tx_counter: CompactShareCounter,
    pfb_counter: CompactShareCounter,
    pff_counter: CompactShareCounter,
    last_tx_size_change: usize,
    last_blob_tx_size_change: usize,
    last_pff_size_change: usize,
    tx_reverted: bool,
    blob_tx_reverted: bool,
    pff_reverted: bool,
    subtree_root_threshold: usize,
}

/// `worstCaseShareIndexes`.
fn worst_case_share_indexes(n: usize) -> Vec<u32> {
    #[allow(clippy::cast_possible_truncation)]
    const WORST: u32 = (SQUARE_SIZE_UPPER_BOUND * SQUARE_SIZE_UPPER_BOUND) as u32;
    vec![WORST; n]
}

impl Builder {
    /// `NewBuilder` (without initial txs).
    pub fn new(max_square_size: usize, subtree_root_threshold: usize) -> Result<Self> {
        if max_square_size == 0 || max_square_size > SQUARE_SIZE_UPPER_BOUND {
            return Err(Error::InvalidSquareSize {
                actual: max_square_size,
            });
        }
        if !is_power_of_two(max_square_size) {
            return Err(Error::NotPowerOfTwo {
                actual: max_square_size,
            });
        }
        Ok(Self {
            max_square_size,
            current_size: 0,
            txs: Vec::new(),
            pfbs: Vec::new(),
            pff_txs: Vec::new(),
            blobs: Vec::new(),
            tx_counter: CompactShareCounter::default(),
            pfb_counter: CompactShareCounter::default(),
            pff_counter: CompactShareCounter::default(),
            last_tx_size_change: 0,
            last_blob_tx_size_change: 0,
            last_pff_size_change: 0,
            tx_reverted: false,
            blob_tx_reverted: false,
            pff_reverted: false,
            subtree_root_threshold,
        })
    }

    fn can_fit(&self, n: usize) -> bool {
        self.current_size + n <= self.max_square_size * self.max_square_size
    }

    /// `AppendTx`.
    pub fn append_tx(&mut self, tx: &[u8]) -> bool {
        let change = self.tx_counter.add(tx.len());
        if self.can_fit(change) {
            self.txs.push(tx.to_vec());
            self.current_size += change;
            self.last_tx_size_change = change;
            self.tx_reverted = false;
            return true;
        }
        self.tx_counter.revert();
        false
    }

    /// `RevertLastTx`.
    pub fn revert_last_tx(&mut self) -> Result<()> {
        if self.txs.is_empty() || self.tx_reverted {
            return Err(Error::BuilderInvariant("no transaction to revert"));
        }
        self.txs.pop();
        self.tx_counter.revert();
        self.current_size -= self.last_tx_size_change;
        self.tx_reverted = true;
        Ok(())
    }

    /// `AppendBlobTx`.
    pub fn append_blob_tx(&mut self, blob_tx: &BlobTx) -> Result<bool> {
        let iw = IndexWrapper::new(
            blob_tx.tx.clone(),
            worst_case_share_indexes(blob_tx.blobs.len()),
        );
        let size = iw.encode()?.len();
        let pfb_share_diff = self.pfb_counter.add(size);
        let mut elements = Vec::with_capacity(blob_tx.blobs.len());
        let mut max_blob_share_count = 0usize;
        for (idx, blob) in blob_tx.blobs.iter().enumerate() {
            let element = match Element::new(
                blob.clone(),
                Some(self.pfbs.len()),
                idx,
                self.subtree_root_threshold,
            ) {
                Ok(e) => e,
                Err(e) => {
                    self.pfb_counter.revert();
                    return Err(e);
                }
            };
            max_blob_share_count += element.max_share_offset();
            elements.push(element);
        }
        let total = pfb_share_diff + max_blob_share_count;
        if self.can_fit(total) {
            self.blobs.extend(elements);
            self.pfbs.push(iw);
            self.current_size += total;
            self.last_blob_tx_size_change = total;
            self.blob_tx_reverted = false;
            return Ok(true);
        }
        self.pfb_counter.revert();
        Ok(false)
    }

    /// `RevertLastBlobTx`.
    pub fn revert_last_blob_tx(&mut self) -> Result<()> {
        if self.pfbs.is_empty() || self.blob_tx_reverted {
            return Err(Error::BuilderInvariant("no blob transaction to revert"));
        }
        let last = self.pfbs.len() - 1;
        self.blobs.retain(|b| b.pfb_index != Some(last));
        self.pfbs.pop();
        self.pfb_counter.revert();
        self.current_size -= self.last_blob_tx_size_change;
        self.blob_tx_reverted = true;
        Ok(())
    }

    /// `AppendFibreTx`.
    pub fn append_fibre_tx(&mut self, fibre: &FibreTx) -> Result<bool> {
        if fibre.system_blob.share_version() != SHARE_VERSION_TWO {
            return Err(Error::Decode(
                "system blobs must use ShareVersionTwo".into(),
            ));
        }
        let diff = self.pff_counter.add(fibre.tx.len());
        let element = match Element::new(
            fibre.system_blob.clone(),
            None,
            0,
            self.subtree_root_threshold,
        ) {
            Ok(e) => e,
            Err(e) => {
                self.pff_counter.revert();
                return Err(e);
            }
        };
        let total = diff + element.max_share_offset();
        if self.can_fit(total) {
            self.pff_txs.push(fibre.tx.clone());
            self.blobs.push(element);
            self.current_size += total;
            self.last_pff_size_change = total;
            self.pff_reverted = false;
            return Ok(true);
        }
        self.pff_counter.revert();
        Ok(false)
    }

    /// `RevertLastPayForFibreTx`.
    pub fn revert_last_pay_for_fibre_tx(&mut self) -> Result<()> {
        if self.pff_txs.is_empty() || self.pff_reverted {
            return Err(Error::BuilderInvariant(
                "no pay-for-fibre transaction to revert",
            ));
        }
        self.pff_txs.pop();
        if let Some(i) = self.blobs.iter().rposition(|b| b.pfb_index.is_none()) {
            self.blobs.remove(i);
        }
        self.pff_counter.revert();
        self.current_size -= self.last_pff_size_change;
        self.pff_reverted = true;
        Ok(())
    }

    /// `IsEmpty`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tx_counter.size() == 0 && self.pfb_counter.size() == 0 && self.pff_counter.size() == 0
    }

    /// Current worst-case share count.
    #[must_use]
    pub fn current_size(&self) -> usize {
        self.current_size
    }

    /// `Export`.
    pub fn export(&mut self) -> Result<Square> {
        if self.is_empty() {
            return Ok(empty_square());
        }
        let square_size = blob_min_square_size(self.current_size)?;
        // sort.SliceStable by namespace (sort_by is stable).
        self.blobs.sort_by(|a, b| {
            a.blob
                .namespace()
                .as_bytes()
                .cmp(b.blob.namespace().as_bytes())
        });

        let mut tx_writer = CompactShareSplitter::new(TX_NAMESPACE, SHARE_VERSION_ZERO)?;
        for tx in &self.txs {
            tx_writer.write_tx(tx)?;
        }
        let mut pff_writer =
            CompactShareSplitter::new(PAY_FOR_FIBRE_NAMESPACE, SHARE_VERSION_ZERO)?;
        for tx in &self.pff_txs {
            pff_writer.write_tx(tx)?;
        }

        let mut non_reserved_start =
            self.tx_counter.size() + self.pfb_counter.size() + self.pff_counter.size();
        let mut cursor = non_reserved_start;
        let mut end_of_last_blob = non_reserved_start;
        let mut blob_writer = SparseShareSplitter::new();
        for (i, element) in self.blobs.iter().enumerate() {
            cursor = next_share_index(cursor, element.num_shares, self.subtree_root_threshold)?;
            if i == 0 {
                non_reserved_start = cursor;
            }
            let padding = cursor - end_of_last_blob;
            if padding > element.max_padding {
                return Err(Error::BuilderInvariant("blob padding exceeds max padding"));
            }
            if let Some(pfb) = element.pfb_index {
                let idx = u32::try_from(cursor)
                    .map_err(|_| Error::ShareIndexOverflow { actual: cursor })?;
                self.pfbs[pfb].share_indexes[element.blob_index] = idx;
            }
            if i > 0 {
                blob_writer.write_namespace_padding_shares(padding)?;
            }
            blob_writer.write(&element.blob)?;
            cursor += element.num_shares;
            end_of_last_blob = cursor;
        }

        let mut pfb_writer = CompactShareSplitter::new(PAY_FOR_BLOB_NAMESPACE, SHARE_VERSION_ZERO)?;
        for iw in &self.pfbs {
            pfb_writer.write_tx(&iw.encode()?)?;
        }
        if self.pff_counter.size() < pff_writer.count() {
            return Err(Error::BuilderInvariant(
                "pay-for-fibre counter smaller than writer",
            ));
        }
        if self.pfb_counter.size() < pfb_writer.count() {
            return Err(Error::BuilderInvariant("pfb counter smaller than writer"));
        }
        write_square(
            tx_writer,
            pfb_writer,
            pff_writer,
            &blob_writer,
            non_reserved_start,
            square_size,
        )
    }
}

/// go-square v4 `WriteSquare`.
pub fn write_square(
    mut tx_writer: CompactShareSplitter,
    mut pfb_writer: CompactShareSplitter,
    mut pff_writer: CompactShareSplitter,
    blob_writer: &SparseShareSplitter,
    non_reserved_start: usize,
    square_size: usize,
) -> Result<Square> {
    let total = square_size * square_size;
    let blob_tx_shares_start = tx_writer.count();
    let fibre_tx_shares_start = blob_tx_shares_start + pfb_writer.count();
    let padding_start = fibre_tx_shares_start + pff_writer.count();
    if non_reserved_start < padding_start {
        return Err(Error::InvalidSquareSize {
            actual: non_reserved_start,
        });
    }
    let end_of_last_blob = non_reserved_start + blob_writer.count();
    if total < end_of_last_blob {
        return Err(Error::InvalidSquareSize {
            actual: square_size,
        });
    }
    let mut shares = Vec::with_capacity(total);
    shares.extend(tx_writer.export()?);
    shares.extend(pfb_writer.export()?);
    shares.extend(pff_writer.export()?);
    if blob_writer.count() > 0 {
        shares.extend(reserved_padding_shares(non_reserved_start - padding_start));
        shares.extend(blob_writer.export());
    }
    // Without blobs `non_reserved_start == padding_start`, so the shares
    // written so far end exactly at `end_of_last_blob`.
    if shares.len() < total {
        shares.extend(tail_padding_shares(total - shares.len()));
    }
    if shares.len() != total {
        return Err(Error::InvalidSquareSize {
            actual: shares.len(),
        });
    }
    Ok(Square::from_shares_unchecked(shares))
}

/// `validateTxOrdering`: normal txs, then blob txs, then pay-for-fibre txs.
pub fn validate_tx_ordering(txs: &[ClassifiedTx]) -> Result<()> {
    let (mut seen_blob, mut seen_fibre) = (false, false);
    for (index, tx) in txs.iter().enumerate() {
        tx.validate()?;
        match tx {
            ClassifiedTx::Fibre(_) => seen_fibre = true,
            ClassifiedTx::Blob(_) => {
                if seen_fibre {
                    return Err(Error::Decode(format!(
                        "blob tx at index {index} cannot be appended after pay-for-fibre tx"
                    )));
                }
                seen_blob = true;
            }
            ClassifiedTx::Normal(_) => {
                if seen_blob {
                    return Err(Error::NormalTxAfterBlobTx { index });
                }
                if seen_fibre {
                    return Err(Error::Decode(format!(
                        "normal tx at index {index} cannot be appended after pay-for-fibre tx"
                    )));
                }
            }
        }
    }
    Ok(())
}

/// go-square v4 `Construct`.
pub fn construct(
    txs: &[ClassifiedTx],
    max_square_size: usize,
    subtree_root_threshold: usize,
) -> Result<Square> {
    validate_tx_ordering(txs)?;
    let mut builder = Builder::new(max_square_size, subtree_root_threshold)?;
    for (index, tx) in txs.iter().enumerate() {
        let added = match tx {
            ClassifiedTx::Fibre(f) => builder.append_fibre_tx(f)?,
            ClassifiedTx::Blob(b) => builder.append_blob_tx(b)?,
            ClassifiedTx::Normal(t) => builder.append_tx(t),
        };
        if !added {
            return Err(Error::InvalidSquareSize { actual: index });
        }
    }
    builder.export()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_matches_splitter() -> Result<()> {
        let mut c = CompactShareCounter::default();
        let mut w = CompactShareSplitter::new(TX_NAMESPACE, 0)?;
        for len in [1usize, 400, 1000, 5, 474, 478, 2000, 3, 90_000, 127, 128] {
            c.add(len);
            w.write_tx(&vec![7u8; len])?;
            assert_eq!(c.size(), w.count(), "len {len}");
        }
        Ok(())
    }

    #[test]
    fn empty_builder_exports_empty_square() -> Result<()> {
        let mut b = Builder::new(64, 64)?;
        assert_eq!(b.export()?, empty_square());
        assert!(Builder::new(1024, 64).is_err());
        assert!(Builder::new(48, 64).is_err());
        Ok(())
    }
}
