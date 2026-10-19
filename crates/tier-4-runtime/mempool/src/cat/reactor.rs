//! `mempool/cat/reactor.go`: the CAT gossip protocol.
//!
//! - A node that accepts a tx (RPC, or from a peer after CheckTx) sends
//!   `SeenTx{key, signer, sequence}` on [`MEMPOOL_DATA_CHANNEL`] to up to 15
//!   peers ranked per signer (sticky peers), plus up to
//!   `max_persistent_sticky_peers` persistent peers beyond that.
//! - A node receiving `SeenTx` for a tx it lacks asks the application for
//!   the signer's expected sequence (`QuerySequence`): at the expected
//!   sequence it sends `WantTx` on [`MEMPOOL_WANTS_CHANNEL`]; ahead of it,
//!   the announcement is queued per signer until the gap closes.
//! - `WantTx` is answered with `Txs` carrying the tx; unrequested `Txs` are
//!   dropped (pull only). Requests time out after `max_gossip_delay` and
//!   move to another peer that announced the tx.
//!
//! Invalid messages (bad key length, missing / oversized signer, empty or
//! batched `Txs`) disconnect the peer: [`Reactor::receive`] returns `Err`.

use std::collections::HashSet;
use std::sync::{Arc, RwLock, Weak};
use std::time::Duration;

use celestia_core_proto::tendermint::mempool::{Message, SeenTx, Txs, WantTx, message::Sum};
use p2p::{ChannelDescriptor, Envelope, Peer, WeakSwitch};
use prost::Message as _;

use super::peers::{MempoolIds, MempoolPeer, PEER_STATE_KEY, PeerState, peer_state_of};
use super::pool::TxPool;
use super::received_buffer::ReceivedTxBuffer;
use super::requests::{DEFAULT_GLOBAL_REQUEST_TIMEOUT, OnTimeout, RequestScheduler};
use super::scan::{ERR_TOO_MANY_TXS, validate_mempool_bytes};
use super::seen::SeenEntry;
use super::sticky::select_sticky_peers;
use super::tx::WrappedTx;
use super::{MAX_RECEIVED_BUFFER_SIZE, MAX_REQUESTS_PER_PEER};
use crate::errors::MempoolError;
use crate::types::{CachedTx, TxInfo, TxKey};
use crate::{MAX_TXS_PER_MESSAGE, MEMPOOL_CHANNEL};

/// `DefaultGossipDelay`.
pub const DEFAULT_GOSSIP_DELAY: Duration = Duration::from_secs(60);
/// `MempoolDataChannel`: SeenTx and Txs.
pub const MEMPOOL_DATA_CHANNEL: u8 = 0x31;
/// `MempoolWantsChannel`: WantTx.
pub const MEMPOOL_WANTS_CHANNEL: u8 = 0x32;
/// `peerHeightDiff`.
pub const PEER_HEIGHT_DIFF: i64 = 10;
/// `ReactorIncomingMessageQueueSize`.
pub const REACTOR_INCOMING_MESSAGE_QUEUE_SIZE: usize = 5000;
/// `maxSeenTxBroadcast`.
pub const MAX_SEEN_TX_BROADCAST: usize = 15;
/// `defaultMaxPersistentStickyPeers`.
pub const DEFAULT_MAX_PERSISTENT_STICKY_PEERS: i32 = 4;
/// `maxSignerLength`.
pub const MAX_SIGNER_LENGTH: usize = 64;
/// Default `MaxTxSize` (`DefaultMempoolConfig().MaxTxBytes`).
pub const DEFAULT_MAX_TX_SIZE: i64 = 1024 * 1024;

/// `errSignerTooLong`.
pub const ERR_SIGNER_TOO_LONG: &str = "signer field exceeds maximum length";
/// `errMissingSigner`.
pub const ERR_MISSING_SIGNER: &str = "SeenTx missing required signer field";
/// `errEmptyTx`.
pub const ERR_EMPTY_TX: &str = "txs message contains an empty transaction";

