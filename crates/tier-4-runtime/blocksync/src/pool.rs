//! `BlockPool` (`blocksync/pool.go`): keeps track of the block sync peers,
//! block requests and block responses.
//!
//! Peers self-report their `[base, height]`. Starting from the pool height
//! we create one requester per height (at most 40 at a time) and each
//! requester asks one peer that has the height. Go runs the requester
//! loops and peer timers as goroutines; here they are state machines
//! advanced by [`BlockPool::tick`] with an explicit `Instant` (the node
//! ticks every `requestIntervalMS` = 2 ms). Outgoing requests and peer
//! errors go to the channels given at construction, like Go's
//! `requestsCh` / `errorsCh`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use ctypes::{Block, ExtendedCommit};

use crate::block_stats::BlockStats;
use crate::flowrate::Monitor;

/// A peer identifier (the p2p node ID as a string).
pub type PeerId = String;

/// `requestIntervalMS`.
pub const REQUEST_INTERVAL: Duration = Duration::from_millis(2);
/// `minBlockSizeBytes`.
pub const MIN_BLOCK_SIZE_BYTES: i64 = 1024;
/// `maxBlockSizeBytes`.
pub const MAX_BLOCK_SIZE_BYTES: i64 = 20 * 1024 * 1024;
/// `minRetrySeconds`.
pub const MIN_RETRY_SECONDS: i64 = 5;
/// `maxRetrySeconds`.
pub const MAX_RETRY_SECONDS: i64 = 60;
/// `minReqLimit`.
pub const MIN_REQ_LIMIT: i64 = 2;
/// `maxReqLimit`.
pub const MAX_REQ_LIMIT: i64 = 10;
/// `blockSizeBufferCapacity`.
pub const BLOCK_SIZE_BUFFER_CAPACITY: usize = 70;
/// `minRecvRate` (bytes/s).
pub const MIN_RECV_RATE: i64 = 128 * 1024;
/// `peerConnWait`.
pub const PEER_CONN_WAIT: Duration = Duration::from_secs(3);
/// `defaultMaxRequesters`.
pub const DEFAULT_MAX_REQUESTERS: usize = 40;
/// `peerTimeout` (a variable in Go so tests can shorten it).
pub const PEER_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a banned peer stays banned.
pub const BAN_DURATION: Duration = Duration::from_secs(60);

/// `BlockRequest`: ask `peer_id` for the block at `height`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockRequest {
    /// Height.
    pub height: i64,
    /// Peer.
    pub peer_id: PeerId,
}

/// `peerError`: the peer should be disconnected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerError {
    /// Reason.
    pub err: String,
    /// Peer.
    pub peer_id: PeerId,
}

impl std::fmt::Display for PeerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "error with peer {}: {}", self.peer_id, self.err)
    }
}

/// `interpolate`: linear interpolation clamped to `[min_out, max_out]`.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::similar_names
)]
pub fn interpolate(
    is_inverse: bool,
    value: i64,
    min_value: i64,
    max_value: i64,
    min_out: i64,
    max_out: i64,
) -> i64 {
    let normalized = (value - min_value) as f64 / (max_value - min_value) as f64;
    let between = ((max_out - min_out) as f64 * normalized) as i64;
    let out = if is_inverse {
        max_out - between
    } else {
        min_out + between
    };
    out.min(max_out).max(min_out)
}

#[derive(Debug)]
struct BpPeer {
    did_timeout: bool,
    cur_rate: i64,
    num_pending: i32,
    height: i64,
    base: i64,
    id: PeerId,
    recv_monitor: Option<Monitor>,
    /// The pending `peerTimeout` timer.
    timeout_at: Option<Instant>,
}

impl BpPeer {
    fn new(id: PeerId, base: i64, height: i64) -> Self {
        Self {
            did_timeout: false,
            cur_rate: 0,
            num_pending: 0,
            height,
            base,
            id,
            recv_monitor: None,
            timeout_at: None,
        }
    }

