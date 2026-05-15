//! Canonical hex helpers.

pub use ::hex::FromHexError;

/// Encode bytes to canonical lowercase hex.
#[must_use]
pub fn encode(bytes: &[u8]) -> String {
    ::hex::encode(bytes)
}

/// Decode lowercase or uppercase hex into bytes.
pub fn decode(input: &str) -> Result<Vec<u8>, FromHexError> {
    ::hex::decode(input)
}
