//! `Handshaker` (`consensus/replay.go`): brings the app and the node's
//! stores in sync after a restart or crash.
//!
//! With `A` = app height (`Info`), `S` = block store height and `T` = state
//! height, the stores guarantee `T <= S <= T + 1`; the cases are Go's:
//!
//! | case | action |
//! |---|---|
//! | `A == 0` | `InitChain` (and, at genesis, seed the state from its response) |
//! | `S == 0` | nothing to replay; app hash must equal the state's |
//! | `A < base - 1` (or `A == 0` with pruned genesis) | error: app too far behind |
//! | `S < A` | error: app ahead of the block store |
//! | `S == T`, `A < S` | replay `A+1..=S` on the app only |
//! | `S == T`, `A == S` | in sync |
//! | `S == T+1`, `A < T` | replay `A+1..=T` on the app, then apply `S` with the real app |
//! | `S == T+1`, `A == T` | apply block `S` with the real app |
//! | `S == T+1`, `A == S` | app committed `S` but the state was not saved: apply `S` against a mock app returning the stored `FinalizeBlock` response |

use ctypes::proto::abci;
use ctypes::{Time, Validator, ValidatorSet};

use crate::block_store::BlockStore;
use crate::evidence::EmptyEvidencePool;
use crate::execution::{AppConn, BlockExecutor, NopMempool, exec_commit_block};
use crate::genesis::GenesisDoc;
use crate::state::{State, Timeouts, validator_updates_from_abci, validator_updates_to_abci};
use crate::state_store::StateStore;
use crate::{Error, Result, ctypes_err};

/// `newMockProxyApp`: replays a stored FinalizeBlock response.
#[derive(Debug)]
struct MockApp {
    resp: abci::ResponseFinalizeBlock,
}

impl AppConn for MockApp {
    fn info(&mut self) -> Result<abci::ResponseInfo> {
        Ok(abci::ResponseInfo::default())
    }
    fn init_chain(&mut self, _: abci::RequestInitChain) -> Result<abci::ResponseInitChain> {
        Ok(abci::ResponseInitChain::default())
    }
    fn prepare_proposal(
        &mut self,
        _: abci::RequestPrepareProposal,
    ) -> Result<abci::ResponsePrepareProposal> {
        Err(Error::new("mock app: PrepareProposal"))
    }
    fn process_proposal(
        &mut self,
        _: abci::RequestProcessProposal,
    ) -> Result<abci::ResponseProcessProposal> {
        Err(Error::new("mock app: ProcessProposal"))
    }
    fn finalize_block(
        &mut self,
        _: abci::RequestFinalizeBlock,
    ) -> Result<abci::ResponseFinalizeBlock> {
        Ok(self.resp.clone())
    }
    fn commit(&mut self) -> Result<abci::ResponseCommit> {
        Ok(abci::ResponseCommit::default())
    }
}

/// `Handshaker`.
#[derive(Debug)]
pub struct Handshaker<'a> {
    state_store: StateStore,
    block_store: BlockStore,
    gen_doc: &'a GenesisDoc,
    initial_state: State,
    n_blocks: usize,
    now: Time,
}

/// `assertAppHashEqualsOneFromState` (Go panics).
fn check_state_app_hash(app_hash: &[u8], state: &State) -> Result<()> {
    if app_hash != state.app_hash.as_slice() {
        return Err(Error::new(format!(
            "state.AppHash does not match AppHash after replay. Got {}, expected {}. Did you reset CometBFT without resetting your application's data?",
            hex::encode_upper(app_hash),
            hex::encode_upper(&state.app_hash)
        )));
    }
    Ok(())
}

impl<'a> Handshaker<'a> {
    /// `NewHandshaker`. `now` is the local clock for block validation.
    #[must_use]
    pub fn new(
        state_store: StateStore,
        state: State,
        block_store: BlockStore,
        gen_doc: &'a GenesisDoc,
        now: Time,
    ) -> Self {
        Self {
            state_store,
            block_store,
            gen_doc,
            initial_state: state,
            n_blocks: 0,
            now,
        }
    }

