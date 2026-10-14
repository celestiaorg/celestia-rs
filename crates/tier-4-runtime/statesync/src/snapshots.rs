//! Snapshot discovery and aggregation across peers (`statesync/snapshots.go`).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex};

use rand::Rng as _;
use sha2::{Digest as _, Sha256};

/// A peer as the syncer sees it: an ID and a way to send it a message.
pub trait PeerHandle: Send + Sync {
    /// The peer's node ID (hex).
    fn id(&self) -> String;
    /// Sends `msg` on channel `ch`; false if it could not be queued.
    fn send(&self, ch: u8, msg: Vec<u8>) -> bool;
}

impl fmt::Debug for dyn PeerHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Peer{{{}}}", self.id())
    }
}

/// A shared peer handle.
pub type PeerRef = Arc<dyn PeerHandle>;

/// `snapshotKey`: SHA-256 over all snapshot fields.
pub type SnapshotKey = [u8; 32];

/// A snapshot advertised by peers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Snapshot {
    /// Height.
    pub height: u64,
    /// Format.
    pub format: u32,
    /// Number of chunks.
    pub chunks: u32,
    /// Snapshot hash.
    pub hash: Vec<u8>,
    /// App metadata.
    pub metadata: Vec<u8>,
    /// App hash from the light client (set during sync).
    pub trusted_app_hash: Vec<u8>,
    /// App version from the light client (set during sync).
    pub trusted_app_version: u64,
}

impl Snapshot {
    /// `snapshot.Key()`: `sha256(fmt("%v:%v:%v", height, format, chunks) || hash || metadata)`.
    #[must_use]
    pub fn key(&self) -> SnapshotKey {
        let mut h = Sha256::new();
        h.update(format!("{}:{}:{}", self.height, self.format, self.chunks).as_bytes());
        h.update(&self.hash);
        h.update(&self.metadata);
        h.finalize().into()
    }
}

#[derive(Default)]
struct PoolInner {
    snapshots: BTreeMap<SnapshotKey, Snapshot>,
    snapshot_peers: BTreeMap<SnapshotKey, BTreeMap<String, PeerRef>>,
    format_index: HashMap<u32, BTreeSet<SnapshotKey>>,
    height_index: HashMap<u64, BTreeSet<SnapshotKey>>,
    peer_index: HashMap<String, BTreeSet<SnapshotKey>>,
    format_blacklist: HashSet<u32>,
    peer_blacklist: HashSet<String>,
    snapshot_blacklist: HashSet<SnapshotKey>,
}

impl PoolInner {
    fn remove_peer(&mut self, peer_id: &str) {
        let keys: Vec<SnapshotKey> = self
            .peer_index
            .get(peer_id)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default();
        for key in keys {
            let empty = self.snapshot_peers.get_mut(&key).is_none_or(|p| {
                p.remove(peer_id);
                p.is_empty()
            });
            if empty {
                self.remove_snapshot(&key);
            }
        }
        self.peer_index.remove(peer_id);
    }

    fn remove_snapshot(&mut self, key: &SnapshotKey) {
        let Some(s) = self.snapshots.remove(key) else {
            return;
        };
        if let Some(i) = self.format_index.get_mut(&s.format) {
            i.remove(key);
        }
        if let Some(i) = self.height_index.get_mut(&s.height) {
            i.remove(key);
        }
        if let Some(peers) = self.snapshot_peers.remove(key) {
            for id in peers.keys() {
                if let Some(i) = self.peer_index.get_mut(id) {
                    i.remove(key);
                }
            }
        }
    }
}

/// `snapshotPool`.
#[derive(Default)]
pub struct SnapshotPool {
    inner: Mutex<PoolInner>,
}

impl fmt::Debug for SnapshotPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SnapshotPool")
    }
}

impl SnapshotPool {
    /// `newSnapshotPool`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, PoolInner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `Add`: true if this is a new, non-blacklisted snapshot.
    pub fn add(&self, peer: &PeerRef, snapshot: Snapshot) -> bool {
        let key = snapshot.key();
        let id = peer.id();
        let mut p = self.lock();
        if p.format_blacklist.contains(&snapshot.format)
            || p.peer_blacklist.contains(&id)
            || p.snapshot_blacklist.contains(&key)
            || p.peer_index.get(&id).map_or(0, BTreeSet::len) >= crate::messages::RECENT_SNAPSHOTS
        {
            return false;
        }
        p.snapshot_peers
            .entry(key)
            .or_default()
            .insert(id.clone(), Arc::clone(peer));
        p.peer_index.entry(id).or_default().insert(key);
        if p.snapshots.contains_key(&key) {
            return false;
        }
        p.format_index
            .entry(snapshot.format)
            .or_default()
            .insert(key);
        p.height_index
            .entry(snapshot.height)
            .or_default()
            .insert(key);
        p.snapshots.insert(key, snapshot);
        true
    }

    /// `Best`.
    #[must_use]
    pub fn best(&self) -> Option<Snapshot> {
        self.ranked().into_iter().next()
    }

    /// `GetPeer`: a random peer for the snapshot.
    #[must_use]
    pub fn get_peer(&self, snapshot: &Snapshot) -> Option<PeerRef> {
        let peers = self.get_peers(snapshot);
        if peers.is_empty() {
            return None;
        }
        let i = rand::rng().random_range(0..peers.len());
        Some(Arc::clone(&peers[i]))
    }

