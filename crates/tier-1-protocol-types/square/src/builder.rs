//! Original data square builder helpers.

use shares::{
    Blob, CompactShareSplitter, DEFAULT_SHARE_VERSION, FIRST_SPARSE_SHARE_CONTENT_SIZE,
    IndexWrapper, PAY_FOR_BLOB_NAMESPACE, ShareRange, SparseShareSplitter, TX_NAMESPACE,
    parse_blobs, parse_txs,
};

use crate::blob_tx::BlobTx;
use crate::inclusion::{blob_min_square_size, next_share_index, sub_tree_width};
use crate::square::{DEFAULT_SUBTREE_ROOT_THRESHOLD, Square, empty_square};
use crate::{Error, Result, write_square};

/// A blob plus builder accounting metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// Blob data to write into sparse shares.
    pub blob: Blob,
    /// PFB index after ordinary transactions are removed.
    pub pfb_index: usize,
    /// Blob index within its PFB.
    pub blob_index: usize,
    /// Number of sparse shares needed by the blob.
    pub num_shares: usize,
    /// Maximum namespace-padding shares this blob may require.
    pub max_padding: usize,
}

/// Result returned by [`build`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildResult {
    /// Constructed original data square.
    pub square: Square,
    /// Transactions included in the square, with ordinary txs before blob txs.
    pub included_txs: Vec<Vec<u8>>,
}

/// Incremental original data square builder.
#[derive(Debug, Clone)]
pub struct Builder {
    max_square_size: usize,
    current_size: usize,
    txs: Vec<Vec<u8>>,
    pfbs: Vec<IndexWrapper>,
    blobs: Vec<Element>,
    tx_counter: CompactShareSplitter,
    pfb_counter: CompactShareSplitter,
    done: bool,
    subtree_root_threshold: usize,
}

impl Builder {
    /// Construct a builder and append an exact ordered transaction list.
    pub fn new(
        max_square_size: usize,
        subtree_root_threshold: usize,
        txs: &[Vec<u8>],
    ) -> Result<Self> {
        if max_square_size == 0 {
            return Err(Error::InvalidSquareSize { actual: 0 });
        }
        if !crate::is_power_of_two(max_square_size) {
            return Err(Error::NotPowerOfTwo {
                actual: max_square_size,
            });
        }

        let mut builder = Self {
            max_square_size,
            current_size: 0,
            txs: Vec::new(),
            pfbs: Vec::new(),
            blobs: Vec::new(),
            tx_counter: CompactShareSplitter::new(TX_NAMESPACE, DEFAULT_SHARE_VERSION)?,
            pfb_counter: CompactShareSplitter::new(PAY_FOR_BLOB_NAMESPACE, DEFAULT_SHARE_VERSION)?,
            done: false,
            subtree_root_threshold,
        };

        let mut seen_blob_tx = false;
        for (index, tx) in txs.iter().enumerate() {
            if let Some(blob_tx) = BlobTx::decode(tx)? {
                seen_blob_tx = true;
                if !builder.append_blob_tx(blob_tx) {
                    return Err(Error::InvalidSquareSize { actual: index });
                }
            } else {
                if seen_blob_tx {
                    return Err(Error::NormalTxAfterBlobTx { index });
                }
                if !builder.append_tx(tx.clone()) {
                    return Err(Error::InvalidSquareSize { actual: index });
                }
            }
        }

        Ok(builder)
    }

    /// Attempt to append an ordinary transaction.
    pub fn append_tx(&mut self, tx: Vec<u8>) -> bool {
        let mut candidate = self.tx_counter.clone();
        if candidate.write_tx(&tx).is_err() {
            return false;
        }
        let len_change = candidate.count().saturating_sub(self.tx_counter.count());
        if self.can_fit(len_change) {
            self.tx_counter = candidate;
            self.txs.push(tx);
            self.current_size += len_change;
            self.done = false;
            return true;
        }
        false
    }

