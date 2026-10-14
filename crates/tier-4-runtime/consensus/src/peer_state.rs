//! `PeerRoundState` (`consensus/types/peer_round_state.go`) and the
//! reactor's `PeerState` (`consensus/reactor.go`): what we know about a
//! peer's consensus progress, used to pick votes and parts to send.

use std::sync::{Arc, Mutex};

use ctypes::vote::{PRECOMMIT_TYPE, PREVOTE_TYPE};
use ctypes::{ExtendedCommit, PartSetHeader, Proposal, Time, Vote};

use crate::types::RoundStep;
use crate::vote_set::{BitArray, VoteSet};

/// `PeerRoundState`. Go's nil bit arrays are empty [`BitArray`]s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerRoundState {
    /// Height the peer is at.
    pub height: i64,
    /// Round the peer is at (-1 if unknown).
    pub round: i32,
    /// Step the peer is at.
    pub step: RoundStep,
    /// Estimated start of round 0 at this height.
    pub start_time: Time,
    /// Whether the peer has the proposal for this round.
    pub proposal: bool,
    /// Part set header of the proposal block.
    pub proposal_block_part_set_header: PartSetHeader,
    /// Parts of the proposal block the peer has.
    pub proposal_block_parts: BitArray,
    /// Proposal's POL round (-1 if none).
    pub proposal_pol_round: i32,
    /// Prevotes the peer has for the POL round (nil until
    /// `ProposalPOLMessage`).
    pub proposal_pol: BitArray,
    /// Prevotes the peer has for this round.
    pub prevotes: BitArray,
    /// Precommits the peer has for this round.
    pub precommits: BitArray,
    /// Round of the commit for the previous height (-1 if none).
    pub last_commit_round: i32,
    /// Precommits of the previous height the peer has.
    pub last_commit: BitArray,
    /// Round we believe a +2/3 commit exists for (-1 if none).
    pub catchup_commit_round: i32,
    /// Precommits of that round the peer has.
    pub catchup_commit: BitArray,
}

impl Default for PeerRoundState {
    /// `NewPeerState`'s initial round state.
    fn default() -> Self {
        Self {
            height: 0,
            round: -1,
            step: RoundStep::NewHeight,
            start_time: Time::ZERO,
            proposal: false,
            proposal_block_part_set_header: PartSetHeader::default(),
            proposal_block_parts: BitArray::default(),
            proposal_pol_round: -1,
            proposal_pol: BitArray::default(),
            prevotes: BitArray::default(),
            precommits: BitArray::default(),
            last_commit_round: -1,
            last_commit: BitArray::default(),
            catchup_commit_round: -1,
            catchup_commit: BitArray::default(),
        }
    }
}

/// `types.VoteSetReader`: a vote set or a (wrapped) extended commit.
pub trait VoteSetReader {
    /// `GetHeight`.
    fn height(&self) -> i64;
    /// `GetRound`.
    fn round(&self) -> i32;
    /// `Type`.
    fn vote_type(&self) -> i32;
    /// `Size`.
    fn size(&self) -> usize;
    /// `BitArray`.
    fn bit_array(&self) -> BitArray;
    /// `GetByIndex`.
    fn get_by_index(&self, idx: i32) -> Option<Vote>;
    /// `IsCommit`.
    fn is_commit(&self) -> bool;
}

impl VoteSetReader for VoteSet {
    fn height(&self) -> i64 {
        VoteSet::height(self)
    }
    fn round(&self) -> i32 {
        VoteSet::round(self)
    }
    fn vote_type(&self) -> i32 {
        VoteSet::vote_type(self)
    }
    fn size(&self) -> usize {
        VoteSet::size(self)
    }
    fn bit_array(&self) -> BitArray {
        VoteSet::bit_array(self)
    }
    fn get_by_index(&self, idx: i32) -> Option<Vote> {
        VoteSet::get_by_index(self, idx).cloned()
    }
    fn is_commit(&self) -> bool {
        VoteSet::is_commit(self)
    }
}

impl VoteSetReader for ExtendedCommit {
    fn height(&self) -> i64 {
        self.height
    }
    fn round(&self) -> i32 {
        self.round
    }
    fn vote_type(&self) -> i32 {
        PRECOMMIT_TYPE
    }
    fn size(&self) -> usize {
        ExtendedCommit::size(self)
    }
    fn bit_array(&self) -> BitArray {
        BitArray(ExtendedCommit::bit_array(self))
    }
    fn get_by_index(&self, idx: i32) -> Option<Vote> {
        self.get_extended_vote(idx).ok()
    }
    fn is_commit(&self) -> bool {
        ExtendedCommit::is_commit(self)
    }
}

