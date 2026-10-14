//! Cosmos SDK runtime for celestia-rs.
//!
//! Mirrors the parts of `baseapp`/`types`/`store` that every module needs:
//!
//! - [`store::MultiStore`]: committed IAVL stores plus nested write-back
//!   cache layers with `cachekv` flush ordering.
//! - [`context::Context`]: header, gas meter, events and gas-metered KV
//!   access with `gaskv` charging semantics.
//! - [`error::SdkError`]: ABCI codespace/code errors.
//! - [`codec`]: `collections` key encodings.

pub mod codec;
pub mod context;
pub mod error;
pub mod event;
pub mod store;

pub use context::{Context, ExecMode, Header};
pub use error::{Result, SdkError};
pub use store::{Kind, MultiStore};
