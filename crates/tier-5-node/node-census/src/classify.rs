//! Implementation of a node, from its P2P `NodeInfo.version`.

use serde::{Deserialize, Serialize};

/// The node software family behind a `NodeInfo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Implementation {
    /// celestia-rs.
    Rust,
    /// celestia-core (Go).
    Go,
    /// Neither pattern matched.
    Unknown,
}

/// The suffix marker celestia-rs appends to the Comet compatibility version.
pub const RUST_MARKER: &str = "-celestia-rs";

/// Classifies a `NodeInfo.version` string.
#[must_use]
pub fn classify(version: &str) -> Implementation {
    if version.contains(RUST_MARKER) {
        return Implementation::Rust;
    }
    let core = version.split(['-', '+']).next().unwrap_or_default();
    let parts: Vec<u64> = core.split('.').map_while(|p| p.parse().ok()).collect();
    // celestia-core releases: Comet 0.34.x / 0.38.x semvers, or 1.x and later.
    match parts.as_slice() {
        [major, minor, _] if *major >= 1 || *minor >= 34 => Implementation::Go,
        _ => Implementation::Unknown,
    }
}
