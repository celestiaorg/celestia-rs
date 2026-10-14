//! `mempool/cat/peers.go`: short (`u16`) mempool IDs for peers, plus the
//! peer interface the reactor talks to.

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::{MAX_ACTIVE_IDS, UNKNOWN_PEER_ID};

/// `types.PeerStateKey`: where the consensus reactor stores its per-peer
/// state on a `p2p::Peer`.
pub const PEER_STATE_KEY: &str = "ConsensusReactor.peerState";

/// `cat.PeerState`: the part of the consensus reactor's peer state the
/// mempool reads.
pub trait PeerState: Send + Sync {
    /// The peer's consensus height.
    fn get_height(&self) -> i64;
}

/// The value the consensus reactor stores under [`PEER_STATE_KEY`]
/// (`peer.set(PEER_STATE_KEY, Arc::new(PeerStateHandle(state)))`) so the
/// mempool can skip peers that are far behind.
pub struct PeerStateHandle(pub Arc<dyn PeerState>);

/// Reads the consensus peer state stored on a `p2p::Peer`.
#[must_use]
pub fn peer_state_of(data: Option<Arc<dyn Any + Send + Sync>>) -> Option<Arc<dyn PeerState>> {
    let data = data?;
    data.downcast_ref::<PeerStateHandle>().map(|h| h.0.clone())
}

/// What the reactor needs from a peer (`p2p.Peer`). Implemented for
/// `p2p::Peer` and by test doubles.
pub trait MempoolPeer: Send + Sync {
    /// `ID()`: the P2P ID (lowercase hex for real peers).
    fn id(&self) -> String;
    /// `IsPersistent()`.
    fn is_persistent(&self) -> bool;
    /// `Get(types.PeerStateKey)`.
    fn peer_state(&self) -> Option<Arc<dyn PeerState>>;
    /// `Send`: queues the message, waiting for queue space if needed.
    fn send(&self, ch: u8, msg: Vec<u8>) -> bool;
    /// `TrySend`: queues the message only if there is space now.
    fn try_send(&self, ch: u8, msg: Vec<u8>) -> bool;
}

#[derive(Default)]
struct Inner {
    peer_map: HashMap<String, u16>,
    next_id: u16,
    active: HashMap<u16, Arc<dyn MempoolPeer>>,
}

/// `mempoolIDs`.
pub struct MempoolIds {
    inner: RwLock<Inner>,
}

impl std::fmt::Debug for MempoolIds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MempoolIds")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl Default for MempoolIds {
    fn default() -> Self {
        Self::new()
    }
}

impl MempoolIds {
    /// `newMempoolIDs`: IDs start at 1 (0 is the RPC sender).
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(Inner {
                next_id: UNKNOWN_PEER_ID + 1,
                ..Inner::default()
            }),
        }
    }

    fn read(&self) -> RwLockReadGuard<'_, Inner> {
        self.inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, Inner> {
        self.inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `ReserveForPeer`: assigns the next unused ID; a reconnecting peer
    /// keeps its ID and the peer object is replaced.
    ///
    /// # Panics
    /// When all 65535 IDs are in use (as Go does).
    pub fn reserve_for_peer(&self, peer: Arc<dyn MempoolPeer>) {
        let mut s = self.write();
        let pid = peer.id();
        if let Some(&cur) = s.peer_map.get(&pid) {
            s.active.insert(cur, peer);
            return;
        }
        assert!(
            s.active.len() != MAX_ACTIVE_IDS,
            "node has maximum {MAX_ACTIVE_IDS} active IDs and wanted to get one more"
        );
        while s.active.contains_key(&s.next_id) {
            s.next_id = s.next_id.wrapping_add(1);
        }
        let cur = s.next_id;
        s.next_id = s.next_id.wrapping_add(1);
        s.peer_map.insert(pid, cur);
        s.active.insert(cur, peer);
    }

    /// `Reclaim`: frees the peer's ID, returning it (0 if unknown).
    pub fn reclaim(&self, peer_id: &str) -> u16 {
        let mut s = self.write();
        let Some(id) = s.peer_map.remove(peer_id) else {
            return 0;
        };
        s.active.remove(&id);
        id
    }

    /// `GetIDForPeer` (0 if unknown).
    #[must_use]
    pub fn get_id_for_peer(&self, peer_id: &str) -> u16 {
        self.read().peer_map.get(peer_id).copied().unwrap_or(0)
    }

    /// `GetPeer`.
    #[must_use]
    pub fn get_peer(&self, id: u16) -> Option<Arc<dyn MempoolPeer>> {
        self.read().active.get(&id).cloned()
    }

    /// `GetAll`: a copy of the active peers.
    #[must_use]
    pub fn get_all(&self) -> HashMap<u16, Arc<dyn MempoolPeer>> {
        self.read().active.clone()
    }

    /// `Len`.
    #[must_use]
    pub fn len(&self) -> usize {
        self.read().active.len()
    }

    /// Whether no peer is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::type_complexity,
    clippy::needless_range_loop
)]
mod tests {
    use super::*;

