//! The consensus reactor (`consensus/reactor.go`) on the `p2p` switch.
//!
//! Channels as celestia-core v0.42.3 registers them: `State` (0x20),
//! `Vote` (0x22) and `VoteSetBits` (0x23). The legacy `Data` channel
//! (0x21, proposals and block parts) is only registered with
//! `EnableLegacyBlockProp`, which celestia-core disables by default:
//! proposals and parts travel through the propagation reactor (`RECOVERY`,
//! 0x50/0x51) instead.
//!
//! The state machine runs on its own thread ([`crate::service`]); the
//! reactor sees it through a published [`Snapshot`] of the round state
//! (Go's `conR.rs`, refreshed by `updateRoundStateRoutine`) and feeds it
//! peer messages through an input channel. Per peer, tokio tasks run
//! `gossipVotesRoutine`, `queryMaj23Routine` and (legacy only)
//! `gossipDataRoutine`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration;

use ctypes::params::AbciParams;
use ctypes::vote::{PRECOMMIT_TYPE, PREVOTE_TYPE};
use ctypes::{BlockId, ExtendedCommit, PartSetHeader, Time};
use nodestore::BlockStore;
use p2p::{ChannelDescriptor, Envelope, NodeId, Peer, WeakSwitch};

use crate::peer_state::{self, NewRoundStep, PeerRoundState, PeerState, VoteSetReader};
use crate::service::Input;
use crate::types::{ConsensusMessage, RoundState, RoundStep};
use crate::vote_set::BitArray;

/// `StateChannel`.
pub const STATE_CHANNEL: u8 = 0x20;
/// `DataChannel` (legacy block propagation only).
pub const DATA_CHANNEL: u8 = 0x21;
/// `VoteChannel`.
pub const VOTE_CHANNEL: u8 = 0x22;
/// `VoteSetBitsChannel`.
pub const VOTE_SET_BITS_CHANNEL: u8 = 0x23;
/// `maxMsgSize`.
pub const MAX_MSG_SIZE: usize = 512 * 1024;
/// `ReactorIncomingMessageQueueSize`.
pub const REACTOR_INCOMING_MESSAGE_QUEUE_SIZE: usize = 5000;

/// Reactor settings (`cfg.ConsensusConfig`).
#[derive(Debug, Clone)]
pub struct ReactorConfig {
    /// `PeerGossipSleepDuration`.
    pub peer_gossip_sleep: Duration,
    /// `PeerQueryMaj23SleepDuration`.
    pub peer_query_maj23_sleep: Duration,
    /// `EnableLegacyBlockProp`: register and gossip on the data channel.
    pub gossip_data_enabled: bool,
}

impl Default for ReactorConfig {
    fn default() -> Self {
        Self {
            peer_gossip_sleep: Duration::from_millis(100),
            peer_query_maj23_sleep: Duration::from_secs(2),
            gossip_data_enabled: false,
        }
    }
}

/// What the reactor knows of the state machine (Go's `conR.rs` plus the
/// state fields the reactor reads under `stateMtx`).
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// The round state.
    pub rs: RoundState,
    /// `state.InitialHeight`.
    pub initial_height: i64,
    /// `state.ConsensusParams.ABCI`.
    pub abci: AbciParams,
}

/// The consensus reactor.
pub struct Reactor {
    config: ReactorConfig,
    block_store: BlockStore,
    inputs: Mutex<Option<std::sync::mpsc::Sender<Input>>>,
    snapshot: RwLock<Arc<Snapshot>>,
    peers: Mutex<HashMap<NodeId, Arc<PeerState>>>,
    switch: Mutex<WeakSwitch>,
    runtime: Mutex<Option<tokio::runtime::Handle>>,
    wait_sync: AtomicBool,
    running: AtomicBool,
    me: Weak<Self>,
}

impl std::fmt::Debug for Reactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConsensusReactor")
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `int64(time.Since(t).Seconds())`.
fn seconds_since(now: Time, t: Time) -> i64 {
    nodestore::state::sub_times(now, t) / 1_000_000_000
}

