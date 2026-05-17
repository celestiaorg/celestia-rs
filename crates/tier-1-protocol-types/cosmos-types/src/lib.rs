//! Cosmos SDK protocol types for celestia-rs.
//!
//! See `docs/superpowers/specs/m1/cosmos-types.md` for the API contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![allow(clippy::doc_markdown)]

/// Celestia address helpers.
pub mod address;
/// Cosmos protobuf Any.
pub mod any;
/// Cosmos coin helpers.
pub mod coin;
/// Error types.
pub mod error;
/// Re-exported Cosmos SDK protobuf modules.
pub mod proto;
/// Cosmos transaction helpers.
pub mod tx;

pub use address::{
    ADDRESS_LENGTH, AccountAddress, ValconsAddress, ValoperAddress, account_address_from_bech32,
    account_address_to_bech32, valcons_address_from_bech32, valcons_address_to_bech32,
    valoper_address_from_bech32, valoper_address_to_bech32,
};
pub use any::Any;
pub use coin::Coin;
pub use error::{Error, Result};
pub use tx::{
    AuthInfo, Fee, ModeInfo, SignDoc, SignMode, SignerInfo, Tip, Tx, TxBody, TxRaw,
    secp256k1_public_key_any,
};
