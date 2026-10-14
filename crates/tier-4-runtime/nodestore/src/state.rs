//! `state.State` (`state/state.go`) and `updateState` (`state/execution.go`).

use ctypes::header::{BLOCK_PROTOCOL, Consensus};
use ctypes::proto::{abci, state as pbs, types as pb, version as pbv};
use ctypes::time::{WeightedTime, duration_from_proto, duration_to_proto, weighted_median};
use ctypes::{
    BLOCK_ID_FLAG_ABSENT, BLOCK_PART_SIZE_BYTES, Block, BlockId, Commit, ConsensusParams, Data,
    Evidence, PartSet, Time, Validator, ValidatorSet,
};
use prost::Message;

use crate::genesis::GenesisDoc;
use crate::{Error, Result, ctypes_err};

/// `version.TMCoreSemVer` of celestia-core v0.42.3.
pub const TM_CORE_SEM_VER: &str = "0.38.17";

/// celestia-core's app-driven timeouts (`cmtstate.TimeoutInfo`), as Go
/// `time.Duration` nanoseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Timeouts {
    /// `TimeoutPropose`.
    pub propose: i64,
    /// `TimeoutProposeDelta`.
    pub propose_delta: i64,
    /// `TimeoutPrevote`.
    pub prevote: i64,
    /// `TimeoutPrevoteDelta`.
    pub prevote_delta: i64,
    /// `TimeoutPrecommit`.
    pub precommit: i64,
    /// `TimeoutPrecommitDelta`.
    pub precommit_delta: i64,
    /// `TimeoutCommit`.
    pub commit: i64,
    /// `DelayedPrecommitTimeout`.
    pub delayed_precommit: i64,
}

impl Timeouts {
    /// From the app's `abci.TimeoutInfo` (absent = zero, like Go's
    /// non-nullable struct).
    pub fn from_abci(t: Option<&abci::TimeoutInfo>) -> Result<Self> {
        let Some(t) = t else {
            return Ok(Self::default());
        };
        let d = |x: Option<&pbjson_types::Duration>| duration_from_proto(x).map_err(ctypes_err);
        Ok(Self {
            propose: d(t.timeout_propose.as_ref())?,
            propose_delta: d(t.timeout_propose_delta.as_ref())?,
            prevote: d(t.timeout_prevote.as_ref())?,
            prevote_delta: d(t.timeout_prevote_delta.as_ref())?,
            precommit: d(t.timeout_precommit.as_ref())?,
            precommit_delta: d(t.timeout_precommit_delta.as_ref())?,
            commit: d(t.timeout_commit.as_ref())?,
            delayed_precommit: d(t.delayed_precommit_timeout.as_ref())?,
        })
    }

    /// `cmtstate.TimeoutInfo`.
    #[must_use]
    pub fn to_proto(&self) -> pbs::TimeoutInfo {
        let d = |x| Some(duration_to_proto(x));
        pbs::TimeoutInfo {
            timeout_propose: d(self.propose),
            timeout_commit: d(self.commit),
            timeout_propose_delta: d(self.propose_delta),
            timeout_prevote: d(self.prevote),
            timeout_prevote_delta: d(self.prevote_delta),
            timeout_precommit: d(self.precommit),
            timeout_precommit_delta: d(self.precommit_delta),
            delayed_precommit_timeout: d(self.delayed_precommit),
        }
    }

    /// From `cmtstate.TimeoutInfo`.
    pub fn from_proto(t: Option<&pbs::TimeoutInfo>) -> Result<Self> {
        let Some(t) = t else {
            return Ok(Self::default());
        };
        let d = |x: Option<&pbjson_types::Duration>| duration_from_proto(x).map_err(ctypes_err);
        Ok(Self {
            propose: d(t.timeout_propose.as_ref())?,
            propose_delta: d(t.timeout_propose_delta.as_ref())?,
            prevote: d(t.timeout_prevote.as_ref())?,
            prevote_delta: d(t.timeout_prevote_delta.as_ref())?,
            precommit: d(t.timeout_precommit.as_ref())?,
            precommit_delta: d(t.timeout_precommit_delta.as_ref())?,
            commit: d(t.timeout_commit.as_ref())?,
            delayed_precommit: d(t.delayed_precommit_timeout.as_ref())?,
        })
    }
}

