//! CometBFT ABCI++ protocol types for celestia-rs.
//!
//! See `docs/specs/m1/abci-types.md` for the API contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![allow(
    clippy::doc_markdown,
    clippy::large_enum_variant,
    clippy::needless_pass_by_value
)]

/// Error types.
pub mod error;
/// ABCI event types.
pub mod event;
/// Transaction execution result types.
pub mod exec;
/// Re-exported CometBFT protobuf modules.
pub mod proto;
/// ABCI request envelope types.
pub mod request;
/// ABCI response envelope types.
pub mod response;
/// State-sync snapshot types.
pub mod snapshot;
/// ABCI validator and misbehavior types.
pub mod validator;
/// Commit and vote-extension types.
pub mod vote_extension;

mod validation;

pub use error::{Error, Result};
pub use event::{Event, EventAttribute};
pub use exec::ExecTxResult;
pub use request::{Request, RequestValue};
pub use response::{Response, ResponseValue};
pub use snapshot::Snapshot;
pub use validator::{Misbehavior, MisbehaviorType, Validator, ValidatorUpdate};
pub use vote_extension::{CommitInfo, ExtendedCommitInfo, ExtendedVoteInfo, VoteInfo};
