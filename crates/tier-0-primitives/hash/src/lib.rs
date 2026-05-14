//! Cryptographic hash primitives for celestia-rs.
//!
//! This crate provides sha256, sha512, ripemd160, and Tendermint's
//! length-prefix-then-sha256 (`tmhash_len_prefixed`). It is a thin façade
//! over the [RustCrypto](https://github.com/RustCrypto/hashes) family.
//!
//! No other crate in the workspace may pull in `sha2`, `sha3`, or `ripemd`
//! directly — use this crate instead.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use digest::{Digest, Update};
use ripemd::Ripemd160;
use sha2::{Sha256, Sha512};

/// Output of SHA-256: 32 bytes.
pub type Sha256Digest = [u8; 32];

/// Output of SHA-512: 64 bytes.
pub type Sha512Digest = [u8; 64];

/// Output of RIPEMD-160: 20 bytes.
pub type Ripemd160Digest = [u8; 20];

/// Compute the SHA-256 digest of `input`.
///
/// # Examples
/// ```
/// let digest = hash::sha256(b"");
/// assert_eq!(
///     hex::encode(digest),
///     "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
/// );
/// ```
#[must_use]
pub fn sha256(input: &[u8]) -> Sha256Digest {
    Sha256::digest(input).into()
}

/// Compute the SHA-512 digest of `input`.
///
/// # Examples
/// ```
/// let digest = hash::sha512(b"");
/// assert_eq!(digest.len(), 64);
/// ```
#[must_use]
pub fn sha512(input: &[u8]) -> Sha512Digest {
    Sha512::digest(input).into()
}

/// Compute the RIPEMD-160 digest of `input`.
///
/// # Examples
/// ```
/// let digest = hash::ripemd160(b"");
/// assert_eq!(
///     hex::encode(digest),
///     "9c1185a5c5e9fc54612808977ee8f548b2258d31"
/// );
/// ```
#[must_use]
pub fn ripemd160(input: &[u8]) -> Ripemd160Digest {
    Ripemd160::digest(input).into()
}

/// An incremental SHA-256 hasher.
///
/// Allows feeding data in multiple chunks and then finalising to a
/// [`Sha256Digest`].  Produces byte-identical output to [`sha256`] for the
/// same bytes regardless of chunking.
///
/// # Examples
/// ```
/// let mut h = hash::Sha256Hasher::new();
/// h.update(b"hel");
/// h.update(b"lo");
/// let digest = h.finalize();
/// assert_eq!(digest, hash::sha256(b"hello"));
/// ```
pub struct Sha256Hasher {
    inner: Sha256,
}

impl Sha256Hasher {
    /// Create a new [`Sha256Hasher`] with an empty state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Sha256::new(),
        }
    }

    /// Feed a chunk of bytes into the hasher.
    pub fn update(&mut self, chunk: &[u8]) {
        Update::update(&mut self.inner, chunk);
    }

    /// Consume the hasher and return the final digest.
    #[must_use]
    pub fn finalize(self) -> Sha256Digest {
        self.inner.finalize().into()
    }
}

