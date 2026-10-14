//! The consensus state a state-synced node bootstraps from
//! ([`nodestore::State`], Go `sm.State`) and the node-store interface that
//! receives it.

use ctypes::{BlockId, Commit, ConsensusParams, Time, ValidatorSet};
pub use nodestore::State;
pub use nodestore::state::TM_CORE_SEM_VER;
use nodestore::{BlockStore, StateStore, Timeouts};

/// An empty state for `chain_id` (`sm.State{}` with the given fields set).
#[must_use]
pub fn empty_state(chain_id: &str, version: ctypes::Consensus, initial_height: i64) -> State {
    State {
        version,
        software: TM_CORE_SEM_VER.to_owned(),
        chain_id: chain_id.to_owned(),
        initial_height,
        last_block_height: 0,
        last_block_id: BlockId::default(),
        last_block_time: Time::default(),
        next_validators: ValidatorSet::default(),
        validators: ValidatorSet::default(),
        last_validators: ValidatorSet::default(),
        last_height_validators_changed: 0,
        consensus_params: ConsensusParams::default(),
        last_height_consensus_params_changed: 0,
        last_results_hash: Vec::new(),
        app_hash: Vec::new(),
        timeouts: Timeouts::default(),
    }
}

/// What a node persists after state sync (Go `startStateSync`:
/// `stateStore.Bootstrap(state)`, then `blockStore.SaveSeenCommit(h, commit)`).
pub trait Bootstrapper: Send + Sync {
    /// `state.Store.Bootstrap`: validators at h-1, h, h+1, consensus params
    /// and the state itself.
    fn bootstrap_state(&self, state: &State) -> Result<(), String>;

    /// `BlockStore.SaveSeenCommit(height, commit)`.
    fn save_seen_commit(&self, height: i64, commit: &Commit) -> Result<(), String>;

    /// Both steps, in Go's order.
    fn bootstrap(&self, state: &State, commit: &Commit) -> Result<(), String> {
        self.bootstrap_state(state)
            .map_err(|e| format!("Failed to bootstrap node with new state: {e}"))?;
        self.save_seen_commit(state.last_block_height, commit)
            .map_err(|e| format!("Failed to store last seen commit: {e}"))
    }
}

/// The node's stores ([`nodestore::StateStore`] + [`nodestore::BlockStore`]).
#[derive(Debug)]
pub struct NodeStores {
    /// State store.
    pub state_store: StateStore,
    /// Block store.
    pub block_store: BlockStore,
}

impl Bootstrapper for NodeStores {
    fn bootstrap_state(&self, state: &State) -> Result<(), String> {
        self.state_store.bootstrap(state).map_err(|e| e.to_string())
    }

    fn save_seen_commit(&self, height: i64, commit: &Commit) -> Result<(), String> {
        self.block_store
            .save_seen_commit(height, commit)
            .map_err(|e| e.to_string())
    }
}
