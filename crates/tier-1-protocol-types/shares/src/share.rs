//! Fixed-size share helpers.

use crate::info_byte::InfoByte;
use crate::namespace::{NAMESPACE_SIZE, Namespace};
use crate::reserved::ReservedBytes;
use crate::{Error, Result};

/// Size of a share in bytes.
pub const SHARE_SIZE: usize = 512;
/// Number of share info bytes.
pub const SHARE_INFO_BYTES: usize = 1;
/// Number of bytes in a sequence length field.
pub const SEQUENCE_LEN_BYTES: usize = 4;
/// Initial share version.
pub const SHARE_VERSION_ZERO: u8 = 0;
/// Default share version.
pub const DEFAULT_SHARE_VERSION: u8 = SHARE_VERSION_ZERO;
/// Number of compact-share reserved bytes.
pub const COMPACT_SHARE_RESERVED_BYTES: usize = 4;
/// Usable content bytes in the first compact share of a sequence.
pub const FIRST_COMPACT_SHARE_CONTENT_SIZE: usize = SHARE_SIZE
    - NAMESPACE_SIZE
    - SHARE_INFO_BYTES
    - SEQUENCE_LEN_BYTES
    - COMPACT_SHARE_RESERVED_BYTES;
/// Usable content bytes in a compact continuation share.
pub const CONTINUATION_COMPACT_SHARE_CONTENT_SIZE: usize =
    SHARE_SIZE - NAMESPACE_SIZE - SHARE_INFO_BYTES - COMPACT_SHARE_RESERVED_BYTES;
/// Usable content bytes in the first sparse share of a sequence.
pub const FIRST_SPARSE_SHARE_CONTENT_SIZE: usize =
    SHARE_SIZE - NAMESPACE_SIZE - SHARE_INFO_BYTES - SEQUENCE_LEN_BYTES;
/// Usable content bytes in a sparse continuation share.
pub const CONTINUATION_SPARSE_SHARE_CONTENT_SIZE: usize =
    SHARE_SIZE - NAMESPACE_SIZE - SHARE_INFO_BYTES;
/// Smallest original data-square width.
pub const MIN_SQUARE_SIZE: usize = 1;
/// Smallest original data-square share count.
pub const MIN_SHARE_COUNT: usize = MIN_SQUARE_SIZE * MIN_SQUARE_SIZE;
/// Maximum share version representable in an info byte.
pub const MAX_SHARE_VERSION: u8 = 127;
/// Supported share versions.
pub const SUPPORTED_SHARE_VERSIONS: &[u8] = &[SHARE_VERSION_ZERO];

/// A fixed 512-byte Celestia share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share([u8; SHARE_SIZE]);

impl Share {
    /// Construct a share from fixed-size bytes.
    pub fn from_bytes(bytes: [u8; SHARE_SIZE]) -> Result<Self> {
        Ok(Self(bytes))
    }

