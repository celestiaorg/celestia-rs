//! The syncer (`statesync/syncer.go`): picks snapshots from the pool,
//! offers them to the app, fetches chunks from peers, applies them with
//! Go's result-code handling (retry, refetch, reject senders, reject
//! snapshot/format, abort) and verifies the restored app against the
//! light-client-verified header.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use celestia_core_proto::tendermint::abci;
use ctypes::Commit;
use ctypes::gofmt::hex_upper;

use crate::chunks::{Chunk, ChunkError, ChunkQueue, Wait};
use crate::conn::AppConn;
use crate::messages::{CHUNK_CHANNEL, Msg};
use crate::snapshots::{PeerRef, Snapshot, SnapshotPool};
use crate::state::State;
use crate::stateprovider::StateProvider;
use celestia_core_proto::tendermint::statesync as pb;

/// `minimumDiscoveryTime`.
pub const MINIMUM_DISCOVERY_TIME: Duration = Duration::from_secs(5);

/// Errors of a sync attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    /// `errAbort`.
    Abort,
    /// `errRetrySnapshot`.
    RetrySnapshot,
    /// `errRejectSnapshot`.
    RejectSnapshot,
    /// `errRejectFormat`.
    RejectFormat,
    /// `errRejectSender`.
    RejectSender,
    /// `errVerifyFailed`.
    VerifyFailed,
    /// `errTimeout`.
    Timeout,
    /// `errNoSnapshots`.
    NoSnapshots,
    /// `light.ErrNoWitnesses` from the state provider.
    NoWitnesses,
    /// The node is shutting down ([`Syncer::stop`]).
    Stopped,
    /// Any other error (Go message).
    Msg(String),
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Abort => f.write_str("state sync aborted"),
            Self::RetrySnapshot => f.write_str("retry snapshot"),
            Self::RejectSnapshot => f.write_str("snapshot was rejected"),
            Self::RejectFormat => f.write_str("snapshot format was rejected"),
            Self::RejectSender => f.write_str("snapshot sender was rejected"),
            Self::VerifyFailed => f.write_str("verification failed"),
            Self::Timeout => f.write_str("timed out waiting for chunk"),
            Self::NoSnapshots => f.write_str("no suitable snapshots found"),
            Self::NoWitnesses => f.write_str("no witnesses connected. please reset light client"),
            Self::Stopped => f.write_str("state sync stopped"),
            Self::Msg(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for SyncError {}

/// State sync settings (`config.StateSyncConfig`).
#[derive(Debug, Clone)]
pub struct Config {
    /// Parent directory for chunk files (empty = system temp dir).
    pub temp_dir: PathBuf,
    /// `chunk_request_timeout` (default 10s).
    pub chunk_request_timeout: Duration,
    /// `chunk_fetchers` (default 4).
    pub chunk_fetchers: u32,
    /// `max_snapshot_chunks` (default 100000).
    pub max_snapshot_chunks: u32,
    /// `chunkTimeout` while waiting for the next chunk (2 minutes).
    pub chunk_timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            temp_dir: PathBuf::new(),
            chunk_request_timeout: Duration::from_secs(10),
            chunk_fetchers: 4,
            max_snapshot_chunks: 100_000,
            chunk_timeout: crate::chunks::CHUNK_TIMEOUT,
        }
    }
}

/// Callback invoked when the app rejects a chunk sender.
pub type PeerRejectedFn = Arc<dyn Fn(&str) + Send + Sync>;

/// `syncer`.
pub struct Syncer {
    cfg: Config,
    state_provider: Arc<dyn StateProvider>,
    conn: Arc<dyn AppConn>,
    snapshots: Arc<SnapshotPool>,
    chunks: Mutex<Option<Arc<ChunkQueue>>>,
    on_peer_rejected: Mutex<Option<PeerRejectedFn>>,
    stopped: AtomicBool,
    wake: (Mutex<()>, Condvar),
}

impl std::fmt::Debug for Syncer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Syncer")
            .field("cfg", &self.cfg)
            .finish_non_exhaustive()
    }
}

fn snapshot_request_bytes() -> Vec<u8> {
    Msg::SnapshotsRequest.encode()
}

impl Syncer {
    /// `newSyncer`.
    #[must_use]
    pub fn new(
        cfg: Config,
        conn: Arc<dyn AppConn>,
        state_provider: Arc<dyn StateProvider>,
    ) -> Self {
        Self {
            cfg,
            state_provider,
            conn,
            snapshots: Arc::new(SnapshotPool::new()),
            chunks: Mutex::new(None),
            on_peer_rejected: Mutex::new(None),
            stopped: AtomicBool::new(false),
            wake: (Mutex::new(()), Condvar::new()),
        }
    }

