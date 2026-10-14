//! `mempool/cat/cache.go`: the `SeenTracker`, which records which peers
//! have which txs and queues future-sequence SeenTx announcements by
//! signer until the sequence gap closes.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use super::go_search;
use crate::types::TxKey;

/// `seenPerPeerLimit`.
pub const SEEN_PER_PEER_LIMIT: usize = 1000;
/// `seenPerSignerLimit`.
pub const SEEN_PER_SIGNER_LIMIT: usize = 1000;
/// `seenEntryTTL`.
pub const SEEN_ENTRY_TTL: Duration = Duration::from_secs(120);

/// A wall clock (Go's `clock.Clock`), replaceable in tests.
pub trait Clock: Send + Sync {
    /// Now.
    fn now(&self) -> SystemTime;
}

/// The system clock.
#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// A settable clock for tests (`clock.Mock`).
#[derive(Debug)]
pub struct MockClock(Mutex<SystemTime>);

impl MockClock {
    /// A mock clock at `now`.
    #[must_use]
    pub fn new(now: SystemTime) -> Self {
        Self(Mutex::new(now))
    }

    /// Sets the time.
    pub fn set(&self, t: SystemTime) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = t;
    }
}

impl Clock for MockClock {
    fn now(&self) -> SystemTime {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// `PendingTxInfo`: signer metadata of a queued future tx.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingTxInfo {
    /// Signer.
    pub signer: Vec<u8>,
    /// Sequence.
    pub sequence: u64,
}

/// `SeenEntry` (returned as a copy).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenEntry {
    /// The tx.
    pub tx_key: TxKey,
    /// Peers known to have it.
    pub peers: HashSet<u16>,
    /// When it was first tracked.
    pub added_at: SystemTime,
    /// Set while queued for future-sequence gap filling.
    pub pending_tx_info: Option<PendingTxInfo>,
}

impl SeenEntry {
    /// `peerIDs`: candidate peers to request from.
    #[must_use]
    pub fn peer_ids(&self) -> Vec<u16> {
        self.peers.iter().copied().collect()
    }
}

#[derive(Debug, Default)]
struct Inner {
    tx_by_key: HashMap<TxKey, SeenEntry>,
    /// Per signer: queued keys ascending by sequence.
    pending_by_signer: HashMap<Vec<u8>, Vec<TxKey>>,
    tx_count_by_peer: HashMap<u16, usize>,
}

impl Inner {
    fn decrement_peer(&mut self, peer: u16) {
        if peer == 0 {
            return;
        }
        if let Some(c) = self.tx_count_by_peer.get_mut(&peer) {
            *c = c.saturating_sub(1);
            if *c == 0 {
                self.tx_count_by_peer.remove(&peer);
            }
        }
    }

    fn count(&self, peer: u16) -> usize {
        self.tx_count_by_peer.get(&peer).copied().unwrap_or(0)
    }

    /// `removeFromPendingQueueLocked`.
    fn remove_from_pending_queue(&mut self, key: &TxKey, signer: &[u8]) {
        let Some(queue) = self.pending_by_signer.get_mut(signer) else {
            return;
        };
        if let Some(i) = queue.iter().position(|k| k == key) {
            queue.remove(i);
        }
        if queue.is_empty() {
            self.pending_by_signer.remove(signer);
        }
    }

    /// `removeEntryLocked`.
    fn remove_entry(&mut self, key: &TxKey) {
        let Some(entry) = self.tx_by_key.remove(key) else {
            return;
        };
        if let Some(info) = &entry.pending_tx_info {
            self.remove_from_pending_queue(key, &info.signer);
        }
        for p in entry.peers {
            self.decrement_peer(p);
        }
    }
}

/// `SeenTracker`.
pub struct SeenTracker {
    inner: Mutex<Inner>,
    per_peer_limit: usize,
    per_signer_limit: usize,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for SeenTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SeenTracker")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl Default for SeenTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl SeenTracker {
    /// `NewSeenTracker`.
    #[must_use]
    pub fn new() -> Self {
        Self::with_limits(
            SEEN_PER_PEER_LIMIT,
            SEEN_PER_SIGNER_LIMIT,
            Arc::new(SystemClock),
        )
    }