    /// Attempt to append a blob transaction.
    pub fn append_blob_tx(&mut self, blob_tx: BlobTx) -> bool {
        if blob_tx.validate_basic().is_err() {
            return false;
        }

        let pfb_index = self.pfbs.len();
        let index_wrapper =
            IndexWrapper::new(blob_tx.tx, worst_case_share_indexes(blob_tx.blobs.len()));
        let Ok(index_wrapper_bytes) = index_wrapper.encode() else {
            return false;
        };

        let mut pfb_candidate = self.pfb_counter.clone();
        if pfb_candidate.write_tx(&index_wrapper_bytes).is_err() {
            return false;
        }
        let pfb_share_diff = pfb_candidate
            .count()
            .saturating_sub(self.pfb_counter.count());

        let mut elements = Vec::with_capacity(blob_tx.blobs.len());
        let mut max_blob_share_count = 0usize;
        for (blob_index, blob) in blob_tx.blobs.into_iter().enumerate() {
            let Ok(element) =
                Element::new(blob, pfb_index, blob_index, self.subtree_root_threshold)
            else {
                return false;
            };
            let Some(next_count) = max_blob_share_count.checked_add(element.max_share_offset())
            else {
                return false;
            };
            max_blob_share_count = next_count;
            elements.push(element);
        }

        let Some(size_change) = pfb_share_diff.checked_add(max_blob_share_count) else {
            return false;
        };
        if self.can_fit(size_change) {
            self.pfb_counter = pfb_candidate;
            self.pfbs.push(index_wrapper);
            self.blobs.extend(elements);
            self.current_size += size_change;
            self.done = false;
            return true;
        }
        false
    }

    /// Construct the current original data square.
    pub fn export(&mut self) -> Result<Square> {
        if self.is_empty() {
            self.done = true;
            return Ok(empty_square());
        }

        let square_size = blob_min_square_size(self.current_size)?;
        if square_size > self.max_square_size {
            return Err(Error::InvalidSquareSize {
                actual: square_size,
            });
        }

        self.blobs
            .sort_by_key(|element| element.blob.namespace().into_bytes());

        let mut tx_writer = CompactShareSplitter::new(TX_NAMESPACE, DEFAULT_SHARE_VERSION)?;
        for tx in &self.txs {
            tx_writer.write_tx(tx)?;
        }

        let mut non_reserved_start = self
            .tx_counter
            .count()
            .checked_add(self.pfb_counter.count())
            .ok_or(Error::ArithmeticOverflow)?;
        let mut cursor = non_reserved_start;
        let mut end_of_last_blob = non_reserved_start;
        let mut blob_writer = SparseShareSplitter::new();

        for (index, element) in self.blobs.iter().enumerate() {
            cursor = next_share_index(cursor, element.num_shares, self.subtree_root_threshold)?;
            if index == 0 {
                non_reserved_start = cursor;
            }

            let padding = cursor
                .checked_sub(end_of_last_blob)
                .ok_or(Error::ArithmeticOverflow)?;
            if padding > element.max_padding {
                return Err(Error::BuilderInvariant("blob padding exceeds max padding"));
            }

            let share_index =
                u32::try_from(cursor).map_err(|_| Error::ShareIndexOverflow { actual: cursor })?;
            self.pfbs[element.pfb_index].share_indexes[element.blob_index] = share_index;

            if index > 0 {
                blob_writer.write_namespace_padding_shares(padding)?;
            }
            blob_writer.write(&element.blob)?;
            cursor = cursor
                .checked_add(element.num_shares)
                .ok_or(Error::ArithmeticOverflow)?;
            end_of_last_blob = cursor;
        }

        let mut pfb_writer =
            CompactShareSplitter::new(PAY_FOR_BLOB_NAMESPACE, DEFAULT_SHARE_VERSION)?;
        for pfb in &self.pfbs {
            pfb_writer.write_tx(&pfb.encode()?)?;
        }
        if self.pfb_counter.count() < pfb_writer.count() {
            return Err(Error::BuilderInvariant(
                "pfb counter smaller than exported pfb writer",
            ));
        }

        let square = write_square(
            tx_writer,
            pfb_writer,
            blob_writer,
            non_reserved_start,
            square_size,
        )?;
        self.done = true;
        Ok(square)
    }

    /// Return the share range occupied by the transaction at `tx_index`.
    pub fn find_tx_share_range(&mut self, tx_index: usize) -> Result<ShareRange> {
        if tx_index >= self.num_txs() {
            return Err(Error::TxIndexOutOfRange { index: tx_index });
        }
        if !self.done {
            self.export()?;
        }

        let mut ordered_txs = self.txs.clone();
        for pfb in &self.pfbs {
            ordered_txs.push(pfb.encode()?);
        }
        let target = ordered_txs
            .get(tx_index)
            .ok_or(Error::TxIndexOutOfRange { index: tx_index })?;
        let ranges = shares::split_txs(&ordered_txs)?.share_ranges;
        ranges
            .get(&hash::sha256(target))
            .copied()
            .ok_or(Error::TxIndexOutOfRange { index: tx_index })
    }