    /// Stops a running (or later) [`Syncer::sync_any`] for node shutdown:
    /// discovery and chunk waits end and it returns
    /// [`SyncError::Stopped`]. Go stops the syncer with the reactor's
    /// `OnStop`; here a SIGTERM / SIGINT must also end discovery.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        {
            let _g = self
                .wake
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.wake.1.notify_all();
        }
        if let Some(q) = self.current_chunks() {
            q.notify();
        }
    }

    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    /// Sleeps up to `d`; `Err(Stopped)` when stopped meanwhile.
    fn sleep(&self, d: Duration) -> Result<(), SyncError> {
        let deadline = std::time::Instant::now() + d;
        let mut g = self
            .wake
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            if self.is_stopped() {
                return Err(SyncError::Stopped);
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return Ok(());
            }
            g = self
                .wake
                .1
                .wait_timeout(g, deadline - now)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }

    /// `setOnPeerRejected`.
    pub fn set_on_peer_rejected(&self, f: PeerRejectedFn) {
        if let Ok(mut g) = self.on_peer_rejected.lock() {
            *g = Some(f);
        }
    }

    /// The snapshot pool.
    #[must_use]
    pub fn snapshots(&self) -> &SnapshotPool {
        &self.snapshots
    }

    fn current_chunks(&self) -> Option<Arc<ChunkQueue>> {
        self.chunks.lock().ok().and_then(|g| g.clone())
    }

    /// `AddChunk`.
    pub fn add_chunk(&self, chunk: &Chunk) -> Result<bool, String> {
        let Some(q) = self.current_chunks() else {
            return Err("no state sync in progress".into());
        };
        q.add(chunk).map_err(|e| e.to_string())
    }

    /// `AddSnapshot`.
    pub fn add_snapshot(&self, peer: &PeerRef, snapshot: Snapshot) -> bool {
        let (h, f, hash) = (snapshot.height, snapshot.format, hex_upper(&snapshot.hash));
        let added = self.snapshots.add(peer, snapshot);
        if added {
            tracing::info!(height = h, format = f, hash = %hash, "Discovered new snapshot");
        }
        added
    }

    /// `AddPeer`: asks the peer for its snapshots.
    pub fn add_peer(&self, peer: &PeerRef) {
        tracing::debug!(peer = %peer.id(), "Requesting snapshots from peer");
        peer.send(crate::messages::SNAPSHOT_CHANNEL, snapshot_request_bytes());
    }

    /// `RemovePeer`.
    pub fn remove_peer(&self, peer_id: &str) {
        self.snapshots.remove_peer(peer_id);
    }

    /// `RejectPeer`.
    pub fn reject_peer(&self, peer_id: &str) {
        self.snapshots.reject_peer(peer_id);
    }

    /// `SyncAny`: restores the best snapshot, retrying others on failure.
    /// Blocks; `retry_hook` re-requests snapshots from peers.
    pub fn sync_any(
        &self,
        mut discovery_time: Duration,
        retry_hook: &dyn Fn(),
    ) -> Result<(State, Commit), SyncError> {
        if !discovery_time.is_zero() && discovery_time < MINIMUM_DISCOVERY_TIME {
            discovery_time = MINIMUM_DISCOVERY_TIME * 5;
        }
        if !discovery_time.is_zero() {
            tracing::info!(?discovery_time, "Discovering snapshots");
        }
        self.sleep(discovery_time)?;
        let mut snapshot: Option<Snapshot> = None;
        let mut chunks: Option<Arc<ChunkQueue>> = None;
        loop {
            if self.is_stopped() {
                return Err(SyncError::Stopped);
            }
            if snapshot.is_none() {
                snapshot = self.snapshots.best();
                chunks = None;
            }
            let Some(snap) = snapshot.clone() else {
                if discovery_time.is_zero() {
                    return Err(SyncError::NoSnapshots);
                }
                retry_hook();
                tracing::info!("Discovering snapshots for {discovery_time:?}");
                self.sleep(discovery_time)?;
                continue;
            };
            let queue = if let Some(q) = &chunks {
                Arc::clone(q)
            } else {
                let q = ChunkQueue::new(&snap, &self.cfg.temp_dir)
                    .map_err(|e| SyncError::Msg(format!("failed to create chunk queue: {e}")))?
                    .with_chunk_timeout(self.cfg.chunk_timeout);
                let q = Arc::new(q);
                chunks = Some(Arc::clone(&q));
                q
            };
            let mut snap_mut = snap.clone();
            match self.sync(&mut snap_mut, &queue) {
                Ok(r) => return Ok(r),
                Err(e @ (SyncError::Abort | SyncError::Stopped)) => {
                    if let Err(e) = queue.close() {
                        tracing::error!(err = %e, "Failed to clean up chunk queue");
                    }
                    return Err(e);
                }
                Err(SyncError::RetrySnapshot) => {
                    queue.retry_all();
                    tracing::info!(
                        height = snap.height,
                        format = snap.format,
                        "Retrying snapshot"
                    );
                    continue;
                }
                Err(SyncError::Timeout) => {
                    self.snapshots.reject(&snap);
                    tracing::error!(
                        height = snap.height,
                        "Timed out waiting for snapshot chunks, rejected snapshot"
                    );
                }
                Err(SyncError::RejectSnapshot) => {
                    self.snapshots.reject(&snap);
                    tracing::info!(
                        height = snap.height,
                        format = snap.format,
                        "Snapshot rejected"
                    );
                }
                Err(SyncError::RejectFormat) => {
                    self.snapshots.reject_format(snap.format);
                    tracing::info!(format = snap.format, "Snapshot format rejected");
                }
                Err(SyncError::RejectSender) => {
                    tracing::info!(height = snap.height, "Snapshot senders rejected");
                    for peer in self.snapshots.get_peers(&snap) {
                        self.snapshots.reject_peer(&peer.id());
                        tracing::info!(peer = %peer.id(), "Snapshot sender rejected");
                    }
                }
                Err(SyncError::NoWitnesses) => {
                    return Err(SyncError::Msg(format!(
                        "snapshot restoration failed: {}",
                        SyncError::NoWitnesses
                    )));
                }
                Err(e) => return Err(SyncError::Msg(format!("snapshot restoration failed: {e}"))),
            }
            if let Err(e) = queue.close() {
                tracing::error!(err = %e, "Failed to clean up chunk queue");
            }
            snapshot = None;
            chunks = None;
        }
    }

    /// `Sync`: restores one snapshot.
    pub fn sync(
        &self,
        snapshot: &mut Snapshot,
        chunks: &Arc<ChunkQueue>,
    ) -> Result<(State, Commit), SyncError> {
        {
            let mut g = self
                .chunks
                .lock()
                .map_err(|_| SyncError::Msg("poisoned".into()))?;
            if g.is_some() {
                return Err(SyncError::Msg("a state sync is already in progress".into()));
            }
            *g = Some(Arc::clone(chunks));
        }
        let res = self.sync_inner(snapshot, chunks);
        if let Ok(mut g) = self.chunks.lock() {
            *g = None;
        }
        res
    }

    fn light_err(e: &light::Error, what: &str) -> SyncError {
        tracing::info!(err = %e, "failed to fetch and verify {what}");
        if matches!(e, light::Error::NoWitnesses) {
            SyncError::NoWitnesses
        } else {
            SyncError::RejectSnapshot
        }
    }

    fn sync_inner(
        &self,
        snapshot: &mut Snapshot,
        chunks: &Arc<ChunkQueue>,
    ) -> Result<(State, Commit), SyncError> {
        let app_hash = self
            .state_provider
            .app_hash(snapshot.height)
            .map_err(|e| Self::light_err(&e, "app hash"))?;
        snapshot.trusted_app_hash = app_hash;
        let mut state = self
            .state_provider
            .state(snapshot.height)
            .map_err(|e| Self::light_err(&e, "CometBFT state"))?;
        snapshot.trusted_app_version = state.consensus_params.version.app;

        self.offer_snapshot(snapshot)?;

        let cancel = Arc::new(AtomicBool::new(false));
        let mut fetchers = Vec::new();
        for _ in 0..self.cfg.chunk_fetchers {
            let (q, s, c) = (Arc::clone(chunks), snapshot.clone(), Arc::clone(&cancel));
            let pool = Arc::clone(&self.snapshots);
            let retry = self.cfg.chunk_request_timeout;
            fetchers.push(std::thread::spawn(move || {
                fetch_chunks(&pool, &s, &q, &c, retry);
            }));
        }
        let stop = || {
            cancel.store(true, Ordering::SeqCst);
            chunks.notify();
        };

        let commit = match self.state_provider.commit(snapshot.height) {
            Ok(c) => c,
            Err(e) => {
                stop();
                return Err(Self::light_err(&e, "commit"));
            }
        };
        let applied = self.apply_chunks(chunks);
        stop();
        for f in fetchers {
            let _ = f.join();
        }
        applied?;

        let timeouts = self.verify_app(snapshot, state.version.app)?;
        state.timeouts = nodestore::Timeouts::from_abci(timeouts.as_ref())
            .map_err(|e| SyncError::Msg(format!("invalid app timeouts: {e}")))?;
        tracing::info!(height = snapshot.height, format = snapshot.format, hash = %hex_upper(&snapshot.hash), "Snapshot restored");
        Ok((state, commit))
    }

    fn offer_snapshot(&self, snapshot: &Snapshot) -> Result<(), SyncError> {
        use abci::response_offer_snapshot::Result as R;
        tracing::info!(height = snapshot.height, format = snapshot.format, hash = %hex_upper(&snapshot.hash), "Offering snapshot to ABCI app");
        let resp = self
            .conn
            .offer_snapshot(abci::RequestOfferSnapshot {
                snapshot: Some(abci::Snapshot {
                    height: snapshot.height,
                    format: snapshot.format,
                    chunks: snapshot.chunks,
                    hash: snapshot.hash.clone(),
                    metadata: snapshot.metadata.clone(),
                }),
                app_hash: snapshot.trusted_app_hash.clone(),
                app_version: snapshot.trusted_app_version,
            })
            .map_err(|e| SyncError::Msg(format!("failed to offer snapshot: {e}")))?;
        match R::try_from(resp.result) {
            Ok(R::Accept) => {
                tracing::info!(height = snapshot.height, "Snapshot accepted, restoring");
                Ok(())
            }
            Ok(R::Abort) => Err(SyncError::Abort),
            Ok(R::Reject) => Err(SyncError::RejectSnapshot),
            Ok(R::RejectFormat) => Err(SyncError::RejectFormat),
            Ok(R::RejectSender) => Err(SyncError::RejectSender),
            _ => Err(SyncError::Msg(format!(
                "unknown ResponseOfferSnapshot result {}",
                offer_result_name(resp.result)
            ))),
        }
    }

    fn apply_chunks(&self, chunks: &Arc<ChunkQueue>) -> Result<(), SyncError> {
        use abci::response_apply_snapshot_chunk::Result as R;
        loop {
            if self.is_stopped() {
                return Err(SyncError::Stopped);
            }
            let chunk = match chunks.next_until(Some(&self.stopped)) {
                Ok(c) => c,
                Err(ChunkError::Done) if self.is_stopped() => return Err(SyncError::Stopped),
                Err(ChunkError::Done) => return Ok(()),
                // `fmt.Errorf("failed to fetch chunk: %w", errTimeout)`:
                // SyncAny matches it with errors.Is
                Err(ChunkError::Timeout) => return Err(SyncError::Timeout),
                Err(e) => return Err(SyncError::Msg(format!("failed to fetch chunk: {e}"))),
            };
            let resp = self
                .conn
                .apply_snapshot_chunk(abci::RequestApplySnapshotChunk {
                    index: chunk.index,
                    chunk: chunk.chunk,
                    sender: chunk.sender.clone(),
                })
                .map_err(|e| {
                    SyncError::Msg(format!("failed to apply chunk {}: {e}", chunk.index))
                })?;
            tracing::info!(
                height = chunk.height,
                format = chunk.format,
                chunk = chunk.index,
                total = chunks.size(),
                "Applied snapshot chunk to ABCI app"
            );
            for index in &resp.refetch_chunks {
                chunks
                    .discard(*index)
                    .map_err(|e| SyncError::Msg(format!("failed to discard chunk {index}: {e}")))?;
            }
            for sender in &resp.reject_senders {
                if sender.is_empty() {
                    continue;
                }
                self.snapshots.reject_peer(sender);
                let cb = self.on_peer_rejected.lock().ok().and_then(|g| g.clone());
                if let Some(cb) = cb {
                    cb(sender);
                }
                chunks
                    .discard_sender(sender)
                    .map_err(|e| SyncError::Msg(format!("failed to reject sender: {e}")))?;
            }
            match R::try_from(resp.result) {
                Ok(R::Accept) => {}
                Ok(R::Abort) => return Err(SyncError::Abort),
                Ok(R::Retry) => chunks.retry(chunk.index),
                Ok(R::RetrySnapshot) => return Err(SyncError::RetrySnapshot),
                Ok(R::RejectSnapshot) => return Err(SyncError::RejectSnapshot),
                _ => {
                    return Err(SyncError::Msg(format!(
                        "unknown ResponseApplySnapshotChunk result {}",
                        apply_result_name(resp.result)
                    )));
                }
            }
        }
    }

    fn verify_app(
        &self,
        snapshot: &Snapshot,
        app_version: u64,
    ) -> Result<Option<abci::TimeoutInfo>, SyncError> {
        let resp = self
            .conn
            .info(request_info())
            .map_err(|e| SyncError::Msg(format!("failed to query ABCI app for appHash: {e}")))?;
        if resp.app_version != app_version {
            return Err(SyncError::Msg(format!(
                "app version mismatch. Expected: {app_version}, got: {}",
                resp.app_version
            )));
        }
        if snapshot.trusted_app_hash != resp.last_block_app_hash {
            tracing::error!(expected = %hex_upper(&snapshot.trusted_app_hash), actual = %hex_upper(&resp.last_block_app_hash), "appHash verification failed");
            return Err(SyncError::VerifyFailed);
        }
        if u64::try_from(resp.last_block_height).ok() != Some(snapshot.height) {
            tracing::error!(
                expected = snapshot.height,
                actual = resp.last_block_height,
                "ABCI app reported unexpected last block height"
            );
            return Err(SyncError::VerifyFailed);
        }
        tracing::info!(height = snapshot.height, app_hash = %hex_upper(&snapshot.trusted_app_hash), "Verified ABCI app");
        Ok(resp.timeout_info)
    }
}

