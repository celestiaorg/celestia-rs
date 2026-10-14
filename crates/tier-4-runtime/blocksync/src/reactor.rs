//! The block sync reactor (`blocksync/reactor.go`).
//!
//! Serving: `StatusRequest` / `BlockRequest` are answered from the
//! [`BlockStore`] (with the extended commit when vote extensions are
//! enabled at that height). Syncing: [`Reactor::run_sync`] is Go's
//! `poolRoutine` — a helper thread ticks the [`BlockPool`] every 2 ms,
//! sends its block requests, disconnects peers it reports and broadcasts a
//! `StatusRequest` every 10 s, while the calling thread verifies and
//! executes blocks in order and checks every second whether to switch to
//! consensus.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use celestia_core_proto::tendermint::blocksync as bcproto;
use ctypes::{Block, BlockId, ExtendedCommit, Time};
use nodestore::{BlockExecutor, BlockStore, State, StateStore};
use p2p::{ChannelDescriptor, Envelope, Peer, WeakSwitch};
use prost::Message as _;

use crate::msgs::{self, BLOCKSYNC_CHANNEL, MAX_MSG_SIZE, validate_block_sync_bytes, validate_msg};
use crate::pool::{BlockPool, BlockRequest, PeerError, REQUEST_INTERVAL};

/// `trySyncIntervalMS`.
pub const TRY_SYNC_INTERVAL: Duration = Duration::from_millis(10);
/// `statusUpdateIntervalSeconds`.
pub const STATUS_UPDATE_INTERVAL: Duration = Duration::from_secs(10);
/// `switchToConsensusIntervalSeconds`.
pub const SWITCH_TO_CONSENSUS_INTERVAL: Duration = Duration::from_secs(1);
/// `ReactorIncomingMessageQueueSize`.
pub const REACTOR_INCOMING_MESSAGE_QUEUE_SIZE: usize = 100;

/// What the reactor needs from the p2p layer (implemented for the
/// [`p2p::Switch`]; tests plug in an in-memory network).
pub trait Network: Send + Sync {
    /// `peer.TrySend` on the block sync channel.
    fn try_send(&self, peer: &str, msg: Vec<u8>) -> bool;
    /// `Switch.Broadcast` on the block sync channel.
    fn broadcast(&self, msg: Vec<u8>);
    /// `Switch.StopPeerForError`.
    fn stop_peer_for_error(&self, peer: &str, reason: &str);
}

impl Network for WeakSwitch {
    fn try_send(&self, peer: &str, msg: Vec<u8>) -> bool {
        let Some(sw) = self.upgrade() else {
            return false;
        };
        sw.peers()
            .into_iter()
            .find(|p| p.id().to_string() == peer)
            .is_some_and(|p| p.try_send(BLOCKSYNC_CHANNEL, msg))
    }

    fn broadcast(&self, msg: Vec<u8>) {
        // `Switch::broadcast` spawns on the async runtime; the sync threads
        // here are plain threads, so queue to every peer directly.
        if let Some(sw) = self.upgrade() {
            for p in sw.peers() {
                let _ = p.try_send(BLOCKSYNC_CHANNEL, msg.clone());
            }
        }
    }

    fn stop_peer_for_error(&self, peer: &str, reason: &str) {
        let Some(sw) = self.upgrade() else { return };
        if let Some(p) = sw.peers().into_iter().find(|p| p.id().to_string() == peer) {
            sw.stop_peer_for_error(&p, reason);
        }
    }
}

/// Reactor options (`ReactorOption`s and node config).
#[derive(Debug, Clone)]
pub struct ReactorOptions {
    /// `ReactorVerifyData`: ask the app (`ProcessProposal`) about every
    /// synced block (default true).
    pub verify_data: bool,
    /// Our validator address (`NewReactorWithAddr`): a node holding 1/3+
    /// of the power switches to consensus without waiting.
    pub local_addr: Option<Vec<u8>>,
    /// How often to check whether to switch to consensus.
    pub switch_to_consensus_interval: Duration,
}

