//! `mempool/cat/store.go`: the thread-safe tx store with signer-bundled
//! sets ordered by aggregated priority (ties: earlier first arrival).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::SystemTime;

use super::go_search;
use super::tx::{TxSet, WrappedTx};
use crate::types::TxKey;

#[derive(Debug, Default)]
struct Inner {
    bytes: i64,
    txs: HashMap<TxKey, Arc<WrappedTx>>,
    reserved: HashSet<TxKey>,
    /// Sets by identity; `ordered` and `sets_by_signer` refer to them.
    sets: HashMap<u64, TxSet>,
    sets_by_signer: HashMap<Vec<u8>, u64>,
    /// Set ids, highest aggregated priority first.
    ordered: Vec<u64>,
    next_set_id: u64,
}

impl Inner {
    fn new_set(&mut self, wtx: Arc<WrappedTx>) -> u64 {
        self.next_set_id += 1;
        let mut set = TxSet::new(&[wtx]);
        set.id = self.next_set_id;
        self.sets.insert(set.id, set);
        self.next_set_id
    }

    /// `orderSet`.
    fn order_set(&mut self, id: u64) {
        let (prio, ts) = {
            let s = &self.sets[&id];
            (s.aggregated_priority, s.first_timestamp)
        };
        let idx = go_search(self.ordered.len(), |i| {
            let other = &self.sets[&self.ordered[i]];
            if other.aggregated_priority == prio {
                return ts < other.first_timestamp;
            }
            other.aggregated_priority < prio
        });
        self.ordered.insert(idx, id);
    }

    /// `deleteOrderedSet`.
    fn delete_ordered_set(&mut self, id: u64) {
        assert!(!self.ordered.is_empty(), "ordered tx sets list is empty");
        let pos = self.ordered.iter().position(|&s| s == id);
        assert!(pos.is_some(), "tx set not found in ordered list: {id}");
        if let Some(i) = pos {
            self.ordered.remove(i);
        }
    }
}

/// The tx store.
#[derive(Debug, Default)]
pub struct Store {
    inner: RwLock<Inner>,
}

impl Store {
    /// `newStore`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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

    /// `set`: adds the tx to its signer's set (or a single-tx set for an
    /// empty signer). False if the key is already stored.
    pub fn set(&self, wtx: Arc<WrappedTx>) -> bool {
        let mut s = self.write();
        let key = wtx.key();
        if s.txs.contains_key(&key) {
            return false;
        }
        s.txs.insert(key, wtx.clone());
        s.bytes = s.bytes.wrapping_add(wtx.size());
        if wtx.sender.is_empty() {
            let id = s.new_set(wtx);
            s.order_set(id);
            return true;
        }
        let Some(&id) = s.sets_by_signer.get(&wtx.sender) else {
            let sender = wtx.sender.clone();
            let id = s.new_set(wtx);
            s.sets_by_signer.insert(sender, id);
            s.order_set(id);
            return true;
        };
        s.delete_ordered_set(id);
        if let Some(set) = s.sets.get_mut(&id) {
            set.add_tx(wtx);
        }
        s.order_set(id);
        true
    }

    /// `get`.
    #[must_use]
    pub fn get(&self, key: &TxKey) -> Option<Arc<WrappedTx>> {
        self.read().txs.get(key).cloned()
    }

    /// `has`.
    #[must_use]
    pub fn has(&self, key: &TxKey) -> bool {
        self.read().txs.contains_key(key)
    }

