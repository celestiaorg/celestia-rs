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
pub const fn encoded_uvarint_len(mut value: u64) -> usize {
    let mut len = 1;
    while value >= 0x80 {
        value >>= 7;
        len += 1;
    }
    len
}

/// Return the canonical byte length of `value` when encoded as a signed varint.
#[must_use]
pub const fn encoded_varint_len(value: i64) -> usize {
    encoded_uvarint_len(signed_to_uvarint(value))
}

/// Encode `value` as Go `binary.PutUvarint` bytes.
#[must_use]
pub fn encode_uvarint(value: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(encoded_uvarint_len(value));
    append_uvarint(&mut out, value);
    out
}

/// Encode `value` as Go `binary.PutVarint` bytes.
#[must_use]
pub fn encode_varint(value: i64) -> Vec<u8> {
    encode_uvarint(signed_to_uvarint(value))
}

/// Encode `value` into `dst`, returning the number of bytes written.
pub fn put_uvarint(dst: &mut [u8], value: u64) -> Result<usize, Error> {
    let required = encoded_uvarint_len(value);
    if dst.len() < required {
        return Err(Error::BufferTooSmall {
            required,
            available: dst.len(),
        });
    }

    let mut value = value;
    let mut i = 0;
    while value >= 0x80 {
        dst[i] = (value as u8 & 0x7f) | 0x80;
        value >>= 7;
        i += 1;
    }
    dst[i] = value as u8;
    Ok(required)
}

/// Encode `value` into `dst`, returning the number of bytes written.
pub fn put_varint(dst: &mut [u8], value: i64) -> Result<usize, Error> {
    put_uvarint(dst, signed_to_uvarint(value))
}

/// Decode a canonical Go `binary.Uvarint` byte sequence.
pub fn decode_uvarint(src: &[u8]) -> Result<(u64, usize), Error> {
    let mut value = 0u64;

    for (i, byte) in src.iter().copied().enumerate() {
        if i == MAX_VARINT_LEN64 {
            return Err(Error::Overflow);
        }

        if byte < 0x80 {
            if i == MAX_VARINT_LEN64 - 1 && byte > 1 {
                return Err(Error::Overflow);
            }
            value |= u64::from(byte) << (i * 7);
            let consumed = i + 1;
            if encoded_uvarint_len(value) != consumed {
                return Err(Error::NonCanonical);
            }
            return Ok((value, consumed));
        }

        if i == MAX_VARINT_LEN64 - 1 {
            return Err(Error::Overflow);
        }
        value |= u64::from(byte & 0x7f) << (i * 7);
    }

    Err(Error::Incomplete)
}

/// Decode a canonical Go `binary.Varint` byte sequence.
pub fn decode_varint(src: &[u8]) -> Result<(i64, usize), Error> {
    let (encoded, consumed) = decode_uvarint(src)?;
    let shifted = (encoded >> 1) as i64;
    let value = if encoded & 1 == 0 {
        shifted
    } else {
        !shifted
    };

    if encoded_varint_len(value) != consumed {
        return Err(Error::NonCanonical);
    }

    Ok((value, consumed))
}

const fn signed_to_uvarint(value: i64) -> u64 {
    let shifted = (value as u64) << 1;
    if value < 0 { !shifted } else { shifted }
}

fn append_uvarint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}