    /// Blocks applied (to app or state) during the handshake.
    #[must_use]
    pub fn n_blocks(&self) -> usize {
        self.n_blocks
    }

    /// `Handshake`: `Info`, then replay. Returns the state to start
    /// consensus from (reloaded from the store).
    pub fn handshake(&mut self, exec: &mut BlockExecutor) -> Result<State> {
        let info = exec.app().info()?;
        let app_height = info.last_block_height;
        if app_height < 0 {
            return Err(Error::new(format!(
                "got a negative last block height ({app_height}) from the app"
            )));
        }
        tracing::info!(
            height = app_height,
            hash = %hex::encode_upper(&info.last_block_app_hash),
            protocol_version = info.app_version,
            "ABCI Handshake App Info"
        );
        let mut state = self.initial_state.clone();
        if state.last_block_height == 0 && state.version.app == 0 && info.app_version != 0 {
            state.version.app = info.app_version;
        }
        let app_hash = self.replay_blocks(state, info.last_block_app_hash, app_height, exec)?;
        tracing::info!(
            app_height,
            app_hash = %hex::encode_upper(&app_hash),
            "Completed ABCI Handshake - CometBFT and App are synced"
        );
        match self.state_store.load()? {
            Some(s) => Ok(s),
            None => crate::state::make_genesis_state(self.gen_doc),
        }
    }

    /// `ReplayBlocks`.
    pub fn replay_blocks(
        &mut self,
        mut state: State,
        mut app_hash: Vec<u8>,
        app_height: i64,
        exec: &mut BlockExecutor,
    ) -> Result<Vec<u8>> {
        let store_base = self.block_store.base();
        let store_height = self.block_store.height();
        let state_height = state.last_block_height;
        tracing::info!(app_height, store_height, state_height, "ABCI Replay Blocks");

        if app_height == 0 {
            let vals: Vec<Validator> = self
                .gen_doc
                .validators
                .iter()
                .map(|v| Validator::new(v.pub_key.clone(), v.power))
                .collect();
            let vset = if vals.is_empty() {
                ValidatorSet::default()
            } else {
                ValidatorSet::new(&vals).map_err(ctypes_err)?
            };
            let req = abci::RequestInitChain {
                time: Some(self.gen_doc.genesis_time.to_proto()),
                chain_id: self.gen_doc.chain_id.clone(),
                consensus_params: Some(self.gen_doc.consensus_params.to_proto()),
                validators: validator_updates_to_abci(&vset),
                app_state_bytes: self.gen_doc.app_state.clone(),
                initial_height: self.gen_doc.initial_height,
            };
            let res = exec.app().init_chain(req)?;
            app_hash.clone_from(&res.app_hash);
            if state_height == 0 {
                if !res.app_hash.is_empty() {
                    state.app_hash.clone_from(&res.app_hash);
                }
                if !res.validators.is_empty() {
                    let vals = validator_updates_from_abci(&res.validators)?;
                    state.validators = ValidatorSet::new(&vals).map_err(ctypes_err)?;
                    state.next_validators = ValidatorSet::new(&vals)
                        .and_then(|v| v.copy_increment_proposer_priority(1))
                        .map_err(ctypes_err)?;
                } else if self.gen_doc.validators.is_empty() {
                    return Err(Error::new(
                        "validator set is nil in genesis and still empty after InitChain",
                    ));
                }
                if let Some(cp) = &res.consensus_params {
                    state.consensus_params = state
                        .consensus_params
                        .update(Some(cp))
                        .map_err(ctypes_err)?;
                    state.version.app = state.consensus_params.version.app;
                }
                state.timeouts = Timeouts::from_abci(res.timeout_info.as_ref())?;
                state.last_results_hash =
                    ctypes::merkle_proof::hash_from_byte_slices::<Vec<u8>>(&[]);
                self.state_store.save(&state)?;
            }
        }

        if store_height == 0 {
            check_state_app_hash(&app_hash, &state)?;
            return Ok(app_hash);
        }
        if app_height == 0 && state.initial_height < store_base {
            return Err(Error::new(format!(
                "app block height ({app_height}) is too far below the block store base ({store_base})"
            )));
        }
        if app_height > 0 && app_height < store_base - 1 {
            return Err(Error::new(format!(
                "app block height ({app_height}) is too far below the block store base ({store_base})"
            )));
        }
        if store_height < app_height {
            return Err(Error::new(format!(
                "app block height ({app_height}) is higher than core ({store_height})"
            )));
        }
        if store_height < state_height {
            return Err(Error::new(format!(
                "StateBlockHeight ({state_height}) > StoreBlockHeight ({store_height})"
            )));
        }
        if store_height > state_height + 1 {
            return Err(Error::new(format!(
                "StoreBlockHeight ({store_height}) > StateBlockHeight + 1 ({})",
                state_height + 1
            )));
        }

        if store_height == state_height {
            if app_height < store_height {
                return self.replay_range(state, exec, app_height, store_height, false);
            }
            check_state_app_hash(&app_hash, &state)?;
            return Ok(app_hash);
        }
        // store_height == state_height + 1
        if app_height < state_height {
            return self.replay_range(state, exec, app_height, store_height, true);
        }
        if app_height == state_height {
            tracing::info!("Replay last block using real app");
            let s = self.replay_block(&state, store_height, exec)?;
            return Ok(s.app_hash);
        }
        // app_height == store_height
        let mut resp = self
            .state_store
            .load_last_finalize_block_response(store_height)?;
        if resp.app_hash.is_empty() {
            resp.app_hash.clone_from(&app_hash);
        }
        tracing::info!("Replay last block using mock app");
        let mut mock = BlockExecutor::new(
            self.state_store.clone(),
            self.block_store.clone(),
            Box::new(MockApp { resp }),
            std::sync::Arc::new(NopMempool),
            Box::new(EmptyEvidencePool),
        );
        let s = self.replay_block(&state, store_height, &mut mock)?;
        Ok(s.app_hash)
    }

