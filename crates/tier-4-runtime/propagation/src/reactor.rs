//! The propagation reactor (`reactor.go`, `commitment.go`,
//! `commitment_state.go`, `have_wants.go`, `catchup.go`).
//!
//! Go spreads the reactor over goroutines (a request routine per peer, the
//! retry ticker) guarded by several mutexes; here all mutable state sits
//! behind one mutex and the per-peer request routine runs inline whenever
//! its triggers fire (new haves queued, a requested part arrived, request
//! counts reset), with the same request selection. Every send is a
//! non-blocking `TrySend`, as in Go.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use ctypes::merkle_proof::HASH_SIZE;
use ctypes::{
    BLOCK_PART_SIZE_BYTES, BitArray, Part, PartSet, PartSetHeader, Proof, Proposal, PubKey,
};
use nodestore::BlockStore;
use p2p::{ChannelDescriptor, Envelope, NodeId, Peer, WeakSwitch};
use prost::Message as _;
use rand::seq::SliceRandom as _;

use crate::peer_state::{PeerState, Request};
use crate::types::{
    CombinedPartSet, CompactBlock, HaveParts, Message, PartMetaData, RecoveryPart, UnmarshalledTx,
    WantParts, txs_to_parts,
};
use crate::{
    COMPACT_BLOCK_UID, DATA_CHANNEL, EditorHandle, Mempool, NoOpEditor, Output, OutputSink,
    PEER_STATE_EDITOR_KEY, Propagator, TxMetaData, WANT_CHANNEL, concurrent_request_limit,
    count_remaining_parts, erasure, req_limit,
};

/// `maxMsgSize`.
pub const MAX_MSG_SIZE: usize = 512 * 1024;
/// `ReactorIncomingMessageQueueSize`.
pub const REACTOR_INCOMING_MESSAGE_QUEUE_SIZE: usize = 20_000;
/// `RetryTime`: the catch-up retry period.
pub const RETRY_TIME: Duration = Duration::from_millis(2500);

/// `propagation.Config`.
#[derive(Clone)]
pub struct Config {
    /// Block store (catch-up parts are served from it).
    pub store: BlockStore,
    /// Mempool (parts are recovered from known txs).
    pub mempool: Arc<dyn Mempool>,
    /// Chain ID.
    pub chain_id: String,
    /// `ConsensusParams.Block.MaxBytes` at start.
    pub block_max_bytes: i64,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("chain_id", &self.chain_id)
            .field("block_max_bytes", &self.block_max_bytes)
            .finish_non_exhaustive()
    }
}

/// `proposalData`.
#[derive(Debug, Clone)]
struct ProposalData {
    compact_block: Arc<CompactBlock>,
    block: CombinedPartSet,
    max_requests: BitArray,
    catchup: bool,
}

/// All mutable state: the `ProposalCache`, the current proposer and the
/// peer states.
struct State {
    proposals: BTreeMap<i64, BTreeMap<i32, ProposalData>>,
    height: i64,
    round: i32,
    current_proposer: Option<PubKey>,
    current_proposal_parts_count: i64,
    peers: HashMap<NodeId, PeerState>,
    retry_deadline: Instant,
}

/// Where the state of a height/round came from (`getAllState`).
enum Found {
    /// In the proposal cache.
    Cached,
    /// Rebuilt from the block store.
    Stored(Box<CombinedPartSet>),
}

/// The propagation reactor (`Reactor`, p2p name `Recovery`).
pub struct Reactor {
    self_id: NodeId,
    config: Config,
    state: Mutex<State>,
    started: AtomicBool,
    switch: Mutex<WeakSwitch>,
    sink: Mutex<Option<OutputSink>>,
    retry_notify: tokio::sync::Notify,
    stopped: AtomicBool,
    /// Peers to disconnect once the state lock is released (the switch
    /// calls back into `remove_peer`).
    deferred_stops: Mutex<Vec<(Arc<Peer>, String)>>,
    me: Weak<Self>,
}

impl std::fmt::Debug for Reactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("propagation::Reactor")
            .field("self", &self.self_id)
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn wrap(m: &Message) -> Vec<u8> {
    m.encode()
}

/// `chunkIndexes`.
fn chunk_indexes(total: usize, chunk: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < total {
        out.push((start, (start + chunk).min(total)));
        start += chunk;
    }
    out
}

/// `chunkParts`.
#[must_use]
pub fn chunk_parts(p: &BitArray, peer_count: usize, redundancy: usize) -> Vec<BitArray> {
    let size = p.size();
    let peer_count = peer_count.max(1);
    let mut chunk = size / peer_count;
    if !size.is_multiple_of(peer_count) || chunk == 0 {
        chunk += 1;
    }
    let mut parts = vec![BitArray::new(size); peer_count];
    if size == 0 {
        return parts;
    }
    let chunks = chunk_indexes(size, chunk);
    let mut cursor = 0;
    for part in &mut parts {
        for _ in 0..redundancy {
            let (s, e) = chunks[cursor];
            for i in s..e {
                part.set(i, true);
            }
            cursor += 1;
            if cursor >= chunks.len() {
                cursor = 0;
            }
        }
    }
    parts
}

/// `sameProposalIdentity`.
fn same_proposal_identity(existing: &ProposalData, cb: &CompactBlock) -> bool {
    if existing.block.original().header() != cb.proposal.block_id.part_set_header {
        return false;
    }
    if !existing.compact_block.proposal.block_id.is_zero() {
        return existing.compact_block.proposal.block_id == cb.proposal.block_id;
    }
    true
}

impl State {
    /// `relevant`.
    fn relevant(&self, height: i64, round: i32) -> bool {
        height >= self.height && round >= self.round
    }

    fn cached(&self, height: i64, round: i32) -> Option<&ProposalData> {
        self.proposals.get(&height)?.get(&round)
    }

    fn cached_mut(&mut self, height: i64, round: i32) -> Option<&mut ProposalData> {
        self.proposals.get_mut(&height)?.get_mut(&round)
    }

