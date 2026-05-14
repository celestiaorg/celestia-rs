//! Base64 helpers.

pub use ::base64::DecodeError;

/// Encode bytes with padded RFC 4648 standard base64.
#[must_use]
pub fn encode_standard(_bytes: &[u8]) -> String {
    String::new()
}

/// Decode padded RFC 4648 standard base64.
pub fn decode_standard(_input: &str) -> Result<Vec<u8>, DecodeError> {
    Ok(Vec::new())
}

/// Encode bytes with padded RFC 4648 URL-safe base64.
#[must_use]
pub fn encode_url_safe(_bytes: &[u8]) -> String {
    String::new()
}

/// Decode padded RFC 4648 URL-safe base64.
pub fn decode_url_safe(_input: &str) -> Result<Vec<u8>, DecodeError> {
    Ok(Vec::new())
}