/// `ReactorOptions`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReactorOptions {
    /// Never broadcast txs.
    pub listen_only: bool,
    /// Maximum tx size (0: 1 MiB).
    pub max_tx_size: i64,
    /// Time to wait for a requested tx (zero: [`DEFAULT_GOSSIP_DELAY`]).
    pub max_gossip_delay: Duration,
    /// Salt of the sticky peer ranking (empty: the shared default order).
    pub sticky_peer_salt: Vec<u8>,
    /// Persistent peers guaranteed a SeenTx per signer (<= 0: 4).
    pub max_persistent_sticky_peers: i32,
}

impl ReactorOptions {
    /// `VerifyAndComplete`.
    pub fn verify_and_complete(&mut self) -> Result<(), String> {
        if self.max_tx_size == 0 {
            self.max_tx_size = DEFAULT_MAX_TX_SIZE;
        }
        if self.max_gossip_delay.is_zero() {
            self.max_gossip_delay = DEFAULT_GOSSIP_DELAY;
        }
        if self.max_persistent_sticky_peers <= 0 {
            self.max_persistent_sticky_peers = DEFAULT_MAX_PERSISTENT_STICKY_PEERS;
        }
        if self.max_tx_size < 0 {
            return Err(format!(
                "max tx size ({}) cannot be negative",
                self.max_tx_size
            ));
        }
        Ok(())
    }
}

fn encode(sum: Sum) -> Vec<u8> {
    Message { sum: Some(sum) }.encode_to_vec()
}

fn want_tx_msg(key: &TxKey) -> Vec<u8> {
    encode(Sum::WantTx(WantTx {
        tx_key: key.to_vec(),
    }))
}

/// `types.TxKeyFromBytes`.
fn tx_key_from_bytes(b: &[u8]) -> Result<TxKey, String> {
    b.try_into()
        .map_err(|_| format!("incorrect tx key size. Expected 32 bytes, got {}", b.len()))
}

/// Size of `Message{Txs{[tx of n bytes]}}`.
fn txs_msg_size(n: usize) -> usize {
    let inner = 1 + prost::encoding::encoded_len_varint(n as u64) + n;
    1 + prost::encoding::encoded_len_varint(inner as u64) + inner
}

/// The CAT reactor.
pub struct Reactor {
    opts: ReactorOptions,
    pub(crate) mempool: Arc<TxPool>,
    pub(crate) ids: MempoolIds,
    pub(crate) requests: RequestScheduler,
    pub(crate) received_buffer: ReceivedTxBuffer,
    sticky_salt: RwLock<Vec<u8>>,
    me: Weak<Reactor>,
    quit: tokio::sync::watch::Sender<bool>,
}

impl std::fmt::Debug for Reactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reactor")
            .field("opts", &self.opts)
            .field("peers", &self.ids.len())
            .finish_non_exhaustive()
    }
}

impl Reactor {
    /// `NewReactor`.
    pub fn new(mempool: Arc<TxPool>, mut opts: ReactorOptions) -> Result<Arc<Self>, String> {
        opts.verify_and_complete()?;
        let (quit, _) = tokio::sync::watch::channel(false);
        Ok(Arc::new_cyclic(|me| Self {
            requests: RequestScheduler::new(opts.max_gossip_delay, DEFAULT_GLOBAL_REQUEST_TIMEOUT),
            sticky_salt: RwLock::new(opts.sticky_peer_salt.clone()),
            opts,
            mempool,
            ids: MempoolIds::new(),
            received_buffer: ReceivedTxBuffer::new(),
            me: me.clone(),
            quit,
        }))
    }

    /// The options (after `VerifyAndComplete`).
    #[must_use]
    pub fn options(&self) -> &ReactorOptions {
        &self.opts
    }

    /// The pool.
    #[must_use]
    pub fn mempool(&self) -> &Arc<TxPool> {
        &self.mempool
    }