    /// A tracker with custom limits and clock (tests).
    #[must_use]
    pub fn with_limits(per_peer: usize, per_signer: usize, clock: Arc<dyn Clock>) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            per_peer_limit: per_peer,
            per_signer_limit: per_signer,
            clock,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `addPeerLocked`: true if the peer is now recorded for the key.
    fn add_peer_locked(&self, s: &mut Inner, key: TxKey, peer: u16) -> bool {
        if peer == 0 {
            return false;
        }
        if s.tx_by_key.contains_key(&key) {
            if s.tx_by_key[&key].peers.contains(&peer) {
                return true;
            }
            if s.count(peer) >= self.per_peer_limit {
                return false;
            }
            if let Some(e) = s.tx_by_key.get_mut(&key) {
                e.peers.insert(peer);
            }
            *s.tx_count_by_peer.entry(peer).or_default() += 1;
            return true;
        }
        if s.count(peer) >= self.per_peer_limit {
            return false;
        }
        s.tx_by_key.insert(
            key,
            SeenEntry {
                tx_key: key,
                peers: HashSet::from([peer]),
                added_at: self.clock.now(),
                pending_tx_info: None,
            },
        );
        *s.tx_count_by_peer.entry(peer).or_default() += 1;
        true
    }

    /// `AddPeer`: notes that `peer` has the tx. False if not stored.
    pub fn add_peer(&self, key: TxKey, peer: u16) -> bool {
        let mut s = self.lock();
        self.add_peer_locked(&mut s, key, peer)
    }

    /// `AddPendingTx`: `AddPeer` plus queueing by signer/sequence (once).
    pub fn add_pending_tx(&self, key: TxKey, peer: u16, signer: &[u8], sequence: u64) -> bool {
        if signer.is_empty() {
            return false;
        }
        let mut s = self.lock();
        if !self.add_peer_locked(&mut s, key, peer) {
            return false;
        }
        if s.tx_by_key[&key].pending_tx_info.is_none() {
            self.index_pending_tx_locked(&mut s, key, signer, sequence);
        }
        true
    }

    /// `indexPendingTxLocked`: a full queue keeps only the lowest sequences.
    fn index_pending_tx_locked(&self, s: &mut Inner, key: TxKey, signer: &[u8], sequence: u64) {
        let queue = s.pending_by_signer.get(signer).cloned().unwrap_or_default();
        let seq_of = |k: &TxKey, s: &Inner| {
            s.tx_by_key[k]
                .pending_tx_info
                .as_ref()
                .map_or(0, |i| i.sequence)
        };
        let insert_idx = go_search(queue.len(), |i| seq_of(&queue[i], s) >= sequence);
        if queue.len() >= self.per_signer_limit && insert_idx == queue.len() {
            return;
        }
        if let Some(e) = s.tx_by_key.get_mut(&key) {
            e.pending_tx_info = Some(PendingTxInfo {
                signer: signer.to_vec(),
                sequence,
            });
        }
        let mut queue = queue;
        queue.insert(insert_idx, key);
        while queue.len() > self.per_signer_limit {
            if let Some(last) = queue.pop()
                && let Some(e) = s.tx_by_key.get_mut(&last)
            {
                e.pending_tx_info = None;
            }
        }
        s.pending_by_signer.insert(signer.to_vec(), queue);
    }

    /// `ClearPendingTx`: drops the future-sequence state, keeps the peers.
    pub fn clear_pending_tx(&self, key: &TxKey) {
        let mut s = self.lock();
        let Some(signer) = s
            .tx_by_key
            .get(key)
            .and_then(|e| e.pending_tx_info.as_ref().map(|i| i.signer.clone()))
        else {
            return;
        };
        s.remove_from_pending_queue(key, &signer);
        if let Some(e) = s.tx_by_key.get_mut(key) {
            e.pending_tx_info = None;
        }
    }

    /// `RemoveKey`.
    pub fn remove_key(&self, key: &TxKey) {
        self.lock().remove_entry(key);
    }