    fn reset_monitor(&mut self, epoch: Instant, now: Instant) {
        let mut m = Monitor::new(Duration::from_secs(1), Duration::from_secs(40), epoch, now);
        #[allow(clippy::cast_precision_loss)]
        m.set_rema(MIN_RECV_RATE as f64 * std::f64::consts::E);
        self.recv_monitor = Some(m);
    }

    fn incr_pending(&mut self, epoch: Instant, now: Instant) {
        if self.num_pending == 0 {
            self.reset_monitor(epoch, now);
            self.timeout_at = Some(now + PEER_TIMEOUT);
        }
        self.num_pending += 1;
    }

    fn decr_pending(&mut self, recv_size: usize, now: Instant) {
        self.num_pending -= 1;
        if self.num_pending == 0 {
            self.timeout_at = None;
        } else {
            if let Some(m) = &mut self.recv_monitor {
                m.update(recv_size, now);
            }
            self.timeout_at = Some(now + PEER_TIMEOUT);
        }
    }
}

/// Where a requester's `requestRoutine` is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    /// `pickPeerAndSendRequest(ignore)`: waiting for an available peer.
    Picking { ignore: Option<PeerId> },
    /// Request sent; the retry timer is pending (None once it fired while
    /// a block was held).
    Waiting { retry_at: Option<Instant> },
}

#[derive(Debug)]
struct Requester {
    /// The peer currently being requested from.
    peer_id: Option<PeerId>,
    requested_from: HashSet<PeerId>,
    got_block_from: Option<PeerId>,
    block: Option<Block>,
    ext_commit: Option<ExtendedCommit>,
    /// `gotBlock` in `requestRoutine`.
    got_block: bool,
    /// `gotBlockCh` (capacity 1).
    got_block_signal: bool,
    /// `redoCh` (capacity 1).
    redo: Option<PeerId>,
    phase: Phase,
}

impl Requester {
    fn new() -> Self {
        Self {
            peer_id: None,
            requested_from: HashSet::new(),
            got_block_from: None,
            block: None,
            ext_commit: None,
            got_block: false,
            got_block_signal: false,
            redo: None,
            phase: Phase::Picking { ignore: None },
        }
    }

    fn did_request_from(&self, peer: &str) -> bool {
        self.requested_from.contains(peer)
    }

    /// `redo`: non-blocking, dropped if one is already queued.
    fn redo(&mut self, peer: &str) {
        if self.redo.is_none() {
            self.redo = Some(peer.to_owned());
        }
    }

    /// `tryRemoveBlock`.
    fn try_remove_block(&mut self, peer: &str, num_pending: &mut i32) -> bool {
        if self.got_block_from.as_deref() == Some(peer) {
            self.block = None;
            self.ext_commit = None;
            self.got_block_from = None;
            *num_pending += 1;
            return true;
        }
        false
    }

    /// `resetAll`.
    fn reset_all(&mut self, num_pending: &mut i32) -> Option<PeerId> {
        let cur = self.peer_id.take();
        if let Some(p) = &cur {
            self.try_remove_block(p, num_pending);
        }
        cur
    }

    /// `reset`.
    fn reset(&mut self, peer: &str, num_pending: &mut i32) -> bool {
        let removed = self.try_remove_block(peer, num_pending);
        if self.peer_id.as_deref() == Some(peer) {
            self.peer_id = None;
        }
        removed
    }
}

#[derive(Debug)]
struct Inner {
    running: bool,
    start_time: Instant,
    start_height: i64,
    requesters: BTreeMap<i64, Requester>,
    height: i64,
    peers: HashMap<PeerId, BpPeer>,
    banned_peers: HashMap<PeerId, Instant>,
    /// Peer IDs sorted by `cur_rate`, highest first.
    sorted_peers: Vec<PeerId>,
    max_peer_height: i64,
    req_limit: i64,
    retry_timeout: Duration,
    last_received_blocks: BlockStats,
    num_pending: i32,
    epoch: Instant,
}

