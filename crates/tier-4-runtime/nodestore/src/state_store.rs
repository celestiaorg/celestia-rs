//! `state.Store` (`state/store.go`) on a [`KvBackend`].
//!
//! Validators are stored per height with Go's checkpointing: the full set
//! only when it changed at that height or every
//! [`VAL_SET_CHECKPOINT_INTERVAL`] heights, otherwise a pointer to the last
//! change; consensus params likewise. FinalizeBlock responses are kept per
//! height plus the last one for crash recovery.

use std::sync::Arc;

use ctypes::proto::{abci, state as pbs, types as pb};
use ctypes::{ConsensusParams, ValidatorSet};
use kvdb::{KvBackend, WriteBatch};
use prost::Message;

use crate::genesis::GenesisDoc;
use crate::state::{State, make_genesis_state};
use crate::{Error, Result, ctypes_err, kv_err};

/// `valSetCheckpointInterval`.
pub const VAL_SET_CHECKPOINT_INTERVAL: i64 = 100_000;

const STATE_KEY: &[u8] = b"stateKey";
const LAST_ABCI_RESPONSE_KEY: &[u8] = b"lastABCIResponseKey";

fn validators_key(h: i64) -> Vec<u8> {
    format!("validatorsKey:{h}").into_bytes()
}
fn params_key(h: i64) -> Vec<u8> {
    format!("consensusParamsKey:{h}").into_bytes()
}
fn abci_responses_key(h: i64) -> Vec<u8> {
    format!("abciResponsesKey:{h}").into_bytes()
}

/// `lastStoredHeightFor`.
fn last_stored_height_for(height: i64, last_height_changed: i64) -> i64 {
    let checkpoint = height - height % VAL_SET_CHECKPOINT_INTERVAL;
    checkpoint.max(last_height_changed)
}

fn decode<M: Message + Default>(bz: &[u8], what: &str) -> Result<M> {
    ctypes::gogo_decode::<M>(bz)
        .map_err(|e| Error::new(format!("{what}: data has been corrupted: {e}")))
}

/// State store handle (cheap to clone).
#[derive(Debug, Clone)]
pub struct StateStore {
    db: Arc<dyn KvBackend>,
    /// `DiscardABCIResponses`: keep only the last FinalizeBlock response.
    pub discard_abci_responses: bool,
}

impl StateStore {
    /// `NewStore`.
    #[must_use]
    pub fn new(db: Arc<dyn KvBackend>) -> Self {
        Self {
            db,
            discard_abci_responses: false,
        }
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.db.get(key).map_err(kv_err)
    }

    /// `Load`: the latest state, `None` when nothing was saved.
    pub fn load(&self) -> Result<Option<State>> {
        let Some(bz) = self.get(STATE_KEY)? else {
            return Ok(None);
        };
        if bz.is_empty() {
            return Ok(None);
        }
        let p: pbs::State = decode(&bz, "state")?;
        State::from_proto(&p).map(Some)
    }

    /// `LoadFromDBOrGenesisDoc`.
    pub fn load_from_db_or_genesis(&self, genesis: &GenesisDoc) -> Result<State> {
        if let Some(s) = self.load()? {
            return Ok(s);
        }
        make_genesis_state(genesis)
    }

    /// `Save`: state plus the validators / params it makes known.
    pub fn save(&self, state: &State) -> Result<()> {
        let mut batch = WriteBatch::new();
        let mut next_height = state.last_block_height + 1;
        if next_height == 1 {
            next_height = state.initial_height;
            Self::save_validators_info(next_height, next_height, &state.validators, &mut batch)?;
        }
        Self::save_validators_info(
            next_height + 1,
            state.last_height_validators_changed,
            &state.next_validators,
            &mut batch,
        )?;
        Self::save_params_info(
            next_height,
            state.last_height_consensus_params_changed,
            &state.consensus_params,
            &mut batch,
        );
        batch.put(STATE_KEY.to_vec(), state.bytes()?);
        self.db.write(batch).map_err(kv_err)
    }