    /// `remove`.
    pub fn remove(&self, key: &TxKey) -> bool {
        let mut s = self.write();
        let Some(tx) = s.txs.remove(key) else {
            return false;
        };
        s.bytes = s.bytes.wrapping_sub(tx.size());
        if tx.sender.is_empty() {
            // Empty-signer sets hold one tx: drop the set holding it.
            let found = s.ordered.iter().position(|id| {
                let set = &s.sets[id];
                set.txs.len() == 1 && Arc::ptr_eq(&set.txs[0], &tx)
            });
            let Some(i) = found else {
                return false;
            };
            let id = s.ordered.remove(i);
            s.sets.remove(&id);
            return true;
        }
        let Some(&id) = s.sets_by_signer.get(&tx.sender) else {
            return false;
        };
        s.delete_ordered_set(id);
        let Some(set) = s.sets.get_mut(&id) else {
            return false;
        };
        assert!(
            set.remove_tx(&tx),
            "remove tx: transaction not found in set: {}",
            hex::encode_upper(key)
        );
        if set.txs.is_empty() {
            s.sets.remove(&id);
            s.sets_by_signer.remove(&tx.sender);
            return true;
        }
        s.order_set(id);
        true
    }

    /// `reserve`: marks a key as being added. False if already reserved.
    pub fn reserve(&self, key: TxKey) -> bool {
        self.write().reserved.insert(key)
    }

    /// `isReserved`.
    #[must_use]
    pub fn is_reserved(&self, key: &TxKey) -> bool {
        self.read().reserved.contains(key)
    }

    /// `release`.
    pub fn release(&self, key: &TxKey) {
        self.write().reserved.remove(key);
    }

    /// `size`.
    #[must_use]
    pub fn size(&self) -> usize {
        self.read().txs.len()
    }

    /// `totalBytes`.
    #[must_use]
    pub fn total_bytes(&self) -> i64 {
        self.read().bytes
    }

    /// `getAllKeys`.
    #[must_use]
    pub fn get_all_keys(&self) -> Vec<TxKey> {
        self.read().txs.keys().copied().collect()
    }

    /// `getTxSetsBelowPriority`: sets with aggregated priority below
    /// `priority`, lowest first, and their total bytes.
    #[must_use]
    pub fn get_tx_sets_below_priority(&self, priority: i64) -> (Vec<TxSet>, i64) {
        let s = self.read();
        let mut out = Vec::new();
        let mut bytes = 0i64;
        for id in s.ordered.iter().rev() {
            let set = &s.sets[id];
            if set.aggregated_priority >= priority {
                break;
            }
            bytes = bytes.wrapping_add(set.bytes);
            out.push(set.clone());
        }
        (out, bytes)
    }

    /// `purgeExpiredTxs`: removes every set whose first height is below
    /// `expiration_height` or whose first arrival is before
    /// `expiration_age` (whole sets: later sequences would be invalid).
    pub fn purge_expired_txs(
        &self,
        expiration_height: i64,
        expiration_age: Option<SystemTime>,
    ) -> (Vec<Arc<WrappedTx>>, usize) {
        let mut s = self.write();
        let mut purged = Vec::new();
        let ordered = s.ordered.clone();
        for id in ordered {
            let (first_height, first_ts) = {
                let set = &s.sets[&id];
                (set.first_height, set.first_timestamp)
            };
            if first_height < expiration_height || expiration_age.is_some_and(|age| first_ts < age)
            {
                s.delete_ordered_set(id);
                let Some(set) = s.sets.remove(&id) else {
                    continue;
                };
                s.bytes = s.bytes.wrapping_sub(set.bytes);
                if !set.signer.is_empty() {
                    s.sets_by_signer.remove(&set.signer);
                }
                for tx in set.txs {
                    s.txs.remove(&tx.key());
                    purged.push(tx);
                }
            }
        }
        let n = purged.len();
        (purged, n)
    }

    /// `reset` (reservations are kept, as in Go).
    pub fn reset(&self) {
        let mut s = self.write();
        s.bytes = 0;
        s.txs.clear();
        s.sets.clear();
        s.sets_by_signer.clear();
        s.ordered.clear();
    }

    /// `processOrderedTxSets`: runs `f` over the ordered sets under the read
    /// lock.
    pub fn process_ordered_tx_sets<R>(&self, f: impl FnOnce(&[&TxSet]) -> R) -> R {
        let s = self.read();
        let sets: Vec<&TxSet> = s.ordered.iter().map(|id| &s.sets[id]).collect();
        f(&sets)
    }

