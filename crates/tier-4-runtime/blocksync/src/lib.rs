//! Exact port of celestia-core v0.42.3 `blocksync/` (ADR 0003).
//!
//! - [`msgs`]: the channel 0x40 wire messages, `ValidateMsg` and the
//!   signature-count precheck run before decoding a `BlockResponse`.
//! - [`pool`]: the [`BlockPool`] — up to 40 requesters, one per height,
//!   each asking one peer at a time; peers report their `[base, height]`
//!   range, are dropped when they time out (120 s without a block, or a
//!   receive rate below 128 KB/s) and banned for 60 s when they lie or
//!   serve a bad block; request limits and retry timeouts adapt to the
//!   size of recent blocks.
//! - [`reactor`]: the p2p reactor (serving blocks from the
//!   [`nodestore::BlockStore`], with extended commits when vote extensions
//!   are enabled) and the sync routine: verify block `H` with the
//!   `LastCommit` of `H+1` (full `VerifyCommit`), validate it against the
//!   state, ask the app (`ProcessProposal`), check the extended commit,
//!   save and execute it (`ApplyVerifiedBlock`), and switch to consensus
//!   once caught up.
//!
//! Go's goroutines (the pool's requester routines, timers) are replaced by
//! a pool that is driven by [`BlockPool::tick`] with an explicit clock, so
//! the same code runs in deterministic tests and in the node (a thread
//! ticking every 2 ms).

#![forbid(unsafe_code)]
// Ported Go functions keep Go's structure (and length) for reviewability.
#![allow(clippy::too_many_lines)]

pub mod block_stats;
pub mod flowrate;
pub mod msgs;
pub mod pool;
pub mod reactor;

pub use msgs::BLOCKSYNC_CHANNEL;
pub use pool::{BlockPool, BlockRequest, PeerError};
pub use reactor::{Reactor, ReactorOptions, SwitchToConsensus, SyncOutcome};
