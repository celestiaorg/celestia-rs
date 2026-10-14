//! The consensus state machine (`consensus/state.go`).
//!
//! [`ConsensusState`] is single-threaded and deterministic: inputs are peer
//! messages, fired timeouts and "txs available"; own proposals, block parts
//! and votes go through an internal queue (WAL-synced before they are
//! handled, as in Go); outputs are [`Event`]s for the gossip layer. Time
//! comes from a [`Clock`], so the same code runs under the system clock in
//! the node and under a simulated clock in tests.
//!
//! Where Go panics ("CONSENSUS FAILURE") the state machine records the
//! failure, stops processing and reports it through [`ConsensusState::failure`].

use std::collections::VecDeque;

use ctypes::proto::types as pb;
use ctypes::vote::{PRECOMMIT_TYPE, PREVOTE_TYPE};
use ctypes::{
    BLOCK_ID_FLAG_COMMIT, BLOCK_PART_SIZE_BYTES, Block, BlockId, ExtendedCommit, PartSet,
    PartSetHeader, Proposal, PubKey, Time, Vote,
};
use nodestore::state::{State, add_duration, sub_times};
use nodestore::{BlockExecutor, BlockStore};
use privval::PrivValidator;
use prost::Message as _;

use crate::height_vote_set::HeightVoteSet;
use crate::ticker::{Clock, TimeoutTicker};
use crate::types::{ConsensusMessage, RoundState, RoundStep, TimeoutInfo};
use crate::vote_set::{VoteError, VoteSet};
use crate::wal::{TimedWalMessage, Wal, WalError, WalMessage};

/// `cfg.ConsensusConfig` (durations in Go nanoseconds). The `timeout_*`
/// values are fallbacks for zero app-provided timeouts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsensusConfig {
    /// `TimeoutPropose`.
    pub timeout_propose: i64,
    /// `TimeoutProposeDelta`.
    pub timeout_propose_delta: i64,
    /// `TimeoutPrevote`.
    pub timeout_prevote: i64,
    /// `TimeoutPrevoteDelta`.
    pub timeout_prevote_delta: i64,
    /// `TimeoutPrecommit`.
    pub timeout_precommit: i64,
    /// `TimeoutPrecommitDelta`.
    pub timeout_precommit_delta: i64,
    /// `TimeoutCommit`.
    pub timeout_commit: i64,
    /// `SkipTimeoutCommit`.
    pub skip_timeout_commit: bool,
    /// `CreateEmptyBlocks`.
    pub create_empty_blocks: bool,
    /// `CreateEmptyBlocksInterval`.
    pub create_empty_blocks_interval: i64,
    /// `DoubleSignCheckHeight`.
    pub double_sign_check_height: i64,
    /// `OnlyInternalWal`: peer messages are not written to the WAL.
    pub only_internal_wal: bool,
}

const MS: i64 = 1_000_000;

impl Default for ConsensusConfig {
    /// `DefaultConsensusConfig`.
    fn default() -> Self {
        Self {
            timeout_propose: 3000 * MS,
            timeout_propose_delta: 500 * MS,
            timeout_prevote: 1000 * MS,
            timeout_prevote_delta: 500 * MS,
            timeout_precommit: 1000 * MS,
            timeout_precommit_delta: 500 * MS,
            timeout_commit: 1000 * MS,
            skip_timeout_commit: false,
            create_empty_blocks: true,
            create_empty_blocks_interval: 0,
            double_sign_check_height: 0,
            only_internal_wal: true,
        }
    }
}

impl ConsensusConfig {
    /// `WaitForTxs`.
    #[must_use]
    pub fn wait_for_txs(&self) -> bool {
        !self.create_empty_blocks || self.create_empty_blocks_interval > 0
    }

    /// `CommitWithCustomTimeout`.
    #[must_use]
    pub fn commit_with_custom_timeout(&self, t: Time, custom: i64) -> Time {
        add_duration(
            t,
            if custom == 0 {
                self.timeout_commit
            } else {
                custom
            },
        )
    }
}

/// A message for the state machine (`msgInfo`); an empty `peer_id` means
/// an own (internal) message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsgInfo {
    /// Proposal, block part or vote.
    pub msg: ConsensusMessage,
    /// Sender (empty for internal).
    pub peer_id: String,
}

/// What the state machine tells the gossip layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A new step (`EventNewRoundStep`).
    NewRoundStep {
        /// Height.
        height: i64,
        /// Round.
        round: i32,
        /// Step.
        step: RoundStep,
    },
    /// A vote was added (`EventVote`).
    Vote(Vote),
    /// We have (or are fetching) a block with a polka / commit
    /// (`EventValidBlock`).
    ValidBlock {
        /// Height.
        height: i64,
        /// Round.
        round: i32,
        /// Part set header.
        parts_header: PartSetHeader,
        /// Whether we are in the commit step.
        is_commit: bool,
    },
    /// An own message to broadcast (proposal, block part, vote).
    Broadcast(ConsensusMessage),
    /// A block was committed and applied.
    NewBlock {
        /// Height.
        height: i64,
        /// Block id.
        block_id: BlockId,
        /// App hash after the block.
        app_hash: Vec<u8>,
    },
    /// Conflicting votes reported to the evidence pool.
    ConflictingVotes {
        /// Vote A.
        vote_a: Box<Vote>,
        /// Vote B.
        vote_b: Box<Vote>,
    },
}

/// The consensus state machine.
pub struct ConsensusState {
    config: ConsensusConfig,
    priv_validator: Option<Box<dyn PrivValidator>>,
    priv_validator_pub_key: Option<PubKey>,
    /// Block execution (and the block / state stores).
    pub block_exec: BlockExecutor,
    rs: RoundState,
    state: State,
    state_set: bool,
    ticker: TimeoutTicker,
    wal: Box<dyn Wal>,
    clock: Box<dyn Clock>,
    internal_queue: VecDeque<MsgInfo>,
    events: Vec<Event>,
    replay_mode: bool,
    /// Steps taken (for tests).
    pub n_steps: u64,
    failure: Option<String>,
    /// Whether to replay the WAL on start.
    pub do_wal_catchup: bool,
    /// WAL records replayed by the last [`ConsensusState::start`].
    pub replayed_messages: usize,
}

impl std::fmt::Debug for ConsensusState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConsensusState")
            .field("height", &self.rs.height)
            .field("round", &self.rs.round)
            .field("step", &self.rs.step)
            .finish_non_exhaustive()
    }
}

fn block_hashes_to(block: Option<&Block>, hash: &[u8]) -> bool {
    match block {
        Some(b) if !hash.is_empty() => b.header.hash().as_deref() == Some(hash),
        _ => false,
    }
}

fn parts_has_header(parts: Option<&PartSet>, h: &PartSetHeader) -> bool {
    parts.is_some_and(|p| p.has_header(h))
}

/// `blockMatchesBlockID`.
fn block_matches(block: Option<&Block>, parts: Option<&PartSet>, id: &BlockId) -> bool {
    block_hashes_to(block, &id.hash) && parts_has_header(parts, &id.part_set_header)
}

fn new_height_vote_set(state: &State, height: i64) -> HeightVoteSet {
    let ext = state
        .consensus_params
        .abci
        .vote_extensions_enabled(height)
        .unwrap_or(false);
    HeightVoteSet::new(&state.chain_id, height, state.validators.clone(), ext)
}

macro_rules! fail {
    ($self:ident, $($arg:tt)*) => {{
        $self.set_failure(format!($($arg)*));
        return;
    }};
}

