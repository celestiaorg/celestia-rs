//! Library part of the `celestia-rs` node binary (shared with its tests).

#![forbid(unsafe_code)]

#[cfg(feature = "bridge")]
mod bridge;
#[cfg(feature = "da-light")]
pub use bridge::peer::light::{LightArgs, run_light};
#[cfg(feature = "bridge")]
pub use bridge::auth::operator_token;
pub mod client;
pub mod config;
pub mod export;
pub mod follow;
mod follow_receipts;
pub mod genesis;
pub mod init;
pub mod keys;
pub mod local;
pub mod memstats;
pub mod metrics;
pub mod net;
pub mod networks;
pub mod node;
pub mod ops;
pub mod rollback;
pub mod rpc_service;
pub mod sync;
pub mod txs;
pub mod util;
pub mod version;
