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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use kvdb::{KvBackend, WriteBatch};

use crate::clock::ClockCache;
use crate::node::{NODE_KEY_LEN, Node, NodeKey, StoredRef, decode_node, decode_root, read_ref};
use crate::{Error, Result, Version};

const NODE: u8 = b'n';
const ROOT: u8 = b'r';
const ORPHAN: u8 = b'o';

static NEXT_DB_ID: AtomicU64 = AtomicU64::new(1);

// Resolving a compact parent's child references reads the child records.
// Keep those bytes in the same bounded cache until a traversal needs them.
#[derive(Clone)]
enum CachedNode {
    Decoded(Arc<Node>),
    Encoded(Arc<EncodedNode>),
}

struct EncodedNode {
    reference: StoredRef,
    bytes: Vec<u8>,
}

/// Bounded CLOCK cache of persisted nodes, shareable by many trees.
///
/// A hit only sets a reference bit (see [`ClockCache`]), so lookups do not
/// relink a list the way an LRU does.
pub struct NodeCache {
    clock: Mutex<ClockCache<(u64, NodeKey), CachedNode>>,
    hits: AtomicU64,
    misses: AtomicU64,
    disk_reads: AtomicU64,
    disk_bytes: AtomicU64,
    ref_reads: AtomicU64,
    ref_decodes: AtomicU64,
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
    /// Node records read from the backend.
    pub disk_reads: u64,
    /// Bytes of those records.
    pub disk_bytes: u64,
    /// Of `disk_reads`, those made only to resolve a parent's child
    /// reference (the child itself was not needed yet).
    pub ref_reads: u64,
    /// Records first read for a child reference and later decoded for a
    /// traversal (`ref_reads - ref_decodes` read only for the reference,
    /// as long as the record stayed cached).
    pub ref_decodes: u64,
}

impl NodeCache {
    /// Creates a cache holding at most `capacity` nodes (at least one).
    #[must_use]
    pub fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            clock: Mutex::new(ClockCache::new(capacity)),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            disk_reads: AtomicU64::new(0),
            disk_bytes: AtomicU64::new(0),
            ref_reads: AtomicU64::new(0),
            ref_decodes: AtomicU64::new(0),
        })
    }

    /// Current counters.
    #[must_use]
    pub fn stats(&self) -> CacheStats {
        let (len, capacity) = self
            .clock
            .lock()
            .map_or((0, 0), |clock| (clock.len(), clock.capacity()));
        CacheStats {
            len,
            capacity,
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            disk_reads: self.disk_reads.load(Ordering::Relaxed),
            disk_bytes: self.disk_bytes.load(Ordering::Relaxed),
            ref_reads: self.ref_reads.load(Ordering::Relaxed),
            ref_decodes: self.ref_decodes.load(Ordering::Relaxed),
        }
    }

    fn get(&self, key: &(u64, NodeKey)) -> Option<CachedNode> {
        let found = self.clock.lock().ok()?.get(key);
        if found.is_some() {
            self.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
        }
        found
    }

    fn put(&self, key: (u64, NodeKey), node: CachedNode) {
        if let Ok(mut clock) = self.clock.lock() {
            clock.put(key, node);
        }
    }

    fn remove(&self, key: &(u64, NodeKey)) {
        if let Ok(mut clock) = self.clock.lock() {
            clock.remove(key);
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
        let decode = |bytes: &[u8]| decode_node(bytes, r, |nk| self.child_ref(nk));
        let node = Arc::new(match self.cache.get(&cache_key) {
            Some(CachedNode::Decoded(node)) => return Ok(node),
            Some(CachedNode::Encoded(node)) => {
                self.cache.ref_decodes.fetch_add(1, Ordering::Relaxed);
                decode(&node.bytes)?
            }
            None => decode(&self.node_bytes(r.nk)?)?,
        });
        self.cache
            .put(cache_key, CachedNode::Decoded(Arc::clone(&node)));
        Ok(node)
    }

    fn node_bytes(&self, nk: NodeKey) -> Result<Vec<u8>> {
        let bytes = self
            .backend
            .get(&self.node_key(nk))
            .map_err(db_err)?
            .ok_or(Error::MissingNode {
                version: nk.version,
                nonce: nk.nonce,
            })?;
        self.cache.disk_reads.fetch_add(1, Ordering::Relaxed);
        self.cache
            .disk_bytes
            .fetch_add(bytes.len() as u64, Ordering::Relaxed);
        Ok(bytes)
    }

    fn child_ref(&self, nk: NodeKey) -> Result<StoredRef> {
        let key = (self.id, nk);
        match self.cache.get(&key) {
            Some(CachedNode::Decoded(node)) => Ok(StoredRef {
                nk,
                hash: node.hash_for(nk.version),
                height: node.height(),
                size: node.size(),
            }),
            Some(CachedNode::Encoded(node)) => Ok(node.reference),
            None => {
                let bytes = self.node_bytes(nk)?;
                self.cache.ref_reads.fetch_add(1, Ordering::Relaxed);
                let reference = read_ref(&bytes, nk)?;
                self.cache.put(
                    key,
                    CachedNode::Encoded(Arc::new(EncodedNode { reference, bytes })),
                );
                Ok(reference)
            }
        }
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
        self.cache.put((self.id, nk), CachedNode::Decoded(node));
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