impl ConsensusState {
    /// `NewState`: reconstructs the last commit from the store and moves to
    /// the state's next height.
    pub fn new(
        config: ConsensusConfig,
        state: State,
        block_exec: BlockExecutor,
        wal: Box<dyn Wal>,
        clock: Box<dyn Clock>,
        priv_validator: Option<Box<dyn PrivValidator>>,
    ) -> Result<Self, String> {
        let rs = RoundState {
            height: 0,
            round: 0,
            step: RoundStep::NewHeight,
            start_time: Time::ZERO,
            commit_time: Time::ZERO,
            validators: ctypes::ValidatorSet::default(),
            proposal: None,
            proposal_block: None,
            proposal_block_parts: None,
            locked_round: -1,
            locked_block: None,
            locked_block_parts: None,
            valid_round: -1,
            valid_block: None,
            valid_block_parts: None,
            votes: HeightVoteSet::new(&state.chain_id, 1, ctypes::ValidatorSet::default(), false),
            commit_round: -1,
            last_commit: None,
            last_validators: ctypes::ValidatorSet::default(),
            triggered_timeout_precommit: false,
        };
        let mut cs = Self {
            config,
            priv_validator: None,
            priv_validator_pub_key: None,
            block_exec,
            rs,
            state: state.clone(),
            state_set: false,
            ticker: TimeoutTicker::default(),
            wal,
            clock,
            internal_queue: VecDeque::new(),
            events: Vec::new(),
            replay_mode: false,
            n_steps: 0,
            failure: None,
            do_wal_catchup: true,
            replayed_messages: 0,
        };
        if let Some(pv) = priv_validator {
            cs.set_priv_validator(pv)?;
        }
        if state.last_block_height > 0 {
            cs.reconstruct_last_commit(&state)?;
        }
        cs.update_to_state(state);
        if let Some(f) = cs.failure.take() {
            return Err(f);
        }
        Ok(cs)
    }

    /// `SetPrivValidator`.
    pub fn set_priv_validator(&mut self, pv: Box<dyn PrivValidator>) -> Result<(), String> {
        self.priv_validator_pub_key = Some(pv.pub_key().map_err(|e| e.to_string())?);
        self.priv_validator = Some(pv);
        Ok(())
    }

    /// The round state.
    #[must_use]
    pub fn round_state(&self) -> &RoundState {
        &self.rs
    }

    /// The chain state (until height - 1).
    #[must_use]
    pub fn state(&self) -> &State {
        &self.state
    }

    /// The block store.
    #[must_use]
    pub fn block_store(&self) -> &BlockStore {
        &self.block_exec.block_store
    }

    /// The consensus failure that stopped this state machine, if any.
    #[must_use]
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    /// When the pending timeout fires.
    #[must_use]
    pub fn next_timeout(&self) -> Option<Time> {
        self.ticker.deadline()
    }

    /// Drains the events produced so far.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// The clock.
    pub fn clock_mut(&mut self) -> &mut dyn Clock {
        self.clock.as_mut()
    }

    fn now(&self) -> Time {
        self.clock.now()
    }

    /// The state machine's current time.
    #[must_use]
    pub fn clock_now(&self) -> Time {
        self.clock.now()
    }

    fn set_failure(&mut self, msg: String) {
        if self.failure.is_none() {
            tracing::error!(err = %msg, "CONSENSUS FAILURE!!!");
            self.failure = Some(msg);
        }
    }

    fn priv_addr(&self) -> Option<Vec<u8>> {
        self.priv_validator_pub_key.as_ref().map(PubKey::address)
    }

    // ------------------------------------------------------------------
    // Start / replay

    /// `OnStart`: WAL catch-up replay (repairing a corrupted WAL once),
    /// the double-sign risk check and scheduling round 0.
    pub fn start(&mut self) -> Result<(), String> {
        if self.do_wal_catchup {
            let mut repaired = false;
            loop {
                match self.catchup_replay(self.rs.height) {
                    Ok(()) => break,
                    Err(WalError::Corruption(e)) if !repaired => {
                        tracing::error!(err = %e, "the WAL file is corrupted; attempting repair");
                        self.wal.repair().map_err(|e| e.to_string())?;
                        repaired = true;
                    }
                    Err(WalError::Corruption(e)) => {
                        return Err(format!("DataCorruptionError[{e}]"));
                    }
                    Err(WalError::Io(e)) => {
                        tracing::error!(err = %e, "error on catchup replay; proceeding to start state anyway");
                        break;
                    }
                }
            }
        }
        if let Some(f) = &self.failure {
            return Err(f.clone());
        }
        self.check_double_signing_risk(self.rs.height)?;
        let start = self.rs.start_time;
        self.schedule_round0(start, self.rs.height);
        self.drain_internal();
        Ok(())
    }

    /// `catchupReplay`.
    fn catchup_replay(&mut self, cs_height: i64) -> Result<(), WalError> {
        if self.wal.search_for_end_height(cs_height)?.is_some() {
            return Err(WalError::Io(format!(
                "wal should not contain #ENDHEIGHT {cs_height}"
            )));
        }
        if cs_height < self.state.initial_height {
            return Err(WalError::Io(format!(
                "cannot replay height {cs_height}, below initial height {}",
                self.state.initial_height
            )));
        }
        let end_height = if cs_height == self.state.initial_height {
            0
        } else {
            cs_height - 1
        };
        let Some(msgs) = self.wal.search_for_end_height(end_height)? else {
            return Err(WalError::Io(format!(
                "cannot replay height {cs_height}. WAL does not contain #ENDHEIGHT for {end_height}"
            )));
        };
        tracing::info!(
            height = cs_height,
            n = msgs.len(),
            "Catchup by replaying consensus messages"
        );
        self.replay_mode = true;
        self.replayed_messages = msgs.len();
        for m in msgs {
            match m.msg {
                WalMessage::EndHeight(_) | WalMessage::RoundState { .. } => {}
                WalMessage::MsgInfo { msg, peer_id } => {
                    self.handle_msg(MsgInfo { msg, peer_id });
                }
                WalMessage::Timeout(ti) => {
                    let (h, r, s) = (self.rs.height, self.rs.round, self.rs.step);
                    self.handle_timeout(ti, h, r, s);
                }
            }
        }
        self.replay_mode = false;
        Ok(())
    }

