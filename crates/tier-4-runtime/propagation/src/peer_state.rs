//! `consensus/propagation/peer_state.go`: per-peer haves, wants and
//! requests by height and round, outstanding request accounting and the
//! cache of compact blocks that could not be verified yet.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use ctypes::BitArray;

use crate::PeerStateEditor;
use crate::types::CompactBlock;

/// `MaxUnverifiedProposals`.
pub const MAX_UNVERIFIED_PROPOSALS: usize = 24;
/// Capacity of the received-haves queue (`receivedHaves`).
pub const RECEIVED_HAVES_CAPACITY: usize = 20_000;

/// `request`: an announced part to maybe request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    /// Height.
    pub height: i64,
    /// Round.
    pub round: i32,
    /// Combined part index.
    pub index: u32,
}

/// `partState`.
#[derive(Debug, Clone)]
pub struct PartState {
    /// Parts the peer has.
    pub haves: BitArray,
    /// Parts the peer wants from us.
    pub wants: BitArray,
    /// Parts we requested from the peer.
    pub requests: BitArray,
}

impl PartState {
    fn new(size: usize) -> Self {
        Self {
            haves: BitArray::new(size),
            wants: BitArray::new(size),
            requests: BitArray::new(size),
        }
    }
}

/// `PeerState`.
pub struct PeerState {
    /// The peer.
    pub peer: Arc<p2p::Peer>,
    state: HashMap<i64, HashMap<i32, PartState>>,
    /// `concurrentReqs`.
    pub concurrent_reqs: i64,
    /// `receivedHaves`.
    pub received_haves: VecDeque<Request>,
    remaining_requests: HashMap<i64, HashMap<i32, i64>>,
    /// `consensusPeerState`.
    pub consensus_peer_state: Arc<dyn PeerStateEditor>,
    unverified_proposals: HashMap<i64, Arc<CompactBlock>>,
}

impl std::fmt::Debug for PeerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerState")
            .field("peer", &self.peer.id())
            .field("concurrent_reqs", &self.concurrent_reqs)
            .finish_non_exhaustive()
    }
}

impl PeerState {
    /// `newPeerState`.
    #[must_use]
    pub fn new(peer: Arc<p2p::Peer>, editor: Arc<dyn PeerStateEditor>) -> Self {
        Self {
            peer,
            state: HashMap::new(),
            concurrent_reqs: 0,
            received_haves: VecDeque::new(),
            remaining_requests: HashMap::new(),
            consensus_peer_state: editor,
            unverified_proposals: HashMap::new(),
        }
    }

    /// `Initialize`.
    pub fn initialize(&mut self, height: i64, round: i32, size: usize) {
        self.state
            .entry(height)
            .or_default()
            .entry(round)
            .or_insert_with(|| PartState::new(size));
    }

    /// `DecreaseRemainingRequests`.
    pub fn decrease_remaining_requests(&mut self, height: i64, round: i32, sub: i64) {
        let Some(h) = self.remaining_requests.get_mut(&height) else {
            return;
        };
        let r = h.entry(round).or_insert(0);
        if *r == 0 {
            return;
        }
        *r = (*r - sub).max(0);
    }

    /// `SetRemainingRequests`.
    pub fn set_remaining_requests(&mut self, height: i64, round: i32, count: i64) {
        self.remaining_requests
            .entry(height)
            .or_default()
            .insert(round, count);
    }

    /// `GetRemainingRequests`.
    #[must_use]
    pub fn remaining_requests(&self, height: i64, round: i32) -> i64 {
        self.remaining_requests
            .get(&height)
            .and_then(|h| h.get(&round))
            .copied()
            .unwrap_or(0)
    }

    /// `DecreaseConcurrentReqs`.
    pub fn decrease_concurrent_reqs(&mut self, sub: i64) {
        if self.concurrent_reqs == 0 {
            return;
        }
        self.concurrent_reqs = (self.concurrent_reqs - sub).max(0);
    }

    /// `AddHaves`.
    pub fn add_haves(&mut self, height: i64, round: i32, haves: &BitArray) {
        self.initialize(height, round, haves.size());
        if let Some(s) = self.part_state_mut(height, round) {
            s.haves.add(haves);
        }
    }

    /// `AddWants`.
    pub fn add_wants(&mut self, height: i64, round: i32, wants: &BitArray) {
        self.initialize(height, round, wants.size());
        if let Some(s) = self.part_state_mut(height, round) {
            s.wants.add(wants);
        }
    }