impl Default for ReactorOptions {
    fn default() -> Self {
        Self {
            verify_data: true,
            local_addr: None,
            switch_to_consensus_interval: SWITCH_TO_CONSENSUS_INTERVAL,
        }
    }
}

/// Why [`Reactor::run_sync`] returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchToConsensus {
    /// Caught up (or we block the chain): consensus should take over.
    CaughtUp,
    /// The caller's stop condition fired (halt height, shutdown).
    Stopped,
}

/// The result of a block sync run.
#[derive(Debug)]
pub struct SyncOutcome {
    /// The state after the last synced block.
    pub state: State,
    /// Blocks synced in this run.
    pub blocks_synced: u64,
    /// Why the run ended.
    pub reason: SwitchToConsensus,
    /// Go's `skipWAL` argument of `SwitchToConsensus`.
    pub skip_wal: bool,
}

/// The block sync reactor.
pub struct Reactor {
    store: BlockStore,
    state_store: StateStore,
    pool: Arc<BlockPool>,
    requests_rx: Mutex<Option<Receiver<BlockRequest>>>,
    errors_rx: Mutex<Option<Receiver<PeerError>>>,
    net: Mutex<Option<Arc<dyn Network>>>,
    opts: ReactorOptions,
    syncing: AtomicBool,
    quit: AtomicBool,
}

impl std::fmt::Debug for Reactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BlockSyncReactor")
    }
}

fn now_time() -> Time {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    Time::from_unix(
        i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
        i32::try_from(d.subsec_nanos()).unwrap_or(0),
    )
}

impl Reactor {
    /// `NewReactorWithAddr`: the pool starts at the block store height + 1
    /// (`state.InitialHeight` for an empty store; `offline_state_sync_height`
    /// for stores bootstrapped offline).
    pub fn new(
        state: &State,
        store: BlockStore,
        state_store: StateStore,
        offline_state_sync_height: i64,
        opts: ReactorOptions,
    ) -> Result<Self, String> {
        let mut store_height = store.height();
        if store_height == 0 {
            store_height = offline_state_sync_height;
        }
        if state.last_block_height != store_height {
            return Err(format!(
                "state ({}) and store ({store_height}) height mismatch, stores were left in an inconsistent state",
                state.last_block_height
            ));
        }
        let mut start_height = store_height + 1;
        if start_height == 1 {
            start_height = state.initial_height;
        }
        let (rt, rr) = channel();
        let (et, er) = channel();
        Ok(Self {
            store,
            state_store,
            pool: Arc::new(BlockPool::new(start_height, rt, et, Instant::now())),
            requests_rx: Mutex::new(Some(rr)),
            errors_rx: Mutex::new(Some(er)),
            net: Mutex::new(None),
            opts,
            syncing: AtomicBool::new(false),
            quit: AtomicBool::new(false),
        })
    }

    /// Plugs in a network (instead of the switch from `set_switch`).
    pub fn set_network(&self, net: Arc<dyn Network>) {
        if let Ok(mut g) = self.net.lock() {
            *g = Some(net);
        }
    }

    fn net(&self) -> Option<Arc<dyn Network>> {
        self.net.lock().ok().and_then(|g| g.clone())
    }

    /// The block pool.
    #[must_use]
    pub fn pool(&self) -> &Arc<BlockPool> {
        &self.pool
    }

    /// Whether [`Self::run_sync`] is running.
    #[must_use]
    pub fn is_syncing(&self) -> bool {
        self.syncing.load(Ordering::SeqCst)
    }

    /// Stops a running sync (`OnStop`).
    pub fn stop(&self) {
        self.quit.store(true, Ordering::SeqCst);
        self.pool.stop();
    }

    fn status_response(&self) -> Vec<u8> {
        msgs::status_response(self.store.base(), self.store.height())
    }

    /// `AddPeer`: sends our status; the peer joins the pool once it answers
    /// with its own.
    pub fn add_peer_id(&self, peer: &str) {
        if let Some(net) = self.net() {
            net.try_send(peer, self.status_response());
        }
    }