/// Peer statistics (`peerStateStats`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PeerStats {
    /// Useful votes received.
    pub votes: usize,
    /// Useful block parts received.
    pub block_parts: usize,
}

/// `PeerState`: a peer's round state behind a mutex.
#[derive(Debug, Default)]
pub struct PeerState {
    prs: Mutex<(PeerRoundState, PeerStats)>,
}

/// `IsVoteTypeValid`.
fn is_vote_type_valid(t: i32) -> bool {
    t == PREVOTE_TYPE || t == PRECOMMIT_TYPE
}

impl PeerRoundState {
    /// `getVoteBitArray`: the array tracking the peer's votes of a kind
    /// (`None` where Go returns nil).
    pub fn vote_bit_array_mut(
        &mut self,
        height: i64,
        round: i32,
        vote_type: i32,
    ) -> Option<&mut BitArray> {
        if !is_vote_type_valid(vote_type) {
            return None;
        }
        let ba = if self.height == height {
            if self.round == round {
                if vote_type == PREVOTE_TYPE {
                    &mut self.prevotes
                } else {
                    &mut self.precommits
                }
            } else if self.catchup_commit_round == round {
                if vote_type == PREVOTE_TYPE {
                    return None;
                }
                &mut self.catchup_commit
            } else if self.proposal_pol_round == round {
                if vote_type == PRECOMMIT_TYPE {
                    return None;
                }
                &mut self.proposal_pol
            } else {
                return None;
            }
        } else if self.height == height + 1 {
            if self.last_commit_round == round && vote_type == PRECOMMIT_TYPE {
                &mut self.last_commit
            } else {
                return None;
            }
        } else {
            return None;
        };
        (!ba.is_nil()).then_some(ba)
    }

    /// `ensureCatchupCommitRound`.
    fn ensure_catchup_commit_round(&mut self, height: i64, round: i32, num_validators: usize) {
        if self.height != height || self.catchup_commit_round == round {
            return;
        }
        self.catchup_commit_round = round;
        self.catchup_commit = if round == self.round {
            self.precommits.clone()
        } else {
            BitArray::new(num_validators)
        };
    }

    /// `ensureVoteBitArrays`.
    fn ensure_vote_bit_arrays(&mut self, height: i64, n: usize) {
        if self.height == height {
            for ba in [
                &mut self.prevotes,
                &mut self.precommits,
                &mut self.catchup_commit,
                &mut self.proposal_pol,
            ] {
                if ba.is_nil() {
                    *ba = BitArray::new(n);
                }
            }
        } else if self.height == height + 1 && self.last_commit.is_nil() {
            self.last_commit = BitArray::new(n);
        }
    }

    /// `setHasVote`.
    fn set_has_vote(&mut self, height: i64, round: i32, vote_type: i32, index: i32) {
        if let Some(ba) = self.vote_bit_array_mut(height, round, vote_type) {
            ba.set(usize::try_from(index).unwrap_or(usize::MAX), true);
        }
    }
}

/// Fields of `NewRoundStepMessage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewRoundStep {
    /// Height.
    pub height: i64,
    /// Round.
    pub round: i32,
    /// Step.
    pub step: RoundStep,
    /// Seconds since the round started.
    pub seconds_since_start_time: i64,
    /// Round of the last commit.
    pub last_commit_round: i32,
}

impl PeerState {
    /// `NewPeerState`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, (PeerRoundState, PeerStats)> {
        self.prs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `GetRoundState`: a copy.
    #[must_use]
    pub fn round_state(&self) -> PeerRoundState {
        self.lock().0.clone()
    }

    /// `GetHeight`.
    #[must_use]
    pub fn height(&self) -> i64 {
        self.lock().0.height
    }

    /// Statistics.
    #[must_use]
    pub fn stats(&self) -> PeerStats {
        self.lock().1
    }

    /// `SetHasProposal`.
    pub fn set_has_proposal(&self, proposal: &Proposal) {
        let mut g = self.lock();
        let prs = &mut g.0;
        if prs.height != proposal.height || prs.round != proposal.round || prs.proposal {
            return;
        }
        prs.proposal = true;
        if !prs.proposal_block_parts.is_nil() {
            return;
        }
        let psh = &proposal.block_id.part_set_header;
        prs.proposal_block_part_set_header = psh.clone();
        prs.proposal_block_parts = BitArray::new(psh.total as usize);
        prs.proposal_pol_round = proposal.pol_round;
        prs.proposal_pol = BitArray::default();
    }