    /// `replayBlocks`.
    fn replay_range(
        &mut self,
        state: State,
        exec: &mut BlockExecutor,
        app_height: i64,
        store_height: i64,
        mutate_state: bool,
    ) -> Result<Vec<u8>> {
        let mut app_hash: Vec<u8> = Vec::new();
        let final_block = if mutate_state {
            store_height - 1
        } else {
            store_height
        };
        let mut first = app_height + 1;
        if first == 1 {
            first = state.initial_height;
        }
        for h in first..=final_block {
            tracing::info!(height = h, "Applying block");
            let mut block = self
                .block_store
                .load_block(h)?
                .ok_or_else(|| Error::new(format!("block {h} missing from the block store")))?;
            if !app_hash.is_empty() && app_hash != block.header.app_hash {
                return Err(Error::new(format!(
                    "block.AppHash does not match AppHash after replay. Got {}, expected {}",
                    hex::encode_upper(&app_hash),
                    hex::encode_upper(&block.header.app_hash)
                )));
            }
            app_hash = exec_commit_block(
                exec.app(),
                &mut block,
                &self.state_store,
                self.gen_doc.initial_height,
            )?;
            self.n_blocks += 1;
        }
        let mut state = state;
        if mutate_state {
            state = self.replay_block(&state, store_height, exec)?;
            app_hash.clone_from(&state.app_hash);
        }
        check_state_app_hash(&app_hash, &state)?;
        Ok(app_hash)
    }

    /// `replayBlock`: `ApplyBlock` of a stored block.
    fn replay_block(
        &mut self,
        state: &State,
        height: i64,
        exec: &mut BlockExecutor,
    ) -> Result<State> {
        let mut block = self
            .block_store
            .load_block(height)?
            .ok_or_else(|| Error::new(format!("block {height} missing from the block store")))?;
        let meta = self
            .block_store
            .load_block_meta(height)?
            .ok_or_else(|| Error::new(format!("block meta {height} missing")))?;
        let s = exec.apply_block(state, meta.block_id, &mut block, self.now)?;
        self.n_blocks += 1;
        Ok(s)
    }
}