    /// Construct a share by copying bytes from a slice.
    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        let raw: [u8; SHARE_SIZE] = bytes.try_into().map_err(|_| Error::InvalidShareLength {
            actual: bytes.len(),
            expected: SHARE_SIZE,
        })?;
        Self::from_bytes(raw)
    }

    /// Validate this share's fixed byte length.
    pub fn validate(&self) -> Result<()> {
        Ok(())
    }

    /// Return the encoded share bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; SHARE_SIZE] {
        &self.0
    }

    /// Return the encoded share bytes by value.
    #[must_use]
    pub const fn into_bytes(self) -> [u8; SHARE_SIZE] {
        self.0
    }

    /// Return the share byte length.
    #[allow(clippy::len_without_is_empty)]
    #[must_use]
    pub const fn len(&self) -> usize {
        SHARE_SIZE
    }

    /// Return the share namespace.
    pub fn namespace(&self) -> Result<Namespace> {
        Namespace::from_slice(&self.0[..NAMESPACE_SIZE])
    }

    /// Return the share info byte.
    pub fn info_byte(&self) -> Result<InfoByte> {
        InfoByte::parse(self.0[NAMESPACE_SIZE])
    }

    /// Return the share version.
    pub fn version(&self) -> Result<u8> {
        Ok(self.info_byte()?.version())
    }

    /// Validate this share's version against a supported-version list.
    pub fn supports_versions(&self, supported_share_versions: &[u8]) -> Result<()> {
        let version = self.version()?;
        if supported_share_versions.contains(&version) {
            return Ok(());
        }
        Err(Error::UnsupportedShareVersion {
            version,
            supported: supported_share_versions.to_vec(),
        })
    }

    /// Return true when this share starts a sequence.
    pub fn is_sequence_start(&self) -> Result<bool> {
        Ok(self.info_byte()?.is_sequence_start())
    }

    /// Return true when this share is in the transaction or `PayForBlob` namespace.
    pub fn is_compact_share(&self) -> Result<bool> {
        let namespace = self.namespace()?;
        Ok(namespace.is_tx() || namespace.is_pay_for_blob())
    }

    /// Return the sequence length for sequence-start shares.
    pub fn sequence_len(&self) -> Result<Option<u32>> {
        if !self.is_sequence_start()? {
            return Ok(None);
        }
        let start = NAMESPACE_SIZE + SHARE_INFO_BYTES;
        let end = start + SEQUENCE_LEN_BYTES;
        let raw: [u8; SEQUENCE_LEN_BYTES] = self.0[start..end]
            .try_into()
            .map_err(|_| Error::InvalidSequenceLen)?;
        Ok(Some(u32::from_be_bytes(raw)))
    }

    /// Return true when this share is namespace, primary-reserved, or tail padding.
    pub fn is_padding(&self) -> Result<bool> {
        let namespace = self.namespace()?;
        if namespace.is_tail_padding() || namespace.is_primary_reserved_padding() {
            return Ok(true);
        }
        Ok(self.is_sequence_start()? && self.sequence_len()? == Some(0))
    }

    /// Return raw content excluding namespace, info, sequence length, and reserved bytes.
    pub fn raw_data(&self) -> Result<&[u8]> {
        Ok(&self.0[self.raw_data_start_index()?..])
    }

    /// Return raw content using compact reserved bytes for the first share.
    pub fn raw_data_using_reserved(&self) -> Result<&[u8]> {
        let index = self.raw_data_start_index_using_reserved()?;
        if index == 0 {
            return Ok(&[]);
        }
        Ok(&self.0[index..])
    }

    pub(crate) fn raw_data_start_index(&self) -> Result<usize> {
        let mut index = NAMESPACE_SIZE + SHARE_INFO_BYTES;
        if self.is_sequence_start()? {
            index += SEQUENCE_LEN_BYTES;
        }
        if self.is_compact_share()? {
            index += COMPACT_SHARE_RESERVED_BYTES;
        }
        Ok(index)
    }

    fn raw_data_start_index_using_reserved(&self) -> Result<usize> {
        if !self.is_compact_share()? {
            return self.raw_data_start_index();
        }

        let mut reserved_index = NAMESPACE_SIZE + SHARE_INFO_BYTES;
        if self.is_sequence_start()? {
            reserved_index += SEQUENCE_LEN_BYTES;
        }
        let reserved = ReservedBytes::parse(
            &self.0[reserved_index..reserved_index + COMPACT_SHARE_RESERVED_BYTES],
        )?;
        Ok(reserved.byte_index() as usize)
    }
}

/// Convert shares to their fixed-size byte arrays.
#[must_use]
pub fn shares_to_bytes(shares: &[Share]) -> Vec<[u8; SHARE_SIZE]> {
    shares
        .iter()
        .map(|share| share.as_bytes().to_owned())
        .collect()
}

/// Convert fixed-size byte arrays to shares.
pub fn shares_from_bytes(bytes: &[[u8; SHARE_SIZE]]) -> Result<Vec<Share>> {
    bytes.iter().copied().map(Share::from_bytes).collect()
}