    /// `RemovePeer`: forgets the peer everywhere; entries without peers go.
    pub fn remove_peer(&self, peer: u16) {
        if peer == 0 {
            return;
        }
        let mut s = self.lock();
        let keys: Vec<TxKey> = s.tx_by_key.keys().copied().collect();
        for key in keys {
            let Some(e) = s.tx_by_key.get_mut(&key) else {
                continue;
            };
            let had = e.peers.remove(&peer);
            let empty = e.peers.is_empty();
            if had {
                s.decrement_peer(peer);
            }
            if empty {
                s.remove_entry(&key);
            }
        }
        s.tx_count_by_peer.remove(&peer);
    }

    /// `PruneExpired`: removes entries older than [`SEEN_ENTRY_TTL`].
    pub fn prune_expired(&self) {
        let cutoff = self
            .clock
            .now()
            .checked_sub(SEEN_ENTRY_TTL)
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let mut s = self.lock();
        let old: Vec<TxKey> = s
            .tx_by_key
            .values()
            .filter(|e| e.added_at < cutoff)
            .map(|e| e.tx_key)
            .collect();
        for key in old {
            s.remove_entry(&key);
        }
    }

    /// `Has`.
    #[must_use]
    pub fn has(&self, key: &TxKey, peer: u16) -> bool {
        self.lock()
            .tx_by_key
            .get(key)
            .is_some_and(|e| e.peers.contains(&peer))
    }

    /// `Get`: a copy of the entry.
    #[must_use]
    pub fn get(&self, key: &TxKey) -> Option<SeenEntry> {
        self.lock().tx_by_key.get(key).cloned()
    }

    /// `PendingTxInfo`: `(signer, sequence)` if queued.
    #[must_use]
    pub fn pending_tx_info(&self, key: &TxKey) -> Option<(Vec<u8>, u64)> {
        self.lock()
            .tx_by_key
            .get(key)
            .and_then(|e| e.pending_tx_info.as_ref())
            .map(|i| (i.signer.clone(), i.sequence))
    }

    /// `PrunePending`: drops future-sequence metadata of entries added
    /// before `cutoff` (the peer records stay).
    pub fn prune_pending(&self, cutoff: SystemTime) {
        let mut s = self.lock();
        let stale: Vec<(TxKey, Vec<u8>)> = s
            .tx_by_key
            .values()
            .filter(|e| e.added_at < cutoff)
            .filter_map(|e| {
                e.pending_tx_info
                    .as_ref()
                    .map(|i| (e.tx_key, i.signer.clone()))
            })
            .collect();
        for (key, signer) in stale {
            s.remove_from_pending_queue(&key, &signer);
            if let Some(e) = s.tx_by_key.get_mut(&key) {
                e.pending_tx_info = None;
            }
        }
    }

    /// `PeersForTx`: a copy (`None` if unknown).
    #[must_use]
    pub fn peers_for_tx(&self, key: &TxKey) -> Option<HashSet<u16>> {
        self.lock().tx_by_key.get(key).map(|e| e.peers.clone())
    }

