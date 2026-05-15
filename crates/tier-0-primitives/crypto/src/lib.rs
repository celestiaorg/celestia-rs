//! Public-key cryptography primitives for celestia-rs.
//!
//! This crate centralizes Ed25519, secp256k1, and BLS12-381 behavior used by
//! downstream protocol crates.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// BLS12-381 signature helpers.
pub mod bls12381;
/// `CometBFT` Ed25519 consensus signature helpers.
pub mod ed25519;
/// Cosmos SDK secp256k1 account signature helpers.
pub mod secp256k1;
