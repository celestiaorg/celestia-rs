//! Compact share reserved-byte helpers.

use crate::share::COMPACT_SHARE_RESERVED_BYTES;
use crate::{Error, Result};

const SHARE_SIZE_U32: u32 = 512;

/// Compact share reserved bytes interpreted as the first unit byte index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReservedBytes(u32);

impl ReservedBytes {
    /// Construct reserved bytes for a byte index within a share.
    pub fn new(byte_index: u32) -> Result<Self> {
        if byte_index >= SHARE_SIZE_U32 {
            return Err(Error::InvalidReservedByteIndex {
                actual: byte_index,
                max_exclusive: SHARE_SIZE_U32,
            });
        }
        Ok(Self(byte_index))
    }

    /// Parse reserved bytes from a big-endian 4-byte slice.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let raw: [u8; COMPACT_SHARE_RESERVED_BYTES] =
            bytes
                .try_into()
                .map_err(|_| Error::InvalidReservedBytesLength {
                    actual: bytes.len(),
                    expected: COMPACT_SHARE_RESERVED_BYTES,
                })?;
        Self::new(u32::from_be_bytes(raw))
    }

    /// Return the first unit byte index.
    #[must_use]
    pub const fn byte_index(self) -> u32 {
        self.0
    }

    /// Return the encoded big-endian bytes.
    #[must_use]
    pub fn to_bytes(self) -> [u8; COMPACT_SHARE_RESERVED_BYTES] {
        self.0.to_be_bytes()
    }
}

/// Parse compact share reserved bytes.
pub fn parse_reserved_bytes(bytes: &[u8]) -> Result<ReservedBytes> {
    ReservedBytes::parse(bytes)
}