    /// `AddRequests`.
    pub fn add_requests(&mut self, height: i64, round: i32, requests: &BitArray) {
        if requests.is_nil() {
            tracing::error!("peer state requests is nil or empty");
            return;
        }
        self.initialize(height, round, requests.size());
        if let Some(s) = self.part_state_mut(height, round) {
            s.requests.add(requests);
        }
    }

    fn part_state_mut(&mut self, height: i64, round: i32) -> Option<&mut PartState> {
        self.state.get_mut(&height)?.get_mut(&round)
    }

    fn part_state(&self, height: i64, round: i32) -> Option<&PartState> {
        self.state.get(&height)?.get(&round)
    }

    /// `SetHave` (error when the height/round is not initialized).
    pub fn set_have(&mut self, height: i64, round: i32, part: usize) -> Result<(), String> {
        let s = self
            .part_state_mut(height, round)
            .ok_or("peer state: height or round not found")?;
        s.haves.set(part, true);
        Ok(())
    }

    /// `SetWant`.
    pub fn set_want(
        &mut self,
        height: i64,
        round: i32,
        part: usize,
        wants: bool,
    ) -> Result<(), String> {
        let s = self
            .part_state_mut(height, round)
            .ok_or("peer state: height or round not found")?;
        s.wants.set(part, wants);
        Ok(())
    }

    /// `GetHaves`.
    pub fn haves_mut(&mut self, height: i64, round: i32) -> Option<&mut BitArray> {
        self.part_state_mut(height, round).map(|s| &mut s.haves)
    }

    /// `GetWants`.
    #[must_use]
    pub fn wants(&self, height: i64, round: i32) -> Option<&BitArray> {
        self.part_state(height, round).map(|s| &s.wants)
    }

    /// `GetRequests`.
    #[must_use]
    pub fn requests(&self, height: i64, round: i32) -> Option<&BitArray> {
        self.part_state(height, round).map(|s| &s.requests)
    }

    /// `WantsPart`.
    #[must_use]
    pub fn wants_part(&self, height: i64, round: i32, part: u32) -> bool {
        self.wants(height, round)
            .is_some_and(|w| w.get(part as usize))
    }

    /// `DeleteRound`.
    pub fn delete_round(&mut self, height: i64, round: i32) {
        if let Some(h) = self.state.get_mut(&height) {
            h.remove(&round);
        }
        if let Some(h) = self.remaining_requests.get_mut(&height) {
            h.remove(&round);
        }
    }

    /// `prune`: drops heights below `prune_past_height` (unverified compact
    /// blocks up to and including it).
    pub fn prune(&mut self, prune_past_height: i64) {
        self.state.retain(|h, _| *h >= prune_past_height);
        self.remaining_requests
            .retain(|h, _| *h >= prune_past_height);
        self.unverified_proposals
            .retain(|h, _| *h > prune_past_height);
    }

    /// `StoreUnverifiedProposal`: keeps the lowest heights when full.
    pub fn store_unverified_proposal(&mut self, cb: Arc<CompactBlock>) -> bool {
        let height = cb.proposal.height;
        if self.unverified_proposals.contains_key(&height)
            || self.unverified_proposals.len() < MAX_UNVERIFIED_PROPOSALS
        {
            self.unverified_proposals.insert(height, cb);
            return true;
        }
        let max = self
            .unverified_proposals
            .keys()
            .copied()
            .max()
            .unwrap_or(-1);
        if height < max {
            self.unverified_proposals.remove(&max);
            self.unverified_proposals.insert(height, cb);
            return true;
        }
        false
    }

    /// `GetUnverifiedProposal`.
    #[must_use]
    pub fn unverified_proposal(&self, height: i64) -> Option<Arc<CompactBlock>> {
        self.unverified_proposals.get(&height).cloned()
    }

    /// `DeleteUnverifiedProposal`.
    pub fn delete_unverified_proposal(&mut self, height: i64) {
        self.unverified_proposals.remove(&height);
    }

    /// `MaxUnverifiedProposalHeight`.
    #[must_use]
    pub fn max_unverified_proposal_height(&self) -> i64 {
        self.unverified_proposals.keys().copied().max().unwrap_or(0)
    }

    /// Queues an announced part (`receivedHaves <- request`, dropped when
    /// full).
    pub fn push_have(&mut self, r: Request) {
        if self.received_haves.len() < RECEIVED_HAVES_CAPACITY {
            self.received_haves.push_back(r);
        }
    }
}