    /// `SetStickySalt`.
    pub fn set_sticky_salt(&self, salt: &[u8]) {
        *self
            .sticky_salt
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = salt.to_vec();
    }

    fn sticky_salt(&self) -> Vec<u8> {
        self.sticky_salt
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// `OnStart`: spawns the broadcast loop for RPC txs (unless listen
    /// only) and the height-signal loop. Needs a tokio runtime.
    pub fn start(&self) {
        let pool = self.mempool.clone();
        if self.opts.listen_only {
            tracing::info!("Tx broadcasting is disabled");
        } else {
            let me = self.me.clone();
            let mut quit = self.quit.subscribe();
            let pool = pool.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = quit.changed() => return,
                        w = pool.next_to_broadcast() => {
                            let Some(r) = me.upgrade() else { return };
                            r.broadcast_new_tx(&w);
                        }
                    }
                }
            });
        }
        let me = self.me.clone();
        let mut quit = self.quit.subscribe();
        let signal = pool.height_signal();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = quit.changed() => return,
                    () = signal.notified() => {
                        let Some(r) = me.upgrade() else { return };
                        let _ = tokio::task::spawn_blocking(move || r.refresh_pending_seen_queues()).await;
                    }
                }
            }
        });
    }

    /// `OnStop`.
    pub fn stop(&self) {
        let _ = self.quit.send(true);
        self.requests.close();
    }

    /// `GetChannels`.
    #[must_use]
    pub fn channels(&self) -> Vec<ChannelDescriptor> {
        let max_tx = usize::try_from(self.opts.max_tx_size).unwrap_or(0);
        let tx_msg = txs_msg_size(max_tx);
        // Message{SeenTx{tx_key: 32 bytes}}
        let state_msg = 1 + 1 + (1 + 1 + 32);
        vec![
            ChannelDescriptor {
                send_queue_capacity: 10,
                recv_message_capacity: tx_msg,
                ..ChannelDescriptor::new(MEMPOOL_CHANNEL, 1)
            },
            ChannelDescriptor {
                send_queue_capacity: 1000,
                recv_message_capacity: tx_msg,
                ..ChannelDescriptor::new(MEMPOOL_DATA_CHANNEL, 3)
            },
            ChannelDescriptor {
                send_queue_capacity: 1000,
                recv_message_capacity: state_msg,
                ..ChannelDescriptor::new(MEMPOOL_WANTS_CHANNEL, 3)
            },
        ]
    }

    /// `InitPeer`.
    pub fn init_peer(&self, peer: Arc<dyn MempoolPeer>) {
        self.ids.reserve_for_peer(peer);
    }

    /// `RemovePeer`: forgets the peer and re-requests its outstanding txs
    /// elsewhere.
    pub fn remove_peer(&self, peer_id: &str) {
        let id = self.ids.reclaim(peer_id);
        self.mempool.seen_tracker.remove_peer(id);
        for key in self.requests.clear_all_requests_from(id) {
            self.find_new_peer_to_request_tx(key);
        }
    }

    /// `Receive`: handles one message from `src` on `ch`. `Err` means the
    /// peer must be disconnected (Go's `StopPeerForError`).
    pub fn receive(&self, src: &Arc<dyn MempoolPeer>, ch: u8, msg: &[u8]) -> Result<(), String> {
        if ch == MEMPOOL_CHANNEL || ch == MEMPOOL_DATA_CHANNEL {
            validate_mempool_bytes(msg)?;
        }
        let m = celestia_core_proto::gogo_decode::<Message>(msg).map_err(|e| e.to_string())?;
        match m.sum {
            None => Err("unknown message: <nil>".into()),
            Some(Sum::Txs(txs)) => self.receive_txs(src, txs),
            Some(Sum::SeenTx(seen)) => self.receive_seen_tx(src, &seen),
            Some(Sum::WantTx(want)) => self.receive_want_tx(src, &want),
        }
    }

    fn receive_txs(&self, src: &Arc<dyn MempoolPeer>, msg: Txs) -> Result<(), String> {
        if msg.txs.is_empty() {
            tracing::error!(src = %src.id(), "received empty txs from peer");
            return Err(ERR_EMPTY_TX.into());
        }
        if msg.txs.len() > MAX_TXS_PER_MESSAGE {
            tracing::error!(count = msg.txs.len(), src = %src.id(), "received too many txs from peer");
            return Err(ERR_TOO_MANY_TXS.into());
        }
        let tx = msg.txs.into_iter().next().unwrap_or_default();
        self.handle_received_tx(tx, src)
    }

    /// `handleReceivedTx`.
    fn handle_received_tx(&self, tx: Vec<u8>, src: &Arc<dyn MempoolPeer>) -> Result<(), String> {
        if tx.is_empty() {
            tracing::error!(src = %src.id(), "received empty tx from peer");
            return Err(ERR_EMPTY_TX.into());
        }
        let src_id = src.id();
        let peer_id = self.ids.get_id_for_peer(&src_id);
        let info = TxInfo {
            sender_id: peer_id,
            sender_p2p_id: src_id.clone(),
            from_broadcast: false,
        };
        let cached = CachedTx::new(tx);
        let key = cached.key();
        if !self.requests.has(peer_id, &key) {
            tracing::debug!(peer_id, tx = %hex::encode_upper(key), "dropping unrequested tx from peer");
            // The peer proved it has the tx.
            self.mempool.peer_has_tx(peer_id, key);
            return Ok(());
        }
        self.requests.mark_received(peer_id, &key);
        if self.process_now_or_buffer(&cached, key, &info, &src_id) {
            self.process_received_tx(cached, key, &info, &src_id);
        }
        Ok(())
    }

    /// `processNowOrBuffer`: true if the tx can be checked now; otherwise it
    /// is buffered (or dropped).
    fn process_now_or_buffer(
        &self,
        cached: &CachedTx,
        key: TxKey,
        info: &TxInfo,
        src_id: &str,
    ) -> bool {
        let Some((signer, sequence)) = self.mempool.seen_tracker.pending_tx_info(&key) else {
            return true;
        };
        let Some(expected) = self.query_sequence(&signer) else {
            return true;
        };
        if sequence == expected {
            return true;
        }
        if sequence < expected {
            tracing::debug!(tx = %hex::encode_upper(key), sequence, expected, "dropping tx with stale sequence");
            self.mempool.seen_tracker.remove_key(&key);
            return false;
        }
        if sequence <= expected.wrapping_add(MAX_RECEIVED_BUFFER_SIZE as u64)
            && self.received_buffer.add(
                &signer,
                sequence,
                cached.clone(),
                key,
                info.clone(),
                src_id,
            )
        {
            self.mempool.seen_tracker.clear_pending_tx(&key);
        }
        false
    }

    fn receive_seen_tx(&self, src: &Arc<dyn MempoolPeer>, msg: &SeenTx) -> Result<(), String> {
        let key = match tx_key_from_bytes(&msg.tx_key) {
            Ok(k) => k,
            Err(e) => {
                tracing::error!(err = %e, "peer sent SeenTx with incorrect tx key");
                return Err(e);
            }
        };
        if msg.signer.is_empty() {
            tracing::error!(tx = %hex::encode_upper(key), "peer sent SeenTx without required signer");
            return Err(ERR_MISSING_SIGNER.into());
        }
        if msg.signer.len() > MAX_SIGNER_LENGTH {
            tracing::error!(
                len = msg.signer.len(),
                "peer sent SeenTx with signer too long"
            );
            return Err(ERR_SIGNER_TOO_LONG.into());
        }
        let peer_id = self.ids.get_id_for_peer(&src.id());
        self.mempool.peer_has_tx(peer_id, key);
        if self.mempool.has(&key) {
            return Ok(());
        }
        if self.requests.for_tx(&key) != 0 {
            return Ok(());
        }
        let Some(expected) = self.query_sequence(&msg.signer) else {
            tracing::error!(signer = %hex::encode_upper(&msg.signer), "no signer sequence found in application");
            return Ok(());
        };
        match msg.sequence.cmp(&expected) {
            std::cmp::Ordering::Equal => {}
            std::cmp::Ordering::Greater => {
                // Hold it until the gap closes.
                self.mempool
                    .seen_tracker
                    .add_pending_tx(key, peer_id, &msg.signer, msg.sequence);
                return Ok(());
            }
            std::cmp::Ordering::Less => {
                tracing::debug!(tx = %hex::encode_upper(key), sequence = msg.sequence, expected, "dropping SeenTx due to lower than expected sequence");
                return Ok(());
            }
        }
        if !self.request_tx(key, src) && self.requests.for_tx(&key) == 0 {
            tracing::debug!(peer_id, tx = %hex::encode_upper(key), "tx not requested, queueing SeenTx for retry");
            self.mempool
                .seen_tracker
                .add_pending_tx(key, peer_id, &msg.signer, msg.sequence);
        }
        Ok(())
    }

    fn receive_want_tx(&self, src: &Arc<dyn MempoolPeer>, msg: &WantTx) -> Result<(), String> {
        let key = match tx_key_from_bytes(&msg.tx_key) {
            Ok(k) => k,
            Err(e) => {
                tracing::error!(err = %e, "peer sent WantTx with incorrect tx key");
                return Err(e);
            }
        };
        if let Some(tx) = self.mempool.get_tx_by_key(&key)
            && !self.opts.listen_only
        {
            let peer_id = self.ids.get_id_for_peer(&src.id());
            tracing::trace!(peer_id, "sending a tx in response to a want msg");
            let out = encode(Sum::Txs(Txs {
                txs: vec![tx.tx().to_vec()],
            }));
            if src.send(MEMPOOL_DATA_CHANNEL, out) {
                self.mempool.peer_has_tx(peer_id, key);
            }
        }
        Ok(())
    }

    /// `broadcastSeenTx` at the pool's height.
    pub(crate) fn broadcast_seen_tx(&self, key: TxKey, signer: &[u8], sequence: u64) {
        self.broadcast_seen_tx_with_height(key, self.mempool.height(), signer, sequence);
    }

    /// `broadcastNewTx`.
    pub(crate) fn broadcast_new_tx(&self, w: &WrappedTx) {
        self.broadcast_seen_tx_with_height(w.key(), w.height, &w.sender, w.sequence);
    }

    /// `broadcastSeenTxWithHeight`: the natural top
    /// [`MAX_SEEN_TX_BROADCAST`] sticky peers plus up to
    /// `max_persistent_sticky_peers` persistent peers beyond them; peers more
    /// than [`PEER_HEIGHT_DIFF`] blocks behind or known to have the tx are
    /// skipped.
    pub(crate) fn broadcast_seen_tx_with_height(
        &self,
        key: TxKey,
        height: i64,
        signer: &[u8],
        sequence: u64,
    ) {
        tracing::debug!(tx_key = %hex::encode_upper(key), "broadcasting seen tx to limited peers");
        let msg = encode(Sum::SeenTx(SeenTx {
            tx_key: key.to_vec(),
            sequence,
            signer: signer.to_vec(),
        }));
        let peers = self.ids.get_all();
        if peers.is_empty() {
            return;
        }
        let ordered = select_sticky_peers(signer, &peers, peers.len(), &self.sticky_salt());
        let max_persistent = usize::try_from(self.opts.max_persistent_sticky_peers).unwrap_or(0);
        let (mut sent, mut sent_persistent) = (0usize, 0usize);
        for p in ordered {
            let persistent = p.peer.is_persistent();
            let can_send =
                sent < MAX_SEEN_TX_BROADCAST || (persistent && sent_persistent < max_persistent);
            if !can_send {
                continue;
            }
            if let Some(ps) = p.peer.peer_state()
                && ps.get_height() < height.wrapping_sub(PEER_HEIGHT_DIFF)
            {
                tracing::trace!("peer is too far behind us. Skipping broadcast of seen tx");
                continue;
            }
            if self.mempool.seen_tracker.has(&key, p.id) {
                continue;
            }
            if p.peer.send(MEMPOOL_DATA_CHANNEL, msg.clone()) {
                self.mempool.peer_has_tx(p.id, key);
                sent += 1;
                if persistent {
                    sent_persistent += 1;
                }
            }
        }
    }

    /// `requestTx`: reserves the request, then sends `WantTx`. True only if
    /// it was sent.
    pub(crate) fn request_tx(&self, key: TxKey, peer: &Arc<dyn MempoolPeer>) -> bool {
        let peer_id = self.ids.get_id_for_peer(&peer.id());
        let me = self.me.clone();
        let on_timeout: OnTimeout = Arc::new(move |k, p| {
            if let Some(r) = me.upgrade() {
                r.on_request_timeout(k, p);
            }
        });
        if !self.requests.add(key, peer_id, Some(on_timeout)) {
            tracing::trace!(tx = %hex::encode_upper(key), peer = %peer.id(), "tx already requested or peer at capacity");
            return false;
        }
        if !peer.try_send(MEMPOOL_WANTS_CHANNEL, want_tx_msg(&key)) {
            // The request may have been reassigned meanwhile.
            if !self.requests.remove(&key, peer_id) {
                return false;
            }
            self.mempool.seen_tracker.remove_peer_from_tx(&key, peer_id);
            self.find_new_peer_to_request_tx(key);
            return false;
        }
        true
    }

    /// `tryAddNewTx` (with Go's trace / log side effects as logs).
    fn try_add_new_tx(
        &self,
        tx: CachedTx,
        key: TxKey,
        info: &TxInfo,
    ) -> Result<celestia_core_proto::abci::ResponseCheckTx, MempoolError> {
        let res = self.mempool.try_add_new_tx(tx, key, info);
        match &res {
            Err(e) if !e.is_tx_in_mempool() => {
                tracing::debug!(tx = %hex::encode_upper(key), err = %e, "Could not add tx");
            }
            Ok(rsp) if rsp.code != 0 => {
                tracing::debug!(tx = %hex::encode_upper(key), code = rsp.code, log = %rsp.log, "tx rejected by CheckTx");
            }
            _ => {}
        }
        res
    }

    /// `processReceivedTx`: CheckTx, then drain the signer's buffered and
    /// queued txs, then announce.
    fn process_received_tx(&self, tx: CachedTx, key: TxKey, info: &TxInfo, _src_id: &str) {
        let res = self.try_add_new_tx(tx, key, info);
        if res.as_ref().is_ok() || res.as_ref().is_err_and(MempoolError::is_tx_in_mempool) {
            self.mempool.seen_tracker.clear_pending_tx(&key);
        }
        let Ok(rsp) = res else { return };
        if !rsp.address.is_empty() {
            self.process_received_buffer(&rsp.address);
            self.process_pending_seen_for_signer(&rsp.address);
        }
        if !self.opts.listen_only && rsp.code == 0 {
            self.broadcast_seen_tx(key, &rsp.address, rsp.sequence);
        }
    }

    /// `processReceivedBuffer`: checks buffered txs while they match the
    /// expected sequence.
    pub(crate) fn process_received_buffer(&self, signer: &[u8]) {
        if signer.is_empty() {
            return;
        }
        while let Some(expected) = self.query_sequence(signer) {
            let Some(b) = self.received_buffer.get(signer, expected) else {
                break;
            };
            self.received_buffer.remove_lower_seqs(signer, expected);
            let res = self.try_add_new_tx(b.tx, b.tx_key, &b.tx_info);
            if res.as_ref().is_ok() || res.as_ref().is_err_and(MempoolError::is_tx_in_mempool) {
                self.mempool.seen_tracker.clear_pending_tx(&b.tx_key);
            }
            let Ok(rsp) = res else { break };
            if !self.opts.listen_only && rsp.code == 0 {
                self.broadcast_seen_tx(b.tx_key, signer, expected);
            }
        }
    }

    /// `processPendingSeenForSigner`: requests consecutive queued sequences
    /// starting at the expected one (up to [`MAX_RECEIVED_BUFFER_SIZE`]).
    pub(crate) fn process_pending_seen_for_signer(&self, signer: &[u8]) {
        if signer.is_empty() {
            return;
        }
        let entries = self.mempool.seen_tracker.pending_txs_for_signer(signer);
        if entries.is_empty() {
            return;
        }
        let Some(expected) = self.query_sequence(signer) else {
            tracing::error!("no signer found in application");
            return;
        };
        let mut next = expected;
        let mut requested = 0usize;
        for entry in &entries {
            let Some(seq) = entry.pending_tx_info.as_ref().map(|i| i.sequence) else {
                continue;
            };
            if seq < expected {
                self.mempool.seen_tracker.clear_pending_tx(&entry.tx_key);
                continue;
            }
            if seq != next {
                break;
            }
            if requested >= MAX_RECEIVED_BUFFER_SIZE {
                break;
            }
            if self.mempool.has(&entry.tx_key) {
                self.mempool.seen_tracker.clear_pending_tx(&entry.tx_key);
                next = next.wrapping_add(1);
                continue;
            }
            if self.requests.for_tx(&entry.tx_key) != 0 {
                next = next.wrapping_add(1);
                requested += 1;
                continue;
            }
            if self.try_request_queued_tx(entry) {
                requested += 1;
            }
            next = next.wrapping_add(1);
        }
        if requested > 0 {
            tracing::trace!(count = requested, "parallel requests sent");
        }
    }

    /// `tryRequestQueuedTx`.
    pub(crate) fn try_request_queued_tx(&self, entry: &SeenEntry) -> bool {
        for peer_id in entry.peer_ids() {
            if self.requests.count_for_peer(peer_id) >= MAX_REQUESTS_PER_PEER {
                continue;
            }
            let Some(peer) = self.ids.get_peer(peer_id) else {
                continue;
            };
            if self.request_tx(entry.tx_key, &peer) {
                return true;
            }
            // Another requester reserved it first.
            if self.requests.for_tx(&entry.tx_key) != 0 {
                return true;
            }
        }
        self.find_new_peer_to_request_tx(entry.tx_key);
        self.requests.for_tx(&entry.tx_key) != 0
    }

    /// `onRequestTimeout`.
    fn on_request_timeout(&self, key: TxKey, peer: u16) {
        self.mempool.seen_tracker.remove_peer_from_tx(&key, peer);
        self.find_new_peer_to_request_tx(key);
    }

    /// `refreshPendingSeenQueues` (on every height signal).
    pub(crate) fn refresh_pending_seen_queues(&self) {
        self.mempool.seen_tracker.prune_expired();
        let mut signers: HashSet<Vec<u8>> = HashSet::new();
        signers.extend(self.mempool.seen_tracker.signers_with_pending_txs());
        signers.extend(self.received_buffer.signer_keys());
        for signer in signers {
            self.process_received_buffer(&signer);
            self.process_pending_seen_for_signer(&signer);
        }
    }

    /// `querySequenceFromApplication`.
    fn query_sequence(&self, signer: &[u8]) -> Option<u64> {
        self.mempool.app().query_sequence(signer).ok()
    }

    /// `findNewPeerToRequestTx`.
    pub(crate) fn find_new_peer_to_request_tx(&self, key: TxKey) {
        if self.ids.is_empty() {
            return;
        }
        let peers = self
            .mempool
            .seen_tracker
            .peers_for_tx(&key)
            .unwrap_or_default();
        for peer_id in peers {
            let Some(peer) = self.ids.get_peer(peer_id) else {
                continue;
            };
            if self.request_tx(key, &peer) {
                return;
            }
            if self.requests.for_tx(&key) != 0 {
                return;
            }
        }
        tracing::trace!(tx = %hex::encode_upper(key), "no other peer has the tx we are looking for");
    }
}

