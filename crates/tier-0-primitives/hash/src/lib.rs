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

// Placeholder — public API will be added next (implementation step).

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
}
