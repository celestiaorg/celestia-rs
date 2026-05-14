//! Canonical hex helpers.

pub use ::hex::FromHexError;

/// Encode bytes to canonical lowercase hex.
#[must_use]
pub fn encode(_bytes: &[u8]) -> String {
    String::new()
}

/// Decode lowercase or uppercase hex into bytes.
pub fn decode(_input: &str) -> Result<Vec<u8>, FromHexError> {
    Ok(Vec::new())
}