/// The blockchain state after `last_block_height` (Go `sm.State`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    /// Block / app protocol versions.
    pub version: Consensus,
    /// Software version string (`Version.Software`).
    pub software: String,
    /// Chain id.
    pub chain_id: String,
    /// Initial height.
    pub initial_height: i64,
    /// Last block height (0 at genesis).
    pub last_block_height: i64,
    /// Last block id.
    pub last_block_id: BlockId,
    /// Last block time (genesis time at genesis).
    pub last_block_time: Time,
    /// Validators of the next block (`last_block_height + 2`).
    pub next_validators: ValidatorSet,
    /// Validators of the block being built (`last_block_height + 1`).
    pub validators: ValidatorSet,
    /// Validators of the last block (signers of the last commit).
    pub last_validators: ValidatorSet,
    /// Height the validator set last changed.
    pub last_height_validators_changed: i64,
    /// Consensus params for the next block.
    pub consensus_params: ConsensusParams,
    /// Height the params last changed.
    pub last_height_consensus_params_changed: i64,
    /// Results hash of the last block.
    pub last_results_hash: Vec<u8>,
    /// App hash after the last block.
    pub app_hash: Vec<u8>,
    /// App-driven timeouts.
    pub timeouts: Timeouts,
}

fn valset_to_proto(v: &ValidatorSet) -> Result<pb::ValidatorSet> {
    v.to_proto().map_err(ctypes_err)
}

fn valset_from_proto(v: Option<&pb::ValidatorSet>) -> Result<ValidatorSet> {
    match v {
        None => Ok(ValidatorSet::default()),
        Some(v) if v.validators.is_empty() => Ok(ValidatorSet::default()),
        Some(v) => ValidatorSet::from_proto(v).map_err(ctypes_err),
    }
}

impl State {
    /// `IsEmpty` (no validators).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.validators.is_nil_or_empty()
    }

    /// `ToProto`.
    pub fn to_proto(&self) -> Result<pbs::State> {
        Ok(pbs::State {
            version: Some(pbs::Version {
                consensus: Some(pbv::Consensus {
                    block: self.version.block,
                    app: self.version.app,
                }),
                software: self.software.clone(),
            }),
            chain_id: self.chain_id.clone(),
            initial_height: self.initial_height,
            last_block_height: self.last_block_height,
            last_block_id: Some(self.last_block_id.to_proto()),
            last_block_time: Some(self.last_block_time.to_proto()),
            next_validators: Some(valset_to_proto(&self.next_validators)?),
            validators: Some(valset_to_proto(&self.validators)?),
            last_validators: if self.last_block_height >= 1 {
                Some(valset_to_proto(&self.last_validators)?)
            } else {
                None
            },
            last_height_validators_changed: self.last_height_validators_changed,
            consensus_params: Some(self.consensus_params.to_proto()),
            last_height_consensus_params_changed: self.last_height_consensus_params_changed,
            last_results_hash: self.last_results_hash.clone(),
            app_hash: self.app_hash.clone(),
            timeout_info: Some(self.timeouts.to_proto()),
        })
    }

    /// `Bytes`.
    pub fn bytes(&self) -> Result<Vec<u8>> {
        Ok(self.to_proto()?.encode_to_vec())
    }

    /// `FromProto`.
    pub fn from_proto(p: &pbs::State) -> Result<Self> {
        let v = p.version.clone().unwrap_or_default();
        let c = v.consensus.unwrap_or_default();
        let last_block_id = BlockId::from_proto(&p.last_block_id.clone().unwrap_or_default())
            .map_err(ctypes_err)?;
        Ok(Self {
            version: Consensus {
                block: c.block,
                app: c.app,
            },
            software: v.software,
            chain_id: p.chain_id.clone(),
            initial_height: p.initial_height,
            last_block_height: p.last_block_height,
            last_block_id,
            last_block_time: Time::from_proto(p.last_block_time.as_ref()).map_err(ctypes_err)?,
            next_validators: valset_from_proto(p.next_validators.as_ref())?,
            validators: valset_from_proto(p.validators.as_ref())?,
            last_validators: if p.last_block_height >= 1 {
                valset_from_proto(p.last_validators.as_ref())?
            } else {
                ValidatorSet::default()
            },
            last_height_validators_changed: p.last_height_validators_changed,
            consensus_params: ConsensusParams::from_proto(
                &p.consensus_params.clone().unwrap_or_default(),
            )
            .map_err(ctypes_err)?,
            last_height_consensus_params_changed: p.last_height_consensus_params_changed,
            last_results_hash: p.last_results_hash.clone(),
            app_hash: p.app_hash.clone(),
            timeouts: Timeouts::from_proto(p.timeout_info.as_ref())?,
        })
    }

    /// `MakeBlock`: a block with header fields from the state plus its part
    /// set (64 KiB parts).
    pub fn make_block(
        &self,
        height: i64,
        data: Data,
        last_commit: Commit,
        evidence: Vec<Evidence>,
        proposer_address: Vec<u8>,
    ) -> Result<(Block, PartSet)> {
        let block =
            self.make_block_without_partset(height, data, last_commit, evidence, proposer_address)?;
        let ps = block
            .make_part_set(BLOCK_PART_SIZE_BYTES)
            .map_err(ctypes_err)?;
        Ok((block, ps))
    }

    /// `MakeBlockWithoutPartset`.
    pub fn make_block_without_partset(
        &self,
        height: i64,
        data: Data,
        last_commit: Commit,
        evidence: Vec<Evidence>,
        proposer_address: Vec<u8>,
    ) -> Result<Block> {
        let timestamp = if height == self.initial_height {
            self.last_block_time
        } else {
            median_time(&last_commit, &self.last_validators)?
        };
        let mut block = Block::make(height, data, Some(last_commit), evidence);
        block.header.populate(
            self.version,
            self.chain_id.clone(),
            timestamp,
            self.last_block_id.clone(),
            self.validators.hash(),
            self.next_validators.hash(),
            self.consensus_params.hash(),
            self.app_hash.clone(),
            self.last_results_hash.clone(),
            proposer_address,
        );
        Ok(block)
    }

    /// `state.Propose(round)` etc. are on the consensus side (with config
    /// fallbacks); `Commit(t)`.
    #[must_use]
    pub fn commit_time(&self, t: Time) -> Time {
        add_duration(t, self.timeouts.commit)
    }
}

