//! Tx caches (`mempool/cache.go`): a thread-safe LRU of tx keys and the
//! rejected-tx cache that also keeps the rejection code and log.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use crate::types::TxKey;

/// An LRU list (Go's `container/list` + map) with a value per key.
#[derive(Debug)]
struct Lru<V> {
    size: usize,
    next: u64,
    map: HashMap<TxKey, (u64, V)>,
    order: BTreeMap<u64, TxKey>,
}

impl<V> Lru<V> {
    fn new(size: usize) -> Self {
        Self {
            size,
            next: 0,
            map: HashMap::new(),
            order: BTreeMap::new(),
        }
    }

    fn reset(&mut self) {
        self.map.clear();
        self.order.clear();
    }

    /// `Push`: moves an existing key to the back (keeping its value) and
    /// returns false; otherwise evicts the front if `len >= size` (Go
    /// checks `>=` before inserting, so a zero-size cache holds one key)
    /// and inserts.
    fn push(&mut self, key: TxKey, value: V) -> bool {
        if let Some((pos, _)) = self.map.get_mut(&key) {
            self.order.remove(pos);
            self.next += 1;
            *pos = self.next;
            self.order.insert(self.next, key);
            return false;
        }
        if self.order.len() >= self.size
            && let Some((_, front)) = self.order.pop_first()
        {
            self.map.remove(&front);
        }
        self.next += 1;
        self.order.insert(self.next, key);
        self.map.insert(key, (self.next, value));
        true
    }

    fn remove(&mut self, key: &TxKey) {
        if let Some((pos, _)) = self.map.remove(key) {
            self.order.remove(&pos);
        }
    }

    fn get(&self, key: &TxKey) -> Option<&V> {
        self.map.get(key).map(|(_, v)| v)
    }

    fn len(&self) -> usize {
        self.order.len()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `LRUTxCache`: a thread-safe LRU set of tx keys.
#[derive(Debug)]
pub struct LruTxCache {
    inner: Mutex<Lru<()>>,
}

impl LruTxCache {
    /// `NewLRUTxCache`.
    #[must_use]
    pub fn new(size: usize) -> Self {
        Self {
            inner: Mutex::new(Lru::new(size)),
        }
    }

    /// `Reset`.
    pub fn reset(&self) {
        lock(&self.inner).reset();
    }

    /// `Push`: true if the key was newly added.
    pub fn push(&self, key: TxKey) -> bool {
        lock(&self.inner).push(key, ())
    }

    /// `Remove`.
    pub fn remove(&self, key: &TxKey) {
        lock(&self.inner).remove(key);
    }

    /// `Has` / `HasKey`.
    #[must_use]
    pub fn has(&self, key: &TxKey) -> bool {
        lock(&self.inner).get(key).is_some()
    }

    /// Number of cached keys.
    #[must_use]
    pub fn len(&self) -> usize {
        lock(&self.inner).len()
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// `RejectedTxCache`: an LRU of rejected tx keys with their code and log.
#[derive(Debug)]
pub struct RejectedTxCache {
    inner: Mutex<Lru<(u32, String)>>,
}

impl RejectedTxCache {
    /// `NewRejectedTxCache`.
    #[must_use]
    pub fn new(size: usize) -> Self {
        Self {
            inner: Mutex::new(Lru::new(size)),
        }
    }

    /// `Reset`.
    pub fn reset(&self) {
        lock(&self.inner).reset();
    }

    /// `Push`: true if newly added; an existing entry keeps its code/log.
    pub fn push(&self, key: TxKey, code: u32, log: &str) -> bool {
        lock(&self.inner).push(key, (code, log.to_owned()))
    }

    /// `Get`: the rejection code and log.
    #[must_use]
    pub fn get(&self, key: &TxKey) -> Option<(u32, String)> {
        lock(&self.inner).get(key).cloned()
    }

    /// `HasKey`.
    #[must_use]
    pub fn has_key(&self, key: &TxKey) -> bool {
        lock(&self.inner).get(key).is_some()
    }

    /// `Remove`.
    pub fn remove(&self, key: &TxKey) {
        lock(&self.inner).remove(key);
    }

    /// Number of cached keys.
    #[must_use]
    pub fn len(&self) -> usize {
        lock(&self.inner).len()
    }

    /// Whether the cache is empty.
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
    use crate::types::tx_key;

    fn populate(cache: &LruTxCache, n: usize) -> Vec<TxKey> {
        (0..n)
            .map(|_| {
                let k: TxKey = rand::random();
                cache.push(k);
                k
            })
            .collect()
    }

    #[test]
    fn cache_remove() {
        let cache = LruTxCache::new(100);
        let txs = populate(&cache, 10);
        assert_eq!(cache.len(), 10);
        for (i, k) in txs.iter().enumerate() {
            cache.remove(k);
            assert_eq!(cache.len(), 10 - (i + 1));
            assert_eq!(lock(&cache.inner).map.len(), 10 - (i + 1));
        }
    }

    #[test]
    fn lru_evicts_front_and_moves_to_back() {
        let cache = LruTxCache::new(2);
        let (a, b, c) = ([1; 32], [2; 32], [3; 32]);
        assert!(cache.push(a));
        assert!(cache.push(b));
        assert!(!cache.push(a)); // a moves to the back
        assert!(cache.push(c)); // evicts b
        assert!(cache.has(&a) && cache.has(&c) && !cache.has(&b));
    }

    #[test]
    fn zero_size_cache_holds_one_key() {
        let cache = LruTxCache::new(0);
        assert!(cache.push([1; 32]));
        assert_eq!(cache.len(), 1);
        assert!(cache.push([2; 32]));
        assert_eq!(cache.len(), 1);
        assert!(cache.has(&[2; 32]));
    }

    #[test]
    fn rejected_tx_cache() {
        let cache = RejectedTxCache::new(10);
        let key = tx_key(b"test-transaction");
        assert!(!cache.has_key(&key));
        assert_eq!(cache.get(&key), None);

        assert!(cache.push(key, 1001, "initial-log"));
        assert!(cache.has_key(&key));
        assert_eq!(cache.get(&key), Some((1001, "initial-log".into())));

        assert!(!cache.push(key, 1001, "initial-log"));
        // an existing entry is not overwritten
        assert!(!cache.push(key, 2002, "new-log"));
        assert_eq!(cache.get(&key), Some((1001, "initial-log".into())));

        cache.remove(&key);
        assert!(!cache.has_key(&key));
        assert_eq!(cache.get(&key), None);

        for i in 0..15u8 {
            cache.push([i; 32], 1, "x");
        }
        assert_eq!(cache.len(), 10);

        cache.reset();
        assert!(cache.is_empty());
    }
}