    /// `respondToPeer`: the block (with its extended commit when vote
    /// extensions are enabled) or `NoBlockResponse`.
    pub fn respond_to_peer(&self, height: i64, peer: &str) -> bool {
        let Some(net) = self.net() else { return false };
        let Ok(Some(block)) = self.store.load_block(height) else {
            tracing::info!(src = peer, height, "Peer asking for a block we don't have");
            return net.try_send(peer, msgs::no_block_response(height));
        };
        let state = match self.state_store.load() {
            Ok(Some(s)) => s,
            Ok(None) => {
                tracing::error!("loading state: no state");
                return false;
            }
            Err(e) => {
                tracing::error!(err = %e, "loading state");
                return false;
            }
        };
        let mut ext_commit = None;
        if state
            .consensus_params
            .abci
            .vote_extensions_enabled(height)
            .unwrap_or(false)
        {
            if let Ok(Some(ec)) = self.store.load_block_extended_commit(height) {
                ext_commit = Some(ec.to_proto());
            } else {
                tracing::error!(height, "found block in store with no extended commit");
                return false;
            }
        }
        let bl = match block.to_proto() {
            Ok(b) => b,
            Err(e) => {
                tracing::error!(err = %e, "could not convert msg to protobuf");
                return false;
            }
        };
        net.try_send(
            peer,
            msgs::encode(bcproto::message::Sum::BlockResponse(
                bcproto::BlockResponse {
                    block: Some(bl),
                    ext_commit,
                },
            )),
        )
    }

    /// `Receive` for a raw message from `peer`. `Err` means the peer must
    /// be disconnected.
    pub fn receive_bytes(&self, peer: &str, bytes: &[u8]) -> Result<(), String> {
        use bcproto::message::Sum;
        validate_block_sync_bytes(bytes).map_err(|e| e.to_string())?;
        let msg = bcproto::Message::decode(bytes).map_err(|e| e.to_string())?;
        if let Err(e) = validate_msg(&msg) {
            tracing::error!(peer, err = %e, "Peer sent us invalid msg");
            return Err(e.to_string());
        }
        match msg.sum {
            Some(Sum::BlockRequest(m)) => {
                self.respond_to_peer(m.height, peer);
            }
            Some(Sum::BlockResponse(m)) => {
                let size = m.block.as_ref().map_or(0, prost::Message::encoded_len);
                let Some(pb) = m.block.as_ref() else {
                    tracing::error!(peer, "Peer sent us invalid block");
                    return Err("nil block".into());
                };
                let block = Block::from_proto(pb).map_err(|e| {
                    tracing::error!(peer, err = %e, "Peer sent us invalid block");
                    e.to_string()
                })?;
                let ext = match &m.ext_commit {
                    None => None,
                    Some(ec) => Some(ExtendedCommit::from_proto(ec).map_err(|e| {
                        tracing::error!(peer, err = %e, "failed to convert extended commit from proto");
                        e.to_string()
                    })?),
                };
                if let Err(e) = self.pool.add_block(peer, block, ext, size, Instant::now()) {
                    tracing::error!(peer, err = %e, "failed to add block");
                }
            }
            Some(Sum::StatusRequest(_)) => {
                if let Some(net) = self.net() {
                    net.try_send(peer, self.status_response());
                }
            }
            Some(Sum::StatusResponse(m)) => {
                self.pool
                    .set_peer_range(peer, m.base, m.height, Instant::now());
            }
            Some(Sum::NoBlockResponse(m)) => {
                tracing::trace!(
                    peer,
                    height = m.height,
                    "Peer does not have requested block"
                );
                self.pool.redo_request_from(m.height, peer);
            }
            None => {}
        }
        Ok(())
    }

    /// `localNodeBlocksTheChain`.
    fn local_node_blocks_the_chain(&self, state: &State) -> bool {
        let Some(addr) = &self.opts.local_addr else {
            return false;
        };
        let Some((_, val)) = state.validators.get_by_address(addr) else {
            return false;
        };
        val.voting_power >= state.validators.total_voting_power() / 3
    }