/// The block pool.
#[derive(Debug)]
pub struct BlockPool {
    inner: Mutex<Inner>,
    requests: Sender<BlockRequest>,
    errors: Sender<PeerError>,
}

impl Inner {
    /// `recalculateParams`.
    fn recalculate_params(&mut self) {
        let block_size = i64::try_from(self.last_received_blocks.max()).unwrap_or(i64::MAX);
        if self.last_received_blocks.size() == 0 {
            self.req_limit = MAX_REQ_LIMIT / 2;
            self.retry_timeout = Duration::from_secs(MAX_RETRY_SECONDS.unsigned_abs() / 2);
        } else {
            self.req_limit = interpolate(
                true,
                block_size,
                MIN_BLOCK_SIZE_BYTES,
                MAX_BLOCK_SIZE_BYTES,
                MIN_REQ_LIMIT,
                MAX_REQ_LIMIT,
            );
            self.retry_timeout = Duration::from_secs(
                u64::try_from(interpolate(
                    false,
                    block_size,
                    MIN_BLOCK_SIZE_BYTES,
                    MAX_BLOCK_SIZE_BYTES,
                    MIN_RETRY_SECONDS,
                    MAX_RETRY_SECONDS,
                ))
                .unwrap_or(0),
            );
        }
    }

    fn is_peer_banned(&self, peer: &str, now: Instant) -> bool {
        self.banned_peers
            .get(peer)
            .is_some_and(|t| now.saturating_duration_since(*t) < BAN_DURATION)
    }

    fn ban_peer(&mut self, peer: &str, now: Instant) {
        tracing::debug!(peer, "Banning peer");
        self.banned_peers.insert(peer.to_owned(), now);
    }

    /// `removePeer`: redoes every requester that asked the peer.
    fn remove_peer(&mut self, peer: &str) {
        for r in self.requesters.values_mut() {
            if r.did_request_from(peer) {
                r.redo(peer);
            }
        }
        if let Some(p) = self.peers.remove(peer) {
            self.sorted_peers.retain(|x| x != peer);
            if p.height == self.max_peer_height {
                self.update_max_peer_height();
            }
        }
    }

    /// `updateMaxPeerHeight`.
    fn update_max_peer_height(&mut self) {
        let mut max = 0;
        for p in self.peers.values() {
            if self.height > 0 && p.base > self.height {
                continue;
            }
            if p.height > max {
                max = p.height;
            }
        }
        self.max_peer_height = max;
    }

    fn any_peer_has_blocks(&self) -> bool {
        self.peers.values().any(|p| p.height > 0)
    }

    /// `sortPeers` (stable, like Go's `sort.Slice` on equal rates keeps
    /// no particular order; stable is one valid choice).
    fn sort_peers(&mut self) {
        let rate = |id: &PeerId| self.peers.get(id).map_or(0, |p| p.cur_rate);
        let mut v = std::mem::take(&mut self.sorted_peers);
        v.sort_by_key(|id| std::cmp::Reverse(rate(id)));
        self.sorted_peers = v;
    }

    /// `pickIncrAvailablePeer`.
    fn pick_incr_available_peer(
        &mut self,
        height: i64,
        ignore: Option<&str>,
        prev: Option<&str>,
        now: Instant,
    ) -> Option<PeerId> {
        let max_pending = self.req_limit;
        let mut fallback: Option<PeerId> = None;
        let mut timed_out = Vec::new();
        let mut chosen = None;
        for id in &self.sorted_peers {
            if Some(id.as_str()) == ignore {
                continue;
            }
            let Some(p) = self.peers.get(id) else {
                continue;
            };
            if p.did_timeout {
                timed_out.push(id.clone());
                continue;
            }
            if i64::from(p.num_pending) >= max_pending {
                continue;
            }
            if height < p.base || height > p.height {
                continue;
            }
            if Some(id.as_str()) == prev {
                if fallback.is_none() {
                    fallback = Some(id.clone());
                }
                continue;
            }
            chosen = Some(id.clone());
            break;
        }
        for id in timed_out {
            self.remove_peer(&id);
        }
        let pick = chosen.or(fallback)?;
        let epoch = self.epoch;
        self.peers.get_mut(&pick)?.incr_pending(epoch, now);
        Some(pick)
    }