    /// Return the starting share index of a blob referenced by a PFB tx index.
    pub fn find_blob_starting_index(
        &mut self,
        pfb_index: usize,
        blob_index: usize,
    ) -> Result<usize> {
        let pfb_index = self.pfb_offset(pfb_index)?;
        if !self.done {
            self.export()?;
        }
        let pfb = self
            .pfbs
            .get(pfb_index)
            .ok_or(Error::TxIndexOutOfRange { index: pfb_index })?;
        let share_index = pfb
            .share_indexes
            .get(blob_index)
            .ok_or(Error::BlobIndexOutOfRange { index: blob_index })?;
        Ok(*share_index as usize)
    }

    /// Return the number of sparse shares occupied by a blob.
    pub fn blob_share_length(&self, pfb_index: usize, blob_index: usize) -> Result<usize> {
        let pfb_index = self.pfb_offset(pfb_index)?;
        self.blobs
            .iter()
            .find(|blob| blob.pfb_index == pfb_index && blob.blob_index == blob_index)
            .map(|blob| blob.num_shares)
            .ok_or(Error::BlobIndexOutOfRange { index: blob_index })
    }

    /// Return the wrapped PFB transaction at `tx_index`.
    pub fn get_wrapped_pfb(&mut self, tx_index: usize) -> Result<IndexWrapper> {
        let pfb_index = self.pfb_offset(tx_index)?;
        if !self.done {
            self.export()?;
        }
        self.pfbs
            .get(pfb_index)
            .cloned()
            .ok_or(Error::TxIndexOutOfRange { index: tx_index })
    }

    /// Return the builder's current worst-case share count.
    #[must_use]
    pub const fn current_size(&self) -> usize {
        self.current_size
    }

    /// Return the subtree root threshold.
    #[must_use]
    pub const fn subtree_root_threshold(&self) -> usize {
        self.subtree_root_threshold
    }

    /// Return the number of wrapped PFBs.
    #[must_use]
    pub fn num_pfbs(&self) -> usize {
        self.pfbs.len()
    }

    /// Return the total number of ordinary and blob transactions.
    #[must_use]
    pub fn num_txs(&self) -> usize {
        self.txs.len() + self.pfbs.len()
    }

    /// Return true when the builder has no compact tx or PFB shares.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tx_counter.count() == 0 && self.pfb_counter.count() == 0
    }

    fn can_fit(&self, share_count: usize) -> bool {
        let Some(next_size) = self.current_size.checked_add(share_count) else {
            return false;
        };
        let Some(capacity) = self.max_square_size.checked_mul(self.max_square_size) else {
            return false;
        };
        next_size <= capacity
    }

    fn pfb_offset(&self, tx_index: usize) -> Result<usize> {
        if tx_index < self.txs.len() {
            return Err(Error::NotPayForBlob { index: tx_index });
        }
        let pfb_index = tx_index - self.txs.len();
        if pfb_index >= self.pfbs.len() {
            return Err(Error::TxIndexOutOfRange { index: tx_index });
        }
        Ok(pfb_index)
    }
}

impl Element {
    fn new(
        blob: Blob,
        pfb_index: usize,
        blob_index: usize,
        subtree_root_threshold: usize,
    ) -> Result<Self> {
        let num_shares = sparse_shares_needed(blob.data_len());
        let max_padding = sub_tree_width(num_shares, subtree_root_threshold)?
            .checked_sub(1)
            .ok_or(Error::ArithmeticOverflow)?;
        Ok(Self {
            blob,
            pfb_index,
            blob_index,
            num_shares,
            max_padding,
        })
    }

    fn max_share_offset(&self) -> usize {
        self.num_shares + self.max_padding
    }
}

/// Build a square from prioritized transactions, dropping txs that do not fit.
pub fn build(
    txs: &[Vec<u8>],
    max_square_size: usize,
    subtree_root_threshold: usize,
) -> Result<BuildResult> {
    let mut builder = Builder::new(max_square_size, subtree_root_threshold, &[])?;
    let mut normal_txs = Vec::new();
    let mut blob_txs = Vec::new();

    for tx in txs {
        if let Some(blob_tx) = BlobTx::decode(tx)? {
            if builder.append_blob_tx(blob_tx) {
                blob_txs.push(tx.clone());
            }
        } else if builder.append_tx(tx.clone()) {
            normal_txs.push(tx.clone());
        }
    }

    let square = builder.export()?;
    normal_txs.extend(blob_txs);
    Ok(BuildResult {
        square,
        included_txs: normal_txs,
    })
}

