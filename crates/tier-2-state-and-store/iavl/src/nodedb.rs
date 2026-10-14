//! Persistent node store: IAVL nodes, version roots and orphan records of one
//! tree, under a key prefix of a shared [`KvBackend`].
//!
//! Key layout below the tree prefix:
//!
//! | key                         | value                     |
//! |-----------------------------|---------------------------|
//! | `n` ‖ node key (12 B)       | encoded node              |
//! | `r` ‖ version (8 B BE)      | root reference (empty = empty tree) |
//! | `o` ‖ to-version ‖ node key | empty: node orphaned after `to-version` |
//!
//! An orphan record `o‖t‖nk` means node `nk` is reachable from versions
//! `nk.version ..= t` only. Pruning a version `t` either moves such records
//! to the nearest older retained version still containing the node or
//! deletes the node.

use std::fmt;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use kvdb::{KvBackend, WriteBatch};
use lru::LruCache;

use crate::node::{NODE_KEY_LEN, Node, NodeKey, StoredRef, decode_node, decode_root};
use crate::{Error, Result, Version};

const NODE: u8 = b'n';
const ROOT: u8 = b'r';
const ORPHAN: u8 = b'o';

static NEXT_DB_ID: AtomicU64 = AtomicU64::new(1);

/// Bounded LRU cache of decoded nodes, shareable by many trees.
pub struct NodeCache {
    lru: Mutex<LruCache<(u64, NodeKey), Arc<Node>>>,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl fmt::Debug for NodeCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let stats = self.stats();
        f.debug_struct("NodeCache")
            .field("len", &stats.len)
            .field("capacity", &stats.capacity)
            .finish_non_exhaustive()
    }
}

/// Cache counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    /// Cached nodes.
    pub len: usize,
    /// Maximum cached nodes.
    pub capacity: usize,
    /// Lookups served from the cache.
    pub hits: u64,
    /// Lookups that went to the backend.
    pub misses: u64,
}

impl NodeCache {
    /// Creates a cache holding at most `capacity` nodes (at least one).
    #[must_use]
    pub fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            lru: Mutex::new(LruCache::new(
                NonZeroUsize::new(capacity).unwrap_or(NonZeroUsize::MIN),
            )),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        })
    }

    /// Current counters.
    #[must_use]
    pub fn stats(&self) -> CacheStats {
        let (len, capacity) = self
            .lru
            .lock()
            .map_or((0, 0), |lru| (lru.len(), lru.cap().get()));
        CacheStats {
            len,
            capacity,
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
        }
    }

    fn get(&self, key: &(u64, NodeKey)) -> Option<Arc<Node>> {
        let found = self.lru.lock().ok()?.get(key).cloned();
        if found.is_some() {
            self.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
        }
        found
    }

    fn put(&self, key: (u64, NodeKey), node: Arc<Node>) {
        if let Ok(mut lru) = self.lru.lock() {
            lru.put(key, node);
        }
    }

    fn remove(&self, key: &(u64, NodeKey)) {
        if let Ok(mut lru) = self.lru.lock() {
            lru.pop(key);
        }
    }
}

/// Node storage of one tree.
pub struct NodeDb {
    backend: Arc<dyn KvBackend>,
    prefix: Vec<u8>,
    cache: Arc<NodeCache>,
    id: u64,
}

impl fmt::Debug for NodeDb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeDb")
            .field("prefix", &String::from_utf8_lossy(&self.prefix))
            .finish_non_exhaustive()
    }
}

pub(crate) fn db_err(e: kvdb::Error) -> Error {
    Error::Db(e.0)
}

impl NodeDb {
    /// Node store under `prefix` of `backend`, caching decoded nodes in
    /// `cache`.
    #[must_use]
    pub fn new(
        backend: Arc<dyn KvBackend>,
        prefix: impl Into<Vec<u8>>,
        cache: Arc<NodeCache>,
    ) -> Arc<Self> {
        Arc::new(Self {
            backend,
            prefix: prefix.into(),
            cache,
            id: NEXT_DB_ID.fetch_add(1, Ordering::Relaxed),
        })
    }

    /// Node store with a private in-memory backend (tests).
    #[must_use]
    pub fn in_memory(cache_size: usize) -> Arc<Self> {
        Self::new(
            Arc::new(kvdb::MemBackend::new()),
            Vec::new(),
            NodeCache::new(cache_size),
        )
    }

    /// The backend.
    #[must_use]
    pub fn backend(&self) -> &Arc<dyn KvBackend> {
        &self.backend
    }

    /// The key prefix of this tree.
    #[must_use]
    pub fn prefix(&self) -> &[u8] {
        &self.prefix
    }

    /// The node cache.
    #[must_use]
    pub fn cache(&self) -> &Arc<NodeCache> {
        &self.cache
    }

