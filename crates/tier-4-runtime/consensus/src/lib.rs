//! Port of the celestia-core v0.42.3 consensus core (ADR 0003): the
//! Tendermint round state machine with celestia's app-driven timeouts and
//! delayed precommit ([`state::ConsensusState`]), vote sets with
//! conflicting-vote detection ([`vote_set::VoteSet`],
//! [`height_vote_set::HeightVoteSet`]), the timeout ticker, the write-ahead
//! log in Go's format with `#ENDHEIGHT` crash recovery ([`wal`]), the
//! consensus wire messages ([`types::ConsensusMessage`]), the consensus
//! reactor on the `p2p` switch ([`reactor`], with peer round states in
//! [`peer_state`]), the consensus thread ([`service`]) wired to the block
//! propagation reactor (`propagation` crate), and a minimal gossip layer
//! over an abstract [`gossip::Network`] for deterministic simulations.

#![forbid(unsafe_code)]
// Ported Go functions keep Go's structure (and length) for reviewability.
#![allow(clippy::too_many_lines, clippy::cast_sign_loss)]

pub mod gossip;
pub mod height_vote_set;
pub mod peer_state;
pub mod reactor;
pub mod service;
pub mod state;
pub mod ticker;
pub mod types;
pub mod vote_set;
pub mod wal;

pub use gossip::{Gossip, Network};
pub use state::{ConsensusConfig, ConsensusState, Event, MsgInfo};
pub use ticker::{Clock, SystemClock};
pub use types::{ConsensusMessage, RoundState, RoundStep, TimeoutInfo};
pub use vote_set::{BitArray, VoteError, VoteSet};
pub use wal::{FileWal, GroupOptions, MemWal, NilWal, Wal};

#[cfg(test)]
mod tests;
