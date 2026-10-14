//! An in-memory evidence pool (`evidence/pool.go`, `evidence/verify.go`)
//! covering duplicate-vote evidence: conflicting votes reported by
//! consensus become [`DuplicateVoteEvidence`] once their height is
//! committed, are proposed in later blocks until committed or expired, and
//! evidence in received blocks is verified against the stored validator
//! set and block time of its height.
//!
//! Not yet ported: persistence of pending evidence, light-client-attack
//! verification (rejected), the evidence reactor.

use ctypes::{DuplicateVoteEvidence, Evidence, Vote};

use crate::block_store::{BlockStore, is_evidence_expired};
use crate::state::State;
use crate::state_store::StateStore;
use crate::{Error, Result};

/// What block execution and consensus need from an evidence pool.
pub trait EvidencePool: Send {
    /// `PendingEvidence(maxBytes)`: evidence to propose and its size.
    fn pending_evidence(&mut self, max_bytes: i64) -> (Vec<Evidence>, i64);
    /// `CheckEvidence`: validates the evidence of a proposed block.
    fn check_evidence(&mut self, state: &State, evidence: &[Evidence]) -> Result<()>;
    /// `Update`: after a block is committed.
    fn update(&mut self, state: &State, committed: &[Evidence]);
    /// `ReportConflictingVotes` (from consensus).
    fn report_conflicting_votes(&mut self, vote_a: Vote, vote_b: Vote);
}

/// `sm.EmptyEvidencePool`.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyEvidencePool;

impl EvidencePool for EmptyEvidencePool {
    fn pending_evidence(&mut self, _: i64) -> (Vec<Evidence>, i64) {
        (Vec::new(), 0)
    }
    fn check_evidence(&mut self, _: &State, _: &[Evidence]) -> Result<()> {
        Ok(())
    }
    fn update(&mut self, _: &State, _: &[Evidence]) {}
    fn report_conflicting_votes(&mut self, _: Vote, _: Vote) {}
}

/// In-memory duplicate-vote evidence pool.
#[derive(Debug)]
pub struct MemEvidencePool {
    state_store: StateStore,
    block_store: BlockStore,
    pending: Vec<Evidence>,
    committed: Vec<Vec<u8>>,
    buffer: Vec<(Vote, Vote)>,
    state: Option<State>,
}

impl MemEvidencePool {
    /// A pool over the node's stores.
    #[must_use]
    pub fn new(state_store: StateStore, block_store: BlockStore) -> Self {
        Self {
            state_store,
            block_store,
            pending: Vec::new(),
            committed: Vec::new(),
            buffer: Vec::new(),
            state: None,
        }
    }

    /// Evidence waiting to be committed.
    #[must_use]
    pub fn pending(&self) -> &[Evidence] {
        &self.pending
    }

    /// `AddEvidence` (from peers): verified, then pending.
    pub fn add_evidence(&mut self, state: &State, ev: Evidence) -> Result<()> {
        if self.is_pending(&ev) || self.is_committed(&ev) {
            return Ok(());
        }
        self.verify(state, &ev)?;
        self.pending.push(ev);
        Ok(())
    }

    fn is_pending(&self, ev: &Evidence) -> bool {
        let h = ev.hash();
        self.pending.iter().any(|p| p.hash() == h)
    }

    fn is_committed(&self, ev: &Evidence) -> bool {
        self.committed.contains(&ev.hash())
    }

    /// `verify`: expiry, validator set and block time of the height,
    /// `VerifyDuplicateVote`.
    fn verify(&self, state: &State, ev: &Evidence) -> Result<()> {
        let height = ev.height();
        let meta = self
            .block_store
            .load_block_meta(height)?
            .ok_or_else(|| Error::new(format!("don't have header #{height}")))?;
        let ev_time = meta.header.time;
        if ev.time() != ev_time {
            return Err(Error::new(format!(
                "evidence has a different time to the block it is associated with ({} != {})",
                ev.time().go_string(),
                ev_time.go_string()
            )));
        }
        if is_evidence_expired(
            state.last_block_height,
            state.last_block_time,
            height,
            ev_time,
            &state.consensus_params.evidence,
        ) {
            return Err(Error::new(format!(
                "evidence from height {height} (created at: {}) is too old",
                ev_time.go_string()
            )));
        }
        match ev {
            Evidence::DuplicateVote(dve) => {
                let vals = self.state_store.load_validators(height)?;
                verify_duplicate_vote(dve, &state.chain_id, &vals)
            }
            Evidence::LightClientAttack(_) => Err(Error::new(
                "light client attack evidence verification is not supported yet",
            )),
        }
    }