    /// `removeTimedoutPeers`; returns the peers to report.
    fn remove_timedout_peers(&mut self, now: Instant) -> Vec<PeerId> {
        let mut timed_out = Vec::new();
        let ids: Vec<PeerId> = self.peers.keys().cloned().collect();
        for id in ids {
            let Some(p) = self.peers.get_mut(&id) else {
                continue;
            };
            if !p.did_timeout && p.num_pending > 0 {
                let cur = p.recv_monitor.as_mut().map_or(0, |m| m.cur_rate(now));
                if cur != 0 && cur < MIN_RECV_RATE {
                    tracing::error!(
                        peer = %id,
                        reason = "peer is not sending us data fast enough",
                        cur_rate = format!("{} KB/s", cur / 1024),
                        min_rate = format!("{} KB/s", MIN_RECV_RATE / 1024),
                        "SendTimeout"
                    );
                    p.did_timeout = true;
                    timed_out.push(id.clone());
                }
                p.cur_rate = cur;
            }
            if p.did_timeout {
                self.remove_peer(&id);
            }
        }
        let banned: Vec<PeerId> = self.banned_peers.keys().cloned().collect();
        for id in banned {
            if !self.is_peer_banned(&id, now) {
                self.banned_peers.remove(&id);
            }
        }
        self.sort_peers();
        timed_out
    }
}

