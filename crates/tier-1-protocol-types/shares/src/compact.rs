//! Compact transaction share splitting helpers.

use std::collections::BTreeMap;

use crate::share::{DEFAULT_SHARE_VERSION, FIRST_COMPACT_SHARE_CONTENT_SIZE};
use crate::{
    Error, Namespace, PAY_FOR_BLOB_NAMESPACE, Result, Share, ShareBuilder, ShareRange, TX_NAMESPACE,
};
use encoding::varint;

/// Incremental compact share splitter for length-delimited transactions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactShareSplitter {
    shares: Vec<Share>,
    share_builder: ShareBuilder,
    namespace: Namespace,
    done: bool,
    share_version: u8,
    share_ranges: BTreeMap<[u8; 32], ShareRange>,
}

/// Result of splitting ordinary and `PayForBlob` transactions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitTxsResult {
    /// Shares containing ordinary transactions.
    pub tx_shares: Vec<Share>,
    /// Shares containing `PayForBlob` index-wrapper transactions.
    pub pfb_shares: Vec<Share>,
    /// Share ranges keyed by `sha256(tx)`.
    pub share_ranges: BTreeMap<[u8; 32], ShareRange>,
}

impl CompactShareSplitter {
    /// Construct a compact share splitter.
    pub fn new(namespace: Namespace, share_version: u8) -> Result<Self> {
        if !namespace.is_tx() && !namespace.is_pay_for_blob() && !namespace.is_pay_for_fibre() {
            return Err(Error::InvalidCompactShare);
        }
        Ok(Self {
            shares: Vec::new(),
            share_builder: ShareBuilder::new(namespace, share_version, true)?,
            namespace,
            done: false,
            share_version,
            share_ranges: BTreeMap::new(),
        })
    }

    /// Write one transaction into the splitter.
    pub fn write_tx(&mut self, tx: &[u8]) -> Result<()> {
        let raw_data = marshal_delimited_tx(tx);
        let start_share = self.shares.len();
        self.write(&raw_data)?;
        let end_share = self.count();
        self.share_ranges
            .insert(hash::sha256(tx), ShareRange::new(start_share, end_share));
        Ok(())
    }

    /// Finalize and return compact shares.
    pub fn export(&mut self) -> Result<Vec<Share>> {
        if self.is_empty() {
            return Ok(Vec::new());
        }
        if self.done {
            return Ok(self.shares.clone());
        }

        let mut bytes_of_padding = 0;
        if !self.share_builder.is_empty_share() {
            bytes_of_padding = self.share_builder.zero_pad_if_necessary();
            self.stack_pending()?;
        }
        let sequence_len = self.sequence_len(bytes_of_padding)?;
        self.write_sequence_len(sequence_len)?;
        self.done = true;
        Ok(self.shares.clone())
    }

    /// Return the share count that would be produced by export.
    #[must_use]
    pub fn count(&self) -> usize {
        if !self.share_builder.is_empty_share() && !self.done {
            return self.shares.len() + 1;
        }
        self.shares.len()
    }

    /// Return share ranges offset by `share_range_offset`.
    pub fn share_ranges(
        &self,
        share_range_offset: usize,
    ) -> Result<BTreeMap<[u8; 32], ShareRange>> {
        self.share_ranges
            .iter()
            .map(|(key, range)| {
                let mut range = *range;
                range.add(share_range_offset)?;
                Ok((*key, range))
            })
            .collect()
    }

    fn write(&mut self, raw_data: &[u8]) -> Result<()> {
        if self.done {
            self.done = false;
        }

        self.share_builder.maybe_write_reserved_bytes()?;
        // Go's `AddData` leftover loop on a moving slice (no tail copies).
        let mut pending = raw_data;
        loop {
            let fits = pending.len() <= self.share_builder.available_bytes();
            pending = &pending[self.share_builder.add_data_prefix(pending)..];
            if fits {
                break;
            }
            self.stack_pending()?;
        }

        if self.share_builder.available_bytes() == 0 {
            self.stack_pending()?;
        }
        Ok(())
    }

    fn stack_pending(&mut self) -> Result<()> {
        let pending = self.share_builder.build()?;
        self.shares.push(pending);
        self.share_builder = ShareBuilder::new(self.namespace, self.share_version, false)?;
        Ok(())
    }

    fn write_sequence_len(&mut self, sequence_len: u32) -> Result<()> {
        if self.shares.is_empty() {
            return Ok(());
        }
        let mut builder = ShareBuilder::new(self.namespace, self.share_version, true)?;
        builder.import_raw_share(self.shares[0].as_bytes().to_vec())?;
        builder.write_sequence_len(sequence_len)?;
        self.shares[0] = builder.build()?;
        Ok(())
    }

    fn sequence_len(&self, bytes_of_padding: usize) -> Result<u32> {
        if self.shares.is_empty() {
            return Ok(0);
        }
        let len = if self.shares.len() == 1 {
            FIRST_COMPACT_SHARE_CONTENT_SIZE - bytes_of_padding
        } else {
            let continuation = self.shares.len() - 1;
            FIRST_COMPACT_SHARE_CONTENT_SIZE
                + continuation * crate::share::CONTINUATION_COMPACT_SHARE_CONTENT_SIZE
                - bytes_of_padding
        };
        u32::try_from(len).map_err(|_| Error::InvalidSequenceLen)
    }

    fn is_empty(&self) -> bool {
        self.shares.is_empty() && self.share_builder.is_empty_share()
    }
}

/// Prefix a transaction with a Go-compatible unsigned varint length.
#[must_use]
pub fn marshal_delimited_tx(tx: &[u8]) -> Vec<u8> {
    let mut out = varint::encode_uvarint(tx.len() as u64);
    out.extend_from_slice(tx);
    out
}

/// Split ordinary and `PayForBlob` transactions into compact shares.
pub fn split_txs(txs: &[Vec<u8>]) -> Result<SplitTxsResult> {
    let mut tx_writer = CompactShareSplitter::new(TX_NAMESPACE, DEFAULT_SHARE_VERSION)?;
    let mut pfb_writer = CompactShareSplitter::new(PAY_FOR_BLOB_NAMESPACE, DEFAULT_SHARE_VERSION)?;

    for tx in txs {
        if crate::IndexWrapper::decode(tx)?.is_some() {
            pfb_writer.write_tx(tx)?;
        } else {
            tx_writer.write_tx(tx)?;
        }
    }

    let tx_shares = tx_writer.export()?;
    let pfb_shares = pfb_writer.export()?;
    let mut share_ranges = tx_writer.share_ranges(0)?;
    share_ranges.extend(pfb_writer.share_ranges(tx_shares.len())?);

    Ok(SplitTxsResult {
        tx_shares,
        pfb_shares,
        share_ranges,
    })
}
