//! Share info byte helpers.

use crate::share::MAX_SHARE_VERSION;
use crate::{Error, Result};

/// Share info byte containing the share version and sequence-start bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InfoByte(u8);

impl InfoByte {
    /// Construct a share info byte.
    pub fn new(version: u8, is_sequence_start: bool) -> Result<Self> {
        if version > MAX_SHARE_VERSION {
            return Err(Error::InvalidShareVersion {
                version,
                max: MAX_SHARE_VERSION,
            });
        }
        let sequence_bit = u8::from(is_sequence_start);
        Ok(Self((version << 1) | sequence_bit))
    }

    /// Parse a share info byte.
    pub fn parse(byte: u8) -> Result<Self> {
        Self::new(byte >> 1, byte & 1 == 1)
    }

    /// Return the share version.
    #[must_use]
    pub const fn version(self) -> u8 {
        self.0 >> 1
    }

    /// Return true when this byte marks the first share in a sequence.
    #[must_use]
    pub const fn is_sequence_start(self) -> bool {
        self.0 & 1 == 1
    }

    /// Return the encoded byte.
    #[must_use]
    pub const fn into_byte(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for InfoByte {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self> {
        Self::parse(value)
    }
}

impl From<InfoByte> for u8 {
    fn from(value: InfoByte) -> Self {
        value.into_byte()
    }
}
