//! Celestia bech32 address helpers.

use crate::error::{Error, Result};
use encoding::bech32;

/// Size in bytes of Cosmos account and validator addresses.
pub const ADDRESS_LENGTH: usize = 20;

/// A 20-byte Celestia account address.
pub type AccountAddress = [u8; ADDRESS_LENGTH];

/// A 20-byte Celestia validator-operator address.
pub type ValoperAddress = [u8; ADDRESS_LENGTH];

/// A 20-byte Celestia validator-consensus address.
pub type ValconsAddress = [u8; ADDRESS_LENGTH];

/// Encode an account address with the `celestia` bech32 prefix.
#[must_use]
#[allow(clippy::expect_used)]
pub fn account_address_to_bech32(address: &AccountAddress) -> String {
    bech32::encode_account_address(address).expect("static account prefix is valid")
}

/// Decode and validate a `celestia` bech32 account address.
pub fn account_address_from_bech32(value: &str) -> Result<AccountAddress> {
    decode_address(value, bech32::ACCOUNT_PREFIX)
}

/// Derive the legacy Cosmos SDK address for a module account name.
#[must_use]
pub fn module_address(module_name: &str) -> AccountAddress {
    let digest = hash::sha256(module_name.as_bytes());
    let mut address = [0_u8; ADDRESS_LENGTH];
    address.copy_from_slice(&digest[..ADDRESS_LENGTH]);
    address
}

/// Encode a validator-operator address with the `celestiavaloper` prefix.
#[must_use]
#[allow(clippy::expect_used)]
pub fn valoper_address_to_bech32(address: &ValoperAddress) -> String {
    bech32::encode_validator_operator_address(address).expect("static valoper prefix is valid")
}

/// Decode and validate a `celestiavaloper` bech32 validator-operator address.
pub fn valoper_address_from_bech32(value: &str) -> Result<ValoperAddress> {
    decode_address(value, bech32::VALIDATOR_OPERATOR_PREFIX)
}

/// Encode a validator-consensus address with the `celestiavalcons` prefix.
#[must_use]
#[allow(clippy::expect_used)]
pub fn valcons_address_to_bech32(address: &ValconsAddress) -> String {
    bech32::encode_validator_consensus_address(address).expect("static valcons prefix is valid")
}

/// Decode and validate a `celestiavalcons` bech32 validator-consensus address.
pub fn valcons_address_from_bech32(value: &str) -> Result<ValconsAddress> {
    decode_address(value, bech32::VALIDATOR_CONSENSUS_PREFIX)
}

fn decode_address(value: &str, expected_prefix: &'static str) -> Result<[u8; ADDRESS_LENGTH]> {
    let (actual_prefix, bytes) = bech32::decode(value).map_err(|error| match error {
        bech32::Error::WrongPrefix { expected, actual } => {
            Error::InvalidAddressPrefix { expected, actual }
        }
        bech32::Error::WrongLength { actual, .. } => Error::InvalidAddressLength { actual },
        bech32::Error::Invalid(_) => Error::InvalidAddress,
    })?;
    if actual_prefix != expected_prefix {
        return Err(Error::InvalidAddressPrefix {
            expected: expected_prefix,
            actual: actual_prefix,
        });
    }
    if bytes.len() != ADDRESS_LENGTH {
        return Err(Error::InvalidAddressLength {
            actual: bytes.len(),
        });
    }
    let mut out = [0u8; ADDRESS_LENGTH];
    out.copy_from_slice(&bytes);
    Ok(out)
}
