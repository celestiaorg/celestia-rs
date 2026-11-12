//! Bounded, app-local memoization of successful blob commitment validation.

use std::collections::{HashSet, VecDeque};

const CAPACITY: usize = 10_000;
type Key = (u64, hash::Sha256Digest);

/// Only full, successful New CheckTx calls admit entries. Keys bind the
/// application version and complete BlobTx wire bytes, including every blob
/// boundary and metadata field. No transaction or blob payload is retained.
#[derive(Debug)]
pub(crate) struct BlobTxCache {
    capacity: usize,
    entries: HashSet<Key>,
    order: VecDeque<Key>,
}

impl Default for BlobTxCache {
    fn default() -> Self {
        let disabled =
            std::env::var_os("CELESTIA_RS_BLOB_TX_CACHE").is_some_and(|value| value == "off");
        Self::with_capacity(if disabled { 0 } else { CAPACITY })
    }
}

impl BlobTxCache {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity: capacity.min(CAPACITY),
            entries: HashSet::new(),
            order: VecDeque::new(),
        }
    }

    pub(crate) fn contains(&self, version: u64, raw: &[u8]) -> bool {
        self.capacity > 0 && self.entries.contains(&(version, hash::sha256(raw)))
    }

    pub(crate) fn remember(&mut self, version: u64, raw: &[u8]) {
        if self.capacity == 0 {
            return;
        }
        let key = (version, hash::sha256(raw));
        if self.entries.contains(&key) {
            return;
        }
        // FIFO eviction keeps both containers bounded without retaining large
        // blobs. Eviction changes performance only: misses run full validation.
        if self.entries.len() == self.capacity
            && let Some(oldest) = self.order.pop_front()
        {
            self.entries.remove(&oldest);
        }
        self.entries.insert(key);
        self.order.push_back(key);
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_bind_the_full_wire_transaction_and_app_version() {
        let mut cache = BlobTxCache::with_capacity(2);
        cache.remember(10, b"sdk-tx/blob-data");
        assert!(cache.contains(10, b"sdk-tx/blob-data"));
        assert!(!cache.contains(9, b"sdk-tx/blob-data"));
        assert!(!cache.contains(10, b"sdk-tx/blob-datb"));
        assert!(!cache.contains(10, b"sdk-tx/blob-data/metadata"));
    }

    #[test]
    fn eviction_and_repeated_admission_stay_bounded() {
        let mut cache = BlobTxCache::with_capacity(2);
        cache.remember(10, b"a");
        cache.remember(10, b"a");
        cache.remember(10, b"b");
        cache.remember(10, b"c");
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.order.len(), 2);
        assert!(!cache.contains(10, b"a"));
        assert!(cache.contains(10, b"b"));
        assert!(cache.contains(10, b"c"));
        let mut disabled = BlobTxCache::with_capacity(0);
        disabled.remember(10, b"a");
        assert_eq!(disabled.len(), 0);
        assert!(!disabled.contains(10, b"a"));
    }
}