/// Adds Go `time.Duration` nanoseconds to a time.
#[must_use]
pub fn add_duration(t: Time, ns: i64) -> Time {
    let total = t.unix_nanos() + i128::from(ns);
    let secs = total.div_euclid(1_000_000_000);
    let nanos = total.rem_euclid(1_000_000_000);
    Time::from_unix(
        i64::try_from(secs).unwrap_or(i64::MAX),
        i32::try_from(nanos).unwrap_or(0),
    )
}

/// `t.Sub(u)` in nanoseconds (saturating).
#[must_use]
pub fn sub_times(t: Time, u: Time) -> i64 {
    let d = t.unix_nanos() - u.unix_nanos();
    i64::try_from(d).unwrap_or(if d < 0 { i64::MIN } else { i64::MAX })
}

/// `MedianTime`: voting-power weighted median of the commit timestamps.
pub fn median_time(commit: &Commit, validators: &ValidatorSet) -> Result<Time> {
    let mut weighted: Vec<Option<WeightedTime>> = vec![None; commit.signatures.len()];
    let mut total = 0i64;
    for (i, cs) in commit.signatures.iter().enumerate() {
        if cs.block_id_flag == BLOCK_ID_FLAG_ABSENT {
            continue;
        }
        let Some((_, val)) = validators.get_by_address(&cs.validator_address) else {
            return Err(Error::new(format!(
                "commit validator not found in validator set: {}",
                hex::encode_upper(&cs.validator_address)
            )));
        };
        total += val.voting_power;
        weighted[i] = Some(WeightedTime {
            time: cs.timestamp,
            weight: val.voting_power,
        });
    }
    Ok(weighted_median(&mut weighted, total))
}

/// `InitStateVersion`.
#[must_use]
pub fn init_state_version(app_version: u64) -> Consensus {
    Consensus {
        block: BLOCK_PROTOCOL,
        app: app_version,
    }
}

/// `MakeGenesisState`.
pub fn make_genesis_state(genesis: &GenesisDoc) -> Result<State> {
    let (validators, next_validators) = if genesis.validators.is_empty() {
        (ValidatorSet::default(), ValidatorSet::default())
    } else {
        let list: Vec<Validator> = genesis
            .validators
            .iter()
            .map(|v| Validator::new(v.pub_key.clone(), v.power))
            .collect();
        let vs = ValidatorSet::new(&list).map_err(ctypes_err)?;
        let next = vs.copy_increment_proposer_priority(1).map_err(ctypes_err)?;
        (vs, next)
    };
    let app_version = if genesis.consensus_params.version.app != 0 {
        genesis.consensus_params.version.app
    } else {
        1
    };
    Ok(State {
        version: init_state_version(app_version),
        software: TM_CORE_SEM_VER.to_owned(),
        chain_id: genesis.chain_id.clone(),
        initial_height: genesis.initial_height,
        last_block_height: 0,
        last_block_id: BlockId::default(),
        last_block_time: genesis.genesis_time,
        next_validators,
        validators,
        last_validators: ValidatorSet::default(),
        last_height_validators_changed: genesis.initial_height,
        consensus_params: genesis.consensus_params.clone(),
        last_height_consensus_params_changed: genesis.initial_height,
        last_results_hash: Vec::new(),
        app_hash: genesis.app_hash.clone(),
        timeouts: Timeouts::default(),
    })
}

