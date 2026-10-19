//! The state-sync reactor (`statesync/reactor.go`): serves snapshots and
//! chunks from the local app on channels 0x60/0x61 and feeds received ones
//! into a running [`Syncer`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use celestia_core_proto::tendermint::{abci, statesync as pb};
use ctypes::Commit;
use p2p::{ChannelDescriptor, Envelope, Peer, WeakSwitch};

use crate::chunks::Chunk;
use crate::conn::AppConn;
use crate::messages::{
    CHUNK_CHANNEL, CHUNK_MSG_SIZE, ERR_EXCEEDS_MAX_SNAPSHOT_CHUNKS, Msg, RECENT_SNAPSHOTS,
    SNAPSHOT_CHANNEL, SNAPSHOT_MSG_SIZE, validate_msg,
};
use crate::snapshots::{PeerHandle, PeerRef, Snapshot};
use crate::state::State;
use crate::stateprovider::StateProvider;
use crate::syncer::{Config, SyncError, Syncer};

/// `ReactorIncomingMessageQueueSize`.
pub const REACTOR_INCOMING_MESSAGE_QUEUE_SIZE: usize = 100;

/// A p2p peer as a [`PeerHandle`]. Sends from inside the runtime are
/// spawned (Go's blocking `Send`); from other threads they block on the
/// runtime.
pub struct P2pPeer {
    peer: Arc<Peer>,
    rt: Option<tokio::runtime::Handle>,
}

impl P2pPeer {
    /// Wraps a peer.
    #[must_use]
    pub fn new(peer: Arc<Peer>, rt: Option<tokio::runtime::Handle>) -> Self {
        Self { peer, rt }
    }
}

impl PeerHandle for P2pPeer {
    fn id(&self) -> String {
        self.peer.id().to_string()
    }

    fn send(&self, ch: u8, msg: Vec<u8>) -> bool {
        if let Ok(h) = tokio::runtime::Handle::try_current() {
            let peer = Arc::clone(&self.peer);
            h.spawn(async move {
                peer.send(ch, msg).await;
            });
            return true;
        }
        match &self.rt {
            Some(rt) => rt.block_on(self.peer.send(ch, msg)),
            None => self.peer.try_send(ch, msg),
        }
    }
}

/// The state-sync reactor.
pub struct Reactor {
    cfg: Config,
    conn: Arc<dyn AppConn>,
    syncer: RwLock<Option<Arc<Syncer>>>,
    switch: Mutex<WeakSwitch>,
    rt: Mutex<Option<tokio::runtime::Handle>>,
    running: AtomicBool,
    stopped: AtomicBool,
}

impl std::fmt::Debug for Reactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StateSyncReactor")
    }
}

impl Reactor {
    /// `NewReactor`.
    #[must_use]
    pub fn new(cfg: Config, conn: Arc<dyn AppConn>) -> Self {
        Self {
            cfg,
            conn,
            syncer: RwLock::new(None),
            switch: Mutex::new(WeakSwitch::default()),
            rt: Mutex::new(tokio::runtime::Handle::try_current().ok()),
            running: AtomicBool::new(true),
            stopped: AtomicBool::new(false),
        }
    }

    fn rt(&self) -> Option<tokio::runtime::Handle> {
        self.rt.lock().ok().and_then(|g| g.clone())
    }

    fn peer_ref(&self, peer: &Arc<Peer>) -> PeerRef {
        Arc::new(P2pPeer::new(Arc::clone(peer), self.rt()))
    }

    fn current_syncer(&self) -> Option<Arc<Syncer>> {
        self.syncer.read().ok().and_then(|g| g.clone())
    }

    /// `recentSnapshots`: the app's newest snapshots (height, then format,
    /// descending).
    pub fn recent_snapshots(&self, n: usize) -> Result<Vec<Snapshot>, String> {
        let mut resp = self.conn.list_snapshots()?;
        resp.snapshots
            .sort_by(|a, b| b.height.cmp(&a.height).then(b.format.cmp(&a.format)));
        Ok(resp
            .snapshots
            .into_iter()
            .take(n.min(RECENT_SNAPSHOTS))
            .map(|s| Snapshot {
                height: s.height,
                format: s.format,
                chunks: s.chunks,
                hash: s.hash,
                metadata: s.metadata,
                ..Snapshot::default()
            })
            .collect())
    }

