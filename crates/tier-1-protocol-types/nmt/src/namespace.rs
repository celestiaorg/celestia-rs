//! Fixed-width Celestia namespace IDs.

use crate::{Error, Result};

/// Celestia M1 namespace size in bytes.
pub const NAMESPACE_SIZE: usize = 8;

/// Minimum namespace ID.
pub const MIN_NAMESPACE: Namespace = Namespace([0; NAMESPACE_SIZE]);

/// Maximum namespace ID.
pub const MAX_NAMESPACE: Namespace = Namespace([0xff; NAMESPACE_SIZE]);

/// Fixed-width Celestia namespace ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Namespace([u8; NAMESPACE_SIZE]);

impl Namespace {
    /// Construct a namespace ID from fixed bytes.
    #[must_use]
    pub const fn new(bytes: [u8; NAMESPACE_SIZE]) -> Self {
        Self(bytes)
    }

    /// Parse a namespace ID from a byte slice.
    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != NAMESPACE_SIZE {
            return Err(Error::InvalidNamespaceLength {
                actual: bytes.len(),
            });
        }

        let mut namespace = [0u8; NAMESPACE_SIZE];
        namespace.copy_from_slice(bytes);
        Ok(Self(namespace))
    }

    /// Borrow the namespace ID bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; NAMESPACE_SIZE] {
        &self.0
    }

    /// Return the namespace ID bytes.
    #[must_use]
    pub const fn into_bytes(self) -> [u8; NAMESPACE_SIZE] {
        self.0
    }

    /// Return true when this is [`MIN_NAMESPACE`].
    #[must_use]
    pub const fn is_min(self) -> bool {
        let bytes = self.0;
        let mut index = 0;
        while index < NAMESPACE_SIZE {
            if bytes[index] != 0 {
                return false;
            }
            index += 1;
        }
        true
    }

    /// Return true when this is [`MAX_NAMESPACE`].
    #[must_use]
    pub const fn is_max(self) -> bool {
        let bytes = self.0;
        let mut index = 0;
        while index < NAMESPACE_SIZE {
            if bytes[index] != 0xff {
                return false;
            }
            index += 1;
        }
        true
    }
}

impl TryFrom<&[u8]> for Namespace {
    type Error = Error;

    fn try_from(value: &[u8]) -> Result<Self> {
        Self::from_slice(value)
    }
}

impl From<[u8; NAMESPACE_SIZE]> for Namespace {
    fn from(value: [u8; NAMESPACE_SIZE]) -> Self {
        Self::new(value)
    }
}

impl From<Namespace> for [u8; NAMESPACE_SIZE] {
    fn from(value: Namespace) -> Self {
        value.into_bytes()
    }
}

impl core::fmt::LowerHex for Namespace {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}