/// `PB2TM.ValidatorUpdates`.
pub fn validator_updates_from_abci(updates: &[abci::ValidatorUpdate]) -> Result<Vec<Validator>> {
    updates
        .iter()
        .map(|u| {
            let pk = u
                .pub_key
                .as_ref()
                .ok_or_else(|| Error::new("validator update without pub key"))?;
            let pk = ctypes::PubKey::from_proto(pk).map_err(ctypes_err)?;
            Ok(Validator::new(pk, u.power))
        })
        .collect()
}

/// `TM2PB.ValidatorUpdates`.
#[must_use]
pub fn validator_updates_to_abci(vals: &ValidatorSet) -> Vec<abci::ValidatorUpdate> {
    vals.validators
        .iter()
        .map(|v| abci::ValidatorUpdate {
            pub_key: Some(v.pub_key.to_proto()),
            power: v.voting_power,
        })
        .collect()
}

/// `validateValidatorUpdates`.
pub fn validate_validator_updates(
    updates: &[abci::ValidatorUpdate],
    params: &ctypes::params::ValidatorParams,
) -> Result<()> {
    for u in updates {
        if u.power < 0 {
            return Err(Error::new(format!("voting power can't be negative {u:?}")));
        }
        if u.power == 0 {
            continue;
        }
        let pk = u
            .pub_key
            .as_ref()
            .ok_or_else(|| Error::new("fromproto: key type <nil> is not supported"))?;
        let pk = ctypes::PubKey::from_proto(pk).map_err(ctypes_err)?;
        if !ctypes::params::is_valid_pubkey_type(params, pk.key_type()) {
            return Err(Error::new(format!(
                "validator {u:?} is using pubkey {}, which is unsupported for consensus",
                pk.key_type()
            )));
        }
    }
    Ok(())
}

/// `TxResultsHash`.
#[must_use]
pub fn tx_results_hash(results: &[abci::ExecTxResult]) -> Vec<u8> {
    ctypes::results_hash(results)
}

/// `updateState`: the state after `header`'s block given the app's
/// `FinalizeBlock` response (app hash filled in by the caller after Commit).
pub fn update_state(
    state: &State,
    block_id: BlockId,
    header: &ctypes::Header,
    resp: &abci::ResponseFinalizeBlock,
    validator_updates: &[Validator],
) -> Result<State> {
    let mut n_val_set = state.next_validators.copy();
    let mut last_height_vals_changed = state.last_height_validators_changed;
    if !validator_updates.is_empty() {
        n_val_set
            .update_with_change_set(validator_updates)
            .map_err(|e| Error::new(format!("changing validator set: {e}")))?;
        last_height_vals_changed = header.height + 1 + 1;
    }
    n_val_set
        .increment_proposer_priority(1)
        .map_err(ctypes_err)?;

    let mut next_params = state.consensus_params.clone();
    let mut last_height_params_changed = state.last_height_consensus_params_changed;
    let mut version = state.version;
    if let Some(upd) = &resp.consensus_param_updates {
        next_params = state
            .consensus_params
            .update(Some(upd))
            .map_err(ctypes_err)?;
        next_params
            .validate_basic()
            .map_err(|e| Error::new(format!("validating new consensus params: {e}")))?;
        state
            .consensus_params
            .validate_update(Some(upd), header.height)
            .map_err(|e| Error::new(format!("updating consensus params: {e}")))?;
        version.app = next_params.version.app;
        last_height_params_changed = header.height + 1;
    }

    Ok(State {
        version,
        software: state.software.clone(),
        chain_id: state.chain_id.clone(),
        initial_height: state.initial_height,
        last_block_height: header.height,
        last_block_id: block_id,
        last_block_time: header.time,
        next_validators: n_val_set,
        validators: state.next_validators.copy(),
        last_validators: state.validators.copy(),
        last_height_validators_changed: last_height_vals_changed,
        consensus_params: next_params,
        last_height_consensus_params_changed: last_height_params_changed,
        last_results_hash: tx_results_hash(&resp.tx_results),
        app_hash: Vec::new(),
        timeouts: Timeouts::from_abci(resp.timeout_info.as_ref())?,
    })
}
