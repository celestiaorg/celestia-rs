//! Fibre data availability server: the Rust port of celestia-app v11's
//! `fibre.Server`. A validator accepts the blob shards assigned to it,
//! verifies them, stores them until the payment promise expires, signs the
//! promise with its consensus key and serves the shards to download clients.
//!
//! The crate does not depend on the node: the node supplies chain state and
//! the consensus key through [`ChainState`] and [`Signer`], fills a
//! [`ServerConfig`], and drives a [`Server`] (`new`, `start`, `stop`). The
//! server speaks gRPC over TLS 1.3 on its own port (`celestia.fibre.v1.Fibre`
//! and `grpc.health.v1.Health`), with Go's wire behavior.

// `tonic::Status` is the gRPC error type handlers return, as in tonic's
// generated services.
#![allow(clippy::result_large_err)]

pub mod assignment;
pub mod codec;
pub mod config;
mod godur;
pub mod health;
pub mod metrics;
pub mod occupancy;
pub mod params;
pub mod proto;
mod server;
mod service;
pub mod state;
pub mod store;
pub mod tlsid;
mod transport;
pub mod verify;

pub use config::{HealthConfig, ServerConfig};
pub use metrics::ServerMetrics;
pub use server::{Server, StartError, derive_budget};
pub use service::{CallContext, shard_prune_at};
pub use state::{
    BoxFuture, ChainState, NodeStatus, ProviderRegistration, Signer, SignerError, StateError,
    Validator, ValidatorSet, VerifiedPromise,
};
pub use transport::{FIBRE_SERVICE, HEALTH_SERVICE};
