//! Exact port of celestia-core v0.42.3 `light/` (ADR 0003): the light
//! client used by state sync.
//!
//! - [`verifier`]: `VerifyAdjacent`, `VerifyNonAdjacent`, `Verify`,
//!   `VerifyBackwards`, trusting period, trust level and clock drift checks,
//!   with Go's exact error strings.
//! - [`client`]: `light.Client` — trust options, sequential and skipping
//!   (bisection) verification, backwards verification, primary replacement
//!   and witness management.
//! - [`detector`]: divergence detection against witnesses, bifurcation
//!   search and `LightClientAttackEvidence` for both sides.
//! - [`store`]: the trusted store (`light/store/db`) on a [`kvdb::KvBackend`].
//! - [`provider`] / [`http`]: the provider trait, Go's mock providers and
//!   the JSON-RPC provider talking to celestia-core RPC (`/commit`,
//!   `/validators`, `/status`, `/consensus_params`, `/block`; HTTPS
//!   through the system `curl`).
//!
//! Outcomes are checked against celestia-core itself
//! (`conformance/lightvectors`).

#![forbid(unsafe_code)]

pub mod client;
pub mod detector;
pub mod errors;
pub mod http;
pub mod provider;
pub mod store;
pub mod verifier;

pub use client::{Client, Mode, Options, TrustOptions};
pub use errors::{Error, ProviderError, Result, StoreError};
pub use http::{HttpProvider, RpcClient};
pub use provider::{DeadMock, MockProvider, Provider};
pub use store::{DbStore, Store};
pub use verifier::{
    DEFAULT_TRUST_LEVEL, header_expired, validate_trust_level, verify, verify_adjacent,
    verify_backwards, verify_non_adjacent,
};

use ctypes::{LightBlock, Time};

/// `LightBlock.Hash()` (the header hash; empty when absent).
#[must_use]
pub fn lightblock_hash(l: &LightBlock) -> Vec<u8> {
    l.header()
        .and_then(ctypes::Header::hash)
        .unwrap_or_default()
}

/// `LightBlock.Height`.
#[must_use]
pub fn lightblock_height(l: &LightBlock) -> i64 {
    l.header().map_or(0, |h| h.height)
}

/// `LightBlock.Time`.
#[must_use]
pub fn lightblock_time(l: &LightBlock) -> Time {
    l.header().map(|h| h.time).unwrap_or_default()
}