    /// `InitProposalBlockParts`.
    pub fn init_proposal_block_parts(&self, psh: &PartSetHeader) {
        let mut g = self.lock();
        let prs = &mut g.0;
        if !prs.proposal_block_parts.is_nil() {
            return;
        }
        prs.proposal_block_part_set_header = psh.clone();
        prs.proposal_block_parts = BitArray::new(psh.total as usize);
    }

    /// `SetHasProposalBlockPart`.
    pub fn set_has_proposal_block_part(&self, height: i64, round: i32, index: usize) {
        let mut g = self.lock();
        let prs = &mut g.0;
        if prs.height != height || prs.round != round {
            return;
        }
        prs.proposal_block_parts.set(index, true);
    }

    /// `PickVoteToSend`: a vote of `votes` the peer does not have.
    pub fn pick_vote_to_send(&self, votes: &dyn VoteSetReader) -> Option<Vote> {
        let mut g = self.lock();
        let prs = &mut g.0;
        let size = votes.size();
        if size == 0 {
            return None;
        }
        let (height, round, vt) = (votes.height(), votes.round(), votes.vote_type());
        if votes.is_commit() {
            prs.ensure_catchup_commit_round(height, round, size);
        }
        prs.ensure_vote_bit_arrays(height, size);
        let ps_votes = prs.vote_bit_array_mut(height, round, vt)?;
        let index = votes
            .bit_array()
            .sub(ps_votes)
            .pick_random(|n| rand::random_range(0..n))?;
        votes.get_by_index(i32::try_from(index).ok()?)
    }

    /// `EnsureVoteBitArrays`.
    pub fn ensure_vote_bit_arrays(&self, height: i64, num_validators: usize) {
        self.lock().0.ensure_vote_bit_arrays(height, num_validators);
    }

    /// `RecordVote`.
    pub fn record_vote(&self) -> usize {
        let mut g = self.lock();
        g.1.votes += 1;
        g.1.votes
    }

    /// `RecordBlockPart`.
    pub fn record_block_part(&self) -> usize {
        let mut g = self.lock();
        g.1.block_parts += 1;
        g.1.block_parts
    }

    /// `SetHasVote`.
    pub fn set_has_vote(&self, vote: &Vote) {
        self.lock().0.set_has_vote(
            vote.height,
            vote.round,
            vote.vote_type,
            vote.validator_index,
        );
    }

    /// `ApplyNewRoundStepMessage`.
    pub fn apply_new_round_step(&self, msg: &NewRoundStep, now: Time) {
        let mut g = self.lock();
        let prs = &mut g.0;
        if crate::types::compare_hrs(
            msg.height, msg.round, msg.step, prs.height, prs.round, prs.step,
        )
        .is_le()
        {
            return;
        }
        let ps_height = prs.height;
        let ps_round = prs.round;
        let ps_catchup_commit_round = prs.catchup_commit_round;
        let ps_catchup_commit = prs.catchup_commit.clone();
        let last_precommits = prs.precommits.clone();
        prs.height = msg.height;
        prs.round = msg.round;
        prs.step = msg.step;
        prs.start_time = nodestore::state::add_duration(
            now,
            -msg.seconds_since_start_time.saturating_mul(1_000_000_000),
        );
        if ps_height != msg.height || ps_round != msg.round {
            prs.proposal = false;
            prs.proposal_block_part_set_header = PartSetHeader::default();
            prs.proposal_block_parts = BitArray::default();
            prs.proposal_pol_round = -1;
            prs.proposal_pol = BitArray::default();
            prs.prevotes = BitArray::default();
            prs.precommits = BitArray::default();
        }
        if ps_height == msg.height && ps_round != msg.round && msg.round == ps_catchup_commit_round
        {
            prs.precommits = ps_catchup_commit;
        }
        if ps_height != msg.height {
            if ps_height + 1 == msg.height && ps_round == msg.last_commit_round {
                prs.last_commit_round = msg.last_commit_round;
                prs.last_commit = last_precommits;
            } else {
                prs.last_commit_round = msg.last_commit_round;
                prs.last_commit = BitArray::default();
            }
            prs.catchup_commit_round = -1;
            prs.catchup_commit = BitArray::default();
        }
    }

    /// `ApplyNewValidBlockMessage`.
    pub fn apply_new_valid_block(
        &self,
        height: i64,
        round: i32,
        psh: &PartSetHeader,
        parts: &BitArray,
        is_commit: bool,
    ) {
        let mut g = self.lock();
        let prs = &mut g.0;
        if prs.height != height || (prs.round != round && !is_commit) {
            return;
        }
        prs.proposal_block_part_set_header = psh.clone();
        prs.proposal_block_parts = parts.clone();
    }