    /// `getOrderedTxs`: every tx in priority order (a copy).
    #[must_use]
    pub fn get_ordered_txs(&self) -> Vec<Arc<WrappedTx>> {
        let s = self.read();
        s.ordered
            .iter()
            .flat_map(|id| s.sets[id].txs.iter().cloned())
            .collect()
    }

    /// `aggregatedPriorityAfterAdd`.
    #[must_use]
    pub fn aggregated_priority_after_add(&self, wtx: &WrappedTx) -> i64 {
        let s = self.read();
        if wtx.sender.is_empty() {
            return wtx.priority;
        }
        if let Some(id) = s.sets_by_signer.get(&wtx.sender) {
            let set = &s.sets[id];
            let sum = set
                .weighted_priority_sum
                .wrapping_add(wtx.priority.wrapping_mul(wtx.gas_wanted));
            let gas = set.total_gas_wanted.wrapping_add(wtx.gas_wanted);
            if gas <= 0 {
                return 0;
            }
            return sum.wrapping_div(gas);
        }
        wtx.priority
    }

    /// Test helper: the signer's set.
    #[cfg(test)]
    pub(crate) fn set_for_signer(&self, signer: &[u8]) -> Option<TxSet> {
        let s = self.read();
        s.sets_by_signer.get(signer).map(|id| s.sets[id].clone())
    }

    /// Test helper: (sets by signer, ordered sets).
    #[cfg(test)]
    pub(crate) fn set_counts(&self) -> (usize, usize) {
        let s = self.read();
        (s.sets_by_signer.len(), s.ordered.len())
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
    use std::time::Duration;

    use rand::seq::SliceRandom as _;

    use super::*;
    use crate::types::{CachedTx, tx_key};

    fn wtx(
        tx: &str,
        height: i64,
        gas: i64,
        priority: i64,
        signer: &[u8],
        seq: u64,
    ) -> Arc<WrappedTx> {
        Arc::new(WrappedTx::new(
            CachedTx::new(tx.as_bytes().to_vec()),
            height,
            gas,
            priority,
            signer.to_vec(),
            seq,
            false,
        ))
    }

    fn make_txs(n: usize, sender: &[u8], with_sequence: bool) -> Vec<Arc<WrappedTx>> {
        (1..=n)
            .map(|i| {
                let seq = if with_sequence { i as u64 } else { 0 };
                let i = i as i64;
                wtx(&format!("tx{i}"), i, i, i, sender, seq)
            })
            .collect()
    }

    fn same(a: &[Arc<WrappedTx>], b: &[Arc<WrappedTx>]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| Arc::ptr_eq(x, y))
    }

    #[test]
    fn simple() {
        let store = Store::new();
        let w = wtx("tx1", 1, 1, 1, b"", 0);
        let key = w.key();
        assert!(store.get(&key).is_none());
        assert!(!store.has(&key));
        assert!(!store.remove(&key));
        assert_eq!(store.size(), 0);
        assert_eq!(store.total_bytes(), 0);
        assert!(store.get_all_keys().is_empty());
        assert!(store.get_ordered_txs().is_empty());

        store.set(w.clone());
        assert!(store.has(&key));
        assert!(Arc::ptr_eq(&store.get(&key).unwrap(), &w));
        assert_eq!(store.size(), 1);
        assert_eq!(store.total_bytes(), w.size());

        store.remove(&key);
        assert!(!store.has(&key));
        assert_eq!(store.size(), 0);
        assert_eq!(store.total_bytes(), 0);
        assert!(store.get_ordered_txs().is_empty());
        assert_eq!(store.set_counts(), (0, 0));
    }