    /// `getAllState`: the cached entry (round `< -1` means the latest
    /// round), else for past heights the block from the store.
    fn get_all_state(
        &self,
        store: &BlockStore,
        height: i64,
        round: i32,
        catchup: bool,
    ) -> Option<(Found, i32)> {
        if !catchup && !self.relevant(height, round) {
            return None;
        }
        let props = self.proposals.get(&height);
        let mut r = round;
        let mut has_round = props.is_some_and(|p| p.contains_key(&round));
        if round < -1
            && let Some(p) = props.filter(|p| !p.is_empty())
        {
            r = p.keys().copied().max().unwrap_or(0).max(0);
            has_round = p.contains_key(&r);
        }
        if has_round {
            return Some((Found::Cached, r));
        }
        if height < self.height && store.load_block_meta(height).ok().flatten().is_some() {
            let (parts, _) = store.load_part_set(height).ok().flatten()?;
            return Some((
                Found::Stored(Box::new(CombinedPartSet::from_original(parts, false))),
                r,
            ));
        }
        None
    }

    /// `AddProposal`.
    fn add_proposal(&mut self, cb: &Arc<CompactBlock>) -> bool {
        let (h, r) = (cb.proposal.height, cb.proposal.round);
        if !self.relevant(h, r) {
            return false;
        }
        let rounds = self.proposals.entry(h).or_default();
        if rounds.contains_key(&r) {
            return false;
        }
        self.height = h;
        self.round = r;
        let block = CombinedPartSet::from_compact_block(cb);
        let total = block.total();
        rounds.insert(
            r,
            ProposalData {
                compact_block: cb.clone(),
                max_requests: BitArray::new(total as usize),
                block,
                catchup: false,
            },
        );
        self.current_proposal_parts_count = i64::from(total);
        true
    }

    /// `conflictsWith`.
    fn conflicts_with(&self, cb: &CompactBlock) -> bool {
        self.cached(cb.proposal.height, cb.proposal.round)
            .is_some_and(|e| !same_proposal_identity(e, cb))
    }

    /// `countRequests`.
    fn count_requests(&self, height: i64, round: i32, part: usize) -> Vec<NodeId> {
        self.peers
            .values()
            .filter(|p| p.requests(height, round).is_some_and(|r| r.get(part)))
            .map(|p| p.peer.id())
            .collect()
    }

    /// `unfinishedHeights`: (height, round) of the latest round of every
    /// uncommitted height that is incomplete or marked catch-up.
    fn unfinished_heights(&self, committed: i64) -> Vec<(i64, i32)> {
        let mut out = Vec::new();
        for (h, rounds) in &self.proposals {
            if *h <= committed {
                continue;
            }
            let Some((r, pd)) = rounds.iter().max_by_key(|(r, _)| **r) else {
                continue;
            };
            if pd.catchup || !pd.block.is_complete() {
                out.push((*h, *r));
            }
        }
        out
    }

    /// `ResetRequestCounts`.
    fn reset_request_counts(&mut self) {
        for p in self.peers.values_mut() {
            p.concurrent_reqs = 0;
        }
    }

    /// `broadcastHaves`.
    fn broadcast_haves(&mut self, haves: &HaveParts, from: NodeId, part_set_size: usize) {
        let bz = wrap(&Message::HaveParts(haves.clone()));
        let hb = haves.bit_array(part_set_size);
        for peer in self.peers.values_mut() {
            if peer.peer.id() == from {
                continue;
            }
            if !peer.peer.try_send(DATA_CHANNEL, bz.clone()) {
                tracing::debug!(peer = %peer.peer.id(), "failed to send haves to peer");
                continue;
            }
            peer.add_haves(haves.height, haves.round, &hb);
            for h in hb.true_indices() {
                peer.consensus_peer_state
                    .set_has_proposal_block_part(haves.height, haves.round, h);
            }
        }
    }
}

impl Reactor {
    /// `NewReactor`.
    #[must_use]
    pub fn new(self_id: NodeId, config: Config) -> Arc<Self> {
        let h = config.store.height();
        Arc::new_cyclic(|me| Self {
            self_id,
            state: Mutex::new(State {
                proposals: BTreeMap::new(),
                height: h,
                round: 0,
                current_proposer: None,
                current_proposal_parts_count: 0,
                peers: HashMap::new(),
                retry_deadline: Instant::now() + RETRY_TIME,
            }),
            config,
            started: AtomicBool::new(false),
            switch: Mutex::new(WeakSwitch::default()),
            sink: Mutex::new(None),
            retry_notify: tokio::sync::Notify::new(),
            stopped: AtomicBool::new(false),
            deferred_stops: Mutex::new(Vec::new()),
            me: me.clone(),
        })
    }

    /// Installs the consensus sink (Go's part / proposal channels).
    pub fn set_output(&self, sink: OutputSink) {
        *lock(&self.sink) = Some(sink);
    }

    fn emit(&self, out: Output) {
        if let Some(s) = lock(&self.sink).clone() {
            s(out);
        }
    }

    /// Our node ID.
    #[must_use]
    pub fn self_id(&self) -> NodeId {
        self.self_id
    }

    /// Whether `StartProcessing` was called.
    #[must_use]
    pub fn is_started(&self) -> bool {
        self.started.load(Ordering::SeqCst)
    }

    fn reset_retry(st: &mut State, notify: &tokio::sync::Notify) {
        st.retry_deadline = Instant::now() + RETRY_TIME;
        notify.notify_one();
    }

    /// `StopPeerForError`, deferred until the state lock is released.
    fn stop_peer_for_error(&self, peer: &Arc<Peer>, reason: &str) {
        lock(&self.deferred_stops).push((peer.clone(), reason.to_owned()));
    }

    fn run_deferred_stops(&self) {
        let stops = std::mem::take(&mut *lock(&self.deferred_stops));
        if stops.is_empty() {
            return;
        }
        if let Some(sw) = lock(&self.switch).upgrade() {
            for (p, reason) in stops {
                sw.stop_peer_for_error(&p, &reason);
            }
        }
    }

    // ------------------------------------------------------------------
    // Compact blocks (commitment.go)

