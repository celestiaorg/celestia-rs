//! Port of celestia-core v0.42.3 `store/` and `state/` (ADR 0003): the
//! block store and state store on the ADR 0002 [`kvdb::KvBackend`], the
//! chain [`State`], block creation / validation / execution against the
//! in-process application ([`execution::AppConn`]) and the crash-recovery
//! [`Handshaker`].

#![forbid(unsafe_code)]
// Ported Go functions keep Go's structure (and length) for reviewability.
#![allow(clippy::too_many_lines, clippy::many_single_char_names)]

pub mod block_store;
pub mod evidence;
pub mod execution;
pub mod genesis;
pub mod handshake;
pub mod state;
pub mod state_store;

pub use block_store::BlockStore;
pub use evidence::{EmptyEvidencePool, EvidencePool, MemEvidencePool};
pub use execution::{AppConn, BlockEventSink, BlockExecutor, FiredBlock, Mempool, NopMempool};
pub use genesis::GenesisDoc;
pub use handshake::Handshaker;
pub use state::{State, Timeouts};
pub use state_store::StateStore;

use std::fmt;

/// A store / execution error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl Error {
    /// Creates an error.
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// Result alias.
pub type Result<T, E = Error> = core::result::Result<T, E>;

pub(crate) fn ctypes_err(e: ctypes::Error) -> Error {
    Error(e.to_string())
}

pub(crate) fn kv_err(e: kvdb::Error) -> Error {
    Error(e.to_string())
}

#[cfg(test)]
mod tests;