/// `proxy.RequestInfo`.
#[must_use]
pub fn request_info() -> abci::RequestInfo {
    abci::RequestInfo {
        version: crate::state::TM_CORE_SEM_VER.to_owned(),
        block_version: ctypes::header::BLOCK_PROTOCOL,
        p2p_version: 8,
        abci_version: "2.0.0".to_owned(),
    }
}

fn offer_result_name(r: i32) -> String {
    abci::response_offer_snapshot::Result::try_from(r)
        .map_or_else(|_| r.to_string(), |v| v.as_str_name().to_owned())
}

fn apply_result_name(r: i32) -> String {
    abci::response_apply_snapshot_chunk::Result::try_from(r)
        .map_or_else(|_| r.to_string(), |v| v.as_str_name().to_owned())
}

/// `fetchChunks`.
fn fetch_chunks(
    pool: &SnapshotPool,
    snapshot: &Snapshot,
    chunks: &ChunkQueue,
    cancel: &AtomicBool,
    retry: Duration,
) {
    let mut next = true;
    let mut index = 0;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return;
        }
        if next {
            match chunks.allocate() {
                Ok(i) => index = i,
                Err(ChunkError::Done) => {
                    // keep checking until canceled, in case chunks are refetched
                    if cancel.load(Ordering::SeqCst) {
                        return;
                    }
                    sleep_unless(cancel, Duration::from_secs(2));
                    continue;
                }
                Err(e) => {
                    tracing::error!(err = %e, "Failed to allocate chunk from queue");
                    return;
                }
            }
        }
        tracing::info!(
            height = snapshot.height,
            format = snapshot.format,
            chunk = index,
            total = chunks.size(),
            "Fetching snapshot chunk"
        );
        request_chunk(pool.get_peer(snapshot), snapshot, index);
        match chunks.wait_for(index, retry, Some(cancel)) {
            Wait::Arrived | Wait::Closed => next = true,
            Wait::TimedOut => next = false,
            Wait::Canceled => return,
        }
    }
}

fn sleep_unless(cancel: &AtomicBool, d: Duration) {
    let step = Duration::from_millis(100);
    let mut left = d;
    while !left.is_zero() && !cancel.load(Ordering::SeqCst) {
        let s = left.min(step);
        std::thread::sleep(s);
        left -= s;
    }
}

/// `requestChunk`.
fn request_chunk(peer: Option<PeerRef>, snapshot: &Snapshot, chunk: u32) {
    let Some(peer) = peer else {
        tracing::error!(
            height = snapshot.height,
            format = snapshot.format,
            "No valid peers found for snapshot"
        );
        return;
    };
    tracing::debug!(height = snapshot.height, chunk, peer = %peer.id(), "Requesting snapshot chunk");
    peer.send(
        CHUNK_CHANNEL,
        Msg::ChunkRequest(pb::ChunkRequest {
            height: snapshot.height,
            format: snapshot.format,
            index: chunk,
        })
        .encode(),
    );
}