    /// `Sync`: runs a state sync (blocking; call it off the async runtime's
    /// worker threads, e.g. from `spawn_blocking`) and returns the state and
    /// commit to bootstrap the node stores with.
    pub fn sync(
        &self,
        state_provider: Arc<dyn StateProvider>,
        discovery_time: Duration,
    ) -> Result<(State, Commit), SyncError> {
        let syncer = {
            let mut g = self
                .syncer
                .write()
                .map_err(|_| SyncError::Msg("poisoned".into()))?;
            if g.is_some() {
                return Err(SyncError::Msg("a state sync is already in progress".into()));
            }
            let s = Arc::new(Syncer::new(
                self.cfg.clone(),
                Arc::clone(&self.conn),
                state_provider,
            ));
            let sw = self.switch.lock().map(|g| g.clone()).unwrap_or_default();
            s.set_on_peer_rejected(Arc::new(move |peer_id: &str| {
                let Some(sw) = sw.upgrade() else { return };
                if let Some(p) = sw
                    .peers()
                    .into_iter()
                    .find(|p| p.id().to_string() == peer_id)
                {
                    sw.stop_peer_for_error(&p, "chunk sender rejected by ABCI application");
                }
            }));
            *g = Some(Arc::clone(&s));
            s
        };
        // A `stop_sync` that raced the setup above saw no syncer.
        if self.stopped.load(Ordering::SeqCst) {
            syncer.stop();
        }
        let sw = self.switch.lock().map(|g| g.clone()).unwrap_or_default();
        let hook = move || {
            tracing::debug!("Requesting snapshots from known peers");
            if let Some(sw) = sw.upgrade() {
                drop(sw.broadcast(SNAPSHOT_CHANNEL, Msg::SnapshotsRequest.encode()));
            }
        };
        hook();
        let res = syncer.sync_any(discovery_time, &hook);
        if let Ok(mut g) = self.syncer.write() {
            *g = None;
        }
        res
    }

    /// Stops the running [`Reactor::sync`] (and any later one) for node
    /// shutdown: it returns [`SyncError::Stopped`].
    pub fn stop_sync(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(s) = self.current_syncer() {
            s.stop();
        }
    }