impl Reactor {
    /// `NewReactor`: `snapshot` is the state machine's initial round state;
    /// `wait_sync` while block / state sync runs.
    #[must_use]
    pub fn new(
        config: ReactorConfig,
        block_store: BlockStore,
        snapshot: Snapshot,
        wait_sync: bool,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            config,
            block_store,
            inputs: Mutex::new(None),
            snapshot: RwLock::new(Arc::new(snapshot)),
            peers: Mutex::new(HashMap::new()),
            switch: Mutex::new(WeakSwitch::default()),
            runtime: Mutex::new(None),
            wait_sync: AtomicBool::new(wait_sync),
            running: AtomicBool::new(false),
            me: me.clone(),
        })
    }

    /// Connects the reactor to the state machine's input channel.
    pub fn set_inputs(&self, tx: std::sync::mpsc::Sender<Input>) {
        *lock(&self.inputs) = Some(tx);
    }

    /// Publishes a new round state (`updateRoundStateRoutine`).
    pub fn set_snapshot(&self, s: Snapshot) {
        *self
            .snapshot
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(s);
    }

    /// `getRoundState`.
    #[must_use]
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshot
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// `WaitSync`.
    #[must_use]
    pub fn wait_sync(&self) -> bool {
        self.wait_sync.load(Ordering::SeqCst)
    }

    /// Clears `waitSync` (the state machine side of `SwitchToConsensus`
    /// runs on the consensus thread, see [`Input::SwitchToConsensus`]).
    pub fn set_wait_sync(&self, v: bool) {
        self.wait_sync.store(v, Ordering::SeqCst);
    }

    /// The state of a connected peer.
    #[must_use]
    pub fn peer_state(&self, id: &NodeId) -> Option<Arc<PeerState>> {
        lock(&self.peers).get(id).cloned()
    }

    /// Round states of all peers (diagnostics).
    #[must_use]
    pub fn peer_round_states(&self) -> Vec<(NodeId, PeerRoundState)> {
        lock(&self.peers)
            .iter()
            .map(|(id, ps)| (*id, ps.round_state()))
            .collect()
    }

    fn input(&self, i: Input) {
        if let Some(tx) = lock(&self.inputs).as_ref() {
            let _ = tx.send(i);
        }
    }

    fn switch(&self) -> Option<p2p::Switch> {
        lock(&self.switch).upgrade()
    }

    /// `Switch.Broadcast` from any thread.
    fn broadcast(&self, ch: u8, msg: &ConsensusMessage) {
        let Some(sw) = self.switch() else { return };
        let Some(rt) = lock(&self.runtime).clone() else {
            return;
        };
        let _g = rt.enter();
        let _ = sw.broadcast(ch, msg.encode());
    }

    /// Sends one message to one peer without waiting (`TrySend`).
    pub fn try_send_to(&self, peer: &NodeId, ch: u8, msg: &ConsensusMessage) -> bool {
        self.switch()
            .and_then(|sw| sw.get_peer(peer))
            .is_some_and(|p| p.try_send(ch, msg.encode()))
    }

    /// `StopPeerForError` by ID.
    pub fn stop_peer_for_error(&self, peer: &NodeId, reason: &str) {
        let Some(sw) = self.switch() else { return };
        if let Some(p) = sw.get_peer(peer) {
            let Some(rt) = lock(&self.runtime).clone() else {
                return;
            };
            let _g = rt.enter();
            sw.stop_peer_for_error(&p, reason);
        }
    }

    // ------------------------------------------------------------------
    // Broadcasts from state-machine events

    /// `broadcastNewRoundStepMessage`.
    pub fn broadcast_new_round_step(&self, m: NewRoundStep) {
        self.broadcast(STATE_CHANNEL, &new_round_step_msg(m));
    }

    /// `broadcastNewValidBlockMessage`.
    pub fn broadcast_new_valid_block(
        &self,
        height: i64,
        round: i32,
        psh: &PartSetHeader,
        parts: &BitArray,
        is_commit: bool,
    ) {
        self.broadcast(
            STATE_CHANNEL,
            &ConsensusMessage::NewValidBlock {
                height,
                round,
                block_part_set_header: psh.clone(),
                block_parts: parts.clone(),
                is_commit,
            },
        );
    }

    /// `broadcastHasVoteMessage`.
    pub fn broadcast_has_vote(&self, height: i64, round: i32, vote_type: i32, index: i32) {
        self.broadcast(
            STATE_CHANNEL,
            &ConsensusMessage::HasVote {
                height,
                round,
                vote_type,
                index,
            },
        );
    }

    /// `makeRoundStepMessage` for the current snapshot.
    fn current_round_step(&self) -> NewRoundStep {
        let s = self.snapshot();
        NewRoundStep {
            height: s.rs.height,
            round: s.rs.round,
            step: s.rs.step,
            seconds_since_start_time: seconds_since(crate::ticker::system_now(), s.rs.start_time),
            last_commit_round: s
                .rs
                .last_commit
                .as_ref()
                .map_or(-1, crate::vote_set::VoteSet::round),
        }
    }

    /// Broadcasts our current round step (after `SwitchToConsensus`).
    pub fn broadcast_current_round_step(&self) {
        self.broadcast_new_round_step(self.current_round_step());
    }

    // ------------------------------------------------------------------
    // Per-peer routines

    fn spawn_routines(self: &Arc<Self>, peer: Arc<Peer>, ps: Arc<PeerState>) {
        let Some(rt) = lock(&self.runtime).clone() else {
            return;
        };
        if self.config.gossip_data_enabled && peer.node_info().has_channel(DATA_CHANNEL) {
            rt.spawn(self.clone().gossip_data_routine(peer.clone(), ps.clone()));
        }
        rt.spawn(self.clone().gossip_votes_routine(peer.clone(), ps.clone()));
        rt.spawn(self.clone().query_maj23_routine(peer, ps));
    }

    fn alive(&self, peer: &Peer) -> bool {
        peer.is_running() && self.running.load(Ordering::SeqCst)
    }

    /// `PeerState.PickSendVote`.
    async fn pick_send_vote(
        peer: &Peer,
        ps: &PeerState,
        votes: &(dyn VoteSetReader + Sync),
    ) -> bool {
        let Some(vote) = ps.pick_vote_to_send(votes) else {
            return false;
        };
        if peer
            .send(VOTE_CHANNEL, ConsensusMessage::Vote(vote.clone()).encode())
            .await
        {
            ps.set_has_vote(&vote);
            return true;
        }
        false
    }

    /// `gossipVotesRoutine`.
    async fn gossip_votes_routine(self: Arc<Self>, peer: Arc<Peer>, ps: Arc<PeerState>) {
        let mut sleeping = 0;
        loop {
            if !self.alive(&peer) {
                return;
            }
            let snap = self.snapshot();
            let rs = &snap.rs;
            let prs = ps.round_state();
            match sleeping {
                1 => sleeping = 2,
                2 => sleeping = 0,
                _ => {}
            }
            if rs.height == prs.height && self.gossip_votes_for_height(rs, &prs, &peer, &ps).await {
                continue;
            }
            if prs.height != 0 && rs.height == prs.height + 1 {
                if let Some(lc) = &rs.last_commit {
                    if Self::pick_send_vote(&peer, &ps, lc).await {
                        continue;
                    }
                }
            }
            let base = self.block_store.base();
            if base > 0 && prs.height != 0 && rs.height >= prs.height + 2 && prs.height >= base {
                let ve = snap
                    .abci
                    .vote_extensions_enabled(prs.height)
                    .unwrap_or(false);
                let ec: Option<ExtendedCommit> = if ve {
                    self.block_store
                        .load_block_extended_commit(prs.height)
                        .ok()
                        .flatten()
                } else {
                    self.block_store
                        .load_block_commit(prs.height)
                        .ok()
                        .flatten()
                        .map(|c| c.wrapped_extended_commit())
                };
                if let Some(ec) = ec {
                    if Self::pick_send_vote(&peer, &ps, &ec).await {
                        continue;
                    }
                }
            }
            if sleeping == 0 || sleeping == 2 {
                sleeping = 1;
            }
            tokio::time::sleep(self.config.peer_gossip_sleep).await;
        }
    }

    /// `gossipVotesForHeight`.
    async fn gossip_votes_for_height(
        &self,
        rs: &RoundState,
        prs: &PeerRoundState,
        peer: &Peer,
        ps: &PeerState,
    ) -> bool {
        if prs.step == RoundStep::NewHeight {
            if let Some(lc) = &rs.last_commit {
                if Self::pick_send_vote(peer, ps, lc).await {
                    return true;
                }
            }
        }
        if prs.step <= RoundStep::Propose
            && prs.round != -1
            && prs.round <= rs.round
            && prs.proposal_pol_round != -1
        {
            if let Some(pol) = rs.votes.prevotes(prs.proposal_pol_round) {
                if Self::pick_send_vote(peer, ps, pol).await {
                    return true;
                }
            }
        }
        if prs.step <= RoundStep::PrevoteWait && prs.round != -1 && prs.round <= rs.round {
            if let Some(v) = rs.votes.prevotes(prs.round) {
                if Self::pick_send_vote(peer, ps, v).await {
                    return true;
                }
            }
        }
        if prs.step <= RoundStep::PrecommitWait && prs.round != -1 && prs.round <= rs.round {
            if let Some(v) = rs.votes.precommits(prs.round) {
                if Self::pick_send_vote(peer, ps, v).await {
                    return true;
                }
            }
        }
        if prs.round != -1 && prs.round <= rs.round {
            if let Some(v) = rs.votes.prevotes(prs.round) {
                if Self::pick_send_vote(peer, ps, v).await {
                    return true;
                }
            }
        }
        if prs.proposal_pol_round != -1 {
            if let Some(pol) = rs.votes.prevotes(prs.proposal_pol_round) {
                if Self::pick_send_vote(peer, ps, pol).await {
                    return true;
                }
            }
        }
        false
    }

    /// `queryMaj23Routine`.
    async fn query_maj23_routine(self: Arc<Self>, peer: Arc<Peer>, ps: Arc<PeerState>) {
        let sleep = self.config.peer_query_maj23_sleep;
        loop {
            if !self.alive(&peer) {
                return;
            }
            for vt in [PREVOTE_TYPE, PRECOMMIT_TYPE] {
                let snap = self.snapshot();
                let prs = ps.round_state();
                if snap.rs.height != prs.height {
                    continue;
                }
                let set = if vt == PREVOTE_TYPE {
                    snap.rs.votes.prevotes(prs.round)
                } else {
                    snap.rs.votes.precommits(prs.round)
                };
                if let Some(maj23) = set.and_then(crate::vote_set::VoteSet::two_thirds_majority) {
                    let _ = peer.try_send(
                        STATE_CHANNEL,
                        ConsensusMessage::VoteSetMaj23 {
                            height: prs.height,
                            round: prs.round,
                            vote_type: vt,
                            block_id: maj23,
                        }
                        .encode(),
                    );
                    tokio::time::sleep(sleep).await;
                }
            }
            {
                let snap = self.snapshot();
                let prs = ps.round_state();
                if snap.rs.height == prs.height && prs.proposal_pol_round >= 0 {
                    if let Some(maj23) = snap
                        .rs
                        .votes
                        .prevotes(prs.proposal_pol_round)
                        .and_then(crate::vote_set::VoteSet::two_thirds_majority)
                    {
                        let _ = peer.try_send(
                            STATE_CHANNEL,
                            ConsensusMessage::VoteSetMaj23 {
                                height: prs.height,
                                round: prs.proposal_pol_round,
                                vote_type: PREVOTE_TYPE,
                                block_id: maj23,
                            }
                            .encode(),
                        );
                        tokio::time::sleep(sleep).await;
                    }
                }
            }
            {
                let prs = ps.round_state();
                if prs.catchup_commit_round != -1
                    && prs.height > 0
                    && prs.height <= self.block_store.height()
                    && prs.height >= self.block_store.base()
                {
                    if let Some(commit) = self.load_commit(prs.height) {
                        let _ = peer.try_send(
                            STATE_CHANNEL,
                            ConsensusMessage::VoteSetMaj23 {
                                height: prs.height,
                                round: commit.round,
                                vote_type: PRECOMMIT_TYPE,
                                block_id: commit.block_id,
                            }
                            .encode(),
                        );
                        tokio::time::sleep(sleep).await;
                    }
                }
            }
            tokio::time::sleep(sleep).await;
        }
    }

    /// `State.LoadCommit`.
    fn load_commit(&self, height: i64) -> Option<ctypes::Commit> {
        if height == self.block_store.height() {
            return self.block_store.load_seen_commit(height).ok().flatten();
        }
        self.block_store.load_block_commit(height).ok().flatten()
    }

    /// `gossipDataRoutine` (legacy block propagation only).
    async fn gossip_data_routine(self: Arc<Self>, peer: Arc<Peer>, ps: Arc<PeerState>) {
        let sleep = self.config.peer_gossip_sleep;
        loop {
            if !self.config.gossip_data_enabled || !self.alive(&peer) {
                return;
            }
            let snap = self.snapshot();
            let rs = &snap.rs;
            let prs = ps.round_state();
            // Proposal block parts the peer lacks.
            if let Some(parts) = &rs.proposal_block_parts {
                if parts.has_header(&prs.proposal_block_part_set_header) {
                    let ours = BitArray(parts.bit_array());
                    let missing = ours.sub(&prs.proposal_block_parts);
                    if let Some(index) = missing.pick_random(|n| rand::random_range(0..n)) {
                        if let Some(part) = parts.get_part(index) {
                            let m = ConsensusMessage::BlockPart {
                                height: rs.height,
                                round: rs.round,
                                part,
                            };
                            if peer.send(DATA_CHANNEL, m.encode()).await {
                                ps.set_has_proposal_block_part(prs.height, prs.round, index);
                            }
                            continue;
                        }
                    }
                }
            }
            // Catch-up from the block store.
            let base = self.block_store.base();
            if base > 0 && 0 < prs.height && prs.height < rs.height && prs.height >= base {
                if prs.proposal_block_parts.is_nil() {
                    match self.block_store.load_block_meta(prs.height).ok().flatten() {
                        Some(meta) => ps.init_proposal_block_parts(&meta.block_id.part_set_header),
                        None => tokio::time::sleep(sleep).await,
                    }
                    continue;
                }
                self.gossip_data_for_catchup(&prs, &peer, &ps).await;
                continue;
            }
            if rs.height != prs.height || rs.round != prs.round {
                tokio::time::sleep(sleep).await;
                continue;
            }
            if let Some(p) = &rs.proposal {
                if !prs.proposal {
                    if peer
                        .send(DATA_CHANNEL, ConsensusMessage::Proposal(p.clone()).encode())
                        .await
                    {
                        ps.set_has_proposal(p);
                    }
                    if p.pol_round >= 0 {
                        let pol = rs
                            .votes
                            .prevotes(p.pol_round)
                            .map(crate::vote_set::VoteSet::bit_array)
                            .unwrap_or_default();
                        let _ = peer
                            .send(
                                DATA_CHANNEL,
                                ConsensusMessage::ProposalPol {
                                    height: rs.height,
                                    proposal_pol_round: p.pol_round,
                                    proposal_pol: pol,
                                }
                                .encode(),
                            )
                            .await;
                    }
                    continue;
                }
            }
            tokio::time::sleep(sleep).await;
        }
    }

    /// `gossipDataForCatchup`.
    async fn gossip_data_for_catchup(&self, prs: &PeerRoundState, peer: &Peer, ps: &PeerState) {
        let sleep = self.config.peer_gossip_sleep;
        let Some(index) = prs
            .proposal_block_parts
            .not()
            .pick_random(|n| rand::random_range(0..n))
        else {
            tokio::time::sleep(sleep).await;
            return;
        };
        let Some(meta) = self.block_store.load_block_meta(prs.height).ok().flatten() else {
            tokio::time::sleep(sleep).await;
            return;
        };
        if meta.block_id.part_set_header != prs.proposal_block_part_set_header {
            tokio::time::sleep(sleep).await;
            return;
        }
        let Some(part) = self
            .block_store
            .load_block_part(prs.height, index)
            .ok()
            .flatten()
        else {
            tokio::time::sleep(sleep).await;
            return;
        };
        let m = ConsensusMessage::BlockPart {
            height: prs.height,
            round: prs.round,
            part,
        };
        if peer.send(DATA_CHANNEL, m.encode()).await {
            ps.set_has_proposal_block_part(prs.height, prs.round, index);
        } else {
            tokio::time::sleep(sleep).await;
        }
    }

    // ------------------------------------------------------------------
    // Receive

    fn receive_inner(&self, e: &Envelope) -> Result<(), String> {
        let msg = ConsensusMessage::decode(&e.message)?;
        msg.validate_basic()?;
        let id = e.src.id();
        let ps = self
            .peer_state(&id)
            .ok_or_else(|| format!("peer {id} has no state"))?;
        match e.channel_id {
            STATE_CHANNEL => match msg {
                ConsensusMessage::NewRoundStep {
                    height,
                    round,
                    step,
                    seconds_since_start_time,
                    last_commit_round,
                } => {
                    let initial = self.snapshot().initial_height;
                    validate_height(height, last_commit_round, initial)?;
                    ps.apply_new_round_step(
                        &NewRoundStep {
                            height,
                            round,
                            step,
                            seconds_since_start_time,
                            last_commit_round,
                        },
                        crate::ticker::system_now(),
                    );
                }
                ConsensusMessage::NewValidBlock {
                    height,
                    round,
                    block_part_set_header,
                    block_parts,
                    is_commit,
                } => ps.apply_new_valid_block(
                    height,
                    round,
                    &block_part_set_header,
                    &block_parts,
                    is_commit,
                ),
                ConsensusMessage::HasVote {
                    height,
                    round,
                    vote_type,
                    index,
                } => ps.apply_has_vote(height, round, vote_type, index),
                ConsensusMessage::VoteSetMaj23 {
                    height,
                    round,
                    vote_type,
                    block_id,
                } => {
                    if self.snapshot().rs.height != height {
                        return Ok(());
                    }
                    self.input(Input::PeerMaj23 {
                        height,
                        round,
                        vote_type,
                        peer: id,
                        block_id,
                    });
                }
                other => tracing::error!(msg = ?other, "Unknown message type"),
            },
            DATA_CHANNEL => {
                if self.wait_sync() {
                    return Ok(());
                }
                match msg {
                    ConsensusMessage::Proposal(p) => {
                        ps.set_has_proposal(&p);
                        self.input(Input::Peer {
                            msg: ConsensusMessage::Proposal(p),
                            peer: id.to_string(),
                        });
                    }
                    ConsensusMessage::ProposalPol {
                        height,
                        proposal_pol_round,
                        proposal_pol,
                    } => ps.apply_proposal_pol(height, proposal_pol_round, &proposal_pol),
                    m @ ConsensusMessage::BlockPart { .. } => {
                        if let ConsensusMessage::BlockPart {
                            height,
                            round,
                            part,
                        } = &m
                        {
                            ps.set_has_proposal_block_part(*height, *round, part.index as usize);
                        }
                        self.input(Input::Peer {
                            msg: m,
                            peer: id.to_string(),
                        });
                    }
                    other => tracing::error!(msg = ?other, "Unknown message type"),
                }
            }
            VOTE_CHANNEL => {
                if self.wait_sync() {
                    return Ok(());
                }
                if let ConsensusMessage::Vote(v) = msg {
                    let snap = self.snapshot();
                    let rs = &snap.rs;
                    ps.ensure_vote_bit_arrays(rs.height, rs.validators.size());
                    ps.ensure_vote_bit_arrays(
                        rs.height - 1,
                        rs.last_commit
                            .as_ref()
                            .map_or(0, crate::vote_set::VoteSet::size),
                    );
                    ps.set_has_vote(&v);
                    self.input(Input::Peer {
                        msg: ConsensusMessage::Vote(v),
                        peer: id.to_string(),
                    });
                } else {
                    tracing::error!(msg = ?msg, "Unknown message type");
                }
            }
            VOTE_SET_BITS_CHANNEL => {
                if self.wait_sync() {
                    return Ok(());
                }
                if let ConsensusMessage::VoteSetBits {
                    height,
                    round,
                    vote_type,
                    block_id,
                    votes,
                } = msg
                {
                    let snap = self.snapshot();
                    if snap.rs.height == height {
                        let set = if vote_type == PREVOTE_TYPE {
                            snap.rs.votes.prevotes(round)
                        } else {
                            snap.rs.votes.precommits(round)
                        };
                        let ours = set.and_then(|s| s.bit_array_by_block_id(&block_id));
                        ps.apply_vote_set_bits(height, round, vote_type, &votes, ours.as_ref());
                    } else {
                        ps.apply_vote_set_bits(height, round, vote_type, &votes, None);
                    }
                } else {
                    tracing::error!(msg = ?msg, "Unknown message type");
                }
            }
            ch => tracing::error!(ch, "Unknown chId"),
        }
        Ok(())
    }

    /// The reply to a `VoteSetMaj23Message` (sent by the consensus thread
    /// after `SetPeerMaj23`).
    pub fn send_vote_set_bits(
        &self,
        peer: &NodeId,
        height: i64,
        round: i32,
        vote_type: i32,
        block_id: BlockId,
        votes: BitArray,
    ) {
        let _ = self.try_send_to(
            peer,
            VOTE_SET_BITS_CHANNEL,
            &ConsensusMessage::VoteSetBits {
                height,
                round,
                vote_type,
                block_id,
                votes,
            },
        );
    }
}