    fn process_consensus_buffer(&mut self, state: &State) {
        let buffer = std::mem::take(&mut self.buffer);
        for (a, b) in buffer {
            let h = a.height;
            let made = match h.cmp(&state.last_block_height) {
                std::cmp::Ordering::Equal => {
                    DuplicateVoteEvidence::new(a, b, state.last_block_time, &state.last_validators)
                        .map_err(|e| Error::new(e.to_string()))
                }
                std::cmp::Ordering::Less => {
                    let vals = self.state_store.load_validators(h);
                    let meta = self.block_store.load_block_meta(h);
                    match (vals, meta) {
                        (Ok(vals), Ok(Some(meta))) => {
                            DuplicateVoteEvidence::new(a, b, meta.header.time, &vals)
                                .map_err(|e| Error::new(e.to_string()))
                        }
                        _ => continue,
                    }
                }
                std::cmp::Ordering::Greater => {
                    // Not committed yet: keep for a later update.
                    self.buffer.push((a, b));
                    continue;
                }
            };
            let Ok(dve) = made else { continue };
            let ev = Evidence::DuplicateVote(dve);
            if self.is_pending(&ev) || self.is_committed(&ev) {
                continue;
            }
            self.pending.push(ev);
        }
    }
}

impl EvidencePool for MemEvidencePool {
    fn pending_evidence(&mut self, max_bytes: i64) -> (Vec<Evidence>, i64) {
        let mut out = Vec::new();
        let mut size = 0i64;
        for ev in &self.pending {
            let Ok(p) = ev.to_proto() else { continue };
            let sz = i64::try_from(prost::Message::encoded_len(&p)).unwrap_or(i64::MAX);
            if max_bytes != -1 && size + sz > max_bytes {
                break;
            }
            size += sz;
            out.push(ev.clone());
        }
        (out, size)
    }

    fn check_evidence(&mut self, state: &State, evidence: &[Evidence]) -> Result<()> {
        let mut seen = Vec::new();
        for ev in evidence {
            let h = ev.hash();
            if self.is_committed(ev) {
                return Err(Error::new(format!(
                    "evidence was already committed: {}",
                    hex::encode_upper(&h)
                )));
            }
            if !self.is_pending(ev) {
                self.verify(state, ev)?;
            }
            if seen.contains(&h) {
                return Err(Error::new("duplicate evidence"));
            }
            seen.push(h);
        }
        Ok(())
    }

    fn update(&mut self, state: &State, committed: &[Evidence]) {
        for ev in committed {
            let h = ev.hash();
            self.pending.retain(|p| p.hash() != h);
            if !self.committed.contains(&h) {
                self.committed.push(h);
            }
        }
        // Drop expired pending evidence.
        let params = state.consensus_params.evidence;
        let bs = &self.block_store;
        self.pending.retain(|ev| {
            let t = bs
                .load_block_meta(ev.height())
                .ok()
                .flatten()
                .map_or(ev.time(), |m| m.header.time);
            !is_evidence_expired(
                state.last_block_height,
                state.last_block_time,
                ev.height(),
                t,
                &params,
            )
        });
        self.process_consensus_buffer(state);
        self.state = Some(state.clone());
    }

    fn report_conflicting_votes(&mut self, vote_a: Vote, vote_b: Vote) {
        self.buffer.push((vote_a, vote_b));
    }
}

/// `VerifyDuplicateVote`.
pub fn verify_duplicate_vote(
    e: &DuplicateVoteEvidence,
    chain_id: &str,
    vals: &ctypes::ValidatorSet,
) -> Result<()> {
    e.validate_basic().map_err(|x| Error::new(x.to_string()))?;
    let (Some(a), Some(b)) = (&e.vote_a, &e.vote_b) else {
        return Err(Error::new("one or both of the votes are empty"));
    };
    let Some((_, val)) = vals.get_by_address(&a.validator_address) else {
        return Err(Error::new(format!(
            "address {} was not a validator at height {}",
            hex::encode_upper(&a.validator_address),
            e.height()
        )));
    };
    if a.height != b.height || a.round != b.round || a.vote_type != b.vote_type {
        return Err(Error::new(format!(
            "h/r/s does not match: {}/{}/{} vs {}/{}/{}",
            a.height, a.round, a.vote_type, b.height, b.round, b.vote_type
        )));
    }
    if a.validator_address != b.validator_address {
        return Err(Error::new("validator addresses do not match"));
    }
    if a.block_id.equals(&b.block_id) {
        return Err(Error::new(format!(
            "block IDs are the same ({}) - not a real duplicate vote",
            a.block_id
        )));
    }
    if val.voting_power != e.validator_power {
        return Err(Error::new(format!(
            "validator power from evidence and our validator set does not match ({} != {})",
            e.validator_power, val.voting_power
        )));
    }
    if vals.total_voting_power() != e.total_voting_power {
        return Err(Error::new(format!(
            "total voting power from the evidence and our validator set does not match ({} != {})",
            e.total_voting_power,
            vals.total_voting_power()
        )));
    }
    a.verify(chain_id, &val.pub_key)
        .map_err(|x| Error::new(format!("verifying VoteA: {x}")))?;
    b.verify(chain_id, &val.pub_key)
        .map_err(|x| Error::new(format!("verifying VoteB: {x}")))?;
    Ok(())
}