    fn handle(&self, env: &Envelope, msg: Msg) {
        let src = &env.src;
        match (env.channel_id, msg) {
            (SNAPSHOT_CHANNEL, Msg::SnapshotsRequest) => {
                let snapshots = match self.recent_snapshots(RECENT_SNAPSHOTS) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!(err = %e, "Failed to fetch snapshots");
                        return;
                    }
                };
                let peer = self.peer_ref(src);
                for s in snapshots {
                    tracing::debug!(height = s.height, format = s.format, peer = %src.id(), "Advertising snapshot");
                    peer.send(
                        SNAPSHOT_CHANNEL,
                        Msg::SnapshotsResponse(pb::SnapshotsResponse {
                            height: s.height,
                            format: s.format,
                            chunks: s.chunks,
                            hash: s.hash,
                            metadata: s.metadata,
                        })
                        .encode(),
                    );
                }
            }
            (SNAPSHOT_CHANNEL, Msg::SnapshotsResponse(m)) => {
                let Some(syncer) = self.current_syncer() else {
                    tracing::debug!("Received unexpected snapshot, no state sync in progress");
                    return;
                };
                tracing::debug!(height = m.height, format = m.format, peer = %src.id(), "Received snapshot");
                syncer.add_snapshot(
                    &self.peer_ref(src),
                    Snapshot {
                        height: m.height,
                        format: m.format,
                        chunks: m.chunks,
                        hash: m.hash,
                        metadata: m.metadata,
                        ..Snapshot::default()
                    },
                );
            }
            (CHUNK_CHANNEL, Msg::ChunkRequest(m)) => {
                tracing::debug!(height = m.height, format = m.format, chunk = m.index, peer = %src.id(), "Received chunk request");
                let resp = match self
                    .conn
                    .load_snapshot_chunk(abci::RequestLoadSnapshotChunk {
                        height: m.height,
                        format: m.format,
                        chunk: m.index,
                    }) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!(height = m.height, chunk = m.index, err = %e, "Failed to load chunk");
                        return;
                    }
                };
                let missing = resp.chunk.is_empty();
                self.peer_ref(src).send(
                    CHUNK_CHANNEL,
                    Msg::ChunkResponse(pb::ChunkResponse {
                        height: m.height,
                        format: m.format,
                        index: m.index,
                        chunk: resp.chunk,
                        missing,
                    })
                    .encode(),
                );
            }
            (CHUNK_CHANNEL, Msg::ChunkResponse(m)) => {
                let Some(syncer) = self.current_syncer() else {
                    tracing::debug!(peer = %src.id(), "Received unexpected chunk, no state sync in progress");
                    return;
                };
                tracing::debug!(height = m.height, format = m.format, chunk = m.index, peer = %src.id(), "Received chunk, adding to sync");
                if let Err(e) = syncer.add_chunk(&Chunk {
                    height: m.height,
                    format: m.format,
                    index: m.index,
                    chunk: m.chunk,
                    sender: src.id().to_string(),
                }) {
                    tracing::error!(height = m.height, chunk = m.index, err = %e, "Failed to add chunk");
                }
            }
            (ch, m) => tracing::error!(channel = ch, msg = ?m, "Received unknown message"),
        }
    }
}

impl p2p::Reactor for Reactor {
    fn name(&self) -> String {
        "STATESYNC".into()
    }

    fn channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            ChannelDescriptor {
                send_queue_capacity: 10,
                recv_message_capacity: SNAPSHOT_MSG_SIZE,
                ..ChannelDescriptor::new(SNAPSHOT_CHANNEL, 5)
            },
            ChannelDescriptor {
                send_queue_capacity: 10,
                recv_message_capacity: CHUNK_MSG_SIZE,
                ..ChannelDescriptor::new(CHUNK_CHANNEL, 3)
            },
        ]
    }

    fn set_switch(&self, switch: WeakSwitch) {
        if let Ok(mut g) = self.switch.lock() {
            *g = switch;
        }
    }

    fn on_start(self: Arc<Self>) -> Result<(), String> {
        if let Ok(mut g) = self.rt.lock()
            && g.is_none()
        {
            *g = tokio::runtime::Handle::try_current().ok();
        }
        self.running.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn on_stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    fn add_peer(&self, peer: &Arc<Peer>) {
        if let Some(s) = self.current_syncer() {
            s.add_peer(&self.peer_ref(peer));
        }
    }

    fn remove_peer(&self, peer: &Arc<Peer>, _reason: &str) {
        if let Some(s) = self.current_syncer() {
            s.remove_peer(&peer.id().to_string());
        }
    }

    fn receive(&self, env: Envelope) -> Result<(), String> {
        if !self.running.load(Ordering::SeqCst) {
            return Ok(());
        }
        let msg = Msg::decode(&env.message)?;
        if let Err(e) = validate_msg(&msg, self.cfg.max_snapshot_chunks) {
            if e.starts_with(ERR_EXCEEDS_MAX_SNAPSHOT_CHUNKS)
                && let Some(s) = self.current_syncer()
            {
                s.reject_peer(&env.src.id().to_string());
            }
            tracing::error!(peer = %env.src.id(), err = %e, "Invalid message");
            return Err(e);
        }
        self.handle(&env, msg);
        Ok(())
    }

    fn queue_size(&self) -> usize {
        REACTOR_INCOMING_MESSAGE_QUEUE_SIZE
    }
}
