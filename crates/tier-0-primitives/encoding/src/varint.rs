//! Tendermint-compatible varint helpers.

use core::fmt;

/// Maximum number of bytes in a Go-compatible 64-bit varint.
pub const MAX_VARINT_LEN64: usize = 10;

/// Errors returned by canonical varint decoding and bounded-buffer writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The caller-provided output buffer is too small.
    BufferTooSmall {
        /// Number of bytes required to encode the value.
        required: usize,
        /// Number of bytes available in the caller-provided buffer.
        available: usize,
    },
    /// The byte slice ended before a terminating byte was found.
    Incomplete,
    /// The encoded integer exceeds the 64-bit varint range.
    Overflow,
    /// The byte sequence is valid LEB128 but not the shortest canonical form.
    NonCanonical,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BufferTooSmall {
                required,
                available,
            } => write!(
                f,
                "buffer too small for varint: required {required} bytes, available {available}"
            ),
            Self::Incomplete => f.write_str("incomplete varint"),
            Self::Overflow => f.write_str("varint overflows 64-bit range"),
            Self::NonCanonical => f.write_str("non-canonical varint encoding"),
        }
    }
}

impl std::error::Error for Error {}

/// Return the canonical byte length of `value` when encoded as a uvarint.
#[must_use]
pub const fn encoded_uvarint_len(_value: u64) -> usize {
    0
}

/// Return the canonical byte length of `value` when encoded as a signed varint.
#[must_use]
pub const fn encoded_varint_len(_value: i64) -> usize {
    0
}

/// Encode `value` as Go `binary.PutUvarint` bytes.
#[must_use]
pub fn encode_uvarint(_value: u64) -> Vec<u8> {
    Vec::new()
}

/// Encode `value` as Go `binary.PutVarint` bytes.
#[must_use]
pub fn encode_varint(_value: i64) -> Vec<u8> {
    Vec::new()
}

/// Encode `value` into `dst`, returning the number of bytes written.
pub fn put_uvarint(_dst: &mut [u8], _value: u64) -> Result<usize, Error> {
    Err(Error::Incomplete)
}

/// Encode `value` into `dst`, returning the number of bytes written.
pub fn put_varint(_dst: &mut [u8], _value: i64) -> Result<usize, Error> {
    Err(Error::Incomplete)
}

/// Decode a canonical Go `binary.Uvarint` byte sequence.
pub fn decode_uvarint(_src: &[u8]) -> Result<(u64, usize), Error> {
    Err(Error::Incomplete)
}

/// Decode a canonical Go `binary.Varint` byte sequence.
pub fn decode_varint(_src: &[u8]) -> Result<(i64, usize), Error> {
    Err(Error::Incomplete)
}
