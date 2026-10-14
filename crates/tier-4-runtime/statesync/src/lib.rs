//! Exact port of celestia-core v0.42.3 `statesync/` (ADR 0003).
//!
//! - [`reactor`]: the p2p reactor on channels 0x60 (snapshots) and 0x61
//!   (chunks) with Go's wire messages ([`messages`]); it serves the local
//!   app's snapshots and feeds received ones into a running sync.
//! - [`syncer`]: snapshot selection, `OfferSnapshot` / `ApplySnapshotChunk`
//!   with Go's result-code handling (retry, refetch, reject senders,
//!   reject snapshot/format, abort), parallel chunk fetchers with request
//!   timeouts, and app verification (app hash, height, app version).
//! - [`chunks`] / [`snapshots`]: the on-disk chunk queue and the snapshot pool.
//! - [`stateprovider`]: trusted app hashes, commits and the bootstrap
//!   [`State`], verified with the `light` client against RPC servers.
//! - [`state`]: `state.State` and the [`Bootstrapper`] interface the node
//!   stores implement (`Bootstrap(state)` + `SaveSeenCommit(h, commit)`).
//! - [`conn`]: the ABCI snapshot/info calls, with an adapter for in-process
//!   apps.

#![forbid(unsafe_code)]

pub mod chunks;
pub mod conn;
pub mod messages;
pub mod reactor;
pub mod snapshots;
pub mod state;
pub mod stateprovider;
pub mod syncer;

pub use conn::{AppConn, InProcessConn};
pub use messages::{CHUNK_CHANNEL, SNAPSHOT_CHANNEL};
pub use reactor::{P2pPeer, Reactor};
pub use snapshots::{PeerHandle, PeerRef, Snapshot, SnapshotPool};
pub use state::{Bootstrapper, NodeStores, State, empty_state};
pub use stateprovider::{LightClientStateProvider, ParamsSource, StateProvider};
pub use syncer::{Config, SyncError, Syncer};