    /// `Bootstrap` (state sync).
    pub fn bootstrap(&self, state: &State) -> Result<()> {
        let mut batch = WriteBatch::new();
        let mut height = state.last_block_height + 1;
        if height == 1 {
            height = state.initial_height;
        }
        if height > 1 && !state.last_validators.is_nil_or_empty() {
            Self::save_validators_info(height - 1, height - 1, &state.last_validators, &mut batch)?;
        }
        Self::save_validators_info(height, height, &state.validators, &mut batch)?;
        Self::save_validators_info(height + 1, height + 1, &state.next_validators, &mut batch)?;
        Self::save_params_info(
            height,
            state.last_height_consensus_params_changed,
            &state.consensus_params,
            &mut batch,
        );
        batch.put(STATE_KEY.to_vec(), state.bytes()?);
        self.db.write(batch).map_err(kv_err)
    }

    fn save_validators_info(
        height: i64,
        last_height_changed: i64,
        val_set: &ValidatorSet,
        batch: &mut WriteBatch,
    ) -> Result<()> {
        if last_height_changed > height {
            return Err(Error::new(
                "lastHeightChanged cannot be greater than ValidatorsInfo height",
            ));
        }
        let mut info = pbs::ValidatorsInfo {
            validator_set: None,
            last_height_changed,
        };
        if height == last_height_changed || height % VAL_SET_CHECKPOINT_INTERVAL == 0 {
            info.validator_set = Some(val_set.to_proto().map_err(ctypes_err)?);
        }
        batch.put(validators_key(height), info.encode_to_vec());
        Ok(())
    }

    fn load_validators_info(&self, height: i64) -> Result<Option<pbs::ValidatorsInfo>> {
        match self.get(&validators_key(height))? {
            Some(bz) if !bz.is_empty() => decode(&bz, "validators info").map(Some),
            _ => Ok(None),
        }
    }

    /// `LoadValidators`.
    pub fn load_validators(&self, height: i64) -> Result<ValidatorSet> {
        let Some(info) = self.load_validators_info(height)? else {
            return Err(Error::new(format!(
                "could not find validator set for height #{height}"
            )));
        };
        let Some(vs) = info.validator_set else {
            let last_stored = last_stored_height_for(height, info.last_height_changed);
            let info2 = self
                .load_validators_info(last_stored)?
                .and_then(|i| i.validator_set)
                .ok_or_else(|| {
                    Error::new(format!(
                        "couldn't find validators at height {last_stored} (height {height} was originally requested)"
                    ))
                })?;
            let mut vs = ValidatorSet::from_proto(&info2).map_err(ctypes_err)?;
            let times = i32::try_from(height - last_stored).unwrap_or(i32::MAX);
            vs.increment_proposer_priority(times).map_err(ctypes_err)?;
            return Ok(vs);
        };
        ValidatorSet::from_proto(&vs).map_err(ctypes_err)
    }

    fn save_params_info(
        next_height: i64,
        change_height: i64,
        params: &ConsensusParams,
        batch: &mut WriteBatch,
    ) {
        let info = pbs::ConsensusParamsInfo {
            consensus_params: (change_height == next_height).then(|| params.to_proto()),
            last_height_changed: change_height,
        };
        batch.put(params_key(next_height), info.encode_to_vec());
    }

    fn load_params_info(&self, height: i64) -> Result<Option<pbs::ConsensusParamsInfo>> {
        match self.get(&params_key(height))? {
            Some(bz) if !bz.is_empty() => decode(&bz, "consensus params info").map(Some),
            _ => Ok(None),
        }
    }

    /// `LoadConsensusParams`.
    pub fn load_consensus_params(&self, height: i64) -> Result<ConsensusParams> {
        let info = self.load_params_info(height)?.ok_or_else(|| {
            Error::new(format!(
                "could not find consensus params for height #{height}"
            ))
        })?;
        let empty = pb::ConsensusParams::default();
        let cp = match info.consensus_params {
            Some(cp) if cp != empty => cp,
            _ => self
                .load_params_info(info.last_height_changed)?
                .and_then(|i| i.consensus_params)
                .ok_or_else(|| {
                    Error::new(format!(
                        "couldn't find consensus params at height {} as last changed from height {height}",
                        info.last_height_changed
                    ))
                })?,
        };
        ConsensusParams::from_proto(&cp).map_err(ctypes_err)
    }