    fn key(&self, tag: u8, rest: &[u8]) -> Vec<u8> {
        let mut key = Vec::with_capacity(self.prefix.len() + 1 + rest.len());
        key.extend_from_slice(&self.prefix);
        key.push(tag);
        key.extend_from_slice(rest);
        key
    }

    pub(crate) fn node_key(&self, nk: NodeKey) -> Vec<u8> {
        self.key(NODE, &nk.to_bytes())
    }

    pub(crate) fn root_key(&self, version: Version) -> Vec<u8> {
        self.key(ROOT, &version.to_be_bytes())
    }

    pub(crate) fn orphan_key(&self, to: Version, nk: NodeKey) -> Vec<u8> {
        let mut rest = Vec::with_capacity(8 + NODE_KEY_LEN);
        rest.extend_from_slice(&to.to_be_bytes());
        rest.extend_from_slice(&nk.to_bytes());
        self.key(ORPHAN, &rest)
    }

    /// Loads (through the cache) the node `r` points at.
    pub(crate) fn load(&self, r: &StoredRef) -> Result<Arc<Node>> {
        let cache_key = (self.id, r.nk);
        if let Some(node) = self.cache.get(&cache_key) {
            return Ok(node);
        }
        let bytes = self
            .backend
            .get(&self.node_key(r.nk))
            .map_err(db_err)?
            .ok_or(Error::MissingNode {
                version: r.nk.version,
                nonce: r.nk.nonce,
            })?;
        let node = Arc::new(decode_node(&bytes, r)?);
        self.cache.put(cache_key, Arc::clone(&node));
        Ok(node)
    }

    /// Queues a node write and warms the cache with it.
    pub(crate) fn put_node(
        &self,
        batch: &mut WriteBatch,
        nk: NodeKey,
        bytes: Vec<u8>,
        node: Arc<Node>,
    ) {
        batch.put(self.node_key(nk), bytes);
        self.cache.put((self.id, nk), node);
    }

    pub(crate) fn delete_node(&self, batch: &mut WriteBatch, nk: NodeKey) {
        batch.delete(self.node_key(nk));
        self.cache.remove(&(self.id, nk));
    }

    /// All committed versions and their roots, ascending.
    pub(crate) fn roots(&self) -> Result<Vec<(Version, Option<StoredRef>)>> {
        let prefix = self.key(ROOT, &[]);
        let mut out = Vec::new();
        for item in self.backend.prefix_range(&prefix, false).map_err(db_err)? {
            let (key, value) = item.map_err(db_err)?;
            let raw: [u8; 8] = key[prefix.len()..]
                .try_into()
                .map_err(|_| Error::Corrupt("root key"))?;
            out.push((Version::from_be_bytes(raw), decode_root(&value)?));
        }
        Ok(out)
    }

    /// Node keys recorded as orphaned after version `to`.
    pub(crate) fn orphans(&self, to: Version) -> Result<Vec<NodeKey>> {
        let prefix = self.key(ORPHAN, &to.to_be_bytes());
        let mut out = Vec::new();
        for item in self.backend.prefix_range(&prefix, false).map_err(db_err)? {
            let (key, _) = item.map_err(db_err)?;
            out.push(NodeKey::from_bytes(&key[prefix.len()..])?);
        }
        Ok(out)
    }

    /// Keys of the nodes created by `version`.
    pub(crate) fn node_keys_at(&self, version: Version) -> Result<Vec<NodeKey>> {
        let prefix = self.key(NODE, &version.to_be_bytes());
        let mut out = Vec::new();
        for item in self.backend.prefix_range(&prefix, false).map_err(db_err)? {
            let (key, _) = item.map_err(db_err)?;
            out.push(NodeKey::from_bytes(&key[prefix.len() - 8..])?);
        }
        Ok(out)
    }

    /// Number of persisted nodes (diagnostics and pruning tests).
    pub fn node_count(&self) -> Result<usize> {
        let prefix = self.key(NODE, &[]);
        let mut n = 0;
        for item in self.backend.prefix_range(&prefix, false).map_err(db_err)? {
            item.map_err(db_err)?;
            n += 1;
        }
        Ok(n)
    }

    /// Number of orphan records (diagnostics and pruning tests).
    pub fn orphan_count(&self) -> Result<usize> {
        let prefix = self.key(ORPHAN, &[]);
        let mut n = 0;
        for item in self.backend.prefix_range(&prefix, false).map_err(db_err)? {
            item.map_err(db_err)?;
            n += 1;
        }
        Ok(n)
    }

    /// Queues deletion of everything stored under this tree's prefix.
    pub fn clear(&self, batch: &mut WriteBatch) -> Result<()> {
        for item in self
            .backend
            .prefix_range(&self.prefix, false)
            .map_err(db_err)?
        {
            let (key, _) = item.map_err(db_err)?;
            batch.delete(key);
        }
        Ok(())
    }
}
