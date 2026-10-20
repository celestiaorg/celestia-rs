//! CometBFT protocol types for celestia-rs.
//!
//! See `docs/specs/m1/tendermint-types.md` for the API contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![allow(clippy::doc_markdown, clippy::needless_pass_by_value)]

/// Block IDs, part-set headers, and block headers.
pub mod block;
/// Canonical sign-byte encoding helpers.
pub mod canonical;
/// Commit and commit-signature types.
pub mod commit;
/// Error types.
pub mod error;
/// Hash and address helpers.
pub mod hash;
/// Proposal types.
pub mod proposal;
/// Re-exported CometBFT protobuf modules.
pub mod proto;
/// Validator and validator-set types.
pub mod validator;
/// Vote types.
pub mod vote;

pub use block::{BlockId, ConsensusVersion, Header, PartSetHeader};
pub use commit::{BlockIdFlag, Commit, CommitSig};
pub use error::{Error, Result};
pub use hash::{ADDRESS_LENGTH, Address, HASH_LENGTH, Hash};
pub use proposal::Proposal;
pub use validator::{PublicKey, Validator, ValidatorSet};
pub use vote::{SignedMsgType, Vote};