    fn check_double_signing_risk(&self, height: i64) -> Result<(), String> {
        let Some(addr) = self.priv_addr() else {
            return Ok(());
        };
        if self.config.double_sign_check_height <= 0 || height <= 0 {
            return Ok(());
        }
        let n = self.config.double_sign_check_height.min(height - 1);
        for i in 1..=n {
            if let Ok(Some(c)) = self.block_exec.block_store.load_seen_commit(height - i) {
                if c.signatures
                    .iter()
                    .any(|s| s.block_id_flag == BLOCK_ID_FLAG_COMMIT && s.validator_address == addr)
                {
                    return Err("found signature from the same key".into());
                }
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Inputs

    /// A message from a peer (written to the WAL unless
    /// `only_internal_wal`).
    pub fn receive_peer(&mut self, msg: ConsensusMessage, peer_id: &str) {
        if self.failure.is_some() {
            return;
        }
        let mi = MsgInfo {
            msg,
            peer_id: peer_id.to_owned(),
        };
        if !self.config.only_internal_wal {
            let rec = self.timed(WalMessage::MsgInfo {
                msg: mi.msg.clone(),
                peer_id: mi.peer_id.clone(),
            });
            if let Err(e) = self.wal.write(&rec) {
                tracing::error!(err = %e, "failed writing to WAL");
            }
        }
        self.handle_msg(mi);
        self.drain_internal();
    }

    /// Fires the pending timeout if it is due at the clock's time.
    pub fn tick(&mut self) {
        if self.failure.is_some() {
            return;
        }
        let now = self.now();
        if let Some(ti) = self.ticker.poll(now) {
            let rec = self.timed(WalMessage::Timeout(ti));
            if let Err(e) = self.wal.write(&rec) {
                tracing::error!(err = %e, "failed writing to WAL");
            }
            let (h, r, s) = (self.rs.height, self.rs.round, self.rs.step);
            self.handle_timeout(ti, h, r, s);
            self.drain_internal();
        }
    }

    /// The mempool has transactions (`handleTxsAvailable`).
    pub fn txs_available(&mut self) {
        if self.failure.is_some() || self.rs.round != 0 {
            return;
        }
        match self.rs.step {
            RoundStep::NewHeight => {
                if self.need_proof_block(self.rs.height) {
                    return;
                }
                let d = sub_times(self.rs.start_time, self.now()) + MS;
                self.schedule_timeout(d, self.rs.height, 0, RoundStep::NewRound);
            }
            RoundStep::NewRound => self.enter_propose(self.rs.height, 0),
            _ => {}
        }
        self.drain_internal();
    }

    fn timed(&self, msg: WalMessage) -> TimedWalMessage {
        TimedWalMessage {
            time: self.now(),
            msg,
        }
    }

    /// Processes own messages (WAL-synced first, as Go's internal queue).
    fn drain_internal(&mut self) {
        while let Some(mi) = self.internal_queue.pop_front() {
            if self.failure.is_some() {
                return;
            }
            let rec = self.timed(WalMessage::MsgInfo {
                msg: mi.msg.clone(),
                peer_id: String::new(),
            });
            if let Err(e) = self.wal.write_sync(&rec) {
                fail!(
                    self,
                    "failed to write {:?} msg to consensus WAL due to {e}; check your file system and restart the node",
                    mi.msg
                );
            }
            self.handle_msg(mi);
        }
    }

    fn send_internal(&mut self, msg: ConsensusMessage) {
        self.internal_queue.push_back(MsgInfo {
            msg,
            peer_id: String::new(),
        });
    }

    // ------------------------------------------------------------------
    // State updates

    fn reconstruct_last_commit(&mut self, state: &State) -> Result<(), String> {
        let h = state.last_block_height;
        let ext = state
            .consensus_params
            .abci
            .vote_extensions_enabled(h)
            .map_err(|e| e.to_string())?;
        let bs = &self.block_exec.block_store;
        let vs = if ext {
            let ec = bs
                .load_block_extended_commit(h)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("extended commit for height {h} not found"))?;
            VoteSet::from_extended_commit(&state.chain_id, &ec, state.last_validators.clone())?
        } else {
            let c = match bs.load_seen_commit(h).map_err(|e| e.to_string())? {
                Some(c) => c,
                None => bs
                    .load_block_commit(h)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("commit for height {h} not found"))?,
            };
            if c.height != h {
                return Err(format!(
                    "heights don't match in votesFromSeenCommit {}!={h}",
                    c.height
                ));
            }
            VoteSet::from_commit(&state.chain_id, &c, state.last_validators.clone())?
        };
        if !vs.has_two_thirds_majority() {
            return Err("commit does not have +2/3 majority".into());
        }
        self.rs.last_commit = Some(vs);
        Ok(())
    }

    /// `updateToState`.
    fn update_to_state(&mut self, state: State) {
        if self.rs.commit_round > -1
            && 0 < self.rs.height
            && self.rs.height != state.last_block_height
        {
            fail!(
                self,
                "updateToState() expected state height of {} but found {}",
                self.rs.height,
                state.last_block_height
            );
        }
        if self.state_set {
            if self.state.last_block_height > 0
                && self.state.last_block_height + 1 != self.rs.height
            {
                fail!(
                    self,
                    "inconsistent cs.state.LastBlockHeight+1 {} vs cs.rs.Height {}",
                    self.state.last_block_height + 1,
                    self.rs.height
                );
            }
            if state.last_block_height <= self.state.last_block_height {
                self.new_step();
                return;
            }
        }
        if state.last_block_height == 0 {
            self.rs.last_commit = None;
        } else if self.rs.commit_round > -1 {
            let cr = self.rs.commit_round;
            match self.rs.votes.precommits(cr) {
                Some(p) if p.has_two_thirds_majority() => {
                    self.rs.last_commit = Some(p.clone());
                }
                _ => fail!(
                    self,
                    "wanted to form a commit, but precommits (H/R: {}/{cr}) didn't have 2/3+",
                    state.last_block_height
                ),
            }
        } else if self.rs.last_commit.is_none() {
            fail!(
                self,
                "last commit cannot be empty after initial block (H:{})",
                state.last_block_height + 1
            );
        }
        let mut height = state.last_block_height + 1;
        if height == 1 {
            height = state.initial_height;
        }
        self.rs.height = height;
        self.update_round_step(0, RoundStep::NewHeight);
        // Go reads the *previous* state's timeout (zero, i.e. the config
        // fallback, before the first state is set).
        let commit_timeout = if state.last_block_height == 0 {
            state.timeouts.commit
        } else if self.state_set {
            self.state.timeouts.commit
        } else {
            0
        };
        let base = if self.rs.commit_time.is_zero() {
            self.now()
        } else {
            self.rs.commit_time
        };
        self.rs.start_time = self.config.commit_with_custom_timeout(base, commit_timeout);
        self.rs.validators = state.validators.clone();
        self.rs.proposal = None;
        self.rs.proposal_block = None;
        self.rs.proposal_block_parts = None;
        self.rs.locked_round = -1;
        self.rs.locked_block = None;
        self.rs.locked_block_parts = None;
        self.rs.valid_round = -1;
        self.rs.valid_block = None;
        self.rs.valid_block_parts = None;
        self.rs.votes = new_height_vote_set(&state, height);
        self.rs.commit_round = -1;
        self.rs.last_validators = state.last_validators.clone();
        self.rs.triggered_timeout_precommit = false;
        self.state = state;
        self.state_set = true;
        self.new_step();
    }

    fn update_round_step(&mut self, round: i32, step: RoundStep) {
        self.rs.round = round;
        self.rs.step = step;
    }

    /// `newStep`: WAL + event.
    fn new_step(&mut self) {
        let rec = self.timed(WalMessage::RoundState {
            height: self.rs.height,
            round: self.rs.round,
            step: self.rs.step.as_str().to_owned(),
        });
        if let Err(e) = self.wal.write(&rec) {
            tracing::error!(err = %e, "failed writing to WAL");
        }
        self.n_steps += 1;
        self.events.push(Event::NewRoundStep {
            height: self.rs.height,
            round: self.rs.round,
            step: self.rs.step,
        });
    }

    fn schedule_round0(&mut self, start_time: Time, height: i64) {
        let d = sub_times(start_time, self.now());
        self.schedule_timeout(d, height, 0, RoundStep::NewHeight);
    }

    fn schedule_timeout(&mut self, duration: i64, height: i64, round: i32, step: RoundStep) {
        let now = self.now();
        self.ticker.schedule(
            TimeoutInfo {
                duration,
                height,
                round,
                step,
            },
            now,
        );
    }

    fn timeout_value(app: i64, app_delta: i64, cfg: i64, cfg_delta: i64, round: i32) -> i64 {
        let t = if app == 0 { cfg } else { app };
        let d = if app_delta == 0 { cfg_delta } else { app_delta };
        t + d * i64::from(round)
    }

    /// `Propose(round)`.
    #[must_use]
    pub fn propose_timeout(&self, round: i32) -> i64 {
        let t = &self.state.timeouts;
        Self::timeout_value(
            t.propose,
            t.propose_delta,
            self.config.timeout_propose,
            self.config.timeout_propose_delta,
            round,
        )
    }

    /// `Prevote(round)`.
    #[must_use]
    pub fn prevote_timeout(&self, round: i32) -> i64 {
        let t = &self.state.timeouts;
        Self::timeout_value(
            t.prevote,
            t.prevote_delta,
            self.config.timeout_prevote,
            self.config.timeout_prevote_delta,
            round,
        )
    }

    /// `Precommit(round)`.
    #[must_use]
    pub fn precommit_timeout(&self, round: i32) -> i64 {
        let t = &self.state.timeouts;
        Self::timeout_value(
            t.precommit,
            t.precommit_delta,
            self.config.timeout_precommit,
            self.config.timeout_precommit_delta,
            round,
        )
    }

    // ------------------------------------------------------------------
    // Message handling

    /// `handleMsg`.
    fn handle_msg(&mut self, mi: MsgInfo) {
        if self.failure.is_some() {
            return;
        }
        match mi.msg {
            ConsensusMessage::Proposal(p) => {
                let _ = self.set_proposal(p);
            }
            ConsensusMessage::BlockPart {
                height,
                round,
                part,
            } => {
                let added = self.add_proposal_block_part(height, round, &part, &mi.peer_id);
                if added
                    && self
                        .rs
                        .proposal_block_parts
                        .as_ref()
                        .is_some_and(PartSet::is_complete)
                {
                    self.handle_complete_proposal(height);
                }
            }
            ConsensusMessage::Vote(v) => {
                let _ = self.try_add_vote(&v, &mi.peer_id);
            }
            other => {
                tracing::debug!(msg = ?other, "unknown msg type for the state machine");
            }
        }
    }

    /// `handleTimeout`.
    fn handle_timeout(&mut self, ti: TimeoutInfo, h: i64, r: i32, s: RoundStep) {
        if ti.height != h || ti.round < r || (ti.round == r && ti.step < s) {
            return;
        }
        match ti.step {
            RoundStep::NewHeight => self.enter_new_round(ti.height, 0),
            RoundStep::NewRound => self.enter_propose(ti.height, ti.round),
            RoundStep::Propose => self.enter_prevote(ti.height, ti.round),
            RoundStep::PrevoteWait => self.enter_precommit(ti.height, ti.round),
            RoundStep::PrecommitWait => {
                self.enter_precommit(ti.height, ti.round);
                self.enter_new_round(ti.height, ti.round + 1);
            }
            other => fail!(self, "invalid timeout step: {other:?}"),
        }
    }

    // ------------------------------------------------------------------
    // Round steps

    /// `enterNewRound`.
    fn enter_new_round(&mut self, height: i64, round: i32) {
        if self.failure.is_some()
            || self.rs.height != height
            || round < self.rs.round
            || (self.rs.round == round && self.rs.step != RoundStep::NewHeight)
        {
            return;
        }
        let mut validators = self.rs.validators.clone();
        if self.rs.round < round {
            validators = validators.copy();
            if let Err(e) = validators.increment_proposer_priority(round - self.rs.round) {
                fail!(self, "increment proposer priority: {e}");
            }
        }
        self.update_round_step(round, RoundStep::NewRound);
        self.rs.validators = validators;
        if round != 0 {
            self.rs.proposal = None;
            self.rs.proposal_block = None;
            self.rs.proposal_block_parts = None;
        }
        self.rs.votes.set_round(round.saturating_add(1));
        self.rs.triggered_timeout_precommit = false;
        let wait_for_txs =
            self.config.wait_for_txs() && round == 0 && !self.need_proof_block(height);
        if wait_for_txs {
            if self.config.create_empty_blocks_interval > 0 {
                let d = self.config.create_empty_blocks_interval;
                self.schedule_timeout(d, height, round, RoundStep::NewRound);
            }
        } else {
            self.enter_propose(height, round);
        }
    }

    /// `needProofBlock`.
    fn need_proof_block(&self, height: i64) -> bool {
        if height == self.state.initial_height {
            return true;
        }
        match self.block_exec.block_store.load_block_meta(height - 1) {
            Ok(Some(meta)) => self.state.app_hash != meta.header.app_hash,
            _ => true,
        }
    }

    /// `enterPropose`.
    fn enter_propose(&mut self, height: i64, round: i32) {
        if self.failure.is_some()
            || self.rs.height != height
            || round < self.rs.round
            || (self.rs.round == round && RoundStep::Propose <= self.rs.step)
        {
            return;
        }
        let d = self.propose_timeout(round);
        self.schedule_timeout(d, height, round, RoundStep::Propose);
        if self.priv_validator.is_some() {
            if let Some(addr) = self.priv_addr() {
                if self.rs.validators.has_address(&addr) && self.is_proposer(&addr) {
                    self.decide_proposal(height, round);
                }
            }
        }
        // Done enterPropose.
        self.update_round_step(round, RoundStep::Propose);
        self.new_step();
        if self.is_proposal_complete() {
            let r = self.rs.round;
            self.enter_prevote(height, r);
        }
    }

    fn is_proposer(&self, address: &[u8]) -> bool {
        self.rs
            .validators
            .peek_proposer()
            .is_some_and(|p| p.address == address)
    }

    /// `defaultDecideProposal`.
    fn decide_proposal(&mut self, height: i64, round: i32) {
        let (block, parts) = if let (Some(b), Some(_)) =
            (&self.rs.valid_block, &self.rs.valid_block_parts)
        {
            let b = b.clone();
            match b.make_part_set(BLOCK_PART_SIZE_BYTES) {
                Ok(p) => (b, p),
                Err(e) => {
                    tracing::error!(err = %e, "unable to generate the partset from existing block");
                    return;
                }
            }
        } else {
            match self.create_proposal_block() {
                Ok(bp) => bp,
                Err(e) => {
                    if !self.replay_mode {
                        tracing::error!(err = %e, "unable to create proposal block");
                    }
                    return;
                }
            }
        };
        if let Err(e) = self.wal.flush_and_sync() {
            tracing::error!(err = %e, "failed flushing WAL to disk");
        }
        let block_id = BlockId {
            hash: block.header.hash().unwrap_or_default(),
            part_set_header: parts.header(),
        };
        let mut proposal = Proposal::new(height, round, self.rs.valid_round, block_id, self.now());
        let mut p = proposal.to_proto();
        let chain_id = self.state.chain_id.clone();
        let Some(pv) = self.priv_validator.as_mut() else {
            return;
        };
        match pv.sign_proposal(&chain_id, &mut p) {
            Ok(()) => {
                proposal.signature.clone_from(&p.signature);
                if let Some(ts) = p.timestamp {
                    proposal.timestamp = Time::from_unix(ts.seconds, ts.nanos);
                }
                let ok = self.priv_validator_pub_key.as_ref().is_some_and(|pk| {
                    proposal
                        .sign_bytes(&chain_id)
                        .is_ok_and(|sb| pk.verify_signature(&sb, &proposal.signature))
                });
                if !ok {
                    return;
                }
                self.events
                    .push(Event::Broadcast(ConsensusMessage::Proposal(
                        proposal.clone(),
                    )));
                self.send_internal(ConsensusMessage::Proposal(proposal));
                for i in 0..parts.total() as usize {
                    if let Some(part) = parts.get_part(i) {
                        let m = ConsensusMessage::BlockPart {
                            height: self.rs.height,
                            round: self.rs.round,
                            part,
                        };
                        self.events.push(Event::Broadcast(m.clone()));
                        self.send_internal(m);
                    }
                }
            }
            Err(e) => {
                if !self.replay_mode {
                    tracing::error!(height, round, err = %e, "propose step; failed signing proposal");
                }
            }
        }
    }

    /// `isProposalComplete`.
    fn is_proposal_complete(&self) -> bool {
        let (Some(p), Some(_)) = (&self.rs.proposal, &self.rs.proposal_block) else {
            return false;
        };
        if p.pol_round < 0 {
            return true;
        }
        self.rs
            .votes
            .prevotes(p.pol_round)
            .is_some_and(VoteSet::has_two_thirds_majority)
    }

    /// `createProposalBlock`.
    fn create_proposal_block(&mut self) -> Result<(Block, PartSet), String> {
        let last_ext_commit = if self.rs.height == self.state.initial_height {
            ExtendedCommit::default()
        } else {
            match &self.rs.last_commit {
                Some(lc) if lc.has_two_thirds_majority() => {
                    lc.make_extended_commit(&self.state.consensus_params.abci)?
                }
                _ => {
                    return Err(
                        "propose step; cannot propose anything without commit for the previous block"
                            .into(),
                    );
                }
            }
        };
        let addr = self
            .priv_addr()
            .ok_or("propose step; empty priv validator public key")?;
        let state = self.state.clone();
        self.block_exec
            .create_proposal_block(self.rs.height, &state, &last_ext_commit, addr)
            .map_err(|e| e.to_string())
    }

    /// `enterPrevote`.
    fn enter_prevote(&mut self, height: i64, round: i32) {
        if self.failure.is_some()
            || self.rs.height != height
            || round < self.rs.round
            || (self.rs.round == round && RoundStep::Prevote <= self.rs.step)
        {
            return;
        }
        self.do_prevote(height, round);
        self.update_round_step(round, RoundStep::Prevote);
        self.new_step();
    }

    /// `defaultDoPrevote`.
    fn do_prevote(&mut self, _height: i64, _round: i32) {
        if let (Some(b), Some(p)) = (&self.rs.locked_block, &self.rs.locked_block_parts) {
            let hash = b.header.hash().unwrap_or_default();
            let h = p.header();
            self.sign_add_vote(PREVOTE_TYPE, hash, h, None);
            return;
        }
        let Some(block) = self.rs.proposal_block.clone() else {
            self.sign_add_vote(PREVOTE_TYPE, Vec::new(), PartSetHeader::default(), None);
            return;
        };
        let mut b = block.clone();
        let state = self.state.clone();
        let now = self.now();
        if let Err(e) = self.block_exec.validate_block(&state, &mut b, now) {
            tracing::error!(err = %e, "prevote step: consensus deems this block invalid; prevoting nil");
            self.sign_add_vote(PREVOTE_TYPE, Vec::new(), PartSetHeader::default(), None);
            return;
        }
        let mut b = block.clone();
        let accepted = match self.block_exec.process_proposal(&mut b, &state) {
            Ok(a) => a,
            Err(e) => fail!(
                self,
                "state machine returned an error ({e}) when calling ProcessProposal"
            ),
        };
        if !accepted {
            tracing::error!(
                "prevote step: state machine rejected a proposed block; this should not happen: the proposer may be misbehaving; prevoting nil"
            );
            self.sign_add_vote(PREVOTE_TYPE, Vec::new(), PartSetHeader::default(), None);
            return;
        }
        let hash = block.header.hash().unwrap_or_default();
        let psh = self
            .rs
            .proposal_block_parts
            .as_ref()
            .map(PartSet::header)
            .unwrap_or_default();
        self.sign_add_vote(PREVOTE_TYPE, hash, psh, None);
    }

    /// `enterPrevoteWait`.
    fn enter_prevote_wait(&mut self, height: i64, round: i32) {
        if self.failure.is_some()
            || self.rs.height != height
            || round < self.rs.round
            || (self.rs.round == round && RoundStep::PrevoteWait <= self.rs.step)
        {
            return;
        }
        if !self
            .rs
            .votes
            .prevotes(round)
            .is_some_and(VoteSet::has_two_thirds_any)
        {
            fail!(
                self,
                "entering prevote wait step ({height}/{round}), but prevotes does not have any +2/3 votes"
            );
        }
        let d = self.prevote_timeout(round);
        self.schedule_timeout(d, height, round, RoundStep::PrevoteWait);
        self.update_round_step(round, RoundStep::PrevoteWait);
        self.new_step();
    }

    /// `precommitDelay`: celestia's delayed precommit (vote no earlier
    /// than `StartTime + DelayedPrecommitTimeout`).
    fn precommit_delay_until(&self) -> Option<Time> {
        let d = self.state.timeouts.delayed_precommit;
        if d == 0 {
            return None;
        }
        Some(add_duration(self.rs.start_time, d))
    }

    /// `enterPrecommit`.
    fn enter_precommit(&mut self, height: i64, round: i32) {
        if self.failure.is_some()
            || self.rs.height != height
            || round < self.rs.round
            || (self.rs.round == round && RoundStep::Precommit <= self.rs.step)
        {
            return;
        }
        if let Some(until) = self.precommit_delay_until() {
            if !self.replay_mode && until > self.now() {
                self.clock.sleep_until(until);
            }
        }
        let polka = self
            .rs
            .votes
            .prevotes(round)
            .and_then(VoteSet::two_thirds_majority);
        let Some(block_id) = polka else {
            self.sign_add_vote(PRECOMMIT_TYPE, Vec::new(), PartSetHeader::default(), None);
            self.update_round_step(round, RoundStep::Precommit);
            self.new_step();
            return;
        };
        let (pol_round, _) = self.rs.votes.pol_info();
        if pol_round < round {
            fail!(self, "this POLRound should be {round} but got {pol_round}");
        }
        if block_id.hash.is_empty() {
            if self.rs.locked_block.is_some() {
                self.rs.locked_round = -1;
                self.rs.locked_block = None;
                self.rs.locked_block_parts = None;
            }
            self.sign_add_vote(PRECOMMIT_TYPE, Vec::new(), PartSetHeader::default(), None);
        } else if block_matches(
            self.rs.locked_block.as_ref(),
            self.rs.locked_block_parts.as_ref(),
            &block_id,
        ) {
            self.rs.locked_round = round;
            let b = self.rs.locked_block.clone();
            self.sign_add_vote(
                PRECOMMIT_TYPE,
                block_id.hash.clone(),
                block_id.part_set_header.clone(),
                b,
            );
        } else if block_matches(
            self.rs.proposal_block.as_ref(),
            self.rs.proposal_block_parts.as_ref(),
            &block_id,
        ) {
            let mut b = self.rs.proposal_block.clone().unwrap_or_default();
            let state = self.state.clone();
            let now = self.now();
            if let Err(e) = self.block_exec.validate_block(&state, &mut b, now) {
                fail!(
                    self,
                    "precommit step; +2/3 prevoted for an invalid block: {e}"
                );
            }
            self.rs.locked_round = round;
            self.rs.locked_block.clone_from(&self.rs.proposal_block);
            self.rs
                .locked_block_parts
                .clone_from(&self.rs.proposal_block_parts);
            let b = self.rs.proposal_block.clone();
            self.sign_add_vote(
                PRECOMMIT_TYPE,
                block_id.hash.clone(),
                block_id.part_set_header.clone(),
                b,
            );
        } else {
            // A polka for a block we do not have: unlock, fetch, precommit nil.
            self.rs.locked_round = -1;
            self.rs.locked_block = None;
            self.rs.locked_block_parts = None;
            if !parts_has_header(
                self.rs.proposal_block_parts.as_ref(),
                &block_id.part_set_header,
            ) {
                self.rs.proposal_block = None;
                self.rs.proposal_block_parts = Some(PartSet::from_header(
                    &block_id.part_set_header,
                    BLOCK_PART_SIZE_BYTES,
                ));
            }
            self.sign_add_vote(PRECOMMIT_TYPE, Vec::new(), PartSetHeader::default(), None);
        }
        self.update_round_step(round, RoundStep::Precommit);
        self.new_step();
    }

    /// `enterPrecommitWait`.
    fn enter_precommit_wait(&mut self, height: i64, round: i32) {
        if self.failure.is_some()
            || self.rs.height != height
            || round < self.rs.round
            || (self.rs.round == round && self.rs.triggered_timeout_precommit)
        {
            return;
        }
        if !self
            .rs
            .votes
            .precommits(round)
            .is_some_and(VoteSet::has_two_thirds_any)
        {
            fail!(
                self,
                "entering precommit wait step ({height}/{round}), but precommits does not have any +2/3 votes"
            );
        }
        let d = self.precommit_timeout(round);
        self.schedule_timeout(d, height, round, RoundStep::PrecommitWait);
        self.rs.triggered_timeout_precommit = true;
        self.update_round_step(round, RoundStep::PrecommitWait);
        self.new_step();
    }

    /// `enterCommit`.
    fn enter_commit(&mut self, height: i64, commit_round: i32) {
        if self.failure.is_some() || self.rs.height != height || RoundStep::Commit <= self.rs.step {
            return;
        }
        let Some(block_id) = self
            .rs
            .votes
            .precommits(commit_round)
            .and_then(VoteSet::two_thirds_majority)
        else {
            fail!(self, "RunActionCommit() expects +2/3 precommits");
        };
        if block_matches(
            self.rs.locked_block.as_ref(),
            self.rs.locked_block_parts.as_ref(),
            &block_id,
        ) {
            self.rs.proposal_block.clone_from(&self.rs.locked_block);
            self.rs
                .proposal_block_parts
                .clone_from(&self.rs.locked_block_parts);
        }
        if !block_matches(
            self.rs.proposal_block.as_ref(),
            self.rs.proposal_block_parts.as_ref(),
            &block_id,
        ) {
            self.rs.proposal_block = None;
            if !parts_has_header(
                self.rs.proposal_block_parts.as_ref(),
                &block_id.part_set_header,
            ) {
                self.rs.proposal_block_parts = Some(PartSet::from_header(
                    &block_id.part_set_header,
                    BLOCK_PART_SIZE_BYTES,
                ));
                self.events.push(Event::ValidBlock {
                    height,
                    round: self.rs.round,
                    parts_header: block_id.part_set_header.clone(),
                    is_commit: true,
                });
            }
        }
        // Done enterCommit.
        let r = self.rs.round;
        self.update_round_step(r, RoundStep::Commit);
        self.rs.commit_round = commit_round;
        self.rs.commit_time = self.now();
        self.new_step();
        self.try_finalize_commit(height);
    }

    /// `tryFinalizeCommit`.
    fn try_finalize_commit(&mut self, height: i64) {
        if self.failure.is_some() {
            return;
        }
        if self.rs.height != height {
            fail!(
                self,
                "tryFinalizeCommit() cs.rs.Height: {} vs height: {height}",
                self.rs.height
            );
        }
        let Some(block_id) = self
            .rs
            .votes
            .precommits(self.rs.commit_round)
            .and_then(VoteSet::two_thirds_majority)
        else {
            return;
        };
        if block_id.hash.is_empty() {
            return;
        }
        if !block_hashes_to(self.rs.proposal_block.as_ref(), &block_id.hash) {
            return;
        }
        let complete = self
            .rs
            .proposal_block_parts
            .as_ref()
            .is_some_and(|p| p.has_header(&block_id.part_set_header) && p.is_complete());
        if !complete {
            return;
        }
        self.finalize_commit(height);
    }

    /// `finalizeCommit`: save the block, `#ENDHEIGHT`, apply, next height.
    fn finalize_commit(&mut self, height: i64) {
        if self.rs.height != height || self.rs.step != RoundStep::Commit {
            return;
        }
        let commit_round = self.rs.commit_round;
        let Some(precommits) = self.rs.votes.precommits(commit_round).cloned() else {
            fail!(self, "cannot finalize commit; no precommits");
        };
        let Some(block_id) = precommits.two_thirds_majority() else {
            fail!(
                self,
                "cannot finalize commit; commit does not have 2/3 majority"
            );
        };
        let (Some(mut block), Some(parts)) = (
            self.rs.proposal_block.clone(),
            self.rs.proposal_block_parts.clone(),
        ) else {
            fail!(self, "cannot finalize commit; missing proposal block");
        };
        if !parts.has_header(&block_id.part_set_header) {
            fail!(
                self,
                "expected ProposalBlockParts header to be commit header"
            );
        }
        if !block.hashes_to(&block_id.hash) {
            fail!(
                self,
                "cannot finalize commit; proposal block does not hash to commit hash"
            );
        }
        let state = self.state.clone();
        let now = self.now();
        if let Err(e) = self.block_exec.validate_block(&state, &mut block, now) {
            fail!(self, "+2/3 committed an invalid block: {e}");
        }
        tracing::info!(
            height,
            hash = %hex::encode_upper(&block_id.hash),
            root = %hex::encode_upper(&block.header.app_hash),
            num_txs = block.data.txs.len(),
            "finalizing commit of block"
        );
        let mut seen_commit = None;
        if self.block_exec.block_store.height() < block.header.height {
            let seen_ext = match precommits.make_extended_commit(&state.consensus_params.abci) {
                Ok(ec) => ec,
                Err(e) => fail!(self, "{e}"),
            };
            let sc = seen_ext.to_commit();
            let ext_enabled = state
                .consensus_params
                .abci
                .vote_extensions_enabled(block.header.height)
                .unwrap_or(false);
            let res = if ext_enabled {
                self.block_exec
                    .block_store
                    .save_block_with_extended_commit(&mut block, &parts, &seen_ext)
            } else {
                self.block_exec
                    .block_store
                    .save_block(&mut block, &parts, &sc)
            };
            if let Err(e) = res {
                fail!(self, "failed to save block: {e}");
            }
            seen_commit = Some(sc);
        }
        let _ = seen_commit;
        // #ENDHEIGHT: the block store has the block.
        let rec = self.timed(WalMessage::EndHeight(height));
        if let Err(e) = self.wal.write_sync(&rec) {
            fail!(
                self,
                "failed to write EndHeight {height} msg to consensus WAL due to {e}; check your file system and restart the node"
            );
        }
        let bid = BlockId {
            hash: block.hash().unwrap_or_default(),
            part_set_header: parts.header(),
        };
        let new_state = match self
            .block_exec
            .apply_verified_block(&state, bid.clone(), &mut block)
        {
            Ok(s) => s,
            Err(e) => fail!(self, "failed to apply block; error {e}"),
        };
        self.events.push(Event::NewBlock {
            height,
            block_id: bid,
            app_hash: new_state.app_hash.clone(),
        });
        self.update_to_state(new_state);
        if self.failure.is_some() {
            return;
        }
        if let Some(pv) = &self.priv_validator {
            if let Ok(pk) = pv.pub_key() {
                self.priv_validator_pub_key = Some(pk);
            }
        }
        let (st, h) = (self.rs.start_time, self.rs.height);
        self.schedule_round0(st, h);
    }

    // ------------------------------------------------------------------
    // Proposals and parts

    /// `defaultSetProposal`.
    fn set_proposal(&mut self, proposal: Proposal) -> Result<(), String> {
        if self.rs.proposal.is_some() {
            return Ok(());
        }
        if proposal.height != self.rs.height || proposal.round != self.rs.round {
            return Err(format!(
                "invalid proposal height/round: proposal height {} round {} does not match state height {} round {}",
                proposal.height, proposal.round, self.rs.height, self.rs.round
            ));
        }
        if proposal.pol_round < -1
            || (proposal.pol_round >= 0 && proposal.pol_round >= proposal.round)
        {
            return Err(format!(
                "error invalid proposal POL round: POLRound {} Round {}",
                proposal.pol_round, proposal.round
            ));
        }
        let Some(proposer) = self.rs.validators.peek_proposer() else {
            return Err("no proposer".into());
        };
        let sb = proposal
            .sign_bytes(&self.state.chain_id)
            .map_err(|e| e.to_string())?;
        if !proposer.pub_key.verify_signature(&sb, &proposal.signature) {
            return Err("error invalid proposal signature".into());
        }
        let mut max_bytes = self.state.consensus_params.block.max_bytes;
        if max_bytes == -1 {
            max_bytes = ctypes::part_set::MAX_BLOCK_SIZE_BYTES;
        }
        let max_parts = (max_bytes - 1) / i64::from(BLOCK_PART_SIZE_BYTES) + 1;
        if i64::from(proposal.block_id.part_set_header.total) > max_parts {
            return Err("proposal block has too many parts".into());
        }
        if self.rs.proposal_block_parts.is_none() {
            self.rs.proposal_block_parts = Some(PartSet::from_header(
                &proposal.block_id.part_set_header,
                BLOCK_PART_SIZE_BYTES,
            ));
        }
        tracing::info!(
            height = proposal.height,
            round = proposal.round,
            "received proposal"
        );
        self.rs.proposal = Some(proposal);
        Ok(())
    }

    /// `addProposalBlockPart`: whether the part was new.
    fn add_proposal_block_part(
        &mut self,
        height: i64,
        _round: i32,
        part: &ctypes::Part,
        peer: &str,
    ) -> bool {
        if self.rs.height != height {
            return false;
        }
        let Some(parts) = self.rs.proposal_block_parts.as_mut() else {
            return false;
        };
        let added = match parts.add_part(part) {
            Ok(a) => a,
            Err(e) => {
                tracing::debug!(height, peer, err = %e, "failed to add proposal block part");
                return false;
            }
        };
        let mut max_bytes = self.state.consensus_params.block.max_bytes;
        if max_bytes == -1 {
            max_bytes = ctypes::part_set::MAX_BLOCK_SIZE_BYTES;
        }
        if parts.byte_size() > max_bytes {
            return added;
        }
        if added && parts.is_complete() {
            let Some(bz) = parts.get_bytes() else {
                return added;
            };
            match Block::decode(bz) {
                Ok(b) => {
                    tracing::info!(height, "received complete proposal block");
                    self.rs.proposal_block = Some(b);
                }
                Err(e) => {
                    tracing::error!(height, err = %e, "failed to decode completed proposal block");
                }
            }
        }
        added
    }

    /// `handleCompleteProposal`.
    fn handle_complete_proposal(&mut self, height: i64) {
        let round = self.rs.round;
        let polka = self
            .rs
            .votes
            .prevotes(round)
            .and_then(VoteSet::two_thirds_majority);
        if let Some(id) = &polka {
            if !id.is_zero()
                && self.rs.valid_round < round
                && block_matches(
                    self.rs.proposal_block.as_ref(),
                    self.rs.proposal_block_parts.as_ref(),
                    id,
                )
            {
                self.rs.valid_round = round;
                self.rs.valid_block.clone_from(&self.rs.proposal_block);
                self.rs
                    .valid_block_parts
                    .clone_from(&self.rs.proposal_block_parts);
            }
        }
        if self.rs.step <= RoundStep::Propose && self.is_proposal_complete() {
            self.enter_prevote(height, round);
            if polka.is_some() {
                self.enter_precommit(height, round);
            }
        } else if self.rs.step == RoundStep::Commit {
            self.try_finalize_commit(height);
        }
    }

    // ------------------------------------------------------------------
    // Votes

    /// `tryAddVote`.
    fn try_add_vote(&mut self, vote: &Vote, peer: &str) -> Result<bool, VoteError> {
        match self.add_vote(vote, peer) {
            Ok(a) => Ok(a),
            Err((added, VoteError::Conflicting { vote_a, vote_b })) => {
                let mine = self.priv_addr();
                if mine.as_deref() == Some(vote.validator_address.as_slice()) {
                    tracing::error!(
                        height = vote.height,
                        round = vote.round,
                        "found conflicting vote from ourselves; did you unsafe_reset a validator?"
                    );
                } else {
                    self.block_exec
                        .evpool()
                        .report_conflicting_votes((*vote_a).clone(), (*vote_b).clone());
                    self.events.push(Event::ConflictingVotes {
                        vote_a: vote_a.clone(),
                        vote_b: vote_b.clone(),
                    });
                }
                let _ = added;
                Err(VoteError::Conflicting { vote_a, vote_b })
            }
            Err((_, e)) => {
                tracing::debug!(peer, err = %e, "failed attempting to add vote");
                Err(e)
            }
        }
    }

    /// `addVote`.
    fn add_vote(&mut self, vote: &Vote, peer: &str) -> Result<bool, (bool, VoteError)> {
        // A precommit for the previous height (during timeout commit).
        if vote.height + 1 == self.rs.height && vote.vote_type == PRECOMMIT_TYPE {
            if self.rs.step != RoundStep::NewHeight {
                return Ok(false);
            }
            let Some(lc) = self.rs.last_commit.as_mut() else {
                return Ok(false);
            };
            let added = lc.add_vote(vote)?;
            if !added {
                return Ok(false);
            }
            self.events.push(Event::Vote(vote.clone()));
            let has_all = lc.has_all();
            if self.config.skip_timeout_commit && has_all {
                let h = self.rs.height;
                self.enter_new_round(h, 0);
            }
            return Ok(true);
        }
        if vote.height != self.rs.height {
            return Ok(false);
        }
        let ext_enabled = self
            .state
            .consensus_params
            .abci
            .vote_extensions_enabled(vote.height)
            .unwrap_or(false);
        if ext_enabled {
            let mine = self.priv_addr();
            if vote.vote_type == PRECOMMIT_TYPE
                && !vote.block_id.is_zero()
                && mine.as_deref() != Some(vote.validator_address.as_slice())
            {
                let Some((_, val)) = self.state.validators.get_by_index(vote.validator_index)
                else {
                    return Err((
                        false,
                        VoteError::InvalidValidatorIndex(format!(
                            "ValidatorIndex {} is out of bounds [0, {})",
                            vote.validator_index,
                            self.state.validators.size()
                        )),
                    ));
                };
                vote.verify_extension(&self.state.chain_id, &val.pub_key)
                    .map_err(|e| (false, VoteError::InvalidSignature(e.to_string())))?;
                self.block_exec
                    .verify_vote_extension(vote)
                    .map_err(|e| (false, VoteError::Other(e.to_string())))?;
            }
        } else if !vote.extension.is_empty() || !vote.extension_signature.is_empty() {
            return Err((
                false,
                VoteError::Other(format!(
                    "received vote with vote extension for height {} (extensions disabled) from peer ID {peer}",
                    vote.height
                )),
            ));
        }
        let height = self.rs.height;
        let added = self.rs.votes.add_vote(vote, peer)?;
        if !added {
            return Ok(false);
        }
        self.events.push(Event::Vote(vote.clone()));
        match vote.vote_type {
            PREVOTE_TYPE => self.on_prevote_added(height, vote.round),
            PRECOMMIT_TYPE => self.on_precommit_added(height, vote.round),
            _ => {}
        }
        Ok(true)
    }

    fn on_prevote_added(&mut self, height: i64, vote_round: i32) {
        let Some(prevotes) = self.rs.votes.prevotes(vote_round) else {
            return;
        };
        let maj = prevotes.two_thirds_majority();
        let two_thirds_any = prevotes.has_two_thirds_any();
        if let Some(block_id) = &maj {
            // Unlock if `locked_round < vote.round <= round` for another block.
            if self.rs.locked_block.is_some()
                && self.rs.locked_round < vote_round
                && vote_round <= self.rs.round
                && !block_matches(
                    self.rs.locked_block.as_ref(),
                    self.rs.locked_block_parts.as_ref(),
                    block_id,
                )
            {
                self.rs.locked_round = -1;
                self.rs.locked_block = None;
                self.rs.locked_block_parts = None;
            }
            // Update Valid*.
            if !block_id.hash.is_empty()
                && self.rs.valid_round < vote_round
                && vote_round == self.rs.round
            {
                let parts_match = parts_has_header(
                    self.rs.proposal_block_parts.as_ref(),
                    &block_id.part_set_header,
                );
                if block_hashes_to(self.rs.proposal_block.as_ref(), &block_id.hash) && parts_match {
                    self.rs.valid_round = vote_round;
                    self.rs.valid_block.clone_from(&self.rs.proposal_block);
                    self.rs
                        .valid_block_parts
                        .clone_from(&self.rs.proposal_block_parts);
                } else if self.rs.proposal_block.is_some() {
                    self.rs.proposal_block = None;
                }
                if !parts_match {
                    self.rs.proposal_block_parts = Some(PartSet::from_header(
                        &block_id.part_set_header,
                        BLOCK_PART_SIZE_BYTES,
                    ));
                }
                self.events.push(Event::ValidBlock {
                    height,
                    round: self.rs.round,
                    parts_header: block_id.part_set_header.clone(),
                    is_commit: self.rs.step == RoundStep::Commit,
                });
            }
        }
        if self.rs.round < vote_round && two_thirds_any {
            self.enter_new_round(height, vote_round);
        } else if self.rs.round == vote_round && RoundStep::Prevote <= self.rs.step {
            match &maj {
                Some(id) if self.is_proposal_complete() || id.hash.is_empty() => {
                    self.enter_precommit(height, vote_round);
                }
                _ if two_thirds_any => self.enter_prevote_wait(height, vote_round),
                _ => {}
            }
        } else if self
            .rs
            .proposal
            .as_ref()
            .is_some_and(|p| 0 <= p.pol_round && p.pol_round == vote_round)
            && self.is_proposal_complete()
        {
            let r = self.rs.round;
            self.enter_prevote(height, r);
        }
    }

    fn on_precommit_added(&mut self, height: i64, vote_round: i32) {
        let Some(precommits) = self.rs.votes.precommits(vote_round) else {
            return;
        };
        let maj = precommits.two_thirds_majority();
        let two_thirds_any = precommits.has_two_thirds_any();
        let has_all = precommits.has_all();
        if let Some(block_id) = maj {
            self.enter_new_round(height, vote_round);
            self.enter_precommit(height, vote_round);
            if block_id.hash.is_empty() {
                self.enter_precommit_wait(height, vote_round);
            } else {
                self.enter_commit(height, vote_round);
                if self.config.skip_timeout_commit && has_all {
                    let h = self.rs.height;
                    self.enter_new_round(h, 0);
                }
            }
        } else if self.rs.round <= vote_round && two_thirds_any {
            self.enter_new_round(height, vote_round);
            self.enter_precommit_wait(height, vote_round);
        }
    }

    /// `voteTime`: BFT time (strictly after the block being voted on).
    fn vote_time(&self) -> Time {
        let now = self.now();
        let min = if let Some(b) = &self.rs.locked_block {
            add_duration(b.header.time, MS)
        } else if let Some(b) = &self.rs.proposal_block {
            add_duration(b.header.time, MS)
        } else {
            now
        };
        if now > min { now } else { min }
    }

    /// `signVote` + `SignAndCheckVote`.
    fn sign_vote(
        &mut self,
        vote_type: i32,
        hash: Vec<u8>,
        header: PartSetHeader,
        block: Option<Block>,
    ) -> Result<Vote, String> {
        self.wal.flush_and_sync().map_err(|e| e.to_string())?;
        let addr = self.priv_addr().ok_or("pubkey is not set")?;
        let (idx, _) = self
            .rs
            .validators
            .get_by_address(&addr)
            .ok_or("not a validator")?;
        let mut vote = Vote {
            vote_type,
            height: self.rs.height,
            round: self.rs.round,
            block_id: BlockId {
                hash,
                part_set_header: header,
            },
            timestamp: self.vote_time(),
            validator_address: addr,
            validator_index: idx,
            ..Vote::default()
        };
        let ext_enabled = self
            .state
            .consensus_params
            .abci
            .vote_extensions_enabled(vote.height)
            .unwrap_or(false);
        if vote_type == PRECOMMIT_TYPE && !vote.block_id.is_zero() && ext_enabled {
            let mut b = block.ok_or("no block for vote extension")?;
            let state = self.state.clone();
            vote.extension = self
                .block_exec
                .extend_vote(&vote, &mut b, &state)
                .map_err(|e| e.to_string())?;
        }
        let mut v: pb::Vote = vote.to_proto();
        let chain_id = self.state.chain_id.clone();
        let pv = self.priv_validator.as_mut().ok_or("no priv validator")?;
        pv.sign_vote(&chain_id, &mut v).map_err(|e| e.to_string())?;
        vote.signature.clone_from(&v.signature);
        let is_precommit = vote_type == PRECOMMIT_TYPE;
        let is_nil = vote.block_id.is_zero();
        let has_ext_sig = !v.extension_signature.is_empty();
        if has_ext_sig && (!is_precommit || is_nil) {
            return Err("vote extension signature on a prevote or nil precommit".into());
        }
        vote.extension_signature = Vec::new();
        if ext_enabled && is_precommit {
            if !has_ext_sig && !is_nil {
                return Err("missing vote extension signature".into());
            }
            vote.extension_signature = v.extension_signature;
        }
        if let Some(ts) = v.timestamp {
            vote.timestamp = Time::from_unix(ts.seconds, ts.nanos);
        }
        Ok(vote)
    }

    /// `signAddVote`.
    fn sign_add_vote(
        &mut self,
        vote_type: i32,
        hash: Vec<u8>,
        header: PartSetHeader,
        block: Option<Block>,
    ) {
        if self.priv_validator.is_none() {
            return;
        }
        let Some(addr) = self.priv_addr() else {
            return;
        };
        if !self.rs.validators.has_address(&addr) {
            return;
        }
        match self.sign_vote(vote_type, hash, header, block) {
            Ok(vote) => {
                self.events
                    .push(Event::Broadcast(ConsensusMessage::Vote(vote.clone())));
                self.send_internal(ConsensusMessage::Vote(vote));
            }
            Err(e) => {
                if !self.replay_mode {
                    tracing::error!(height = self.rs.height, round = self.rs.round, err = %e, "failed signing vote");
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Gossip helpers

    /// Votes this node holds for its current height (all rounds) and the
    /// last commit (for re-gossip).
    #[must_use]
    pub fn known_votes(&self) -> Vec<Vote> {
        let mut out = Vec::new();
        for r in self.rs.votes.rounds() {
            if let Some(p) = self.rs.votes.prevotes(r) {
                out.extend(p.list());
            }
            if let Some(p) = self.rs.votes.precommits(r) {
                out.extend(p.list());
            }
        }
        if let Some(lc) = &self.rs.last_commit {
            out.extend(lc.list());
        }
        out
    }

    /// Encoded size of the current proposal block parts (diagnostics).
    #[must_use]
    pub fn proposal_parts_bytes(&self) -> usize {
        self.rs
            .proposal_block_parts
            .as_ref()
            .map_or(0, |p| usize::try_from(p.byte_size()).unwrap_or(0))
    }

    /// Debug summary of the commit at a height in the store.
    #[must_use]
    pub fn stored_commit_bytes(&self, height: i64) -> Option<Vec<u8>> {
        self.block_exec
            .block_store
            .load_seen_commit(height)
            .ok()
            .flatten()
            .map(|c| c.to_proto().encode_to_vec())
    }
}
