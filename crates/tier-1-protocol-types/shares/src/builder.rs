//! Share builder helpers.

use crate::info_byte::InfoByte;
use crate::reserved::ReservedBytes;
use crate::share::{
    COMPACT_SHARE_RESERVED_BYTES, SEQUENCE_LEN_BYTES, SHARE_INFO_BYTES, SHARE_SIZE,
    SHARE_VERSION_ONE, SHARE_VERSION_TWO, SIGNER_SIZE,
};
use crate::{Error, Namespace, Result, Share};

const SHARE_SIZE_U32: u32 = 512;

/// Incremental fixed-share builder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareBuilder {
    namespace: Option<Namespace>,
    share_version: u8,
    is_first_share: bool,
    is_compact_share: bool,
    raw_share_data: Vec<u8>,
}

impl ShareBuilder {
    /// Construct an empty builder with no prepared share header.
    #[must_use]
    pub fn new_empty() -> Self {
        Self {
            namespace: None,
            share_version: 0,
            is_first_share: false,
            is_compact_share: false,
            raw_share_data: Vec::with_capacity(SHARE_SIZE),
        }
    }

    /// Construct a builder with namespace, info byte, and required placeholders.
    pub fn new(namespace: Namespace, share_version: u8, is_first_share: bool) -> Result<Self> {
        namespace.validate_basic()?;
        let is_compact_share =
            namespace.is_tx() || namespace.is_pay_for_blob() || namespace.is_pay_for_fibre();
        let mut builder = Self {
            namespace: Some(namespace),
            share_version,
            is_first_share,
            is_compact_share,
            raw_share_data: Vec::with_capacity(SHARE_SIZE),
        };
        builder.init()?;
        Ok(builder)
    }

    /// Return the remaining bytes in the pending share.
    #[must_use]
    pub fn available_bytes(&self) -> usize {
        SHARE_SIZE.saturating_sub(self.raw_share_data.len())
    }

    /// Replace this builder's raw share bytes.
    pub fn import_raw_share(&mut self, raw_bytes: Vec<u8>) -> Result<()> {
        if raw_bytes.len() > SHARE_SIZE {
            return Err(Error::InvalidShareLength {
                actual: raw_bytes.len(),
                expected: SHARE_SIZE,
            });
        }
        self.raw_share_data = raw_bytes;
        Ok(())
    }

    /// Append as much data as fits, returning any leftover bytes.
    pub fn add_data(&mut self, raw_data: &[u8]) -> Option<Vec<u8>> {
        let pending_left = self.available_bytes();
        if raw_data.len() <= pending_left {
            self.raw_share_data.extend_from_slice(raw_data);
            return None;
        }

        self.raw_share_data
            .extend_from_slice(&raw_data[..pending_left]);
        Some(raw_data[pending_left..].to_vec())
    }

    /// Append as much of `raw_data` as fits and return how many bytes were
    /// taken ([`Self::add_data`] without copying the leftover: splitting a
    /// blob share by share with `add_data` copies the remaining tail every
    /// time, quadratic in the blob size).
    pub fn add_data_prefix(&mut self, raw_data: &[u8]) -> usize {
        let n = raw_data.len().min(self.available_bytes());
        self.raw_share_data.extend_from_slice(&raw_data[..n]);
        n
    }

    /// Build a fixed-size share.
    pub fn build(&self) -> Result<Share> {
        Share::from_slice(&self.raw_share_data)
    }

    /// Return true when no content has been written beyond the prepared header.
    #[must_use]
    pub fn is_empty_share(&self) -> bool {
        if self.namespace.is_none() {
            return self.raw_share_data.is_empty();
        }
        self.raw_share_data.len() == self.empty_share_len()
    }

    /// Zero-pad the pending share to the fixed share size.
    pub fn zero_pad_if_necessary(&mut self) -> usize {
        if self.raw_share_data.len() >= SHARE_SIZE {
            return 0;
        }
        let padding = SHARE_SIZE - self.raw_share_data.len();
        self.raw_share_data.resize(SHARE_SIZE, 0);
        padding
    }

