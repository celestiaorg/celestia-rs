//! `HeightVoteSet` (`consensus/types/height_vote_set.go`): prevotes and
//! precommits of every round of one height, plus up to two catch-up rounds
//! per peer.

use std::collections::BTreeMap;

use ctypes::vote::{PRECOMMIT_TYPE, PREVOTE_TYPE, is_vote_type_valid};
use ctypes::{BlockId, ValidatorSet, Vote};

use crate::vote_set::{VoteError, VoteSet};

/// Prevotes and precommits of one round.
#[derive(Debug, Clone)]
pub struct RoundVoteSet {
    /// Prevotes.
    pub prevotes: VoteSet,
    /// Precommits.
    pub precommits: VoteSet,
}

/// `HeightVoteSet`.
#[derive(Debug, Clone)]
pub struct HeightVoteSet {
    chain_id: String,
    height: i64,
    val_set: ValidatorSet,
    extensions_enabled: bool,
    round: i32,
    round_vote_sets: BTreeMap<i32, RoundVoteSet>,
    peer_catchup_rounds: BTreeMap<String, Vec<i32>>,
}

impl HeightVoteSet {
    /// `NewHeightVoteSet` / `NewExtendedHeightVoteSet`.
    #[must_use]
    pub fn new(
        chain_id: &str,
        height: i64,
        val_set: ValidatorSet,
        extensions_enabled: bool,
    ) -> Self {
        let mut hvs = Self {
            chain_id: chain_id.to_owned(),
            height,
            val_set,
            extensions_enabled,
            round: 0,
            round_vote_sets: BTreeMap::new(),
            peer_catchup_rounds: BTreeMap::new(),
        };
        hvs.add_round(0);
        hvs
    }

    /// Height.
    #[must_use]
    pub fn height(&self) -> i64 {
        self.height
    }

    /// Max tracked round.
    #[must_use]
    pub fn round(&self) -> i32 {
        self.round
    }

    /// Whether precommits verify vote extensions.
    #[must_use]
    pub fn extensions_enabled(&self) -> bool {
        self.extensions_enabled
    }

    /// `SetRound`: tracks rounds up to `round` (callers pass `round + 1`).
    pub fn set_round(&mut self, round: i32) {
        let new_round = self.round.saturating_sub(1);
        if self.round != 0 && round < new_round {
            // Go panics ("SetRound() must increment hvs.round").
            return;
        }
        for r in new_round..=round {
            if !self.round_vote_sets.contains_key(&r) {
                self.add_round(r);
            }
        }
        self.round = round;
    }

    fn add_round(&mut self, round: i32) {
        let prevotes = VoteSet::new(
            &self.chain_id,
            self.height,
            round,
            PREVOTE_TYPE,
            self.val_set.clone(),
        );
        let precommits = if self.extensions_enabled {
            VoteSet::new_extended(
                &self.chain_id,
                self.height,
                round,
                PRECOMMIT_TYPE,
                self.val_set.clone(),
            )
        } else {
            VoteSet::new(
                &self.chain_id,
                self.height,
                round,
                PRECOMMIT_TYPE,
                self.val_set.clone(),
            )
        };
        self.round_vote_sets.insert(
            round,
            RoundVoteSet {
                prevotes,
                precommits,
            },
        );
    }

    /// `AddVote`. Votes for untracked rounds create the round when the peer
    /// has used fewer than two catch-up rounds.
    pub fn add_vote(&mut self, vote: &Vote, peer: &str) -> Result<bool, (bool, VoteError)> {
        if !is_vote_type_valid(vote.vote_type) {
            return Ok(false);
        }
        if !self.round_vote_sets.contains_key(&vote.round) {
            let rounds = self.peer_catchup_rounds.entry(peer.to_owned()).or_default();
            if rounds.len() < 2 {
                rounds.push(vote.round);
                self.add_round(vote.round);
            } else {
                return Err((false, VoteError::UnwantedRound));
            }
        }
        match self.get_mut(vote.round, vote.vote_type) {
            Some(vs) => vs.add_vote(vote),
            None => Ok(false),
        }
    }

    fn get_mut(&mut self, round: i32, vote_type: i32) -> Option<&mut VoteSet> {
        let rvs = self.round_vote_sets.get_mut(&round)?;
        match vote_type {
            PREVOTE_TYPE => Some(&mut rvs.prevotes),
            PRECOMMIT_TYPE => Some(&mut rvs.precommits),
            _ => None,
        }
    }

    /// `Prevotes(round)`.
    #[must_use]
    pub fn prevotes(&self, round: i32) -> Option<&VoteSet> {
        self.round_vote_sets.get(&round).map(|r| &r.prevotes)
    }

    /// `Precommits(round)`.
    #[must_use]
    pub fn precommits(&self, round: i32) -> Option<&VoteSet> {
        self.round_vote_sets.get(&round).map(|r| &r.precommits)
    }

    /// All tracked rounds.
    #[must_use]
    pub fn rounds(&self) -> Vec<i32> {
        self.round_vote_sets.keys().copied().collect()
    }

    /// `POLInfo`: the latest round with a +2/3 prevote majority.
    #[must_use]
    pub fn pol_info(&self) -> (i32, BlockId) {
        let mut r = self.round;
        while r >= 0 {
            if let Some(id) = self.prevotes(r).and_then(VoteSet::two_thirds_majority) {
                return (r, id);
            }
            r -= 1;
        }
        (-1, BlockId::default())
    }

    /// `SetPeerMaj23`.
    pub fn set_peer_maj23(
        &mut self,
        round: i32,
        vote_type: i32,
        peer: &str,
        block_id: BlockId,
    ) -> Result<(), String> {
        if !is_vote_type_valid(vote_type) {
            return Err(format!("setPeerMaj23: Invalid vote type {vote_type:X}"));
        }
        match self.get_mut(round, vote_type) {
            Some(vs) => vs.set_peer_maj23(peer, block_id),
            None => Ok(()),
        }
    }
}
