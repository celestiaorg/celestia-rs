//! Encoding primitives for celestia-rs.
//!
//! This crate provides protobuf helpers, Tendermint-compatible varints,
//! canonical hex, base64, and Celestia bech32 address helpers.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// Base64 encoding helpers.
pub mod base64;
/// Bech32 encoding helpers for Celestia addresses.
pub mod bech32;
/// Hex encoding helpers.
pub mod hex;
/// Protobuf encoding helpers backed by `prost`.
pub mod protobuf;
/// Tendermint-compatible varint encoding helpers.
pub mod varint;
