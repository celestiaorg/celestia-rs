use crate::{Error, Result};
use tendermint_types::hash::{ADDRESS_LENGTH, Address, HASH_LENGTH};

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

pub fn validate_hash_bytes(bytes: &[u8]) -> Result<()> {
    if !bytes.is_empty() && bytes.len() != HASH_LENGTH {
        return Err(Error::InvalidHashLength {
            actual: bytes.len(),
        });
    }
    Ok(())
}

pub fn validate_address_bytes(bytes: &[u8]) -> Result<()> {
    address_from_bytes(bytes).map(|_| ())
}

pub fn validate_positive_height(height: i64) -> Result<()> {
    if height <= 0 {
        return Err(Error::InvalidHeight);
    }
    Ok(())
}

pub fn validate_non_negative_round(round: i32) -> Result<()> {
    if round < 0 {
        return Err(Error::InvalidRound);
    }
    Ok(())
}