    #[test]
    fn ordering() {
        let store = Store::new();
        let w1 = wtx("tx1", 1, 1, 1, b"", 0);
        let w2 = wtx("tx2", 2, 2, 2, b"", 0);
        let w3 = wtx("tx3", 3, 3, 3, b"", 0);
        store.set(w1.clone());
        store.set(w2.clone());
        store.set(w3.clone());
        assert!(same(&store.get_ordered_txs(), &[w3, w2, w1]));
    }

    #[test]
    fn remove_maintains_order_and_cleans_up() {
        let store = Store::new();
        let original = make_txs(100, b"", false);
        let mut expected: Vec<_> = original.iter().rev().cloned().collect();
        let mut shuffled = original.clone();
        shuffled.shuffle(&mut rand::rng());
        for w in &shuffled {
            store.set(w.clone());
        }
        assert!(same(&store.get_ordered_txs(), &expected));

        let (size, bytes, sets) = (store.size(), store.total_bytes(), store.set_counts().1);
        let to_remove = expected[30].clone();
        assert!(store.remove(&to_remove.key()));
        expected.remove(30);
        assert!(same(&store.get_ordered_txs(), &expected));
        assert!(store.get(&to_remove.key()).is_none());
        assert!(!store.is_reserved(&to_remove.key()));
        assert_eq!(store.size(), size - 1);
        assert_eq!(store.total_bytes(), bytes - to_remove.size());
        assert_eq!(store.set_counts().1, sets - 1);

        let second = expected[60].clone();
        assert!(store.remove(&second.key()));
        expected.remove(60);
        assert!(same(&store.get_ordered_txs(), &expected));
        assert!(!store.remove(&to_remove.key()));

        for w in &expected {
            store.remove(&w.key());
        }
        assert_eq!(store.set_counts(), (0, 0));
        assert_eq!(store.size(), 0);
        assert!(store.get_all_keys().is_empty());
    }

    #[test]
    fn remove_from_non_empty_signer_set() {
        let store = Store::new();
        let txs = make_txs(10, b"signer1", true);
        for w in &txs {
            store.set(w.clone());
        }
        assert!(same(&store.get_ordered_txs(), &txs));
        assert_eq!(store.set_counts().0, 1);
        assert_eq!(store.set_for_signer(b"signer1").unwrap().txs.len(), 10);

        let (size, bytes, sets) = (store.size(), store.total_bytes(), store.set_counts().1);
        let to_remove = txs[5].clone();
        assert!(store.remove(&to_remove.key()));
        assert_eq!(store.set_for_signer(b"signer1").unwrap().txs.len(), 9);
        let remaining = store.get_ordered_txs();
        assert_eq!(remaining.len(), 9);
        for (i, r) in remaining.iter().enumerate().take(5) {
            assert_eq!(r.sequence, i as u64 + 1);
        }
        for (i, r) in remaining.iter().enumerate().skip(5) {
            assert_eq!(r.sequence, i as u64 + 2);
        }
        assert_eq!(store.size(), size - 1);
        assert_eq!(store.total_bytes(), bytes - to_remove.size());
        assert_eq!(store.set_counts().1, sets);

        assert!(store.remove(&remaining[0].key()));
        let remaining = store.get_ordered_txs();
        assert_eq!(remaining[0].sequence, 2);
        let last = remaining.last().unwrap().clone();
        assert_eq!(last.sequence, 10);
        assert!(store.remove(&last.key()));
        let remaining = store.get_ordered_txs();
        assert_eq!(remaining.len(), 7);
        assert_eq!(remaining.last().unwrap().sequence, 9);
        for w in &remaining {
            store.remove(&w.key());
        }
        assert_eq!(store.set_counts(), (0, 0));
        assert_eq!(store.total_bytes(), 0);
    }

