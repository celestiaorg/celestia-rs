//! Port of celestia-core v0.42.3 `consensus/propagation` (ADR 0003): the
//! `RECOVERY` reactor that distributes proposals as signed compact blocks
//! plus erasure-coded parts.
//!
//! - The proposer erasure-codes its block part set (2x, [`erasure`]),
//!   signs a [`types::CompactBlock`] (proposal, parity root, every part's
//!   leaf hash, tx locations) and gives each peer a share of the parity
//!   parts to announce (`HaveParts`).
//! - Peers verify the compact block against the round's proposer, forward
//!   it, recover the parts whose txs are already in their mempool, request
//!   (`WantParts`) announced parts within a per-peer concurrency limit,
//!   relay `RecoveryPart`s to peers that want them, and decode the block as
//!   soon as any half of the original + parity parts arrived.
//! - Catch-up: when consensus learns of a commit / polka for a block it does
//!   not have ([`Propagator::add_commitment`]), the original parts are
//!   requested with proofs from peers, which serve them from the block
//!   store; compact blocks for future heights are cached per peer and
//!   replayed once consensus reaches that height and round.
//!
//! Consensus talks to the reactor through [`Propagator`] and receives
//! proposals and original parts through the [`Output`] sink.

#![forbid(unsafe_code)]
// Ported Go functions keep Go's structure (and length) for reviewability;
// Go's int / uint32 / int64 index conversions are kept as casts.
#![allow(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

use std::sync::Arc;

use ctypes::{Part, PartSet, PartSetHeader, Proposal, PubKey};

pub mod erasure;
pub mod peer_state;
pub mod reactor;
pub mod types;

pub use reactor::{Config, Reactor};
pub use types::{CompactBlock, TxMetaData};

/// `DataChannel`: compact blocks, haves and recovery parts.
pub const DATA_CHANNEL: u8 = 0x50;
/// `WantChannel`: wants.
pub const WANT_CHANNEL: u8 = 0x51;
/// `CompactBlockUID`: the unique id of compact block raw-bytes signatures.
pub const COMPACT_BLOCK_UID: &str = "compactBlock";
/// Peer data key under which the consensus reactor stores its
/// [`PeerStateEditor`] (Go keeps it under `types.PeerStateKey` and type
/// asserts).
pub const PEER_STATE_EDITOR_KEY: &str = "ConsensusReactor.peerStateEditor";

/// `PeerStateEditor`: the consensus reactor's per-peer state as the
/// propagation reactor updates it.
pub trait PeerStateEditor: Send + Sync {
    /// `SetHasProposal`.
    fn set_has_proposal(&self, proposal: &Proposal);
    /// `SetHasProposalBlockPart`.
    fn set_has_proposal_block_part(&self, height: i64, round: i32, index: usize);
    /// `GetHeight`.
    fn get_height(&self) -> i64;
}

/// The value stored under [`PEER_STATE_EDITOR_KEY`].
pub struct EditorHandle(pub Arc<dyn PeerStateEditor>);

/// `noOpPSE`.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoOpEditor;

impl PeerStateEditor for NoOpEditor {
    fn set_has_proposal(&self, _: &Proposal) {}
    fn set_has_proposal_block_part(&self, _: i64, _: i32, _: usize) {}
    fn get_height(&self) -> i64 {
        1
    }
}

/// What the reactor hands to consensus (Go's `proposalChan` / `partChan`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    /// A verified proposal (`ProposalAndSrc`); `from` is empty or our own
    /// ID for cached proposals.
    Proposal {
        /// The proposal.
        proposal: Proposal,
        /// Sender.
        from: String,
    },
    /// An original block part (`types.PartInfo`).
    Part {
        /// Height.
        height: i64,
        /// Round.
        round: i32,
        /// The part (with proof).
        part: Part,
    },
}

/// The sink consensus receives [`Output`]s through.
pub type OutputSink = Arc<dyn Fn(Output) + Send + Sync>;

/// `Propagator`: what consensus calls.
pub trait Propagator: Send + Sync {
    /// `GetProposal`: the proposal and original parts held for a height and
    /// round (cached, or loaded from the block store for past heights).
    fn get_proposal(&self, height: i64, round: i32) -> Option<(Proposal, PartSet)>;
    /// `ProposeBlock`: encodes, signs (with `signer.sign_raw_bytes`) and
    /// distributes our own proposal.
    fn propose_block(
        &self,
        proposal: &Proposal,
        parts: &PartSet,
        txs: Vec<TxMetaData>,
        signer: &dyn privval::PrivValidator,
    ) -> Result<(), String>;
    /// `AddCommitment`: start fetching the block with this part-set header.
    fn add_commitment(&self, height: i64, round: i32, psh: &PartSetHeader);
    /// `Prune`: forget everything below a committed height.
    fn prune(&self, committed_height: i64);
    /// `SetConsensusState`: the height, round and proposer compact blocks
    /// are verified against.
    fn set_consensus_state(&self, height: i64, round: i32, proposer: &PubKey);
    /// `StartProcessing`.
    fn start_processing(&self);
    /// `IsCatchingUp`: unfinished heights remain.
    fn is_catching_up(&self) -> bool;
}

/// `NoOpPropagator`.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoOpPropagator;

impl Propagator for NoOpPropagator {
    fn get_proposal(&self, _: i64, _: i32) -> Option<(Proposal, PartSet)> {
        None
    }
    fn propose_block(
        &self,
        _: &Proposal,
        _: &PartSet,
        _: Vec<TxMetaData>,
        _: &dyn privval::PrivValidator,
    ) -> Result<(), String> {
        Ok(())
    }
    fn add_commitment(&self, _: i64, _: i32, _: &PartSetHeader) {}
    fn prune(&self, _: i64) {}
    fn set_consensus_state(&self, _: i64, _: i32, _: &PubKey) {}
    fn start_processing(&self) {}
    fn is_catching_up(&self) -> bool {
        false
    }
}

/// `propagation.Mempool`: transaction lookup by key.
pub trait Mempool: Send + Sync {
    /// `GetTxByKey`.
    fn get_tx_by_key(&self, key: &[u8; 32]) -> Option<Vec<u8>>;
}

impl Mempool for mempool::TxPool {
    fn get_tx_by_key(&self, key: &[u8; 32]) -> Option<Vec<u8>> {
        mempool::TxPool::get_tx_by_key(self, key).map(|t| t.tx().to_vec())
    }
}

/// `ConcurrentRequestLimit`.
#[must_use]
pub fn concurrent_request_limit(peers_count: usize, parts_count: usize) -> i64 {
    if peers_count == 0 || parts_count == 0 {
        return 1;
    }
    let faulty_val_count = (peers_count as f64 * 0.33).ceil();
    let faulty_part_count = parts_count as f64 / 2.0;
    (faulty_part_count / faulty_val_count).ceil() as i64
}

/// `ReqLimit`.
#[must_use]
pub fn req_limit(parts_count: usize) -> usize {
    if parts_count == 0 {
        return 1;
    }
    (34.0 / parts_count as f64).max(1.0).ceil() as usize
}

/// `countRemainingParts`.
#[must_use]
pub fn count_remaining_parts(total_parts: usize, existing_parts: usize) -> i32 {
    (total_parts / 2).saturating_sub(existing_parts) as i32
}

#[cfg(test)]
mod tests;
