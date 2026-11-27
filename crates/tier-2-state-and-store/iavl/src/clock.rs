//! A bounded CLOCK (second-chance) cache.
//!
//! A hit only sets the entry's reference bit; it does not move the entry
//! (an LRU list relinks on every hit, which on a cache of millions of
//! nodes costs several cache misses per lookup). On insertion into a full
//! cache the hand sweeps the slots, clearing reference bits, and evicts
//! the first entry that was not referenced since the hand last passed it.

use std::collections::HashMap;
use std::hash::{BuildHasher, Hash};

struct Slot<K, V> {
    entry: Option<(K, V)>,
    referenced: bool,
}

/// A CLOCK cache holding at most `capacity` entries.
pub(crate) struct ClockCache<K, V, S = foldhash::fast::RandomState> {
    map: HashMap<K, usize, S>,
    slots: Vec<Slot<K, V>>,
    free: Vec<usize>,
    hand: usize,
    capacity: usize,
}

impl<K: Hash + Eq + Clone, V: Clone> ClockCache<K, V> {
    /// An empty cache of `capacity` (at least 1) entries.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self::with_hasher(capacity, foldhash::fast::RandomState::default())
    }
}

impl<K: Hash + Eq + Clone, V: Clone, S: BuildHasher> ClockCache<K, V, S> {
    /// An empty cache of `capacity` (at least 1) entries hashing with `s`.
    pub fn with_hasher(capacity: usize, s: S) -> Self {
        Self {
            map: HashMap::with_hasher(s),
            slots: Vec::new(),
            free: Vec::new(),
            hand: 0,
            capacity: capacity.max(1),
        }
    }

    /// Cached entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Maximum entries.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Every cached value (diagnostics; does not mark them referenced).
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.slots
            .iter()
            .filter_map(|slot| slot.entry.as_ref().map(|(_, v)| v))
    }

    /// The value of `key`, marking it referenced.
    pub fn get(&mut self, key: &K) -> Option<V> {
        let &i = self.map.get(key)?;
        let slot = &mut self.slots[i];
        slot.referenced = true;
        slot.entry.as_ref().map(|(_, v)| v.clone())
    }

    /// Inserts or replaces `key`, evicting an unreferenced entry when full.
    pub fn put(&mut self, key: K, value: V) {
        if let Some(&i) = self.map.get(&key) {
            let slot = &mut self.slots[i];
            slot.entry = Some((key, value));
            slot.referenced = true;
            return;
        }
        let i = if let Some(i) = self.free.pop() {
            i
        } else if self.slots.len() < self.capacity {
            self.slots.push(Slot {
                entry: None,
                referenced: false,
            });
            self.slots.len() - 1
        } else {
            self.evict()
        };
        self.map.insert(key.clone(), i);
        // New entries start unreferenced: one sweep of the hand evicts
        // them unless they are used again (scan resistance).
        self.slots[i] = Slot {
            entry: Some((key, value)),
            referenced: false,
        };
    }

    /// Removes `key`.
    pub fn remove(&mut self, key: &K) {
        if let Some(i) = self.map.remove(key) {
            self.slots[i].entry = None;
            self.slots[i].referenced = false;
            self.free.push(i);
        }
    }

    /// Frees the slot of the first unreferenced entry at or after the hand
    /// (clearing reference bits on the way) and returns its index.
    fn evict(&mut self) -> usize {
        loop {
            let i = self.hand;
            self.hand = (self.hand + 1) % self.slots.len();
            let slot = &mut self.slots[i];
            if slot.referenced {
                slot.referenced = false;
                continue;
            }
            if let Some((k, _)) = slot.entry.take() {
                self.map.remove(&k);
            }
            return i;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_at_most_capacity_and_returns_what_was_put() {
        let mut c: ClockCache<u32, u32> = ClockCache::new(4);
        for i in 0..10 {
            c.put(i, i * 10);
            assert!(c.len() <= 4);
        }
        assert_eq!(c.len(), 4);
        for i in 6..10 {
            assert_eq!(c.get(&i), Some(i * 10));
        }
        c.put(7, 70);
        assert_eq!(c.get(&7), Some(70));
        c.remove(&7);
        assert_eq!(c.get(&7), None);
        assert_eq!(c.len(), 3);
        c.put(100, 1);
        assert_eq!(c.len(), 4);
        assert_eq!(c.get(&100), Some(1));
    }

    #[test]
    fn referenced_entries_get_a_second_chance() {
        let mut c: ClockCache<u32, u32> = ClockCache::new(4);
        for i in 0..4 {
            c.put(i, i);
        }
        // 0 and 2 are used; inserting two more evicts 1 and 3.
        assert_eq!(c.get(&0), Some(0));
        assert_eq!(c.get(&2), Some(2));
        c.put(10, 10);
        c.put(11, 11);
        assert_eq!(c.get(&0), Some(0));
        assert_eq!(c.get(&2), Some(2));
        assert_eq!(c.get(&1), None);
        assert_eq!(c.get(&3), None);
        assert_eq!(c.get(&10), Some(10));
        assert_eq!(c.get(&11), Some(11));
    }

    #[test]
    fn capacity_is_at_least_one() {
        let mut c: ClockCache<u32, u32> = ClockCache::new(0);
        c.put(1, 1);
        c.put(2, 2);
        assert_eq!(c.len(), 1);
        assert_eq!(c.get(&2), Some(2));
    }
}
