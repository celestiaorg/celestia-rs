//! Crawls a Celestia consensus P2P network and counts nodes by implementation.
//!
//! The crawler dials each known address with the node's own P2P stack
//! (SecretConnection + `DefaultNodeInfo` handshake), records the peer's
//! self-reported software version, and asks it for more addresses over PEX.
//! celestia-rs reports `0.38.17-celestia-rs.<version>`
//! (`docs/rpc-version-identity.md`); celestia-core (Go) reports a plain Comet
//! semantic version.

pub mod classify;
pub mod crawl;
pub mod networks;
pub mod report;
pub mod rpc;
