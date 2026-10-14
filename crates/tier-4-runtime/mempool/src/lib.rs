//! celestia-core v0.42.3 mempool (ADR 0003, M10): a port of `mempool/`
//! (interface, tx caches, errors, pre/post-check filters) and of the CAT
//! ("content addressable transactions") pool and reactor in
//! `mempool/cat`.
//!
//! - [`cat::TxPool`]: the priority mempool. Transactions are grouped into
//!   per-signer sets ordered by sequence; sets are ordered by gas-weighted
//!   aggregated priority. Eviction, TTL (blocks / time), the rejected and
//!   evicted tx caches, recheck after commit and `ReapMaxBytesMaxGas` follow
//!   Go exactly.
//! - [`cat::Reactor`]: the SeenTx / WantTx / Txs protocol on channels
//!   `0x31` / `0x32` (plus the legacy `0x30`), with request scheduling,
//!   sequence-aware gap filling via `QuerySequence`, sticky-peer SeenTx
//!   broadcast and the out-of-order receive buffer.
//! - [`Mempool`] / [`MempoolLock`]: the API consensus calls into
//!   (`ReapMaxBytesMaxGas`, `Lock` + `Update`, `TxsAvailable`, ...).
//! - [`AppConnMempool`]: the in-process ABCI mempool connection
//!   (`CheckTx` New / Recheck, `QuerySequence`).

pub mod cache;
pub mod cat;
pub mod errors;
pub mod proxy;
mod types;

pub use cache::{LruTxCache, RejectedTxCache};
pub use cat::{Reactor, ReactorOptions, TxPool};
pub use errors::MempoolError;
pub use proxy::AppConnMempool;
pub use types::{
    CachedTx, Mempool, MempoolConfig, MempoolLock, PostCheckFunc, PreCheckFunc, TxInfo, TxKey,
    compute_proto_size_for_tx, post_check_max_gas, pre_check_max_bytes, tx_key,
};

/// ABCI types used by the mempool connection.
pub use celestia_core_proto::abci;

/// `MempoolChannel`: the legacy flooding channel, still registered by CAT.
pub const MEMPOOL_CHANNEL: u8 = 0x30;
/// `UnknownPeerID`: the sender ID of txs that come from RPC.
pub const UNKNOWN_PEER_ID: u16 = 0;
/// `MaxActiveIDs`.
pub const MAX_ACTIVE_IDS: usize = u16::MAX as usize;
/// `MaxTxsPerMessage`: transaction batching is disabled.
pub const MAX_TXS_PER_MESSAGE: usize = 1;
