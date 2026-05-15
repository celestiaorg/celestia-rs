//! Hash and address helpers for CometBFT protocol types.

use crate::error::{Error, Result};

/// Size in bytes of a CometBFT hash.
pub const HASH_LENGTH: usize = 32;

/// Size in bytes of a CometBFT account/validator address.
pub const ADDRESS_LENGTH: usize = 20;

/// A 32-byte CometBFT hash.
pub type Hash = [u8; HASH_LENGTH];

/// A 20-byte CometBFT address.
pub type Address = [u8; ADDRESS_LENGTH];

/// Parse a protobuf hash field.
pub fn optional_hash_from_bytes(bytes: &[u8]) -> Result<Option<Hash>> {
    match bytes.len() {
        0 => Ok(None),
        HASH_LENGTH => {
            let mut hash = [0u8; HASH_LENGTH];
            hash.copy_from_slice(bytes);
            Ok(Some(hash))
        }
        actual => Err(Error::InvalidHashLength { actual }),
    }
}

/// Encode an optional CometBFT hash as protobuf bytes.
#[must_use]
pub fn optional_hash_to_vec(hash: Option<&Hash>) -> Vec<u8> {
    hash.map_or_else(Vec::new, |hash| hash.to_vec())
}

/// Parse a 20-byte address.
pub fn address_from_bytes(bytes: &[u8]) -> Result<Address> {
    if bytes.len() != ADDRESS_LENGTH {
        return Err(Error::InvalidAddressLength {
            actual: bytes.len(),
        });
    }
    let mut address = [0u8; ADDRESS_LENGTH];
    address.copy_from_slice(bytes);
    Ok(address)
}