impl Default for Sha256Hasher {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute Tendermint's length-prefixed SHA-256 hash of `input`.
///
/// This is `sha256(uvarint(len(input)) || input)`, where `uvarint` is
/// Go's `encoding/binary.PutUvarint` (unsigned LEB-128).
///
/// Used by CometBFT/Tendermint to hash leaf and inner nodes in the simple
/// Merkle tree.
///
/// # Examples
/// ```
/// // tmhash of empty input is sha256 of the single byte 0x00.
/// assert_eq!(hash::tmhash_len_prefixed(b""), hash::sha256(&[0u8]));
/// ```
#[must_use]
pub fn tmhash_len_prefixed(input: &[u8]) -> Sha256Digest {
    let mut buf = [0u8; 10]; // max 10 bytes for a 64-bit uvarint
    let prefix_len = put_uvarint(&mut buf, input.len() as u64);
    let mut hasher = Sha256Hasher::new();
    hasher.update(&buf[..prefix_len]);
    hasher.update(input);
    hasher.finalize()
}

/// Encode `v` as an unsigned LEB-128 (Go `encoding/binary.PutUvarint`)
/// into `buf`, returning the number of bytes written.
///
/// `buf` must be at least 10 bytes long.
fn put_uvarint(buf: &mut [u8; 10], mut v: u64) -> usize {
    let mut i = 0;
    while v >= 0x80 {
        // Keep only the low 7 bits and set the continuation flag.
        buf[i] = (v & 0x7f) as u8 | 0x80;
        v >>= 7;
        i += 1;
    }
    // v < 0x80 at this point; mask to keep only the low 7 bits (lossless).
    buf[i] = (v & 0x7f) as u8;
    i + 1
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    // ---- REQ-1 -------------------------------------------------------
    #[test]
    fn sha256_empty() {
        let digest = sha256(b"");
        let hex = hex::encode(digest);
        assert_eq!(
            hex,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    // ---- REQ-2 -------------------------------------------------------
    #[test]
    fn sha256_abc() {
        let digest = sha256(b"abc");
        let hex = hex::encode(digest);
        assert_eq!(
            hex,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    // ---- REQ-3 -------------------------------------------------------
    #[test]
    fn sha512_empty() {
        let digest = sha512(b"");
        let hex = hex::encode(digest);
        assert_eq!(
            hex,
            "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"
        );
    }

    // ---- REQ-4 -------------------------------------------------------
    #[test]
    fn ripemd160_empty() {
        let digest = ripemd160(b"");
        let hex = hex::encode(digest);
        assert_eq!(hex, "9c1185a5c5e9fc54612808977ee8f548b2258d31");
    }

    // ---- REQ-5 -------------------------------------------------------
    #[test]
    fn sha256_hasher_matches_sha256_single_chunk() {
        let input = b"hello, world";
        let expected = sha256(input);
        let mut h = Sha256Hasher::new();
        h.update(input);
        assert_eq!(h.finalize(), expected);
    }

    #[test]
    fn sha256_hasher_matches_sha256_multiple_chunks() {
        let input = b"hello, world";
        let expected = sha256(input);
        let mut h = Sha256Hasher::new();
        // Feed in 3-byte chunks.
        for chunk in input.chunks(3) {
            h.update(chunk);
        }
        assert_eq!(h.finalize(), expected);
    }

    #[test]
    fn sha256_hasher_default_equals_new() {
        let input = b"test default";
        let mut h1 = Sha256Hasher::new();
        h1.update(input);
        let mut h2 = Sha256Hasher::default();
        h2.update(input);
        assert_eq!(h1.finalize(), h2.finalize());
    }

    // ---- REQ-6 -------------------------------------------------------
    #[test]
    fn tmhash_empty() {
        // tmhash_len_prefixed(b"") == sha256([0u8])
        let expected = sha256(&[0u8]);
        assert_eq!(tmhash_len_prefixed(b""), expected);
    }

    #[test]
    fn tmhash_single_zero_byte() {
        // tmhash_len_prefixed(b"\x00") == sha256(&[0x01, 0x00])
        let expected = sha256(&[0x01, 0x00]);
        assert_eq!(tmhash_len_prefixed(b"\x00"), expected);
    }

    // uvarint boundary: 127 bytes -> prefix 0x7f (1 byte)
    #[test]
    fn tmhash_127_bytes() {
        let input = vec![0xabu8; 127];
        let mut prefixed = vec![0x7fu8];
        prefixed.extend_from_slice(&input);
        let expected = sha256(&prefixed);
        assert_eq!(tmhash_len_prefixed(&input), expected);
    }

    // uvarint boundary: 128 bytes -> prefix [0x80, 0x01] (2 bytes)
    #[test]
    fn tmhash_128_bytes() {
        let input = vec![0xabu8; 128];
        let mut prefixed = vec![0x80u8, 0x01u8];
        prefixed.extend_from_slice(&input);
        let expected = sha256(&prefixed);
        assert_eq!(tmhash_len_prefixed(&input), expected);
    }

    // uvarint: verify put_uvarint helper directly
    #[test]
    fn put_uvarint_zero() {
        let mut buf = [0u8; 10];
        let n = put_uvarint(&mut buf, 0);
        assert_eq!(n, 1);
        assert_eq!(buf[0], 0x00);
    }

    #[test]
    fn put_uvarint_127() {
        let mut buf = [0u8; 10];
        let n = put_uvarint(&mut buf, 127);
        assert_eq!(n, 1);
        assert_eq!(buf[0], 0x7f);
    }

    #[test]
    fn put_uvarint_128() {
        let mut buf = [0u8; 10];
        let n = put_uvarint(&mut buf, 128);
        assert_eq!(n, 2);
        assert_eq!(&buf[..2], &[0x80, 0x01]);
    }

    #[test]
    fn put_uvarint_300() {
        // 300 = 0b100101100, encoded as 0xac 0x02
        let mut buf = [0u8; 10];
        let n = put_uvarint(&mut buf, 300);
        assert_eq!(n, 2);
        assert_eq!(&buf[..2], &[0xac, 0x02]);
    }
}