    /// `handleCompactBlock`.
    fn handle_compact_block(&self, cb: Arc<CompactBlock>, peer: NodeId, proposer: bool) {
        let mut st = lock(&self.state);
        if proposer {
            self.process_validated_compact_block(&mut st, cb, peer, proposer);
            return;
        }
        if let Err(e) = self.validate_compact_block(&st, &cb) {
            if cb.proposal.height >= st.height
                && let Some(p) = st.peers.get_mut(&peer)
                && p.store_unverified_proposal(cb.clone())
            {
                tracing::debug!(
                    height = cb.proposal.height,
                    round = cb.proposal.round,
                    current_height = st.height,
                    err = %e,
                    peer = %peer,
                    "cached compact block that failed validation"
                );
            }
            return;
        }
        self.process_validated_compact_block(&mut st, cb, peer, proposer);
    }

    /// `processValidatedCompactBlock`.
    fn process_validated_compact_block(
        &self,
        st: &mut State,
        cb: Arc<CompactBlock>,
        peer: NodeId,
        proposer: bool,
    ) {
        if let Err(e) = cb.proofs() {
            tracing::error!(err = %e, "received invalid compact block");
            if let Some(p) = st.peers.get(&peer).map(|p| p.peer.clone()) {
                self.stop_peer_for_error(&p, &e);
            }
            return;
        }
        let added = st.add_proposal(&cb);
        if !added && st.conflicts_with(&cb) {
            tracing::info!(
                height = cb.proposal.height,
                round = cb.proposal.round,
                peer = %peer,
                "rejecting compact block conflicting with existing proposal identity"
            );
            return;
        }
        if !proposer {
            self.emit(Output::Proposal {
                proposal: cb.proposal.clone(),
                from: peer.to_string(),
            });
            if let Some(p) = st.peers.get(&peer) {
                p.consensus_peer_state.set_has_proposal(&cb.proposal);
            }
        }
        if !added {
            return;
        }
        if !proposer {
            self.recover_parts_from_mempool(st, &cb);
        }
        Self::broadcast_compact_block(st, &cb, peer);
    }

