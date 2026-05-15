//! Base64 helpers.

use ::base64::Engine as _;

pub use ::base64::DecodeError;

/// Encode bytes with padded RFC 4648 standard base64.
#[must_use]
pub fn encode_standard(bytes: &[u8]) -> String {
    ::base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Decode padded RFC 4648 standard base64.
pub fn decode_standard(input: &str) -> Result<Vec<u8>, DecodeError> {
    ::base64::engine::general_purpose::STANDARD.decode(input)
}

/// Encode bytes with padded RFC 4648 URL-safe base64.
#[must_use]
pub fn encode_url_safe(bytes: &[u8]) -> String {
    ::base64::engine::general_purpose::URL_SAFE.encode(bytes)
}

/// Decode padded RFC 4648 URL-safe base64.
pub fn decode_url_safe(input: &str) -> Result<Vec<u8>, DecodeError> {
    ::base64::engine::general_purpose::URL_SAFE.decode(input)
}