/// `NewRoundStepMessage.ValidateHeight`.
pub fn validate_height(height: i64, last_commit_round: i32, initial: i64) -> Result<(), String> {
    if height < initial {
        return Err(format!(
            "invalid Height {height} (lower than initial height {initial})"
        ));
    }
    if height == initial && last_commit_round != -1 {
        return Err(format!(
            "invalid LastCommitRound {last_commit_round} (must be -1 for initial height {initial})"
        ));
    }
    if height > initial && last_commit_round < 0 {
        return Err(format!(
            "LastCommitRound can only be negative for initial height {initial}"
        ));
    }
    Ok(())
}

/// `makeRoundStepMessage`.
fn new_round_step_msg(m: NewRoundStep) -> ConsensusMessage {
    ConsensusMessage::NewRoundStep {
        height: m.height,
        round: m.round,
        step: m.step,
        seconds_since_start_time: m.seconds_since_start_time,
        last_commit_round: m.last_commit_round,
    }
}

impl p2p::Reactor for Reactor {
    fn name(&self) -> String {
        "CONSENSUS".into()
    }

    fn channels(&self) -> Vec<ChannelDescriptor> {
        let mut ch = vec![
            ChannelDescriptor {
                send_queue_capacity: 200,
                recv_message_capacity: MAX_MSG_SIZE,
                ..ChannelDescriptor::new(STATE_CHANNEL, 230)
            }
            .fill_defaults(),
            ChannelDescriptor {
                send_queue_capacity: 200,
                recv_buffer_capacity: 100 * 100,
                recv_message_capacity: MAX_MSG_SIZE,
                ..ChannelDescriptor::new(VOTE_CHANNEL, 235)
            }
            .fill_defaults(),
            ChannelDescriptor {
                send_queue_capacity: 10,
                recv_buffer_capacity: 1024,
                recv_message_capacity: MAX_MSG_SIZE,
                ..ChannelDescriptor::new(VOTE_SET_BITS_CHANNEL, 206)
            }
            .fill_defaults(),
        ];
        if self.config.gossip_data_enabled {
            ch.push(
                ChannelDescriptor {
                    send_queue_capacity: 100,
                    recv_buffer_capacity: 50 * 4096,
                    recv_message_capacity: MAX_MSG_SIZE,
                    ..ChannelDescriptor::new(DATA_CHANNEL, 10)
                }
                .fill_defaults(),
            );
        }
        ch
    }