    /// `recoverPartsFromMempool`.
    fn recover_parts_from_mempool(&self, st: &mut State, cb: &CompactBlock) {
        let mut found = Vec::new();
        for md in &cb.blobs {
            let Ok(key) = <[u8; 32]>::try_from(md.hash.as_slice()) else {
                tracing::error!("failed to decode tx key");
                continue;
            };
            let Some(tx) = self.config.mempool.get_tx_by_key(&key) else {
                continue;
            };
            let wrapped = ctypes::proto::mempool::Txs { txs: vec![tx] }.encode_to_vec();
            found.push(UnmarshalledTx {
                meta: md.clone(),
                tx_bytes: wrapped,
            });
        }
        if found.is_empty() {
            return;
        }
        let (h, r) = (cb.proposal.height, cb.proposal.round);
        let parts = match txs_to_parts(
            &mut found,
            cb.proposal.block_id.part_set_header.total,
            BLOCK_PART_SIZE_BYTES,
            cb.last_len,
        ) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(err = %e, "invalid compact block");
                return;
            }
        };
        let proofs = match cb.proofs() {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(err = %e, "failed to get proofs from compact block");
                return;
            }
        };
        let Some(pd) = st.relevant(h, r).then(|| st.cached_mut(h, r)).flatten() else {
            tracing::error!(
                height = h,
                round = r,
                "unexpected nil partset while attempting to reuse transactions from the mempool"
            );
            return;
        };
        let mut haves = HaveParts {
            height: h,
            round: r,
            parts: Vec::new(),
        };
        let mut recovered = Vec::new();
        for mut p in parts {
            if pd.block.has_part(p.index as usize) {
                continue;
            }
            let Some(proof) = proofs.get(p.index as usize) else {
                continue;
            };
            p.proof = proof.clone();
            match pd.block.add_original_part(&p) {
                Ok(true) => {}
                Ok(false) => {
                    tracing::error!(part = p.index, "failed to add locally recovered part");
                    continue;
                }
                Err(e) => {
                    tracing::error!(err = %e, "failed to add locally recovered part");
                    continue;
                }
            }
            haves.parts.push(PartMetaData {
                index: p.index,
                hash: p.proof.leaf_hash.clone(),
            });
            recovered.push(p);
        }
        let total = pd.block.total() as usize;
        tracing::debug!(
            height = h,
            round = r,
            n = recovered.len(),
            "recovered parts from the mempool"
        );
        for p in recovered {
            self.emit(Output::Part {
                height: h,
                round: r,
                part: p,
            });
        }
        if !haves.parts.is_empty() {
            st.broadcast_haves(&haves, self.self_id, total);
        }
    }

    /// `broadcastCompactBlock`.
    fn broadcast_compact_block(st: &State, cb: &CompactBlock, from: NodeId) {
        let bz = wrap(&Message::CompactBlock(Box::new(cb.clone())));
        for peer in st.peers.values() {
            if peer.peer.id() == from {
                continue;
            }
            if !peer.peer.try_send(DATA_CHANNEL, bz.clone()) {
                tracing::debug!(peer = %peer.peer.id(), "failed to send proposal to peer");
            }
        }
    }

    /// `validateCompactBlock`.
    fn validate_compact_block(&self, st: &State, cb: &CompactBlock) -> Result<(), String> {
        let proposer = st.current_proposer.as_ref().ok_or("nil proposer key")?;
        let p = &cb.proposal;
        if p.height != st.height || p.round != st.round {
            return Err(format!(
                "proposal height {} round {} does not match state height {} round {}",
                p.height, p.round, st.height, st.round
            ));
        }
        if cb.last_len > BLOCK_PART_SIZE_BYTES {
            return Err(format!(
                "invalid last length {} > {BLOCK_PART_SIZE_BYTES}",
                cb.last_len
            ));
        }
        if p.pol_round < -1 || (p.pol_round >= 0 && p.pol_round >= p.round) {
            return Err(format!(
                "error invalid proposal POL round: {} {}",
                p.pol_round, p.round
            ));
        }
        let sb = p
            .sign_bytes(&self.config.chain_id)
            .map_err(|e| e.to_string())?;
        if !proposer.verify_signature(&sb, &p.signature) {
            return Err("error invalid proposal signature".into());
        }
        let mut max_bytes = self.config.block_max_bytes;
        if max_bytes == -1 {
            max_bytes = ctypes::part_set::MAX_BLOCK_SIZE_BYTES;
        }
        let max_parts = (max_bytes - 1) / i64::from(BLOCK_PART_SIZE_BYTES) + 1;
        if i64::from(p.block_id.part_set_header.total) > max_parts {
            return Err(format!(
                "proposal block has too many parts: {} > {max_parts}",
                p.block_id.part_set_header.total
            ));
        }
        let raw = ctypes::canonical::raw_bytes_message_sign_bytes(
            &self.config.chain_id,
            COMPACT_BLOCK_UID,
            &cb.sign_bytes(),
        )
        .map_err(|e| e.to_string())?;
        if proposer.verify_signature(&raw, &cb.signature) {
            return Ok(());
        }
        Err(format!(
            "invalid signature over the compact block: height {} round {}",
            p.height, p.round
        ))
    }

    // ------------------------------------------------------------------
    // Haves and wants (have_wants.go)

    /// `handleHaves`.
    fn handle_haves(&self, peer: NodeId, haves: &HaveParts) {
        if !self.is_started() {
            return;
        }
        let (height, round) = (haves.height, haves.round);
        let mut st = lock(&self.state);
        if !st.peers.contains_key(&peer) {
            return;
        }
        let Some((Found::Cached, _)) = st.get_all_state(&self.config.store, height, round, false)
        else {
            tracing::debug!(%peer, height, round, "received have part for unknown proposal");
            return;
        };
        let Some(pd) = st.cached(height, round) else {
            return;
        };
        if pd.compact_block.parts_hashes.is_empty() {
            return;
        }
        if let Err(e) = haves.validate_part_hashes(&pd.compact_block.parts_hashes) {
            tracing::error!(height, round, err = %e, "received invalid have part");
            return;
        }
        let total = pd.block.total() as usize;
        let complete = pd.block.original().is_complete();
        let mut hc = haves.bit_array(total).sub(pd.block.bit_array());
        let full_reqs = pd.max_requests.clone();
        let Some(p) = st.peers.get_mut(&peer) else {
            return;
        };
        p.initialize(height, round, total);
        if let Some(bm) = p.haves_mut(height, round) {
            for pmd in &haves.parts {
                bm.set(pmd.index as usize, true);
            }
        }
        for pmd in &haves.parts {
            p.consensus_peer_state
                .set_has_proposal_block_part(height, round, pmd.index as usize);
        }
        if complete || hc.is_empty() {
            return;
        }
        hc = hc.sub(&full_reqs);
        if hc.is_empty() {
            return;
        }
        for index in hc.true_indices() {
            p.push_have(Request {
                height,
                round,
                index: index as u32,
            });
        }
        self.process_requests(&mut st, peer);
    }

    /// The request routine (`requestFromPeer`, `CanRequest` case): turns
    /// queued haves into one `WantParts` within the concurrency limit.
    fn process_requests(&self, st: &mut State, peer: NodeId) {
        let npeers = st.peers.len();
        let limit = concurrent_request_limit(npeers, st.current_proposal_parts_count as usize);
        let Some(ps) = st.peers.get(&peer) else {
            return;
        };
        let can_send = limit - ps.concurrent_reqs;
        if can_send <= 0 || ps.received_haves.is_empty() {
            return;
        }
        let mut i = can_send.min(ps.received_haves.len() as i64);
        let mut wants: Option<WantParts> = None;
        let mut cur: Option<(i64, i32)> = None;
        while i > 0 {
            let Some(have) = st
                .peers
                .get_mut(&peer)
                .and_then(|p| p.received_haves.pop_front())
            else {
                break;
            };
            if wants
                .as_ref()
                .is_some_and(|w| have.height != w.height || have.round != w.round)
            {
                wants = None;
                cur = None;
            }
            if !st.relevant(have.height, have.round) {
                continue;
            }
            if cur.is_none() {
                if st.cached(have.height, have.round).is_none() {
                    tracing::error!(
                        height = have.height,
                        round = have.round,
                        "couldn't find proposal when filtering requests"
                    );
                    break;
                }
                cur = Some((have.height, have.round));
            }
            let Some(pd) = st.cached(have.height, have.round) else {
                break;
            };
            let total = pd.block.total() as usize;
            if count_remaining_parts(total, pd.block.bit_array().count()) == 0 {
                continue;
            }
            let idx = have.index as usize;
            if pd.block.bit_array().get(idx) || pd.max_requests.get(idx) {
                continue;
            }
            let reqs = st.count_requests(have.height, have.round, idx);
            if reqs.len() >= req_limit(total) {
                if let Some(pd) = st.cached_mut(have.height, have.round) {
                    pd.max_requests.set(idx, true);
                }
                continue;
            }
            wants
                .get_or_insert_with(|| WantParts {
                    parts: BitArray::new(total),
                    height: have.height,
                    round: have.round,
                    prove: false,
                    missing_parts_count: 0,
                })
                .parts
                .set(idx, true);
            i -= 1;
        }
        let Some(mut wants) = wants else {
            return;
        };
        let Some(pd) = st.cached(wants.height, wants.round) else {
            return;
        };
        let missing =
            count_remaining_parts(pd.block.total() as usize, pd.block.bit_array().count());
        if missing == 0 {
            return;
        }
        wants.missing_parts_count = missing;
        if let Err(e) = self.send_wants_then_broadcast_haves(st, peer, &wants) {
            tracing::error!(err = %e, "error sending wants");
        }
    }

    /// `sendWantsThenBroadcastHaves`.
    fn send_wants_then_broadcast_haves(
        &self,
        st: &mut State,
        peer: NodeId,
        wants: &WantParts,
    ) -> Result<(), String> {
        let have = Self::convert_want_to_have(st, &self.config.store, wants)?;
        Self::send_want(st, peer, wants);
        st.broadcast_haves(&have, peer, wants.parts.size());
        Ok(())
    }

    /// `convertWantToHave`.
    fn convert_want_to_have(
        st: &State,
        store: &BlockStore,
        want: &WantParts,
    ) -> Result<HaveParts, String> {
        let cb = match st.get_all_state(store, want.height, want.round, false) {
            Some((Found::Cached, r)) => {
                st.cached(want.height, r).map(|pd| pd.compact_block.clone())
            }
            _ => None,
        }
        .ok_or_else(|| {
            format!(
                "couldn't find proposal height {} round {}",
                want.height, want.round
            )
        })?;
        let mut have = HaveParts {
            height: want.height,
            round: want.round,
            parts: Vec::new(),
        };
        for index in want.parts.true_indices() {
            let Some(h) = cb.parts_hashes.get(index) else {
                tracing::error!(index, "couldn't find part hash");
                continue;
            };
            have.parts.push(PartMetaData {
                index: index as u32,
                hash: h.clone(),
            });
        }
        Ok(have)
    }

    /// `sendWant`.
    fn send_want(st: &mut State, peer: NodeId, want: &WantParts) {
        let Some(ps) = st.peers.get_mut(&peer) else {
            return;
        };
        if !ps
            .peer
            .try_send(WANT_CHANNEL, wrap(&Message::WantParts(want.clone())))
        {
            tracing::error!(%peer, height = want.height, round = want.round, "failed to send part state");
            return;
        }
        ps.add_requests(want.height, want.round, &want.parts);
        ps.concurrent_reqs += want.parts.count() as i64;
    }

    /// `handleWants`.
    fn handle_wants(&self, peer: NodeId, wants: &WantParts) {
        if !self.is_started() {
            return;
        }
        let (height, round) = (wants.height, wants.round);
        let mut st = lock(&self.state);
        if !st.peers.contains_key(&peer) {
            return;
        }
        let Some((found, r)) = st.get_all_state(&self.config.store, height, round, wants.prove)
        else {
            tracing::debug!(%peer, height, round, "received part state request for unknown proposal");
            return;
        };
        let parts = match found {
            Found::Cached => match st.cached(height, r) {
                Some(pd) => pd.block.clone(),
                None => return,
            },
            Found::Stored(p) => *p,
        };
        if wants.parts.size() != parts.total() as usize {
            tracing::error!(
                %peer,
                height,
                round,
                expected = parts.total(),
                received = wants.parts.size(),
                "received want part with invalid parts size"
            );
            return;
        }
        let can_send = parts.bit_array().and(&wants.parts);
        let Some(p) = st.peers.get_mut(&peer) else {
            return;
        };
        p.set_remaining_requests(height, round, i64::from(wants.missing_parts_count));
        for idx in can_send.true_indices() {
            if p.remaining_requests(height, round) <= 0 {
                break;
            }
            let Some(part) = parts.get_part(idx as u32) else {
                continue;
            };
            let rp = RecoveryPart {
                height,
                round,
                index: idx as u32,
                data: part.bytes.clone(),
                proof: wants.prove.then(|| part.proof.clone()),
            };
            if !p
                .peer
                .try_send(DATA_CHANNEL, wrap(&Message::RecoveryPart(rp)))
            {
                tracing::error!(%peer, height, round, part = idx, "failed to send part");
                continue;
            }
            p.decrease_remaining_requests(height, round, 1);
        }
        let still_missing = wants.parts.sub(&can_send);
        if !still_missing.is_empty() {
            p.add_wants(height, round, &still_missing);
        }
    }

    /// `handleRecoveryPart`.
    fn handle_recovery_part(&self, peer: NodeId, part: &RecoveryPart) {
        if !self.is_started() {
            return;
        }
        let (height, round) = (part.height, part.round);
        let mut st = lock(&self.state);
        if !st.peers.contains_key(&peer) && peer != self.self_id {
            return;
        }
        let Some((Found::Cached, _)) = st.get_all_state(&self.config.store, height, round, false)
        else {
            tracing::debug!(%peer, height, round, "received part for unknown proposal");
            return;
        };
        let Some(pd) = st.cached_mut(height, round) else {
            return;
        };
        if pd.block.has_part(part.index as usize) || pd.block.is_complete() {
            return;
        }
        let proof = match pd.compact_block.get_proof(part.index) {
            Some(p) => p,
            None => match &part.proof {
                Some(p) if p.leaf_hash.len() == HASH_SIZE => p.clone(),
                Some(_) => return,
                None => {
                    tracing::error!(%peer, height, round, part = part.index, "proof not found");
                    return;
                }
            },
        };
        let added = match pd.block.add_part(part, proof.clone()) {
            Ok(a) => a,
            Err(e) => {
                tracing::error!(%peer, height, round, part = part.index, err = %e, "failed to add part to part set");
                return;
            }
        };
        let original_total = pd.block.original().total();
        let relevant = st.relevant(height, round);
        if let Some(p) = st.peers.get_mut(&peer)
            && relevant
        {
            p.decrease_concurrent_reqs(1);
        }
        if !added {
            self.process_requests(&mut st, peer);
            return;
        }
        if part.index < original_total {
            self.emit(Output::Part {
                height,
                round,
                part: Part {
                    index: part.index,
                    bytes: part.data.clone(),
                    proof: proof.clone(),
                },
            });
        }
        Self::clear_wants(&mut st, part, &proof);
        self.try_decode(&mut st, height, round, peer);
        self.process_requests(&mut st, peer);
    }

    /// The decode branch of `handleRecoveryPart`.
    fn try_decode(&self, st: &mut State, height: i64, round: i32, from: NodeId) {
        let Some(pd) = st.cached_mut(height, round) else {
            return;
        };
        if !pd.block.can_decode() || pd.block.is_decoding {
            return;
        }
        let had = pd.block.original().bit_array();
        if let Err(e) = pd.block.decode() {
            tracing::error!(height, round, err = %e, "failed to decode parts");
            return;
        }
        let total = pd.block.total();
        let original_total = pd.block.original().total();
        let mut haves = HaveParts {
            height,
            round,
            parts: Vec::new(),
        };
        let mut decoded = Vec::new();
        let mut all = Vec::new();
        for i in 0..total {
            let Some(p) = pd.block.get_part(i) else {
                tracing::error!(height, round, part = i, "failed to get decoded part");
                continue;
            };
            if i < original_total && !had.get(i as usize).copied().unwrap_or(false) {
                decoded.push(p.clone());
            }
            haves.parts.push(PartMetaData {
                index: i,
                hash: p.proof.leaf_hash.clone(),
            });
            all.push(p);
        }
        tracing::debug!(height, round, n = decoded.len(), "decoded block");
        for p in decoded {
            self.emit(Output::Part {
                height,
                round,
                part: p,
            });
        }
        st.broadcast_haves(&haves, from, total as usize);
        for p in all {
            let rp = RecoveryPart {
                height,
                round,
                index: p.index,
                data: p.bytes,
                proof: None,
            };
            Self::clear_wants(st, &rp, &p.proof);
        }
    }

    /// `clearWants`: serves parts that peers asked for earlier.
    fn clear_wants(st: &mut State, part: &RecoveryPart, proof: &Proof) {
        let msg = RecoveryPart {
            height: part.height,
            round: part.round,
            index: part.index,
            data: part.data.clone(),
            proof: Some(proof.clone()),
        };
        let bz = wrap(&Message::RecoveryPart(msg));
        for peer in st.peers.values_mut() {
            if !peer.wants_part(part.height, part.round, part.index) {
                continue;
            }
            if peer.remaining_requests(part.height, part.round) <= 0 {
                continue;
            }
            if !peer.peer.try_send(DATA_CHANNEL, bz.clone()) {
                tracing::error!(peer = %peer.peer.id(), part = part.index, "failed to send part");
                continue;
            }
            if peer
                .set_have(part.height, part.round, part.index as usize)
                .is_err()
            {
                continue;
            }
            if peer
                .set_want(part.height, part.round, part.index as usize, false)
                .is_err()
            {
                continue;
            }
            peer.decrease_remaining_requests(part.height, part.round, 1);
        }
    }

    // ------------------------------------------------------------------
    // Catch-up (catchup.go)

    /// `retryWants`: re-requests (with proofs) the missing original parts
    /// of every unfinished height.
    pub fn retry_wants(&self) {
        if !self.is_started() {
            return;
        }
        let committed = self.config.store.height();
        let mut st = lock(&self.state);
        let data = st.unfinished_heights(committed);
        let mut peer_ids: Vec<NodeId> = st.peers.keys().copied().collect();
        for (height, round) in data {
            let Some(pd) = st.cached(height, round) else {
                continue;
            };
            if pd.block.is_complete() {
                continue;
            }
            let mut missing = pd.block.missing_original();
            if missing.is_empty() {
                tracing::error!(height, round, "no missing parts yet block is incomplete");
                continue;
            }
            let total = pd.block.total() as usize;
            let have_count = pd.block.bit_array().count();
            peer_ids.shuffle(&mut rand::rng());
            for id in &peer_ids {
                let Some(peer) = st.peers.get(id) else {
                    continue;
                };
                if peer.consensus_peer_state.get_height() < height - 1 {
                    tracing::debug!("retryWants: skipping peer");
                    continue;
                }
                let mut mc = missing.clone();
                if let Some(reqs) = peer.requests(height, round) {
                    mc = mc.sub(reqs);
                }
                if mc.is_empty() {
                    continue;
                }
                let missing_count = count_remaining_parts(total, have_count);
                if missing_count == 0 {
                    continue;
                }
                let want = WantParts {
                    parts: mc.clone(),
                    height,
                    round,
                    prove: true,
                    missing_parts_count: missing_count,
                };
                if !peer
                    .peer
                    .try_send(WANT_CHANNEL, wrap(&Message::WantParts(want)))
                {
                    tracing::error!(peer = %id, height, round, "failed to send want part");
                    continue;
                }
                tracing::debug!(peer = %id, height, round, parts = %mc, "catchup request");
                for idx in mc.true_indices() {
                    if st.count_requests(height, round, idx).len() >= req_limit(total) {
                        missing.set(idx, false);
                    }
                }
                if let Some(peer) = st.peers.get_mut(id) {
                    peer.add_requests(height, round, &mc);
                }
            }
        }
    }

    /// `applyCachedProposalIfAvailable`.
    fn apply_cached_proposal_if_available(&self) {
        let mut st = lock(&self.state);
        let (h, r) = (st.height, st.round);
        if st.cached(h, r).is_some() {
            return;
        }
        let ids: Vec<NodeId> = st.peers.keys().copied().collect();
        for id in ids {
            let Some(cb) = st.peers.get(&id).and_then(|p| p.unverified_proposal(h)) else {
                continue;
            };
            if cb.proposal.round != r {
                continue;
            }
            if let Err(e) = self.validate_compact_block(&st, &cb) {
                tracing::debug!(height = h, round = r, peer = %id, err = %e, "cached proposal failed validation");
                continue;
            }
            tracing::info!(height = h, round = r, peer = %id, "applying cached proposal from catchup");
            if self.handle_cached_compact_block(&mut st, &cb) {
                if let Some(p) = st.peers.get_mut(&id) {
                    p.delete_unverified_proposal(h);
                }
                drop(st);
                self.retry_wants();
                return;
            }
        }
    }

    /// `handleCachedCompactBlock`.
    fn handle_cached_compact_block(&self, st: &mut State, cb: &Arc<CompactBlock>) -> bool {
        if let Err(e) = cb.proofs() {
            tracing::error!(err = %e, "cached compact block has invalid proofs");
            return false;
        }
        if !st.add_proposal(cb) {
            tracing::debug!(
                height = cb.proposal.height,
                round = cb.proposal.round,
                "cached proposal already exists"
            );
        }
        match st.cached_mut(cb.proposal.height, cb.proposal.round) {
            Some(pd) if same_proposal_identity(pd, cb) => pd.catchup = true,
            Some(_) => {
                tracing::error!(
                    height = cb.proposal.height,
                    round = cb.proposal.round,
                    "cached compact block conflicts with existing proposal, rejecting"
                );
                return false;
            }
            None => return false,
        }
        self.emit(Output::Proposal {
            proposal: cb.proposal.clone(),
            from: self.self_id.to_string(),
        });
        self.recover_parts_from_mempool(st, cb);
        Self::reset_retry(st, &self.retry_notify);
        true
    }

    /// The retry ticker (`ticker.C` → `retryWants`).
    async fn retry_routine(self: Arc<Self>) {
        loop {
            if self.stopped.load(Ordering::SeqCst) {
                return;
            }
            let deadline = lock(&self.state).retry_deadline;
            let now = Instant::now();
            if now >= deadline {
                lock(&self.state).retry_deadline = now + RETRY_TIME;
                self.retry_wants();
                continue;
            }
            tokio::select! {
                () = tokio::time::sleep(deadline - now) => {}
                () = self.retry_notify.notified() => {}
            }
        }
    }

    /// Peer IDs known to the reactor (tests / diagnostics).
    #[must_use]
    pub fn peer_ids(&self) -> Vec<NodeId> {
        lock(&self.state).peers.keys().copied().collect()
    }

    /// `IsLegacyPropagation`.
    #[must_use]
    pub fn is_legacy_propagation(ni: &p2p::NodeInfo) -> bool {
        !ni.channels
            .iter()
            .any(|&c| c == DATA_CHANNEL || c == WANT_CHANNEL)
    }
}

