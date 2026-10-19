//! The evidence pool (`evidence/pool.go`) on a [`KvBackend`].
//!
//! Pending evidence is stored under `0x01 ‖ "%016X/%X"(height, hash)` as
//! `tendermint.types.Evidence`, committed evidence under `0x00 ‖ …` as an
//! `Int64Value` height, exactly like Go. Pending evidence is also kept in
//! insertion order (Go's `clist`) for the reactor's gossip.

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use ctypes::proto::types as pb;
use ctypes::{DuplicateVoteEvidence, Evidence, Time, Vote};
use kvdb::{KvBackend, WriteBatch};
use nodestore::state::add_duration;
use nodestore::{BlockStore, State, StateStore};
use prost::Message as _;

use crate::verify::{BlockSource, verify};

/// `baseKeyCommitted`.
pub const BASE_KEY_COMMITTED: u8 = 0x00;
/// `baseKeyPending`.
pub const BASE_KEY_PENDING: u8 = 0x01;

/// `AddEvidence` failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddError {
    /// `ErrInvalidEvidence`: the sender is punished.
    Invalid(String),
    /// Anything else (storage).
    Other(String),
}

impl std::fmt::Display for AddError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(s) | Self::Other(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for AddError {}

fn key_suffix(ev: &Evidence) -> Vec<u8> {
    format!("{:016X}/{}", ev.height(), hex::encode_upper(ev.hash())).into_bytes()
}

/// `keyCommitted`.
#[must_use]
pub fn key_committed(ev: &Evidence) -> Vec<u8> {
    let mut k = vec![BASE_KEY_COMMITTED];
    k.extend(key_suffix(ev));
    k
}

/// `keyPending`.
#[must_use]
pub fn key_pending(ev: &Evidence) -> Vec<u8> {
    let mut k = vec![BASE_KEY_PENDING];
    k.extend(key_suffix(ev));
    k
}

fn encode_int64_value(v: i64) -> Vec<u8> {
    let mut out = Vec::new();
    if v != 0 {
        out.push(0x08);
        #[allow(clippy::cast_sign_loss)]
        prost::encoding::encode_varint(v as u64, &mut out);
    }
    out
}

fn invalid(ev: &Evidence, reason: &str) -> String {
    format!(
        "Invalid evidence: {reason}. Evidence: {}",
        evidence_string(ev)
    )
}

/// A short description of evidence for logs and errors.
#[must_use]
pub fn evidence_string(ev: &Evidence) -> String {
    match ev {
        Evidence::DuplicateVote(d) => d.to_string(),
        Evidence::LightClientAttack(l) => format!(
            "LightClientAttackEvidence{{ConflictingBlock: {}, CommonHeight: {}, ByzatineValidators: {}, TotalVotingPower: {}, Timestamp: {}}}#{}",
            l.conflicting_block
                .as_ref()
                .and_then(ctypes::LightBlock::header)
                .and_then(ctypes::Header::hash)
                .map_or_else(String::new, hex::encode_upper),
            l.common_height,
            l.byzantine_validators.len(),
            l.total_voting_power,
            l.timestamp.go_string(),
            hex::encode_upper(l.hash())
        ),
    }
}

#[derive(Debug)]
struct Inner {
    db: Arc<dyn KvBackend>,
    state_store: StateStore,
    block_store: BlockStore,
    state: State,
    consensus_buffer: Vec<(Vote, Vote)>,
    pruning_height: i64,
    pruning_time: Time,
    /// The `clist`: insertion sequence -> evidence.
    list: BTreeMap<u64, Evidence>,
    next_seq: u64,
    size: u32,
}

#[derive(Debug)]
struct Shared {
    inner: Mutex<Inner>,
    /// Signalled whenever evidence is added (Go's `EvidenceWaitChan`).
    added: Condvar,
}

/// The evidence pool (cheap to clone; clones share it).
#[derive(Debug, Clone)]
pub struct Pool(Arc<Shared>);

impl Inner {
    fn has(&self, key: &[u8]) -> bool {
        match self.db.get(key) {
            Ok(v) => v.is_some(),
            Err(e) => {
                tracing::error!(err = %e, "Unable to find evidence");
                false
            }
        }
    }

    fn is_pending(&self, ev: &Evidence) -> bool {
        self.has(&key_pending(ev))
    }

    fn is_committed(&self, ev: &Evidence) -> bool {
        self.has(&key_committed(ev))
    }

    /// `isExpired`.
    fn is_expired(&self, height: i64, time: Time) -> bool {
        let params = self.state.consensus_params.evidence;
        let age_duration = nodestore::state::sub_times(self.state.last_block_time, time);
        let age_blocks = self.state.last_block_height - height;
        age_blocks > params.max_age_num_blocks && age_duration > params.max_age_duration
    }

    fn add_pending_evidence(&mut self, ev: &Evidence) -> Result<(), String> {
        let p = ev
            .to_proto()
            .map_err(|e| format!("unable to convert to proto, err: {e}"))?;
        let mut b = WriteBatch::new();
        b.put(key_pending(ev), p.encode_to_vec());
        self.db
            .write(b)
            .map_err(|e| format!("can't persist evidence: {e}"))?;
        self.size += 1;
        Ok(())
    }

    fn remove_pending_evidence(&mut self, ev: &Evidence) {
        let mut b = WriteBatch::new();
        b.delete(key_pending(ev));
        match self.db.write(b) {
            Ok(()) => {
                self.size = self.size.saturating_sub(1);
                tracing::debug!(evidence = %evidence_string(ev), "Deleted pending evidence");
            }
            Err(e) => tracing::error!(err = %e, "Unable to delete pending evidence"),
        }
    }

    fn push_back(&mut self, ev: Evidence) {
        self.list.insert(self.next_seq, ev);
        self.next_seq += 1;
    }

    fn remove_from_list(&mut self, hashes: &[Vec<u8>]) {
        self.list.retain(|_, ev| !hashes.contains(&ev.hash()));
    }

    /// `listEvidence(prefix, maxBytes)`: oldest (lowest height) first, as
    /// long as the `EvidenceList` encoding stays within `max_bytes`.
    fn list_evidence(&self, prefix: u8, max_bytes: i64) -> Result<(Vec<Evidence>, i64), String> {
        let mut total_size = 0i64;
        let mut ev_list_size = 0usize;
        let mut out = Vec::new();
        let iter = self
            .db
            .prefix_range(&[prefix], false)
            .map_err(|e| format!("database error: {e}"))?;
        for kv in iter {
            let (_, v) = kv.map_err(|e| format!("database error: {e}"))?;
            let evpb =
                ctypes::gogo_decode::<pb::Evidence>(v.as_slice()).map_err(|e| e.to_string())?;
            // `evList.Size()` after appending: one length-delimited field.
            let n = evpb.encoded_len();
            ev_list_size += 1 + prost::encoding::encoded_len_varint(n as u64) + n;
            let ev_size = i64::try_from(ev_list_size).unwrap_or(i64::MAX);
            if max_bytes != -1 && ev_size > max_bytes {
                return Ok((out, total_size));
            }
            let ev = Evidence::from_proto(&evpb).map_err(|e| e.to_string())?;
            total_size = ev_size;
            out.push(ev);
        }
        Ok((out, total_size))
    }

    /// `removeExpiredPendingEvidence`: returns when to prune next.
    fn remove_expired_pending_evidence(&mut self) -> (i64, Time) {
        let entries: Vec<Vec<u8>> = match self.db.prefix_range(&[BASE_KEY_PENDING], false) {
            Ok(it) => it.filter_map(Result::ok).map(|(_, v)| v).collect(),
            Err(e) => {
                tracing::error!(err = %e, "Unable to iterate over pending evidence");
                return (self.state.last_block_height, self.state.last_block_time);
            }
        };
        let mut removed = Vec::new();
        for v in entries {
            let ev = match ctypes::gogo_decode::<pb::Evidence>(v.as_slice())
                .map_err(|e| e.to_string())
                .and_then(|p| Evidence::from_proto(&p).map_err(|e| e.to_string()))
            {
                Ok(ev) => ev,
                Err(e) => {
                    tracing::error!(err = %e, "Error in transition evidence from protobuf");
                    continue;
                }
            };
            if !self.is_expired(ev.height(), ev.time()) {
                if !removed.is_empty() {
                    self.remove_from_list(&removed);
                }
                let params = self.state.consensus_params.evidence;
                return (
                    ev.height() + params.max_age_num_blocks + 1,
                    add_duration(
                        add_duration(ev.time(), params.max_age_duration),
                        1_000_000_000,
                    ),
                );
            }
            self.remove_pending_evidence(&ev);
            removed.push(ev.hash());
        }
        if !removed.is_empty() {
            self.remove_from_list(&removed);
        }
        (self.state.last_block_height, self.state.last_block_time)
    }

    /// `markEvidenceAsCommitted`.
    fn mark_evidence_as_committed(&mut self, evidence: &[Evidence]) {
        let mut removed = Vec::new();
        for ev in evidence {
            if self.is_pending(ev) {
                self.remove_pending_evidence(ev);
                removed.push(ev.hash());
            }
            let mut b = WriteBatch::new();
            b.put(key_committed(ev), encode_int64_value(ev.height()));
            if let Err(e) = self.db.write(b) {
                tracing::error!(err = %e, "Unable to save committed evidence");
            }
        }
        if !removed.is_empty() {
            self.remove_from_list(&removed);
        }
    }

    /// `processConsensusBuffer`.
    fn process_consensus_buffer(&mut self, state: &State) -> bool {
        let mut added = false;
        for (a, b) in std::mem::take(&mut self.consensus_buffer) {
            let h = a.height;
            let made = match h.cmp(&state.last_block_height) {
                std::cmp::Ordering::Equal => {
                    DuplicateVoteEvidence::new(a, b, state.last_block_time, &state.last_validators)
                }
                std::cmp::Ordering::Less => {
                    let vals = match self.state_store.load_validators(h) {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::error!(height = h, err = %e, "failed to load validator set for conflicting votes");
                            continue;
                        }
                    };
                    let Ok(Some(meta)) = self.block_store.load_block_meta(h) else {
                        tracing::error!(
                            height = h,
                            "failed to load block time for conflicting votes"
                        );
                        continue;
                    };
                    DuplicateVoteEvidence::new(a, b, meta.header.time, &vals)
                }
                std::cmp::Ordering::Greater => {
                    tracing::error!(
                        duplicate_vote_height = h,
                        last_block_height = state.last_block_height,
                        "inbound duplicate votes from consensus are of a greater height than current state"
                    );
                    continue;
                }
            };
            let dve = match made {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!(err = %e, "error in generating evidence from votes");
                    continue;
                }
            };
            let ev = Evidence::DuplicateVote(dve);
            if self.is_pending(&ev) {
                tracing::info!(evidence = %evidence_string(&ev), "evidence already pending; ignoring");
                continue;
            }
            if self.is_committed(&ev) {
                tracing::info!(evidence = %evidence_string(&ev), "evidence already committed; ignoring");
                continue;
            }
            if let Err(e) = self.add_pending_evidence(&ev) {
                tracing::error!(err = %e, "failed to flush evidence from consensus buffer to pending list");
                continue;
            }
            tracing::info!(evidence = %evidence_string(&ev), "verified new evidence of byzantine behavior");
            self.push_back(ev);
            added = true;
        }
        added
    }

    fn verify(&self, ev: &Evidence) -> Result<(), String> {
        verify(&self.state, ev, &self.block_store, &self.state_store)
    }
}