    #[test]
    fn reserving_txs() {
        let store = Store::new();
        let w = wtx("tx1", 1, 1, 1, b"", 0);
        let key = w.key();
        store.release(&key);
        store.reserve(key);
        assert!(store.is_reserved(&key));
        assert_eq!(store.total_bytes(), 0);
        store.set(w.clone());
        assert_eq!(store.get(&key).unwrap().tx.tx(), b"tx1");
        assert_eq!(store.total_bytes(), w.size());
        store.release(&key);
        assert!(store.has(&key));
        store.remove(&key);
        assert!(!store.has(&key));
        store.reserve(key);
        assert!(store.is_reserved(&key));
        store.release(&key);
        assert!(!store.has(&key));
        assert!(!store.is_reserved(&key));
    }

    #[test]
    fn read_reserved() {
        let store = Store::new();
        let key = tx_key(b"tx1");
        store.reserve(key);
        assert!(store.get(&key).is_none());
        assert!(!store.has(&key));
        assert!(store.get_all_keys().is_empty());
        assert!(store.get_ordered_txs().is_empty());
    }

    #[test]
    fn concurrent_access() {
        let store = Arc::new(Store::new());
        let num_txs = 100;
        let handles: Vec<_> = (0..num_txs)
            .map(|i| {
                let store = store.clone();
                std::thread::spawn(move || {
                    loop {
                        let tx = format!("tx{}", i % (num_txs / 10));
                        let w = wtx(&tx, 1, 1, 1, b"", 0);
                        let key = w.key();
                        if store.get(&key).is_some_and(|e| e.tx.tx() == tx.as_bytes()) {
                            return;
                        }
                        if store.reserve(key) {
                            if i % 3 == 0 {
                                store.release(&key);
                                return;
                            }
                            store.set(w);
                            store.release(&key);
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(store.size(), num_txs / 10);
    }

    #[test]
    fn get_txs_and_sets_below_priority() {
        let store = Store::new();
        for i in 0..100 {
            store.set(wtx(&format!("tx{i}"), 1, 1, i, b"", 0));
        }
        assert_eq!(store.size(), 100);
        assert_eq!(store.get_ordered_txs().len(), 100);
        assert_eq!(store.get_all_keys().len(), 100);
        let (sets, bz) = store.get_tx_sets_below_priority(50);
        assert_eq!(sets.len(), 50);
        let count: usize = sets.iter().map(|s| s.txs.len()).sum();
        let actual: i64 = sets.iter().map(|s| s.bytes).sum();
        assert_eq!(count, 50);
        assert_eq!(actual, bz);
    }

    #[test]
    fn expired_txs() {
        let store = Store::new();
        for i in 0..100 {
            assert!(store.set(wtx(&format!("tx{i}"), i, 1, 1, b"", 0)));
        }
        let (_, purged) = store.purge_expired_txs(50, None);
        assert_eq!(purged, 50);
        for t in store.get_ordered_txs() {
            assert!(t.height >= 50);
        }
        store.purge_expired_txs(0, Some(SystemTime::now() + Duration::from_secs(1)));
        assert!(store.get_ordered_txs().is_empty());
    }

    #[test]
    fn purge_removes_whole_signer_set() {
        let store = Store::new();
        assert!(store.set(wtx("tx1", 4, 1, 10, b"signer1", 1)));
        assert!(store.set(wtx("tx2", 5, 1, 10, b"signer1", 2)));
        assert_eq!(store.set_for_signer(b"signer1").unwrap().txs.len(), 2);
        let (_, purged) = store.purge_expired_txs(5, None);
        assert_eq!(purged, 2);
        assert_eq!(store.size(), 0);
        assert!(store.set_for_signer(b"signer1").is_none());
        assert!(store.get_ordered_txs().is_empty());
    }

    #[test]
    fn ordered_txs_without_signer() {
        let store = Store::new();
        for (i, p) in [5, 1, 8, 3, 7, 2, 6, 4, 9, 0].into_iter().enumerate() {
            store.set(wtx(&format!("tx{i}"), 1, 1, p, b"", 0));
        }
        let ordered = store.get_ordered_txs();
        assert_eq!(ordered.len(), 10);
        for w in ordered.windows(2) {
            assert!(w[0].priority >= w[1].priority);
        }
    }

    #[test]
    fn multi_signer_priority_and_sequence() {
        let store = Store::new();
        for s in 0..5 {
            for seq in 1..=4u64 {
                let prio = rand::random_range(0..100);
                let gas = rand::random_range(1..1001);
                store.set(wtx(
                    &format!("tx_signer{s}_seq{seq}"),
                    1,
                    gas,
                    prio,
                    format!("signer{s}").as_bytes(),
                    seq,
                ));
            }
        }
        let ordered = store.get_ordered_txs();
        assert_eq!(ordered.len(), 20);
        let mut last: Option<(Vec<u8>, i64)> = None;
        let mut seqs: HashMap<Vec<u8>, Vec<u64>> = HashMap::new();
        for w in &ordered {
            seqs.entry(w.sender.clone()).or_default().push(w.sequence);
            let set = store.set_for_signer(&w.sender).unwrap();
            if last.as_ref().is_none_or(|(s, _)| *s != w.sender) {
                if let Some((_, p)) = &last {
                    assert!(*p >= set.aggregated_priority);
                }
                last = Some((w.sender.clone(), set.aggregated_priority));
            }
        }
        for v in seqs.values() {
            assert!(v.windows(2).all(|w| w[1] > w[0]));
        }
    }

    #[test]
    fn aggregated_priority_weighted_by_gas() {
        let store = Store::new();
        store.set(wtx("a1", 1, 1, 10, b"addr1", 1));
        store.set(wtx("a2", 1, 3, 4, b"addr1", 2));
        assert_eq!(
            store.set_for_signer(b"addr1").unwrap().aggregated_priority,
            5
        );
        let cand = wtx("a3", 1, 1, 9, b"addr1", 3);
        assert_eq!(store.aggregated_priority_after_add(&cand), 6);
    }

    #[test]
    fn intra_set_ordering_by_sequence_then_timestamp() {
        let store = Store::new();
        store.set(wtx("s2a", 1, 1, 1, b"addr1", 2));
        std::thread::sleep(Duration::from_millis(5));
        store.set(wtx("s1", 1, 1, 1, b"addr1", 1));
        std::thread::sleep(Duration::from_millis(5));
        store.set(wtx("s2b", 1, 1, 1, b"addr1", 2));
        let o: Vec<_> = store
            .get_ordered_txs()
            .iter()
            .map(|w| w.tx.tx().to_vec())
            .collect();
        assert_eq!(o, vec![b"s1".to_vec(), b"s2a".to_vec(), b"s2b".to_vec()]);
    }

    #[test]
    fn inter_set_ordering_by_priority_and_timestamp() {
        let names = |store: &Store| -> Vec<Vec<u8>> {
            store
                .get_ordered_txs()
                .iter()
                .map(|w| w.tx.tx().to_vec())
                .collect()
        };
        let store = Store::new();
        store.set(wtx("a1", 1, 1, 10, b"A", 1));
        store.set(wtx("a2", 1, 3, 4, b"A", 2));
        std::thread::sleep(Duration::from_millis(5));
        store.set(wtx("b1", 1, 2, 4, b"B", 1));
        store.set(wtx("b2", 1, 2, 6, b"B", 2));
        assert_eq!(
            names(&store),
            vec![
                b"a1".to_vec(),
                b"a2".to_vec(),
                b"b1".to_vec(),
                b"b2".to_vec()
            ]
        );

        let store = Store::new();
        store.set(wtx("a1", 1, 1, 10, b"A", 1));
        store.set(wtx("a2", 1, 3, 4, b"A", 2));
        std::thread::sleep(Duration::from_millis(5));
        store.set(wtx("c1", 1, 1, 10, b"C", 1));
        store.set(wtx("c2", 1, 3, 4, b"C", 2));
        assert_eq!(
            names(&store),
            vec![
                b"a1".to_vec(),
                b"a2".to_vec(),
                b"c1".to_vec(),
                b"c2".to_vec()
            ]
        );
    }
}
