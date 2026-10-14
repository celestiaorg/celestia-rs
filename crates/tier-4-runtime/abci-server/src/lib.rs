//! celestia-core ABCI++ socket server.
//!
//! - [`codec`]: uvarint length-delimited protobuf framing.
//! - [`server`]: tokio server driving an [`server::Application`].
//! - [`record`]: the capture format written by the `abci-recorder` proxy,
//!   used to build differential fixtures from a Go `celestia-appd`.

pub mod codec;
pub mod record;
pub mod server;

pub use server::{Application, serve};
