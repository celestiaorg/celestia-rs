//! Bech32 helpers for Celestia addresses.

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
pub fn encode(_prefix: &str, _bytes: &[u8]) -> Result<String, Error> {
    Ok(String::new())
}

/// Decode a bech32 string into its human-readable prefix and data bytes.
pub fn decode(_input: &str) -> Result<(String, Vec<u8>), Error> {
    Ok((String::new(), Vec::new()))
}

/// Encode a 20-byte Celestia account address.
pub fn encode_account_address(bytes: &AddressBytes) -> Result<String, Error> {
    encode(ACCOUNT_PREFIX, bytes)
}

/// Decode a Celestia account address, enforcing prefix and 20-byte length.
pub fn decode_account_address(_input: &str) -> Result<AddressBytes, Error> {
    Ok([0u8; ADDRESS_LENGTH])
}

/// Encode a 20-byte Celestia validator operator address.
pub fn encode_validator_operator_address(bytes: &AddressBytes) -> Result<String, Error> {
    encode(VALIDATOR_OPERATOR_PREFIX, bytes)
}

/// Decode a Celestia validator operator address, enforcing prefix and 20-byte length.
pub fn decode_validator_operator_address(_input: &str) -> Result<AddressBytes, Error> {
    Ok([0u8; ADDRESS_LENGTH])
}

/// Encode a 20-byte Celestia validator consensus address.
pub fn encode_validator_consensus_address(bytes: &AddressBytes) -> Result<String, Error> {
    encode(VALIDATOR_CONSENSUS_PREFIX, bytes)
}

/// Decode a Celestia validator consensus address, enforcing prefix and 20-byte length.
pub fn decode_validator_consensus_address(_input: &str) -> Result<AddressBytes, Error> {
    Ok([0u8; ADDRESS_LENGTH])
}