    /// `GetPeers`, sorted by ID.
    #[must_use]
    pub fn get_peers(&self, snapshot: &Snapshot) -> Vec<PeerRef> {
        let key = snapshot.key();
        self.lock()
            .snapshot_peers
            .get(&key)
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
    }

    /// `Ranked`: by height, then format, then number of peers (descending).
    #[must_use]
    pub fn ranked(&self) -> Vec<Snapshot> {
        let p = self.lock();
        let mut c: Vec<(Snapshot, usize)> = p
            .snapshots
            .iter()
            .map(|(k, s)| (s.clone(), p.snapshot_peers.get(k).map_or(0, BTreeMap::len)))
            .collect();
        c.sort_by(|(a, an), (b, bn)| {
            b.height
                .cmp(&a.height)
                .then(b.format.cmp(&a.format))
                .then(bn.cmp(an))
        });
        c.into_iter().map(|(s, _)| s).collect()
    }

    /// `Reject`: never use this snapshot again.
    pub fn reject(&self, snapshot: &Snapshot) {
        let key = snapshot.key();
        let mut p = self.lock();
        p.snapshot_blacklist.insert(key);
        p.remove_snapshot(&key);
    }

    /// `RejectFormat`.
    pub fn reject_format(&self, format: u32) {
        let mut p = self.lock();
        p.format_blacklist.insert(format);
        let keys: Vec<SnapshotKey> = p
            .format_index
            .get(&format)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default();
        for k in keys {
            p.remove_snapshot(&k);
        }
    }

    /// `RejectPeer`.
    pub fn reject_peer(&self, peer_id: &str) {
        if peer_id.is_empty() {
            return;
        }
        let mut p = self.lock();
        p.remove_peer(peer_id);
        p.peer_blacklist.insert(peer_id.to_owned());
    }

    /// `RemovePeer`.
    pub fn remove_peer(&self, peer_id: &str) {
        self.lock().remove_peer(peer_id);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[derive(Debug)]
    pub(crate) struct TestPeer(pub String, pub Mutex<Vec<(u8, Vec<u8>)>>);

    impl PeerHandle for TestPeer {
        fn id(&self) -> String {
            self.0.clone()
        }
        fn send(&self, ch: u8, msg: Vec<u8>) -> bool {
            self.1.lock().map(|mut v| v.push((ch, msg))).is_ok()
        }
    }

    pub(crate) fn peer(id: &str) -> PeerRef {
        Arc::new(TestPeer(id.into(), Mutex::new(Vec::new())))
    }

    fn snap(height: u64, format: u32, chunks: u32) -> Snapshot {
        Snapshot {
            height,
            format,
            chunks,
            hash: vec![1],
            ..Snapshot::default()
        }
    }

    #[test]
    fn key_matches_go() {
        // sha256("1:2:3" || 0x010203 || 0x040506)
        let s = Snapshot {
            height: 1,
            format: 2,
            chunks: 3,
            hash: vec![1, 2, 3],
            metadata: vec![4, 5, 6],
            ..Snapshot::default()
        };
        let mut h = Sha256::new();
        h.update(b"1:2:3\x01\x02\x03\x04\x05\x06");
        let want: [u8; 32] = h.finalize().into();
        assert_eq!(s.key(), want);
    }

    /// Ports of Go's `snapshots_test.go`.
    #[test]
    fn add_and_rank() {
        let pool = SnapshotPool::new();
        let (a, b, c) = (peer("aa"), peer("bb"), peer("cc"));
        assert!(pool.add(&a, snap(1, 1, 1)));
        assert!(!pool.add(&b, snap(1, 1, 1)));
        assert!(pool.add(&a, snap(2, 1, 1)));
        assert!(pool.add(&c, snap(2, 2, 1)));
        assert!(pool.add(&b, snap(1, 2, 1)));
        let r: Vec<(u64, u32)> = pool.ranked().iter().map(|s| (s.height, s.format)).collect();
        assert_eq!(r, vec![(2, 2), (2, 1), (1, 2), (1, 1)]);
        assert_eq!(
            pool.get_peers(&snap(1, 1, 1))
                .iter()
                .map(|p| p.id())
                .collect::<Vec<_>>(),
            vec!["aa", "bb"]
        );
        pool.reject_format(2);
        let r: Vec<(u64, u32)> = pool.ranked().iter().map(|s| (s.height, s.format)).collect();
        assert_eq!(r, vec![(2, 1), (1, 1)]);
        assert!(!pool.add(&a, snap(3, 2, 1)));
        pool.reject(&snap(2, 1, 1));
        assert!(!pool.add(&b, snap(2, 1, 1)));
        assert_eq!(pool.best().map(|s| s.height), Some(1));
        pool.reject_peer("aa");
        assert_eq!(pool.get_peers(&snap(1, 1, 1)).len(), 1);
        assert!(!pool.add(&a, snap(5, 1, 1)));
        pool.remove_peer("bb");
        assert!(pool.best().is_none());
    }

    #[test]
    fn peer_limit() {
        let pool = SnapshotPool::new();
        let a = peer("aa");
        for h in 1..=10 {
            assert!(pool.add(&a, snap(h, 1, 1)));
        }
        assert!(!pool.add(&a, snap(11, 1, 1)));
        assert!(pool.add(&peer("bb"), snap(11, 1, 1)));
    }
}