    /// `poolRoutine`: block sync from `state` until caught up (or `stop`
    /// returns true). `exec` executes the synced blocks; `state_synced` is
    /// set when the node just state-synced (`SwitchToBlockSync`).
    ///
    /// With `switch_to_consensus == false` the node never hands over and
    /// keeps following the chain by block sync (a node without a consensus
    /// reactor); Go has no such mode (it stops when no consensus reactor
    /// is registered).
    pub fn run_sync(
        &self,
        state: State,
        exec: &mut BlockExecutor,
        state_synced: bool,
        switch_to_consensus: bool,
        stop: &mut dyn FnMut(&State) -> bool,
    ) -> Result<SyncOutcome, String> {
        let requests = self
            .requests_rx
            .lock()
            .ok()
            .and_then(|mut g| g.take())
            .ok_or("block sync already ran")?;
        let errors = self
            .errors_rx
            .lock()
            .ok()
            .and_then(|mut g| g.take())
            .ok_or("block sync already ran")?;
        let start_height = if state.last_block_height == 0 {
            state.initial_height
        } else {
            state.last_block_height + 1
        };
        self.pool.set_height(start_height);
        self.pool.start(Instant::now());
        self.syncing.store(true, Ordering::SeqCst);
        let ticker_done = Arc::new(AtomicBool::new(false));
        let ticker = {
            let pool = Arc::clone(&self.pool);
            let net = self.net();
            let done = Arc::clone(&ticker_done);
            std::thread::Builder::new()
                .name("blocksync-pool".into())
                .spawn(move || pool_ticker(&pool, net.as_deref(), &requests, &errors, &done))
                .map_err(|e| e.to_string())?
        };
        let res = self.sync_loop(state, exec, state_synced, switch_to_consensus, stop);
        self.pool.stop();
        ticker_done.store(true, Ordering::SeqCst);
        let _ = ticker.join();
        self.syncing.store(false, Ordering::SeqCst);
        res
    }

    fn sync_loop(
        &self,
        mut state: State,
        exec: &mut BlockExecutor,
        state_synced: bool,
        switch_to_consensus: bool,
        stop: &mut dyn FnMut(&State) -> bool,
    ) -> Result<SyncOutcome, String> {
        let chain_id = state.chain_id.clone();
        let mut blocks_synced: u64 = 0;
        let mut last_hundred = Instant::now();
        let mut last_rate = 0.0f64;
        let initial_commit_has_extensions = state.last_block_height > 0
            && matches!(
                self.store
                    .load_block_extended_commit(state.last_block_height),
                Ok(Some(_))
            );
        let mut next_switch_check = Instant::now() + self.opts.switch_to_consensus_interval;
        let outcome = |state: State, blocks_synced: u64, reason| SyncOutcome {
            state,
            blocks_synced,
            reason,
            skip_wal: blocks_synced > 0 || state_synced,
        };
        loop {
            if self.quit.load(Ordering::SeqCst) || stop(&state) {
                return Ok(outcome(state, blocks_synced, SwitchToConsensus::Stopped));
            }
            let now = Instant::now();
            if switch_to_consensus && now >= next_switch_check {
                next_switch_check = now + self.opts.switch_to_consensus_interval;
                let (height, ..) = self.pool.get_status();
                let missing_extension = !(state.last_block_height == 0
                    || !state
                        .consensus_params
                        .abci
                        .vote_extensions_enabled(state.last_block_height)
                        .unwrap_or(false)
                    || blocks_synced > 0
                    || initial_commit_has_extensions);
                if missing_extension {
                    tracing::info!(
                        height,
                        last_block_height = state.last_block_height,
                        initial_height = state.initial_height,
                        max_peer_height = self.pool.max_peer_height(),
                        "no extended commit yet"
                    );
                } else if self.pool.is_caught_up(now) || self.local_node_blocks_the_chain(&state) {
                    tracing::info!(height, "Time to switch to consensus reactor!");
                    return Ok(outcome(state, blocks_synced, SwitchToConsensus::CaughtUp));
                }
            }
            if self.try_sync_one(&mut state, exec, &chain_id, blocks_synced)? {
                blocks_synced += 1;
                if blocks_synced.is_multiple_of(100) {
                    let secs = last_hundred.elapsed().as_secs_f64().max(1e-9);
                    last_rate = 0.9 * last_rate + 0.1 * (100.0 / secs);
                    tracing::info!(
                        height = self.pool.height(),
                        max_peer_height = self.pool.max_peer_height(),
                        blocks_per_s = last_rate,
                        "Block Sync Rate"
                    );
                    last_hundred = Instant::now();
                }
            } else {
                std::thread::sleep(TRY_SYNC_INTERVAL);
            }
        }
    }