impl Propagator for Reactor {
    fn get_proposal(&self, height: i64, round: i32) -> Option<(Proposal, PartSet)> {
        let st = lock(&self.state);
        match st.get_all_state(&self.config.store, height, round, true)? {
            (Found::Cached, r) => st.cached(height, r).map(|pd| {
                (
                    pd.compact_block.proposal.clone(),
                    pd.block.original().clone(),
                )
            }),
            (Found::Stored(_), _) => None,
        }
    }

    fn propose_block(
        &self,
        proposal: &Proposal,
        parts: &PartSet,
        txs: Vec<TxMetaData>,
        signer: &dyn privval::PrivValidator,
    ) -> Result<(), String> {
        let (parity, last_len) = erasure::encode(parts, BLOCK_PART_SIZE_BYTES).map_err(|e| {
            tracing::error!(err = %e, "failed to encode block");
            e
        })?;
        let mut hashes = Vec::with_capacity((parts.total() + parity.total()) as usize);
        let mut proofs = Vec::with_capacity(hashes.capacity());
        for ps in [parts, &parity] {
            for i in 0..ps.total() as usize {
                let p = ps.get_part(i).ok_or("missing part")?;
                hashes.push(p.proof.leaf_hash.clone());
                proofs.push(p.proof);
            }
        }
        let mut cb = CompactBlock {
            proposal: proposal.clone(),
            last_len: last_len as u32,
            bp_hash: parity.hash().to_vec(),
            blobs: txs,
            parts_hashes: hashes,
            ..CompactBlock::default()
        };
        cb.set_proof_cache(proofs);
        cb.signature = signer
            .sign_raw_bytes(&self.config.chain_id, COMPACT_BLOCK_UID, &cb.sign_bytes())
            .map_err(|e| {
                tracing::error!(err = %e, height = proposal.height, round = proposal.round, "failed to sign compact block");
                e.to_string()
            })?;
        let cb = Arc::new(cb);
        self.handle_compact_block(cb.clone(), self.self_id, true);

        let mut st = lock(&self.state);
        let (h, r) = (proposal.height, proposal.round);
        let Some(pd) = st.relevant(h, r).then(|| st.cached_mut(h, r)).flatten() else {
            return Err(format!(
                "failed to get all state for this node's proposal {h}/{r}"
            ));
        };
        pd.block.set_proposal_data(parts.clone(), parity.clone());
        let total = pd.block.total() as usize;
        let mut initial = vec![PartMetaData {
            index: 0,
            hash: parts
                .get_part(0)
                .map(|p| p.proof.leaf_hash)
                .unwrap_or_default(),
        }];
        if parts.total() > 1
            && let Some(last) = parts.get_part(parts.total() as usize - 1)
        {
            initial.push(PartMetaData {
                index: last.index,
                hash: last.proof.leaf_hash,
            });
        }
        let parity_bits = BitArray::from_fn(parity.total() as usize, |i| parity.has_part(i));
        let ids: Vec<NodeId> = st.peers.keys().copied().collect();
        let chunks = chunk_parts(&parity_bits, ids.len(), 1);
        for (index, id) in ids.iter().enumerate() {
            let Some(peer) = st.peers.get_mut(id) else {
                continue;
            };
            let mut meta = initial.clone();
            for pi in chunks[index].true_indices() {
                if let Some(p) = parity.get_part(pi) {
                    meta.push(PartMetaData {
                        index: parity.total() + pi as u32,
                        hash: p.proof.leaf_hash,
                    });
                }
            }
            let have = HaveParts {
                height: h,
                round: r,
                parts: meta,
            };
            if !peer
                .peer
                .try_send(DATA_CHANNEL, wrap(&Message::HaveParts(have.clone())))
            {
                tracing::error!(peer = %id, height = h, round = r, "failed to send have part");
                continue;
            }
            peer.initialize(h, r, total);
            for part in &have.parts {
                if peer.set_have(h, r, part.index as usize).is_err() {
                    continue;
                }
                peer.consensus_peer_state
                    .set_has_proposal_block_part(h, r, part.index as usize);
            }
        }
        Ok(())
    }

