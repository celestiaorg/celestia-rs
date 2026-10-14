//! Sparse blob share splitting helpers.

use crate::share::{DEFAULT_SHARE_VERSION, SHARE_VERSION_TWO, SUPPORTED_SHARE_VERSIONS};
use crate::{Blob, Error, Result, Share, ShareBuilder, namespace_padding_shares};

/// Incremental sparse share splitter for blobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SparseShareSplitter {
    shares: Vec<Share>,
}

impl SparseShareSplitter {
    /// Construct an empty sparse share splitter.
    #[must_use]
    pub fn new() -> Self {
        Self { shares: Vec::new() }
    }

    /// Write one blob into the splitter.
    pub fn write(&mut self, blob: &Blob) -> Result<()> {
        blob.validate_basic()?;
        if !SUPPORTED_SHARE_VERSIONS.contains(&blob.share_version()) {
            return Err(Error::UnsupportedShareVersion {
                version: blob.share_version(),
                supported: SUPPORTED_SHARE_VERSIONS.to_vec(),
            });
        }

        let mut raw_data = blob.data().to_vec();
        let mut builder = ShareBuilder::new(blob.namespace(), blob.share_version(), true)?;
        builder
            .write_sequence_len(u32::try_from(raw_data.len()).map_err(|_| Error::InvalidBlob)?)?;
        if let Some(signer) = blob.signer() {
            builder.write_signer(&signer);
        }
        if blob.share_version() == SHARE_VERSION_TWO {
            // Fibre system blob: blob version + commitment (validated to be
            // exactly 36 bytes) fit in the first share.
            builder.write_fibre_bytes(&raw_data);
            builder.zero_pad_if_necessary();
            self.shares.push(builder.build()?);
            return Ok(());
        }

        loop {
            let leftover = builder.add_data(&raw_data);
            if leftover.is_none() {
                builder.zero_pad_if_necessary();
            }
            self.shares.push(builder.build()?);
            let Some(leftover) = leftover else {
                break;
            };
            builder = ShareBuilder::new(blob.namespace(), blob.share_version(), false)?;
            raw_data = leftover;
        }
        Ok(())
    }

    /// Write namespace padding shares after the last written share.
    pub fn write_namespace_padding_shares(&mut self, count: usize) -> Result<()> {
        if count == 0 {
            return Ok(());
        }
        let last = self.shares.last().ok_or(Error::InvalidPadding)?;
        let namespace = last.namespace()?;
        let share_version = last.version()?;
        self.shares
            .extend(namespace_padding_shares(namespace, share_version, count)?);
        Ok(())
    }

    /// Return finalized sparse shares.
    #[must_use]
    pub fn export(&self) -> Vec<Share> {
        self.shares.clone()
    }

    /// Return the current share count.
    #[must_use]
    pub fn count(&self) -> usize {
        self.shares.len()
    }
}

impl Default for SparseShareSplitter {
    fn default() -> Self {
        Self::new()
    }
}

/// Split blobs into sparse shares.
pub fn split_blobs(blobs: &[Blob]) -> Result<Vec<Share>> {
    let mut splitter = SparseShareSplitter::new();
    for blob in blobs {
        splitter.write(blob)?;
    }
    Ok(splitter.export())
}

#[allow(dead_code)]
const _: u8 = DEFAULT_SHARE_VERSION;
