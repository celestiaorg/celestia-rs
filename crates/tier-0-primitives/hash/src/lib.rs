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

// Placeholder — implementation will follow after failing tests are written.