    fn add_commitment(&self, height: i64, round: i32, psh: &PartSetHeader) {
        tracing::info!(height, round, psh = %psh, "adding commitment");
        {
            let mut st = lock(&self.state);
            if let Some(existing) = st.cached(height, round) {
                if existing.block.original().header() == *psh {
                    return;
                }
                tracing::error!(height, round, "replacing existing proposal with new one");
                for p in st.peers.values_mut() {
                    p.delete_round(height, round);
                }
            }
            let combined = CombinedPartSet::from_original(
                PartSet::from_header(psh, BLOCK_PART_SIZE_BYTES),
                true,
            );
            let cb = CompactBlock {
                proposal: Proposal {
                    height,
                    round,
                    ..Proposal::default()
                },
                ..CompactBlock::default()
            };
            st.proposals.entry(height).or_default().insert(
                round,
                ProposalData {
                    compact_block: Arc::new(cb),
                    catchup: true,
                    block: combined,
                    max_requests: BitArray::new(psh.total as usize * 2),
                },
            );
            st.height = height;
            st.round = 0;
            Self::reset_retry(&mut st, &self.retry_notify);
        }
        self.retry_wants();
    }

    fn prune(&self, committed_height: i64) {
        let mut st = lock(&self.state);
        for p in st.peers.values_mut() {
            p.prune(committed_height);
        }
        st.proposals.retain(|h, _| *h >= committed_height);
        st.height = committed_height;
        st.reset_request_counts();
        Self::reset_retry(&mut st, &self.retry_notify);
        let ids: Vec<NodeId> = st.peers.keys().copied().collect();
        for id in ids {
            self.process_requests(&mut st, id);
        }
    }