impl BlockPool {
    /// `NewBlockPool(start, requestsCh, errorsCh)`.
    #[must_use]
    pub fn new(
        start: i64,
        requests: Sender<BlockRequest>,
        errors: Sender<PeerError>,
        now: Instant,
    ) -> Self {
        let mut inner = Inner {
            running: false,
            start_time: now,
            start_height: start,
            requesters: BTreeMap::new(),
            height: start,
            peers: HashMap::new(),
            banned_peers: HashMap::new(),
            sorted_peers: Vec::new(),
            max_peer_height: 0,
            req_limit: 0,
            retry_timeout: Duration::ZERO,
            last_received_blocks: BlockStats::new(BLOCK_SIZE_BUFFER_CAPACITY),
            num_pending: 0,
            epoch: now,
        };
        inner.recalculate_params();
        Self {
            inner: Mutex::new(inner),
            requests,
            errors,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `OnStart`: records the start time; requesters are created by
    /// [`Self::tick`] from `PEER_CONN_WAIT` after it.
    pub fn start(&self, now: Instant) {
        let mut g = self.lock();
        g.running = true;
        g.start_time = now;
    }

    /// `Stop`.
    pub fn stop(&self) {
        self.lock().running = false;
    }

    /// `IsRunning`.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.lock().running
    }

    fn send_request(&self, height: i64, peer_id: PeerId) {
        if !self.is_running() {
            return;
        }
        let _ = self.requests.send(BlockRequest { height, peer_id });
    }

    fn send_error(&self, err: &str, peer_id: PeerId) {
        if !self.is_running() {
            return;
        }
        let _ = self.errors.send(PeerError {
            err: err.to_owned(),
            peer_id,
        });
    }

    /// One iteration of `makeRequestersRoutine`, the requesters'
    /// `requestRoutine`s and the peer timers.
    pub fn tick(&self, now: Instant) {
        let mut g = self.lock();
        if !g.running {
            return;
        }
        let mut requests = Vec::new();
        let mut errors: Vec<(String, PeerId)> = Vec::new();

        // peer timers (`onTimeout`)
        for p in g.peers.values_mut() {
            if p.timeout_at.is_some_and(|t| now >= t) {
                p.timeout_at = None;
                p.did_timeout = true;
                tracing::error!(peer = %p.id, reason = "peer did not send us anything", timeout = ?PEER_TIMEOUT, "SendTimeout");
                errors.push(("peer did not send us anything".into(), p.id.clone()));
            }
        }

        // makeRequestersRoutine
        if now.saturating_duration_since(g.start_time) >= PEER_CONN_WAIT {
            let max_created = g.requesters.len() >= DEFAULT_MAX_REQUESTERS;
            let next_height = g.height + i64::try_from(g.requesters.len()).unwrap_or(0);
            if max_created {
                for id in g.remove_timedout_peers(now) {
                    errors.push(("peer is not sending us data fast enough".into(), id));
                }
            } else if next_height <= g.max_peer_height {
                g.requesters.insert(next_height, Requester::new());
                g.num_pending += 1;
            }
        }

        // requestRoutine of every requester
        let heights: Vec<i64> = g.requesters.keys().copied().collect();
        for h in heights {
            Self::step_requester(&mut g, h, now, &mut requests);
        }
        drop(g);
        for (h, p) in requests {
            self.send_request(h, p);
        }
        for (e, p) in errors {
            self.send_error(&e, p);
        }
    }

    fn step_requester(g: &mut Inner, h: i64, now: Instant, out: &mut Vec<(i64, PeerId)>) {
        loop {
            let Some(r) = g.requesters.get_mut(&h) else {
                return;
            };
            match r.phase.clone() {
                Phase::Picking { ignore } => {
                    // pickPeerAndSendRequest
                    if r.peer_id.is_some() {
                        let rt = g.retry_timeout;
                        if let Some(r) = g.requesters.get_mut(&h) {
                            r.phase = Phase::Waiting {
                                retry_at: Some(now + rt),
                            };
                        }
                        continue;
                    }
                    let prev = g
                        .requesters
                        .get(&(h - 1))
                        .and_then(|p| p.got_block_from.clone());
                    let Some(peer) =
                        g.pick_incr_available_peer(h, ignore.as_deref(), prev.as_deref(), now)
                    else {
                        // "No peers currently available; will retry shortly"
                        return;
                    };
                    let rt = g.retry_timeout;
                    let Some(r) = g.requesters.get_mut(&h) else {
                        return;
                    };
                    r.peer_id = Some(peer.clone());
                    r.requested_from.insert(peer.clone());
                    r.phase = Phase::Waiting {
                        retry_at: Some(now + rt),
                    };
                    out.push((h, peer));
                    return;
                }
                Phase::Waiting { retry_at } => {
                    if let Some(p) = r.redo.take() {
                        let mut np = g.num_pending;
                        let Some(r) = g.requesters.get_mut(&h) else {
                            return;
                        };
                        if r.reset(&p, &mut np) {
                            r.got_block = false;
                        }
                        let empty = r.peer_id.is_none();
                        if empty {
                            r.phase = Phase::Picking { ignore: Some(p) };
                        }
                        g.num_pending = np;
                        continue;
                    }
                    if r.got_block_signal {
                        r.got_block_signal = false;
                        r.got_block = true;
                        continue;
                    }
                    if retry_at.is_some_and(|t| now >= t) {
                        if r.got_block {
                            r.phase = Phase::Waiting { retry_at: None };
                            return;
                        }
                        let mut np = g.num_pending;
                        let Some(r) = g.requesters.get_mut(&h) else {
                            return;
                        };
                        let ignore = r.reset_all(&mut np);
                        r.phase = Phase::Picking { ignore };
                        g.num_pending = np;
                        continue;
                    }
                    return;
                }
            }
        }
    }

    /// `GetStatus`: height, pending requests and requesters.
    #[must_use]
    pub fn get_status(&self) -> (i64, i32, usize) {
        let g = self.lock();
        (g.height, g.num_pending, g.requesters.len())
    }

    /// `IsCaughtUp`.
    #[must_use]
    pub fn is_caught_up(&self, now: Instant) -> bool {
        let g = self.lock();
        if g.peers.is_empty() {
            tracing::trace!("Blockpool has no peers");
            return false;
        }
        let received_block_or_timed_out =
            g.height > 0 || now.saturating_duration_since(g.start_time) > Duration::from_secs(5);
        let longest = if g.max_peer_height == 0 {
            !g.any_peer_has_blocks()
        } else {
            g.height >= g.max_peer_height - 1
        };
        received_block_or_timed_out && longest
    }

    /// `PeekTwoBlocks`: the blocks at the pool height and the next one, and
    /// the first block's extended commit.
    #[must_use]
    pub fn peek_two_blocks(&self) -> (Option<Block>, Option<Block>, Option<ExtendedCommit>) {
        let g = self.lock();
        let h = g.height;
        let first = g.requesters.get(&h);
        (
            first.and_then(|r| r.block.clone()),
            g.requesters.get(&(h + 1)).and_then(|r| r.block.clone()),
            first.and_then(|r| r.ext_commit.clone()),
        )
    }

    /// Whether both blocks `PeekTwoBlocks` needs are present (cheap check
    /// before cloning them).
    #[must_use]
    pub fn has_two_blocks(&self) -> bool {
        let g = self.lock();
        let h = g.height;
        g.requesters.get(&h).is_some_and(|r| r.block.is_some())
            && g.requesters
                .get(&(h + 1))
                .is_some_and(|r| r.block.is_some())
    }

    /// `SetHeight`.
    pub fn set_height(&self, height: i64) {
        self.lock().height = height;
    }

    /// `PopRequest`: drops the requester at the pool height and advances.
    pub fn pop_request(&self) {
        let mut g = self.lock();
        let h = g.height;
        if g.requesters.remove(&h).is_none() {
            tracing::error!(height = h, "Expected requester to pop, got nothing");
            return;
        }
        g.height += 1;
        g.update_max_peer_height();
    }

    /// `RemovePeerAndRedoAllPeerRequests`: removes and bans the peer that
    /// sent the block at `height`; returns its ID.
    pub fn remove_peer_and_redo_all_peer_requests(&self, height: i64, now: Instant) -> PeerId {
        let mut g = self.lock();
        let Some(r) = g.requesters.get(&height) else {
            tracing::error!(
                height,
                "Unable to remove peer request, because the requester is nil"
            );
            return String::new();
        };
        let peer = r.got_block_from.clone().unwrap_or_default();
        g.remove_peer(&peer);
        g.ban_peer(&peer, now);
        peer
    }

    /// `RedoRequestFrom`.
    pub fn redo_request_from(&self, height: i64, peer: &str) {
        let mut g = self.lock();
        if let Some(r) = g.requesters.get_mut(&height)
            && r.did_request_from(peer)
        {
            r.redo(peer);
        }
    }

    /// `AddBlock`: stores the block if it was requested from `peer`.
    pub fn add_block(
        &self,
        peer: &str,
        block: Block,
        ext_commit: Option<ExtendedCommit>,
        block_size: usize,
        now: Instant,
    ) -> Result<(), String> {
        let (res, remove_peer) = self.add_block_inner(peer, block, ext_commit, block_size, now);
        if remove_peer && let Err(e) = &res {
            self.send_error(e, peer.to_owned());
        }
        res
    }

    fn add_block_inner(
        &self,
        peer: &str,
        block: Block,
        ext_commit: Option<ExtendedCommit>,
        block_size: usize,
        now: Instant,
    ) -> (Result<(), String>, bool) {
        let mut g = self.lock();
        let height = block.header.height;
        if let Some(ec) = &ext_commit
            && height != ec.height
        {
            return (
                Err(format!(
                    "block height {height} != extCommit height {}",
                    ec.height
                )),
                true,
            );
        }
        let (pool_height, start_height) = (g.height, g.start_height);
        let Some(r) = g.requesters.get_mut(&height) else {
            if height > pool_height || height < start_height {
                return (
                    Err(format!(
                        "peer sent us block #{height} we didn't expect (current height: {pool_height}, start height: {start_height})"
                    )),
                    true,
                );
            }
            if let Some(p) = g.peers.get_mut(peer) {
                p.decr_pending(block_size, now);
            }
            return (
                Err(format!(
                    "got an already committed block #{height} (possibly from the slow peer {peer})"
                )),
                false,
            );
        };
        // setBlock
        if !r.requested_from.contains(peer) {
            let current = r.peer_id.clone().unwrap_or_default();
            if g.is_peer_banned(peer, now) {
                tracing::trace!(peer, height, "Ignoring block from recently banned peer");
                return (Ok(()), false);
            }
            return (
                Err(format!(
                    "requested block #{height} from {current}, not {peer}, block not requested from peer"
                )),
                true,
            );
        }
        if r.peer_id.as_deref() == Some(peer) {
            r.peer_id = None;
        }
        let block_set = if r.block.is_some() {
            false
        } else {
            r.block = Some(block);
            r.ext_commit = ext_commit;
            r.got_block_from = Some(peer.to_owned());
            r.got_block_signal = true;
            true
        };
        g.num_pending -= 1;
        if let Some(p) = g.peers.get_mut(peer) {
            p.decr_pending(block_size, now);
        }
        if block_set {
            g.last_received_blocks.add(block_size);
            g.recalculate_params();
            tracing::trace!(
                height,
                peer,
                block_size,
                max_pending = g.req_limit,
                retry_timeout = g.retry_timeout.as_secs_f64(),
                "Block added"
            );
        }
        (Ok(()), false)
    }

    /// `Height`.
    #[must_use]
    pub fn height(&self) -> i64 {
        self.lock().height
    }

    /// `MaxPeerHeight`.
    #[must_use]
    pub fn max_peer_height(&self) -> i64 {
        self.lock().max_peer_height
    }

    /// `SetPeerRange`: the peer's alleged `[base, height]`.
    pub fn set_peer_range(&self, peer: &str, base: i64, height: i64, now: Instant) {
        let mut g = self.lock();
        if base > height {
            tracing::info!(
                peer,
                base,
                height,
                "Peer reporting base greater than height"
            );
            if g.peers.contains_key(peer) {
                g.remove_peer(peer);
            }
            g.ban_peer(peer, now);
            return;
        }
        if let Some(p) = g.peers.get_mut(peer) {
            if base < p.base || height < p.height {
                tracing::info!(
                    peer,
                    height,
                    base,
                    prev_height = p.height,
                    prev_base = p.base,
                    "Peer is reporting height/base that is lower than what it previously reported"
                );
                g.remove_peer(peer);
                g.ban_peer(peer, now);
                return;
            }
            p.base = base;
            p.height = height;
        } else {
            if g.is_peer_banned(peer, now) {
                tracing::trace!(peer, "Ignoring banned peer");
                return;
            }
            g.peers
                .insert(peer.to_owned(), BpPeer::new(peer.to_owned(), base, height));
            g.sorted_peers.insert(0, peer.to_owned());
        }
        g.update_max_peer_height();
    }

    /// `RemovePeer`.
    pub fn remove_peer(&self, peer: &str) {
        self.lock().remove_peer(peer);
    }

    /// `IsPeerBanned`.
    #[must_use]
    pub fn is_peer_banned(&self, peer: &str, now: Instant) -> bool {
        self.lock().is_peer_banned(peer, now)
    }

    /// Current `(reqLimit, retryTimeout)` (tests).
    #[must_use]
    pub fn params(&self) -> (i64, Duration) {
        let g = self.lock();
        (g.req_limit, g.retry_timeout)
    }

    /// Records a received block size (tests of `recalculateParams`).
    pub fn record_block_size(&self, size: usize) {
        let mut g = self.lock();
        g.last_received_blocks.add(size);
        g.recalculate_params();
    }

    /// Number of peers in the pool.
    #[must_use]
    pub fn num_peers(&self) -> usize {
        self.lock().peers.len()
    }
}

#[cfg(test)]
mod tests;