    /// One iteration of the `didProcessCh` branch: verify, save and execute
    /// the block at the pool height if it and its successor are present.
    /// Returns whether a block was applied.
    pub fn try_sync_one(
        &self,
        state: &mut State,
        exec: &mut BlockExecutor,
        chain_id: &str,
        blocks_synced: u64,
    ) -> Result<bool, String> {
        if !self.pool.has_two_blocks() {
            return Ok(false);
        }
        let (first, second, ext_commit) = self.pool.peek_two_blocks();
        let (Some(mut first), Some(second)) = (first, second) else {
            return Ok(false);
        };
        if state.last_block_height > 0 && state.last_block_height + 1 != first.header.height {
            return Err(format!(
                "peeked first block has unexpected height; expected {}, got {}",
                state.last_block_height + 1,
                first.header.height
            ));
        }
        if first.header.height + 1 != second.header.height {
            return Err(format!(
                "heights of first and second block are not consecutive; expected {}, got {}",
                state.last_block_height, first.header.height
            ));
        }
        let first_parts = first
            .make_part_set(ctypes::BLOCK_PART_SIZE_BYTES)
            .map_err(|e| {
                format!(
                    "failed to make part set at height {}: {e}",
                    first.header.height
                )
            })?;
        let first_id = BlockId {
            hash: first.hash().unwrap_or_default(),
            part_set_header: first_parts.header(),
        };
        let second_commit = second.last_commit.clone().unwrap_or_default();
        let height = first.header.height;
        let mut err: Option<String> = ctypes::verify_commit(
            chain_id,
            &state.validators,
            &first_id,
            height,
            &second_commit,
        )
        .err()
        .map(|e| e.to_string());
        if err.is_none() {
            let r = if blocks_synced == 0 {
                exec.validate_block(state, &mut first, now_time())
            } else {
                exec.validate_block_skip_last_commit(state, &mut first, now_time())
            };
            err = r.err().map(|e| e.to_string());
        }
        if err.is_none() && self.opts.verify_data {
            let (valid, perr) = match exec.process_proposal(&mut first, state) {
                Ok(v) => (v, None),
                Err(e) => (false, Some(e.to_string())),
            };
            if !valid {
                err = Some(format!(
                    "application has rejected syncing block ({}) at height {height}, {}",
                    hex::encode_upper(&first_id.hash),
                    perr.unwrap_or_else(|| "%!w(<nil>)".into())
                ));
            }
        }
        let present_ext = ext_commit.is_some();
        let ext_enabled = state
            .consensus_params
            .abci
            .vote_extensions_enabled(height)
            .map_err(|e| e.to_string())?;
        if present_ext != ext_enabled {
            err = Some(format!(
                "non-nil extended commit must be received iff vote extensions are enabled for its height (height {height}, non-nil extended commit {present_ext}, extensions enabled {ext_enabled})"
            ));
        }
        if err.is_none()
            && ext_enabled
            && let Some(ec) = &ext_commit
        {
            err = ec.ensure_extensions(true).err().map(|e| e.to_string());
            if err.is_none() {
                err = ctypes::verify_commit(
                    chain_id,
                    &state.validators,
                    &first_id,
                    height,
                    &ec.to_commit(),
                )
                .err()
                .map(|e| e.to_string());
            }
        }
        if let Some(e) = err {
            tracing::error!(err = %e, height, "Error in validation");
            let now = Instant::now();
            let reason = format!("reactor validation error: {e}");
            let peer1 = self
                .pool
                .remove_peer_and_redo_all_peer_requests(height, now);
            let net = self.net();
            if let Some(net) = &net
                && !peer1.is_empty()
            {
                net.stop_peer_for_error(&peer1, &reason);
            }
            let peer2 = self
                .pool
                .remove_peer_and_redo_all_peer_requests(height + 1, now);
            if let Some(net) = &net
                && !peer2.is_empty()
                && peer2 != peer1
            {
                net.stop_peer_for_error(&peer2, &reason);
            }
            return Ok(false);
        }
        self.pool.pop_request();
        let saved = if ext_enabled {
            match &ext_commit {
                Some(ec) => {
                    self.store
                        .save_block_with_extended_commit(&mut first, &first_parts, ec)
                }
                None => Err(nodestore::Error::new("missing extended commit")),
            }
        } else {
            self.store
                .save_block(&mut first, &first_parts, &second_commit)
        };
        saved.map_err(|e| format!("failed to save block {height}: {e}"))?;
        *state = exec
            .apply_verified_block(state, first_id.clone(), &mut first)
            .map_err(|e| {
                format!(
                    "Failed to process committed block ({height}:{}): {e}",
                    hex::encode_upper(&first_id.hash)
                )
            })?;
        Ok(true)
    }