    fn set_consensus_state(&self, height: i64, round: i32, proposer: &PubKey) {
        {
            let mut st = lock(&self.state);
            st.current_proposer = Some(proposer.clone());
            let committed = self.config.store.height();
            let prev = st.height;
            st.round = round;
            st.height = height;
            if height > prev
                && height == committed + 1
                && let Some(props) = st.proposals.get_mut(&height)
            {
                for pd in props.values_mut() {
                    pd.catchup = true;
                }
            }
            st.reset_request_counts();
            let ids: Vec<NodeId> = st.peers.keys().copied().collect();
            for id in ids {
                self.process_requests(&mut st, id);
            }
        }
        self.apply_cached_proposal_if_available();
    }

    fn start_processing(&self) {
        self.started.store(true, Ordering::SeqCst);
        self.retry_wants();
    }

    fn is_catching_up(&self) -> bool {
        let committed = self.config.store.height();
        !lock(&self.state).unfinished_heights(committed).is_empty()
    }
}

impl p2p::Reactor for Reactor {
    fn name(&self) -> String {
        "Recovery".into()
    }

    fn channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            ChannelDescriptor {
                id: WANT_CHANNEL,
                priority: 145,
                send_queue_capacity: 20_000,
                recv_message_capacity: MAX_MSG_SIZE,
                ..ChannelDescriptor::new(WANT_CHANNEL, 145)
            }
            .fill_defaults(),
            ChannelDescriptor {
                id: DATA_CHANNEL,
                priority: 140,
                send_queue_capacity: 20_000,
                recv_message_capacity: MAX_MSG_SIZE,
                ..ChannelDescriptor::new(DATA_CHANNEL, 140)
            }
            .fill_defaults(),
        ]
    }

    fn set_switch(&self, switch: WeakSwitch) {
        *lock(&self.switch) = switch;
    }

    fn on_start(self: Arc<Self>) -> Result<(), String> {
        tokio::spawn(self.clone().retry_routine());
        Ok(())
    }

    fn on_stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.retry_notify.notify_one();
    }

    fn init_peer(&self, peer: &Arc<Peer>) -> Result<(), String> {
        if peer.id() == self.self_id {
            return Err("cannot connect to self".into());
        }
        if Self::is_legacy_propagation(peer.node_info()) {
            return Err("peer is only using legacy propagation".into());
        }
        if lock(&self.state).peers.contains_key(&peer.id()) {
            return Err("peer already exists".into());
        }
        Ok(())
    }

    fn add_peer(&self, peer: &Arc<Peer>) {
        let editor: Arc<dyn crate::PeerStateEditor> = if let Some(h) = peer
            .get(PEER_STATE_EDITOR_KEY)
            .and_then(|d| d.downcast::<EditorHandle>().ok())
        {
            h.0.clone()
        } else {
            tracing::error!(peer = %peer.id(), "failed to load consensus peer state");
            Arc::new(NoOpEditor)
        };
        let mut st = lock(&self.state);
        st.peers
            .insert(peer.id(), PeerState::new(peer.clone(), editor));
        let (h, r) = (st.height, st.round);
        let Some(pd) = st.cached(h, r) else {
            return;
        };
        if pd.compact_block.parts_hashes.is_empty() {
            return;
        }
        let bz = wrap(&Message::CompactBlock(Box::new(
            (*pd.compact_block).clone(),
        )));
        if !peer.try_send(DATA_CHANNEL, bz) {
            tracing::debug!(peer = %peer.id(), "failed to send proposal to peer");
        }
    }

    fn remove_peer(&self, peer: &Arc<Peer>, _reason: &str) {
        lock(&self.state).peers.remove(&peer.id());
    }

    fn receive(&self, e: Envelope) -> Result<(), String> {
        let msg = Message::decode(&e.message).map_err(|err| {
            tracing::error!(src = %e.src.id(), ch = e.channel_id, err = %err, "Error decoding message");
            err
        })?;
        let from = e.src.id();
        match (e.channel_id, msg) {
            (DATA_CHANNEL, Message::CompactBlock(cb)) => {
                self.handle_compact_block(Arc::new(*cb), from, false);
            }
            (DATA_CHANNEL, Message::HaveParts(h)) => self.handle_haves(from, &h),
            (DATA_CHANNEL, Message::RecoveryPart(p)) => self.handle_recovery_part(from, &p),
            (WANT_CHANNEL, Message::WantParts(w)) => self.handle_wants(from, &w),
            (ch, m) => {
                tracing::error!(ch, msg = ?std::mem::discriminant(&m), "Unknown message type");
            }
        }
        self.run_deferred_stops();
        Ok(())
    }

    fn queue_size(&self) -> usize {
        REACTOR_INCOMING_MESSAGE_QUEUE_SIZE
    }
}

/// Test hook: the reactor's view of a part set (cached proposals only).
impl Reactor {
    /// The combined part-set bit array of a cached proposal.
    #[must_use]
    pub fn cached_parts(&self, height: i64, round: i32) -> Option<BitArray> {
        lock(&self.state)
            .cached(height, round)
            .map(|pd| pd.block.bit_array().clone())
    }

    /// Weak self (for spawned tasks).
    #[must_use]
    pub fn weak(&self) -> Weak<Self> {
        self.me.clone()
    }
}