    struct Named(String);

    impl MempoolPeer for Named {
        fn id(&self) -> String {
            self.0.clone()
        }
        fn is_persistent(&self) -> bool {
            false
        }
        fn peer_state(&self) -> Option<Arc<dyn PeerState>> {
            None
        }
        fn send(&self, _: u8, _: Vec<u8>) -> bool {
            true
        }
        fn try_send(&self, _: u8, _: Vec<u8>) -> bool {
            true
        }
    }

    #[test]
    fn peer_lifecycle() {
        let ids = MempoolIds::new();
        let p1: Arc<dyn MempoolPeer> = Arc::new(Named("peer1".into()));
        assert!(ids.get_peer(1).is_none());
        assert_eq!(ids.get_id_for_peer("peer1"), 0);
        assert!(ids.get_all().is_empty());
        ids.reserve_for_peer(p1.clone());
        let id = ids.get_id_for_peer("peer1");
        assert_eq!(id, 1);
        assert!(Arc::ptr_eq(&ids.get_peer(id).unwrap(), &p1));
        assert_eq!(ids.get_all().len(), 1);

        ids.reserve_for_peer(p1.clone());
        assert_eq!(ids.get_id_for_peer("peer1"), id);
        assert_eq!(ids.get_all().len(), 1);

        let p2: Arc<dyn MempoolPeer> = Arc::new(Named("peer1".into()));
        ids.reserve_for_peer(p2.clone());
        assert_eq!(ids.get_id_for_peer("peer1"), id);
        assert!(Arc::ptr_eq(&ids.get_peer(id).unwrap(), &p2));
        assert_eq!(ids.get_all().len(), 1);

        assert_eq!(ids.reclaim("peer1"), id);
        assert!(ids.get_peer(id).is_none());
        assert_eq!(ids.get_id_for_peer("peer1"), 0);
        assert!(ids.get_all().is_empty());
    }

    #[test]
    fn concurrent_reservation() {
        let ids = Arc::new(MempoolIds::new());
        let p1: Arc<dyn MempoolPeer> = Arc::new(Named("peer1".into()));
        let hs: Vec<_> = (0..20)
            .map(|_| {
                let (ids, p) = (ids.clone(), p1.clone());
                std::thread::spawn(move || ids.reserve_for_peer(p))
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        assert_eq!(ids.get_all().len(), 1);
        let id = ids.get_id_for_peer("peer1");
        assert_ne!(id, 0);
        assert!(Arc::ptr_eq(&ids.get_peer(id).unwrap(), &p1));
    }

    #[test]
    fn ids_skip_active_and_reuse_after_reclaim() {
        let ids = MempoolIds::new();
        for i in 0..3 {
            ids.reserve_for_peer(Arc::new(Named(format!("p{i}"))));
        }
        assert_eq!(ids.reclaim("p1"), 2);
        ids.reserve_for_peer(Arc::new(Named("p3".into())));
        assert_eq!(ids.get_id_for_peer("p3"), 4);
    }
}