impl Pool {
    /// `NewPool`: loads the state, drops expired pending evidence and
    /// reloads the rest (in key order) into the gossip list.
    pub fn new(
        evidence_db: Arc<dyn KvBackend>,
        state_store: StateStore,
        block_store: BlockStore,
    ) -> Result<Self, String> {
        let state = state_store
            .load()
            .map_err(|e| format!("cannot load state: {e}"))?
            .ok_or_else(|| "cannot load state: no state".to_string())?;
        Self::with_state(evidence_db, state_store, block_store, state)
    }

    /// As [`Self::new`] with an explicit state.
    pub fn with_state(
        evidence_db: Arc<dyn KvBackend>,
        state_store: StateStore,
        block_store: BlockStore,
        state: State,
    ) -> Result<Self, String> {
        let mut inner = Inner {
            db: evidence_db,
            state_store,
            block_store,
            state,
            consensus_buffer: Vec::new(),
            pruning_height: 0,
            pruning_time: Time::ZERO,
            list: BTreeMap::new(),
            next_seq: 0,
            size: 0,
        };
        let (h, t) = inner.remove_expired_pending_evidence();
        inner.pruning_height = h;
        inner.pruning_time = t;
        let (evs, _) = inner.list_evidence(BASE_KEY_PENDING, -1)?;
        inner.size = u32::try_from(evs.len()).unwrap_or(u32::MAX);
        for ev in evs {
            inner.push_back(ev);
        }
        Ok(Self(Arc::new(Shared {
            inner: Mutex::new(inner),
            added: Condvar::new(),
        })))
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `Size`: pending evidence count.
    #[must_use]
    pub fn size(&self) -> u32 {
        self.lock().size
    }

    /// `State`.
    #[must_use]
    pub fn state(&self) -> State {
        self.lock().state.clone()
    }

    /// `PendingEvidence(maxBytes)`.
    #[must_use]
    pub fn pending_evidence(&self, max_bytes: i64) -> (Vec<Evidence>, i64) {
        let g = self.lock();
        if g.size == 0 {
            return (Vec::new(), 0);
        }
        match g.list_evidence(BASE_KEY_PENDING, max_bytes) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(err = %e, "Unable to retrieve pending evidence");
                (Vec::new(), 0)
            }
        }
    }

    /// `Update`: after a block is committed.
    pub fn update(&self, state: &State, evidence: &[Evidence]) {
        let mut g = self.lock();
        if state.last_block_height <= g.state.last_block_height {
            // Go panics here.
            tracing::error!(
                new = state.last_block_height,
                prev = g.state.last_block_height,
                "failed EvidencePool.Update new state height is less than or equal to previous state height"
            );
            return;
        }
        tracing::debug!(
            last_block_height = state.last_block_height,
            "Updating evidence pool"
        );
        let added = g.process_consensus_buffer(state);
        g.state = state.clone();
        g.mark_evidence_as_committed(evidence);
        if g.size > 0
            && state.last_block_height > g.pruning_height
            && state.last_block_time > g.pruning_time
        {
            let (h, t) = g.remove_expired_pending_evidence();
            g.pruning_height = h;
            g.pruning_time = t;
        }
        drop(g);
        if added {
            self.0.added.notify_all();
        }
    }

    /// `AddEvidence` (from peers / RPC).
    pub fn add_evidence(&self, ev: Evidence) -> Result<(), AddError> {
        let mut g = self.lock();
        tracing::info!(ev = %evidence_string(&ev), "Attempting to add evidence");
        if g.is_pending(&ev) {
            tracing::info!("Evidence already pending, ignoring this one");
            return Ok(());
        }
        if g.is_committed(&ev) {
            tracing::info!("Evidence was already committed, ignoring this one");
            return Ok(());
        }
        g.verify(&ev)
            .map_err(|e| AddError::Invalid(invalid(&ev, &e)))?;
        g.add_pending_evidence(&ev)
            .map_err(|e| AddError::Other(format!("can't add evidence to pending list: {e}")))?;
        tracing::info!(evidence = %evidence_string(&ev), "Verified new evidence of byzantine behavior");
        g.push_back(ev);
        drop(g);
        self.0.added.notify_all();
        Ok(())
    }

    /// `ReportConflictingVotes` (votes are not verified; evidence is formed
    /// at the next `Update`).
    pub fn report_conflicting_votes(&self, vote_a: Vote, vote_b: Vote) {
        self.lock().consensus_buffer.push((vote_a, vote_b));
    }

    /// `CheckEvidence`: the evidence of a block — not committed, valid
    /// (light client attacks always re-verified), no duplicates.
    pub fn check_evidence(&self, evidence: &[Evidence]) -> Result<(), String> {
        let mut g = self.lock();
        let mut hashes: Vec<Vec<u8>> = Vec::with_capacity(evidence.len());
        for ev in evidence {
            let is_light = matches!(ev, Evidence::LightClientAttack(_));
            if is_light || !g.is_pending(ev) {
                if g.is_committed(ev) {
                    return Err(invalid(ev, "evidence was already committed"));
                }
                g.verify(ev)?;
                if let Err(e) = g.add_pending_evidence(ev) {
                    tracing::error!(err = %e, "Can't add evidence to pending list");
                }
                tracing::info!(evidence = %evidence_string(ev), "Check evidence: verified evidence of byzantine behavior");
                // Go does not push it to the gossip list here.
            }
            let h = ev.hash();
            if hashes.contains(&h) {
                return Err(invalid(ev, "duplicate evidence"));
            }
            hashes.push(h);
        }
        Ok(())
    }

    /// Whether the evidence is pending.
    #[must_use]
    pub fn is_pending(&self, ev: &Evidence) -> bool {
        self.lock().is_pending(ev)
    }

    /// Whether the evidence is committed.
    #[must_use]
    pub fn is_committed(&self, ev: &Evidence) -> bool {
        self.lock().is_committed(ev)
    }

    /// The gossip list from sequence number `after` (exclusive; `None`
    /// for the front): the next element and its sequence number.
    #[must_use]
    pub fn next_after(&self, after: Option<u64>) -> Option<(u64, Evidence)> {
        let g = self.lock();
        match after {
            None => g.list.iter().next(),
            Some(s) => g.list.range(s + 1..).next(),
        }
        .map(|(k, v)| (*k, v.clone()))
    }

    /// Waits up to `timeout` for evidence to be added.
    pub fn wait_added(&self, timeout: std::time::Duration) {
        let g = self.lock();
        let _ = self.0.added.wait_timeout(g, timeout);
    }

    /// A handle for [`nodestore::BlockExecutor`].
    #[must_use]
    pub fn executor_handle(&self) -> Box<dyn nodestore::EvidencePool> {
        Box::new(self.clone())
    }

    /// The block store the pool verifies against.
    #[must_use]
    pub fn block_source(&self) -> BlockStore {
        self.lock().block_store.clone()
    }

    /// Height of the block store (for tests / status).
    #[must_use]
    pub fn store_height(&self) -> i64 {
        BlockSource::height(&self.lock().block_store)
    }
}

impl nodestore::EvidencePool for Pool {
    fn pending_evidence(&mut self, max_bytes: i64) -> (Vec<Evidence>, i64) {
        Pool::pending_evidence(self, max_bytes)
    }

    fn check_evidence(&mut self, _state: &State, evidence: &[Evidence]) -> nodestore::Result<()> {
        Pool::check_evidence(self, evidence).map_err(nodestore::Error::new)
    }

    fn update(&mut self, state: &State, committed: &[Evidence]) {
        Pool::update(self, state, committed);
    }

    fn report_conflicting_votes(&mut self, vote_a: Vote, vote_b: Vote) {
        Pool::report_conflicting_votes(self, vote_a, vote_b);
    }
}

#[cfg(test)]
mod tests;