    /// `ApplyProposalPOLMessage`.
    pub fn apply_proposal_pol(&self, height: i64, pol_round: i32, pol: &BitArray) {
        let mut g = self.lock();
        let prs = &mut g.0;
        if prs.height != height || prs.proposal_pol_round != pol_round {
            return;
        }
        prs.proposal_pol = pol.clone();
    }

    /// `ApplyHasVoteMessage`.
    pub fn apply_has_vote(&self, height: i64, round: i32, vote_type: i32, index: i32) {
        let mut g = self.lock();
        if g.0.height != height {
            return;
        }
        g.0.set_has_vote(height, round, vote_type, index);
    }

    /// `ApplyVoteSetBitsMessage`: `our_votes` is `None` when the message
    /// is not for our height (the peer's claim then overwrites ours).
    pub fn apply_vote_set_bits(
        &self,
        height: i64,
        round: i32,
        vote_type: i32,
        votes: &BitArray,
        our_votes: Option<&BitArray>,
    ) {
        let mut g = self.lock();
        let Some(ba) = g.0.vote_bit_array_mut(height, round, vote_type) else {
            return;
        };
        match our_votes {
            None => ba.update(votes),
            Some(ours) => {
                let other = ba.sub(ours);
                let has = other.or(votes);
                ba.update(&has);
            }
        }
    }
}

impl mempool::cat::PeerState for PeerState {
    fn get_height(&self) -> i64 {
        self.height()
    }
}

impl propagation::PeerStateEditor for PeerState {
    fn set_has_proposal(&self, proposal: &Proposal) {
        PeerState::set_has_proposal(self, proposal);
    }
    fn set_has_proposal_block_part(&self, height: i64, round: i32, index: usize) {
        PeerState::set_has_proposal_block_part(self, height, round, index);
    }
    fn get_height(&self) -> i64 {
        self.height()
    }
}

/// Stores a peer state on a `p2p::Peer` for the mempool
/// (`types.PeerStateKey`) and the propagation reactor.
pub fn attach(peer: &p2p::Peer, ps: &Arc<PeerState>) {
    peer.set(
        mempool::cat::PEER_STATE_KEY,
        Arc::new(mempool::cat::PeerStateHandle(ps.clone())),
    );
    peer.set(
        propagation::PEER_STATE_EDITOR_KEY,
        Arc::new(propagation::EditorHandle(ps.clone())),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nrs(h: i64, r: i32, s: RoundStep, lcr: i32) -> NewRoundStep {
        NewRoundStep {
            height: h,
            round: r,
            step: s,
            seconds_since_start_time: 0,
            last_commit_round: lcr,
        }
    }

    #[test]
    fn new_round_step_shifts_precommits_to_last_commit() {
        let ps = PeerState::new();
        ps.apply_new_round_step(&nrs(5, 0, RoundStep::Prevote, 0), Time::ZERO);
        ps.ensure_vote_bit_arrays(5, 4);
        ps.apply_has_vote(5, 0, PRECOMMIT_TYPE, 2);
        // A duplicate or older step is ignored.
        ps.apply_new_round_step(&nrs(5, 0, RoundStep::Propose, 0), Time::ZERO);
        assert_eq!(ps.round_state().step, RoundStep::Prevote);
        ps.apply_new_round_step(&nrs(6, 0, RoundStep::NewHeight, 0), Time::ZERO);
        let prs = ps.round_state();
        assert_eq!(prs.last_commit_round, 0);
        assert!(prs.last_commit.get(2));
        assert!(prs.prevotes.is_nil());
        // A height jump drops the last commit.
        ps.apply_new_round_step(&nrs(9, 0, RoundStep::NewHeight, 1), Time::ZERO);
        assert!(ps.round_state().last_commit.is_nil());
    }

    #[test]
    fn vote_set_bits_merge() {
        let ps = PeerState::new();
        ps.apply_new_round_step(&nrs(3, 1, RoundStep::Prevote, 0), Time::ZERO);
        ps.ensure_vote_bit_arrays(3, 3);
        ps.apply_has_vote(3, 1, PREVOTE_TYPE, 0);
        let theirs = BitArray(vec![false, true, false]);
        let ours = BitArray(vec![true, true, true]);
        ps.apply_vote_set_bits(3, 1, PREVOTE_TYPE, &theirs, Some(&ours));
        assert_eq!(ps.round_state().prevotes.0, vec![false, true, false]);
        ps.apply_vote_set_bits(3, 1, PREVOTE_TYPE, &BitArray(vec![true, false, true]), None);
        assert_eq!(ps.round_state().prevotes.0, vec![true, false, true]);
    }
}