/// A `p2p::Peer` seen through [`MempoolPeer`].
pub struct P2pPeer(pub Arc<Peer>);

impl MempoolPeer for P2pPeer {
    fn id(&self) -> String {
        self.0.id().to_string()
    }

    fn is_persistent(&self) -> bool {
        self.0.is_persistent()
    }

    fn peer_state(&self) -> Option<Arc<dyn PeerState>> {
        peer_state_of(self.0.get(PEER_STATE_KEY))
    }

    /// `Send`: queues now if possible, otherwise waits for space in the
    /// background (Go blocks the caller for up to 10s).
    fn send(&self, ch: u8, msg: Vec<u8>) -> bool {
        if !self.0.is_running() || !self.0.node_info().has_channel(ch) {
            return false;
        }
        if self.0.can_send(ch) && self.0.try_send(ch, msg.clone()) {
            return true;
        }
        let Ok(h) = tokio::runtime::Handle::try_current() else {
            return false;
        };
        let peer = self.0.clone();
        h.spawn(async move {
            peer.send(ch, msg).await;
        });
        true
    }

    fn try_send(&self, ch: u8, msg: Vec<u8>) -> bool {
        self.0.try_send(ch, msg)
    }
}

/// The reactor plugged into a [`p2p::Switch`] (registered as `MEMPOOL`).
pub struct P2pReactor {
    inner: Arc<Reactor>,
    switch: RwLock<Option<WeakSwitch>>,
}