    /// `Len`.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().tx_by_key.len()
    }

    /// Whether nothing is tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `Reset`.
    pub fn reset(&self) {
        *self.lock() = Inner::default();
    }

    /// `PendingTxsForSigner`: copies, ascending by sequence.
    #[must_use]
    pub fn pending_txs_for_signer(&self, signer: &[u8]) -> Vec<SeenEntry> {
        if signer.is_empty() {
            return Vec::new();
        }
        let s = self.lock();
        s.pending_by_signer
            .get(signer)
            .map(|q| {
                q.iter()
                    .filter_map(|k| s.tx_by_key.get(k).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `SignersWithPendingTxs`.
    #[must_use]
    pub fn signers_with_pending_txs(&self) -> Vec<Vec<u8>> {
        self.lock().pending_by_signer.keys().cloned().collect()
    }

    /// `RemovePeerFromTx`: e.g. after a request timed out.
    pub fn remove_peer_from_tx(&self, key: &TxKey, peer: u16) {
        if peer == 0 {
            return;
        }
        let mut s = self.lock();
        let Some(e) = s.tx_by_key.get_mut(key) else {
            return;
        };
        let had = e.peers.remove(&peer);
        let empty = e.peers.is_empty();
        if had {
            s.decrement_peer(peer);
        }
        if empty {
            s.remove_entry(key);
        }
    }

    /// Test helper: the per-peer entry count.
    #[cfg(test)]
    pub(crate) fn tx_count_by_peer(&self, peer: u16) -> usize {
        self.lock().count(peer)
    }

    /// Test helper: whether no peer counts are tracked.
    #[cfg(test)]
    pub(crate) fn peer_counts_empty(&self) -> bool {
        self.lock().tx_count_by_peer.is_empty()
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
    use crate::types::tx_key;

    fn key(s: &str) -> TxKey {
        tx_key(s.as_bytes())
    }

    fn seqs(entries: &[SeenEntry]) -> Vec<u64> {
        entries
            .iter()
            .map(|e| e.pending_tx_info.as_ref().unwrap().sequence)
            .collect()
    }

    fn base_time() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_767_323_045)
    }

    fn clock_tracker() -> (SeenTracker, Arc<MockClock>) {
        let clock = Arc::new(MockClock::new(base_time()));
        let t = SeenTracker::with_limits(SEEN_PER_PEER_LIMIT, SEEN_PER_SIGNER_LIMIT, clock.clone());
        (t, clock)
    }

    #[test]
    fn add() {
        let k = key("tx");
        let t = SeenTracker::new();
        assert!(!t.add_peer(k, 0));
        assert_eq!(t.len(), 0);

        let t = SeenTracker::new();
        assert!(t.add_peer(k, 1));
        assert!(t.has(&k, 1));
        assert_eq!(t.tx_count_by_peer(1), 1);
        assert!(t.add_peer(k, 1));
        assert_eq!(t.len(), 1);
        assert_eq!(t.tx_count_by_peer(1), 1);

        let t = SeenTracker::new();
        assert!(t.add_peer(k, 1));
        assert!(t.add_peer(k, 2));
        assert_eq!(t.peers_for_tx(&k), Some(HashSet::from([1, 2])));

        let t = SeenTracker::with_limits(1, SEEN_PER_SIGNER_LIMIT, Arc::new(SystemClock));
        assert!(t.add_peer(key("a"), 1));
        assert!(!t.add_peer(key("b"), 1));
        assert_eq!(t.len(), 1);
        let shared = key("shared");
        assert!(t.add_peer(shared, 2));
        assert!(!t.add_peer(shared, 1));
        assert!(!t.has(&shared, 1));
    }

    #[test]
    fn get_returns_copy() {
        let k = key("tx");
        let t = SeenTracker::new();
        assert!(t.get(&k).is_none());
        assert!(t.add_pending_tx(k, 1, b"signer", 3));
        let mut e = t.get(&k).unwrap();
        e.peers.remove(&1);
        e.pending_tx_info.as_mut().unwrap().sequence = 99;
        assert!(t.has(&k, 1));
        assert_eq!(t.pending_tx_info(&k), Some((b"signer".to_vec(), 3)));
    }

    #[test]
    fn reset_and_remove_key() {
        let t = SeenTracker::new();
        assert!(t.add_pending_tx(key("a"), 1, b"signer", 1));
        t.reset();
        assert_eq!(t.len(), 0);
        assert!(t.peer_counts_empty());
        assert!(t.signers_with_pending_txs().is_empty());

        let k = key("tx");
        t.remove_key(&k);
        assert!(t.add_pending_tx(k, 1, b"signer", 1));
        t.remove_key(&k);
        assert_eq!(t.len(), 0);
        assert!(t.peers_for_tx(&k).is_none());
        assert!(t.peer_counts_empty());
        assert!(t.pending_txs_for_signer(b"signer").is_empty());
    }

    #[test]
    fn remove_peer() {
        let k = key("tx");
        let t = SeenTracker::new();
        assert!(t.add_peer(k, 1));
        t.remove_peer(0);
        assert!(t.has(&k, 1));

        let t = SeenTracker::new();
        assert!(t.add_peer(k, 1));
        assert!(t.add_peer(k, 2));
        t.remove_peer(1);
        assert!(!t.has(&k, 1));
        assert!(t.has(&k, 2));
        assert_eq!(t.len(), 1);
        assert_eq!(t.tx_count_by_peer(1), 0);

        let t = SeenTracker::new();
        assert!(t.add_peer(k, 1));
        t.remove_peer(1);
        assert_eq!(t.len(), 0);
        assert!(t.peer_counts_empty());
    }

    #[test]
    fn prune() {
        let (t, clock) = clock_tracker();
        let (old, fresh) = (key("old"), key("fresh"));
        assert!(t.add_peer(old, 1));
        clock.set(base_time() + Duration::from_secs(180));
        assert!(t.add_peer(fresh, 1));
        t.prune_expired();
        assert!(t.get(&old).is_none());
        assert!(t.get(&fresh).is_some());
        assert_eq!(t.tx_count_by_peer(1), 1);
    }

    #[test]
    fn remove_peer_from_tx() {
        let k = key("tx");
        let t = SeenTracker::new();
        assert!(t.add_peer(k, 1));
        t.remove_peer_from_tx(&k, 0);
        assert!(t.has(&k, 1));
        t.remove_peer_from_tx(&key("other"), 1);

        assert!(t.add_peer(k, 2));
        t.remove_peer_from_tx(&k, 1);
        assert!(t.get(&k).is_some());
        assert!(!t.has(&k, 1));
        assert!(t.has(&k, 2));
        t.remove_peer_from_tx(&k, 2);
        assert_eq!(t.len(), 0);
        assert!(t.peer_counts_empty());
    }

    #[test]
    fn add_indexes_pending_tx() {
        let t = SeenTracker::new();
        let k = key("tx");
        assert!(t.add_pending_tx(k, 1, b"signer", 7));
        assert_eq!(t.pending_tx_info(&k), Some((b"signer".to_vec(), 7)));

        let t = SeenTracker::new();
        assert!(t.add_pending_tx(k, 1, b"signer", 0));
        assert_eq!(t.pending_tx_info(&k), Some((b"signer".to_vec(), 0)));

        let t = SeenTracker::new();
        assert!(t.add_peer(k, 1));
        assert!(t.pending_tx_info(&k).is_none());
        assert!(t.signers_with_pending_txs().is_empty());
        assert!(!t.add_pending_tx(k, 1, b"", 1));

        let t = SeenTracker::new();
        let (k2, k0, k1) = (key("seq2"), key("seq0"), key("seq1"));
        assert!(t.add_pending_tx(k2, 1, b"signer", 2));
        assert!(t.add_pending_tx(k0, 1, b"signer", 0));
        assert!(t.add_pending_tx(k1, 2, b"signer", 1));
        let e = t.pending_txs_for_signer(b"signer");
        assert_eq!(seqs(&e), vec![0, 1, 2]);
        assert_eq!(
            e.iter().map(|e| e.tx_key).collect::<Vec<_>>(),
            vec![k0, k1, k2]
        );

        // peer-only then pending
        let t = SeenTracker::new();
        assert!(t.add_peer(k, 1));
        assert!(t.add_pending_tx(k, 2, b"signer", 4));
        assert_eq!(t.pending_tx_info(&k).unwrap().1, 4);
        assert_eq!(t.peers_for_tx(&k), Some(HashSet::from([1, 2])));
    }

    #[test]
    fn per_signer_limit() {
        let t = SeenTracker::with_limits(SEEN_PER_PEER_LIMIT, 2, Arc::new(SystemClock));
        let (k5, k7, k6) = (key("seq5"), key("seq7"), key("seq6"));
        assert!(t.add_pending_tx(k5, 1, b"signer", 5));
        assert!(t.add_pending_tx(k7, 1, b"signer", 7));
        assert!(t.add_pending_tx(k6, 1, b"signer", 6));
        assert_eq!(seqs(&t.pending_txs_for_signer(b"signer")), vec![5, 6]);
        let demoted = t.get(&k7).unwrap();
        assert!(demoted.pending_tx_info.is_none());
        assert!(t.has(&k7, 1));

        let t = SeenTracker::with_limits(SEEN_PER_PEER_LIMIT, 2, Arc::new(SystemClock));
        let k9 = key("seq9");
        assert!(t.add_pending_tx(k5, 1, b"signer", 5));
        assert!(t.add_pending_tx(k6, 1, b"signer", 6));
        assert!(t.add_pending_tx(k9, 1, b"signer", 9));
        assert_eq!(seqs(&t.pending_txs_for_signer(b"signer")), vec![5, 6]);
        assert!(t.get(&k9).unwrap().pending_tx_info.is_none());
        assert!(t.has(&k9, 1));
    }

    #[test]
    fn does_not_reindex() {
        let t = SeenTracker::new();
        let k = key("tx");
        assert!(t.add_pending_tx(k, 1, b"signer-a", 1));
        assert!(t.add_pending_tx(k, 2, b"signer-b", 2));
        assert_eq!(t.pending_tx_info(&k), Some((b"signer-a".to_vec(), 1)));
        assert!(!t.pending_txs_for_signer(b"signer-a").is_empty());
        assert!(t.pending_txs_for_signer(b"signer-b").is_empty());
        assert_eq!(t.peers_for_tx(&k), Some(HashSet::from([1, 2])));
    }

    #[test]
    fn clear_pending() {
        let k = key("tx");
        let t = SeenTracker::new();
        t.clear_pending_tx(&k);
        assert!(t.add_peer(k, 1));
        t.clear_pending_tx(&k);
        assert!(t.has(&k, 1));

        let t = SeenTracker::new();
        assert!(t.add_pending_tx(k, 1, b"signer", 5));
        t.clear_pending_tx(&k);
        assert!(t.pending_tx_info(&k).is_none());
        assert!(t.has(&k, 1));
        assert!(t.pending_txs_for_signer(b"signer").is_empty());
    }

    #[test]
    fn prune_pending() {
        let (t, clock) = clock_tracker();
        let (old, fresh) = (key("old"), key("fresh"));
        assert!(t.add_pending_tx(old, 1, b"signer", 1));
        clock.set(base_time() + Duration::from_secs(180));
        assert!(t.add_pending_tx(fresh, 1, b"signer", 2));
        t.prune_pending(base_time() + Duration::from_secs(60));
        assert!(t.pending_tx_info(&old).is_none());
        assert!(t.has(&old, 1));
        assert!(t.pending_tx_info(&fresh).is_some());
        let keys: Vec<_> = t
            .pending_txs_for_signer(b"signer")
            .iter()
            .map(|e| e.tx_key)
            .collect();
        assert_eq!(keys, vec![fresh]);
    }

    #[test]
    fn signers_with_pending() {
        let t = SeenTracker::new();
        assert!(t.signers_with_pending_txs().is_empty());
        assert!(t.add_pending_tx(key("a"), 1, b"signer-a", 0));
        assert!(t.add_pending_tx(key("b"), 1, b"signer-b", 0));
        let mut s = t.signers_with_pending_txs();
        s.sort();
        assert_eq!(s, vec![b"signer-a".to_vec(), b"signer-b".to_vec()]);
    }

    #[test]
    fn concurrent() {
        let t = Arc::new(SeenTracker::new());
        let keys = [key("a"), key("b")];
        let hs: Vec<_> = (1..=8u16)
            .map(|p| {
                let t = t.clone();
                std::thread::spawn(move || {
                    for _ in 0..100 {
                        for k in keys {
                            t.add_pending_tx(k, p, &[p as u8], u64::from(p));
                            let _ = t.get(&k);
                        }
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        assert_eq!(t.len(), 2);
        for k in keys {
            assert_eq!(t.peers_for_tx(&k).unwrap().len(), 8);
        }
        for p in 1..=8 {
            assert_eq!(t.tx_count_by_peer(p), 2);
        }
        let hs: Vec<_> = (1..=8u16)
            .map(|p| {
                let t = t.clone();
                std::thread::spawn(move || t.remove_peer(p))
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        assert_eq!(t.len(), 0);
        assert!(t.peer_counts_empty());
    }
}