    fn set_switch(&self, switch: WeakSwitch) {
        *lock(&self.switch) = switch;
    }

    fn on_start(self: Arc<Self>) -> Result<(), String> {
        *lock(&self.runtime) = Some(tokio::runtime::Handle::current());
        self.running.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn on_stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    fn init_peer(&self, peer: &Arc<Peer>) -> Result<(), String> {
        let ps = Arc::new(PeerState::new());
        peer_state::attach(peer, &ps);
        lock(&self.peers).insert(peer.id(), ps);
        Ok(())
    }

    fn add_peer(&self, peer: &Arc<Peer>) {
        if !self.running.load(Ordering::SeqCst) {
            return;
        }
        let Some(ps) = self.peer_state(&peer.id()) else {
            tracing::error!(peer = %peer.id(), "peer has no state");
            return;
        };
        if let Some(me) = self.me.upgrade() {
            me.spawn_routines(peer.clone(), ps);
        }
        if !self.wait_sync() {
            let _ = peer.try_send(
                STATE_CHANNEL,
                new_round_step_msg(self.current_round_step()).encode(),
            );
        }
    }

    fn remove_peer(&self, peer: &Arc<Peer>, _reason: &str) {
        lock(&self.peers).remove(&peer.id());
    }

    fn receive(&self, e: Envelope) -> Result<(), String> {
        if !self.running.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.receive_inner(&e).map_err(|err| {
            tracing::error!(peer = %e.src.id(), ch = e.channel_id, err = %err, "Peer sent us invalid msg");
            err
        })
    }

    fn queue_size(&self) -> usize {
        REACTOR_INCOMING_MESSAGE_QUEUE_SIZE
    }
}