impl std::fmt::Debug for P2pReactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(f)
    }
}

impl P2pReactor {
    /// Wraps a reactor.
    #[must_use]
    pub fn new(inner: Arc<Reactor>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            switch: RwLock::new(None),
        })
    }

    /// The reactor.
    #[must_use]
    pub fn reactor(&self) -> &Arc<Reactor> {
        &self.inner
    }

    /// The switch, once added.
    #[must_use]
    pub fn switch(&self) -> Option<WeakSwitch> {
        self.switch
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// Runs blocking reactor work without stalling a multi-thread runtime.
fn blocking<R>(f: impl FnOnce() -> R) -> R {
    match tokio::runtime::Handle::try_current() {
        Ok(h) if h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(f)
        }
        _ => f(),
    }
}

impl p2p::Reactor for P2pReactor {
    fn name(&self) -> String {
        "MEMPOOL".into()
    }

    fn channels(&self) -> Vec<ChannelDescriptor> {
        self.inner.channels()
    }

    fn set_switch(&self, switch: WeakSwitch) {
        *self
            .switch
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(switch);
    }

    fn on_start(self: Arc<Self>) -> Result<(), String> {
        self.inner.start();
        Ok(())
    }

    fn on_stop(&self) {
        self.inner.stop();
    }

    fn init_peer(&self, peer: &Arc<Peer>) -> Result<(), String> {
        self.inner.init_peer(Arc::new(P2pPeer(peer.clone())));
        Ok(())
    }

    fn remove_peer(&self, peer: &Arc<Peer>, _reason: &str) {
        let id = peer.id().to_string();
        blocking(|| self.inner.remove_peer(&id));
    }

    fn receive(&self, env: Envelope) -> Result<(), String> {
        let src: Arc<dyn MempoolPeer> = Arc::new(P2pPeer(env.src.clone()));
        blocking(|| self.inner.receive(&src, env.channel_id, &env.message))
    }

    fn queue_size(&self) -> usize {
        REACTOR_INCOMING_MESSAGE_QUEUE_SIZE
    }
}