    /// `SaveFinalizeBlockResponse`: per height (unless discarded) and as the
    /// last response, durably.
    pub fn save_finalize_block_response(
        &self,
        height: i64,
        resp: &abci::ResponseFinalizeBlock,
    ) -> Result<()> {
        let mut batch = WriteBatch::new();
        if !self.discard_abci_responses {
            batch.put(abci_responses_key(height), resp.encode_to_vec());
        }
        let info = pbs::AbciResponsesInfo {
            legacy_abci_responses: None,
            height,
            response_finalize_block: Some(resp.clone()),
        };
        batch.put(LAST_ABCI_RESPONSE_KEY.to_vec(), info.encode_to_vec());
        self.db.write(batch).map_err(kv_err)
    }

    /// `LoadFinalizeBlockResponse`.
    pub fn load_finalize_block_response(
        &self,
        height: i64,
    ) -> Result<Option<abci::ResponseFinalizeBlock>> {
        match self.get(&abci_responses_key(height))? {
            Some(bz) => decode(&bz, "finalize block response").map(Some),
            None => {
                // The last response is always kept.
                match self.load_last_finalize_block_response(height) {
                    Ok(r) => Ok(Some(r)),
                    Err(_) => Ok(None),
                }
            }
        }
    }

    /// `LoadLastFinalizeBlockResponse`.
    pub fn load_last_finalize_block_response(
        &self,
        height: i64,
    ) -> Result<abci::ResponseFinalizeBlock> {
        let bz = self
            .get(LAST_ABCI_RESPONSE_KEY)?
            .filter(|b| !b.is_empty())
            .ok_or_else(|| Error::new("no last ABCI response has been persisted"))?;
        let info: pbs::AbciResponsesInfo = decode(&bz, "last abci response")?;
        if info.height != height {
            return Err(Error::new(format!(
                "expected height {height} but last stored abci responses was at height {}",
                info.height
            )));
        }
        info.response_finalize_block
            .ok_or_else(|| Error::new("state store contains last abci response but it is empty"))
    }

    /// `PruneStates`: deletes validator / param / response records in
    /// `[from, to)`, keeping what `to` (and the evidence window) needs.
    pub fn prune_states(&self, from: i64, to: i64, evidence_threshold_height: i64) -> Result<u64> {
        if from <= 0 || to <= 0 {
            return Err(Error::new(format!(
                "from height {from} and to height {to} must be greater than 0"
            )));
        }
        if from >= to {
            return Err(Error::new(format!(
                "from height {from} must be lower than to height {to}"
            )));
        }
        let val_info = self
            .load_validators_info(to.min(evidence_threshold_height))?
            .ok_or_else(|| Error::new(format!("validators at height {to} not found")))?;
        let params_info = self
            .load_params_info(to)?
            .ok_or_else(|| Error::new(format!("consensus params at height {to} not found")))?;
        let mut keep_vals = Vec::new();
        if val_info.validator_set.is_none() {
            keep_vals.push(val_info.last_height_changed);
            keep_vals.push(last_stored_height_for(to, val_info.last_height_changed));
        }
        let mut keep_params = Vec::new();
        if params_info
            .consensus_params
            .as_ref()
            .is_none_or(|p| *p == pb::ConsensusParams::default())
        {
            keep_params.push(params_info.last_height_changed);
        }
        let mut batch = WriteBatch::new();
        let mut pruned = 0u64;
        for h in (from..to).rev() {
            if keep_vals.contains(&h) {
                let v = self.load_validators_info(h)?;
                if v.as_ref().is_none_or(|v| v.validator_set.is_none()) {
                    let vs = self.load_validators(h)?;
                    let info = pbs::ValidatorsInfo {
                        validator_set: Some(vs.to_proto().map_err(ctypes_err)?),
                        last_height_changed: h,
                    };
                    batch.put(validators_key(h), info.encode_to_vec());
                }
            } else if h < evidence_threshold_height {
                batch.delete(validators_key(h));
            }
            if keep_params.contains(&h) {
                let p = self.load_params_info(h)?;
                if p.as_ref()
                    .and_then(|p| p.consensus_params.as_ref())
                    .is_none_or(|cp| *cp == pb::ConsensusParams::default())
                {
                    let params = self.load_consensus_params(h)?;
                    let info = pbs::ConsensusParamsInfo {
                        consensus_params: Some(params.to_proto()),
                        last_height_changed: h,
                    };
                    batch.put(params_key(h), info.encode_to_vec());
                }
            } else {
                batch.delete(params_key(h));
            }
            batch.delete(abci_responses_key(h));
            pruned += 1;
        }
        self.db.write(batch).map_err(kv_err)?;
        Ok(pruned)
    }
}