/// Construct a square from an exact ordered transaction list.
pub fn construct(
    txs: &[Vec<u8>],
    max_square_size: usize,
    subtree_root_threshold: usize,
) -> Result<Square> {
    let mut builder = Builder::new(max_square_size, subtree_root_threshold, txs)?;
    builder.export()
}

/// Deconstruct a square back into ordered transaction bytes.
pub fn deconstruct<D>(square: &Square, decoder: D) -> Result<Vec<Vec<u8>>>
where
    D: Fn(&[u8]) -> Result<Vec<u32>>,
{
    if square.is_empty_square() {
        return Ok(Vec::new());
    }

    let shares = square.as_shares();
    let tx_range = shares::get_share_range_for_namespace(shares, TX_NAMESPACE)?;
    if tx_range.start != 0 {
        return Err(Error::BuilderInvariant("tx shares must start at index 0"));
    }

    let mut pfb_range =
        shares::get_share_range_for_namespace(&shares[tx_range.end..], PAY_FOR_BLOB_NAMESPACE)?;
    if pfb_range.is_empty() {
        return Ok(parse_txs(&shares[tx_range.start..tx_range.end])?);
    }
    if pfb_range.start != 0 {
        return Err(Error::BuilderInvariant("PFB shares must follow tx shares"));
    }
    pfb_range.add(tx_range.end)?;

    let mut txs = parse_txs(&shares[tx_range.start..tx_range.end])?;
    let wrapped_pfbs = parse_txs(&shares[pfb_range.start..pfb_range.end])?;

    for (index, wrapped_pfb) in wrapped_pfbs.iter().enumerate() {
        let wrapped_pfb = IndexWrapper::decode(wrapped_pfb)?.ok_or(Error::InvalidBlobTx)?;
        if wrapped_pfb.share_indexes.is_empty() {
            return Err(Error::InvalidBlobTx);
        }
        let blob_sizes = decoder(&wrapped_pfb.tx)?;
        if blob_sizes.len() != wrapped_pfb.share_indexes.len() {
            return Err(Error::BuilderInvariant("PFB blob size count mismatch"));
        }

        let mut blobs = Vec::with_capacity(wrapped_pfb.share_indexes.len());
        for (&share_index, &blob_size) in wrapped_pfb.share_indexes.iter().zip(&blob_sizes) {
            let start = share_index as usize;
            let share_len = sparse_shares_needed(blob_size as usize);
            let end = start
                .checked_add(share_len)
                .ok_or(Error::ArithmeticOverflow)?;
            if end > shares.len() {
                return Err(Error::BlobIndexOutOfRange { index });
            }
            let parsed = parse_blobs(&shares[start..end])?;
            let [blob] = parsed.as_slice() else {
                return Err(Error::InvalidBlobTx);
            };
            blobs.push(blob.clone());
        }

        txs.push(BlobTx::new(wrapped_pfb.tx, blobs)?.encode()?);
    }

    Ok(txs)
}

/// Return the share range occupied by a transaction.
pub fn tx_share_range(
    txs: &[Vec<u8>],
    tx_index: usize,
    max_square_size: usize,
    subtree_root_threshold: usize,
) -> Result<ShareRange> {
    let mut builder = Builder::new(max_square_size, subtree_root_threshold, txs)?;
    builder.find_tx_share_range(tx_index)
}

/// Return the share range occupied by a blob.
pub fn blob_share_range(
    txs: &[Vec<u8>],
    tx_index: usize,
    blob_index: usize,
    max_square_size: usize,
    subtree_root_threshold: usize,
) -> Result<ShareRange> {
    let mut builder = Builder::new(max_square_size, subtree_root_threshold, txs)?;
    let start = builder.find_blob_starting_index(tx_index, blob_index)?;
    let len = builder.blob_share_length(tx_index, blob_index)?;
    Ok(ShareRange::new(start, start + len))
}

/// Return largest share indexes used for protobuf worst-case PFB size estimates.
#[must_use]
pub fn worst_case_share_indexes(blob_count: usize) -> Vec<u32> {
    const WORST_CASE_SHARE_INDEX: u32 = 16_384;
    vec![WORST_CASE_SHARE_INDEX; blob_count]
}

fn sparse_shares_needed(data_len: usize) -> usize {
    if data_len <= FIRST_SPARSE_SHARE_CONTENT_SIZE {
        return 1;
    }
    1 + (data_len - FIRST_SPARSE_SHARE_CONTENT_SIZE)
        .div_ceil(shares::CONTINUATION_SPARSE_SHARE_CONTENT_SIZE)
}

#[allow(dead_code)]
const _: usize = DEFAULT_SUBTREE_ROOT_THRESHOLD;
