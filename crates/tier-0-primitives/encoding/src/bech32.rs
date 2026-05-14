//! Bech32 helpers for Celestia addresses.

use ::bech32::{FromBase32, ToBase32, Variant};
use core::fmt;

/// Celestia account address human-readable prefix.
pub const ACCOUNT_PREFIX: &str = "celestia";

/// Celestia validator operator address human-readable prefix.
pub const VALIDATOR_OPERATOR_PREFIX: &str = "celestiavaloper";

/// Celestia validator consensus address human-readable prefix.
pub const VALIDATOR_CONSENSUS_PREFIX: &str = "celestiavalcons";

/// Canonical Celestia address byte length.
pub const ADDRESS_LENGTH: usize = 20;

/// Canonical Celestia address bytes.
pub type AddressBytes = [u8; ADDRESS_LENGTH];

/// Errors returned by bech32 helpers.
#[derive(Debug)]
pub enum Error {
    /// The bech32 string failed checksum, character-set, or variant validation.
    Invalid(::bech32::Error),
    /// The decoded human-readable prefix did not match the expected prefix.
    WrongPrefix {
        /// Expected human-readable prefix.
        expected: &'static str,
        /// Actual decoded human-readable prefix.
        actual: String,
    },
    /// The decoded data length did not match the expected address length.
    WrongLength {
        /// Expected byte length.
        expected: usize,
        /// Actual byte length.
        actual: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(err) => write!(f, "invalid bech32 string: {err}"),
            Self::WrongPrefix { expected, actual } => {
                write!(f, "wrong bech32 prefix: expected {expected}, got {actual}")
            }
            Self::WrongLength { expected, actual } => {
                write!(f, "wrong address length: expected {expected}, got {actual}")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<::bech32::Error> for Error {
    fn from(value: ::bech32::Error) -> Self {
        Self::Invalid(value)
    }
}

/// Encode arbitrary bytes with an explicit bech32 human-readable prefix.
pub fn encode(prefix: &str, bytes: &[u8]) -> Result<String, Error> {
    ::bech32::encode(prefix, bytes.to_base32(), Variant::Bech32).map_err(Error::from)
}

/// Decode a bech32 string into its human-readable prefix and data bytes.
pub fn decode(input: &str) -> Result<(String, Vec<u8>), Error> {
    let (prefix, data, variant) = ::bech32::decode(input)?;
    if variant != Variant::Bech32 {
        return Err(Error::Invalid(::bech32::Error::InvalidChecksum));
    }
    let bytes = Vec::<u8>::from_base32(&data)?;
    Ok((prefix, bytes))
}

/// Encode a 20-byte Celestia account address.
pub fn encode_account_address(bytes: &AddressBytes) -> Result<String, Error> {
    encode(ACCOUNT_PREFIX, bytes)
}

/// Decode a Celestia account address, enforcing prefix and 20-byte length.
pub fn decode_account_address(input: &str) -> Result<AddressBytes, Error> {
    decode_address(input, ACCOUNT_PREFIX)
}

/// Encode a 20-byte Celestia validator operator address.
pub fn encode_validator_operator_address(bytes: &AddressBytes) -> Result<String, Error> {
    encode(VALIDATOR_OPERATOR_PREFIX, bytes)
}

/// Decode a Celestia validator operator address, enforcing prefix and 20-byte length.
pub fn decode_validator_operator_address(input: &str) -> Result<AddressBytes, Error> {
    decode_address(input, VALIDATOR_OPERATOR_PREFIX)
}

/// Encode a 20-byte Celestia validator consensus address.
pub fn encode_validator_consensus_address(bytes: &AddressBytes) -> Result<String, Error> {
    encode(VALIDATOR_CONSENSUS_PREFIX, bytes)
}

/// Decode a Celestia validator consensus address, enforcing prefix and 20-byte length.
pub fn decode_validator_consensus_address(input: &str) -> Result<AddressBytes, Error> {
    decode_address(input, VALIDATOR_CONSENSUS_PREFIX)
}

fn decode_address(input: &str, expected_prefix: &'static str) -> Result<AddressBytes, Error> {
    let (actual_prefix, bytes) = decode(input)?;
    if actual_prefix != expected_prefix {
        return Err(Error::WrongPrefix {
            expected: expected_prefix,
            actual: actual_prefix,
        });
    }
    if bytes.len() != ADDRESS_LENGTH {
        return Err(Error::WrongLength {
            expected: ADDRESS_LENGTH,
            actual: bytes.len(),
        });
    }

    let mut out = [0u8; ADDRESS_LENGTH];
    out.copy_from_slice(&bytes);
    Ok(out)
}