    /// `BroadcastStatusRequest`.
    pub fn broadcast_status_request(&self) {
        if let Some(net) = self.net() {
            net.broadcast(msgs::status_request());
        }
    }
}

/// The request / error / status goroutine plus the pool's routines.
fn pool_ticker(
    pool: &BlockPool,
    net: Option<&dyn Network>,
    requests: &Receiver<BlockRequest>,
    errors: &Receiver<PeerError>,
    done: &AtomicBool,
) {
    let mut next_status = Instant::now() + STATUS_UPDATE_INTERVAL;
    while !done.load(Ordering::SeqCst) {
        let now = Instant::now();
        pool.tick(now);
        for r in requests.try_iter() {
            let queued = net.is_some_and(|n| n.try_send(&r.peer_id, msgs::block_request(r.height)));
            if !queued {
                tracing::debug!(peer = %r.peer_id, height = r.height, "Send queue is full, drop block request");
            }
        }
        for e in errors.try_iter() {
            if let Some(n) = net {
                n.stop_peer_for_error(&e.peer_id, &e.to_string());
            }
        }
        if now >= next_status {
            next_status = now + STATUS_UPDATE_INTERVAL;
            if let Some(n) = net {
                n.broadcast(msgs::status_request());
            }
        }
        std::thread::sleep(REQUEST_INTERVAL);
    }
}

impl p2p::Reactor for Reactor {
    fn name(&self) -> String {
        "BLOCKSYNC".into()
    }

    fn channels(&self) -> Vec<ChannelDescriptor> {
        vec![ChannelDescriptor {
            send_queue_capacity: 1000,
            recv_buffer_capacity: 50 * 4096,
            recv_message_capacity: MAX_MSG_SIZE,
            ..ChannelDescriptor::new(BLOCKSYNC_CHANNEL, 30)
        }]
    }

    fn set_switch(&self, switch: WeakSwitch) {
        self.set_network(Arc::new(switch));
    }

    fn on_stop(&self) {
        self.stop();
    }

    fn add_peer(&self, peer: &Arc<Peer>) {
        self.add_peer_id(&peer.id().to_string());
    }

    fn remove_peer(&self, peer: &Arc<Peer>, _reason: &str) {
        self.pool.remove_peer(&peer.id().to_string());
    }

    fn receive(&self, envelope: Envelope) -> Result<(), String> {
        self.receive_bytes(&envelope.src.id().to_string(), &envelope.message)
    }

    fn queue_size(&self) -> usize {
        REACTOR_INCOMING_MESSAGE_QUEUE_SIZE
    }
}