    /// Write compact reserved bytes if they are still zero.
    pub fn maybe_write_reserved_bytes(&mut self) -> Result<()> {
        if !self.is_compact_share {
            return Err(Error::InvalidCompactShare);
        }

        let index = self.index_of_reserved_bytes();
        let reserved = ReservedBytes::parse(
            &self.raw_share_data[index..index + COMPACT_SHARE_RESERVED_BYTES],
        )?;
        if reserved.byte_index() != 0 {
            return Ok(());
        }

        let next_unit_index = u32::try_from(self.raw_share_data.len()).map_err(|_| {
            Error::InvalidReservedByteIndex {
                actual: u32::MAX,
                max_exclusive: SHARE_SIZE_U32,
            }
        })?;
        let reserved = ReservedBytes::new(next_unit_index)?.to_bytes();
        self.raw_share_data[index..index + COMPACT_SHARE_RESERVED_BYTES].copy_from_slice(&reserved);
        Ok(())
    }

    /// Write a sequence length to a first-share builder.
    pub fn write_sequence_len(&mut self, sequence_len: u32) -> Result<()> {
        if !self.is_first_share {
            return Err(Error::InvalidSequenceLen);
        }
        let start = crate::namespace::NAMESPACE_SIZE + SHARE_INFO_BYTES;
        let end = start + SEQUENCE_LEN_BYTES;
        if self.raw_share_data.len() < end {
            return Err(Error::InvalidSequenceLen);
        }
        self.raw_share_data[start..end].copy_from_slice(&sequence_len.to_be_bytes());
        Ok(())
    }

    /// Write the signer bytes carried by a version-one or version-two
    /// sequence-start share.
    pub fn write_signer(&mut self, signer: &[u8; SIGNER_SIZE]) {
        if self.is_first_share
            && (self.share_version == SHARE_VERSION_ONE || self.share_version == SHARE_VERSION_TWO)
        {
            self.raw_share_data.extend_from_slice(signer);
        }
    }

    /// Write raw fibre fields (blob version, commitment) into a version-two
    /// sequence-start share (`WriteFibreBlobVersion` / `WriteFibreCommitment`).
    pub fn write_fibre_bytes(&mut self, bytes: &[u8]) {
        if self.is_first_share && self.share_version == SHARE_VERSION_TWO {
            self.raw_share_data.extend_from_slice(bytes);
        }
    }

    /// Flip the sequence-start bit in the prepared info byte.
    pub fn flip_sequence_start(&mut self) {
        let index = crate::namespace::NAMESPACE_SIZE;
        if let Some(byte) = self.raw_share_data.get_mut(index) {
            *byte ^= 1;
        }
    }

    fn init(&mut self) -> Result<()> {
        let namespace = self.namespace.ok_or(Error::InvalidShareLength {
            actual: 0,
            expected: SHARE_SIZE,
        })?;
        let info_byte = InfoByte::new(self.share_version, self.is_first_share)?;

        self.raw_share_data.extend_from_slice(namespace.as_bytes());
        self.raw_share_data.push(info_byte.into_byte());
        if self.is_first_share {
            self.raw_share_data
                .extend_from_slice(&[0; SEQUENCE_LEN_BYTES]);
        }
        if self.is_compact_share {
            self.raw_share_data
                .extend_from_slice(&[0; COMPACT_SHARE_RESERVED_BYTES]);
        }
        Ok(())
    }

    fn empty_share_len(&self) -> usize {
        let mut expected = crate::namespace::NAMESPACE_SIZE + SHARE_INFO_BYTES;
        if self.is_first_share {
            expected += SEQUENCE_LEN_BYTES;
        }
        if self.is_first_share && self.share_version == SHARE_VERSION_ONE {
            expected += SIGNER_SIZE;
        }
        if self.is_compact_share {
            expected += COMPACT_SHARE_RESERVED_BYTES;
        }
        expected
    }

    fn index_of_reserved_bytes(&self) -> usize {
        let mut index = crate::namespace::NAMESPACE_SIZE + SHARE_INFO_BYTES;
        if self.is_first_share {
            index += SEQUENCE_LEN_BYTES;
        }
        index
    }
}

impl Default for ShareBuilder {
    fn default() -> Self {
        Self::new_empty()
    }
}
