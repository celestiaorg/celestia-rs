//! Ordered key/value backends for persistent state (ADR 0002).
//!
//! [`KvBackend`] is the small storage contract the IAVL node store and the
//! multistore build on: point reads, ordered (forward/reverse) range scans and
//! atomic write batches. Two implementations ship here:
//!
//! - [`MemBackend`]: a `BTreeMap` behind a lock, for tests and for nodes that
//!   do not need durability;
//! - [`RedbBackend`]: a single-table [`redb`] database (pure Rust, ACID,
//!   copy-on-write B-tree). Every [`KvBackend::write`] is one redb write
//!   transaction, so a batch is either fully visible after a crash or not at
//!   all.
//!
//! [`FaultyBackend`] wraps another backend and can be told to fail writes; the
//! crash-consistency tests use it to simulate a commit interrupted before its
//! transaction became durable.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::BTreeMap;
use std::fmt;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use redb::TableDefinition;

/// Result alias for backend operations.
pub type Result<T> = core::result::Result<T, Error>;

/// Backend error (I/O, corruption or injected failure).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl Error {
    /// Creates an error from any displayable value.
    pub fn new(message: impl fmt::Display) -> Self {
        Self(message.to_string())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "kvdb: {}", self.0)
    }
}

impl std::error::Error for Error {}

/// One write operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// Insert or overwrite `key` with `value`.
    Put(Vec<u8>, Vec<u8>),
    /// Remove `key` (no-op when absent).
    Delete(Vec<u8>),
}

/// An ordered list of writes applied atomically by [`KvBackend::write`].
/// Later operations on the same key win.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WriteBatch {
    ops: Vec<Op>,
}

impl WriteBatch {
    /// Creates an empty batch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues a put.
    pub fn put(&mut self, key: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) {
        self.ops.push(Op::Put(key.into(), value.into()));
    }

    /// Queues a delete.
    pub fn delete(&mut self, key: impl Into<Vec<u8>>) {
        self.ops.push(Op::Delete(key.into()));
    }

    /// Appends all operations of `other`.
    pub fn extend(&mut self, other: Self) {
        self.ops.extend(other.ops);
    }

    /// Number of queued operations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// True when nothing is queued.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Queued operations in application order.
    #[must_use]
    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    /// Total key + value bytes queued (diagnostics).
    #[must_use]
    pub fn byte_size(&self) -> usize {
        self.ops
            .iter()
            .map(|op| match op {
                Op::Put(k, v) => k.len() + v.len(),
                Op::Delete(k) => k.len(),
            })
            .sum()
    }
}

/// An owned key/value pair.
pub type KvPair = (Vec<u8>, Vec<u8>);

/// A range iterator. It does not borrow the backend.
pub type KvIter = Box<dyn Iterator<Item = Result<KvPair>> + Send>;

/// Ordered key/value storage with atomic batches.
pub trait KvBackend: Send + Sync + fmt::Debug {
    /// Reads one key.
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;

    /// Iterates `[start, end)` in ascending order, or descending when
    /// `reverse` is set. `None` bounds are open.
    fn range(&self, start: Option<&[u8]>, end: Option<&[u8]>, reverse: bool) -> Result<KvIter>;

    /// Applies `batch` atomically: after a crash either every operation is
    /// visible or none is.
    fn write(&self, batch: WriteBatch) -> Result<()>;

    /// Makes every completed write durable.
    fn sync(&self) -> Result<()>;

    /// Iterates all keys starting with `prefix`.
    fn prefix_range(&self, prefix: &[u8], reverse: bool) -> Result<KvIter> {
        let end = prefix_end(prefix);
        self.range(Some(prefix), end.as_deref(), reverse)
    }

    /// Bytes used on disk, when the backend is file based.
    fn disk_size(&self) -> Option<u64> {
        None
    }
}

/// Smallest key greater than every key starting with `prefix`, or `None`
/// when no such key exists (empty or all-`0xff` prefix).
#[must_use]
pub fn prefix_end(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut end = prefix.to_vec();
    while let Some(last) = end.pop() {
        if last < u8::MAX {
            end.push(last + 1);
            return Some(end);
        }
    }
    None
}

fn bounds<'a>(
    start: Option<&'a [u8]>,
    end: Option<&'a [u8]>,
) -> (Bound<&'a [u8]>, Bound<&'a [u8]>) {
    (
        start.map_or(Bound::Unbounded, Bound::Included),
        end.map_or(Bound::Unbounded, Bound::Excluded),
    )
}

fn empty_range(start: Option<&[u8]>, end: Option<&[u8]>) -> bool {
    matches!((start, end), (Some(s), Some(e)) if s >= e)
}

// ---------------------------------------------------------------------------
// In-memory backend
// ---------------------------------------------------------------------------

/// In-memory backend. Cloning shares the underlying map.
#[derive(Debug, Clone, Default)]
pub struct MemBackend {
    map: Arc<RwLock<BTreeMap<Vec<u8>, Vec<u8>>>>,
}

impl MemBackend {
    /// Creates an empty backend.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of stored keys.
    #[must_use]
    pub fn len(&self) -> usize {
        self.map.read().map_or(0, |m| m.len())
    }

    /// True when no key is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Total key + value bytes stored.
    #[must_use]
    pub fn byte_size(&self) -> usize {
        self.map
            .read()
            .map_or(0, |m| m.iter().map(|(k, v)| k.len() + v.len()).sum())
    }
}

fn poisoned<T>(_: T) -> Error {
    Error::new("lock poisoned")
}

impl KvBackend for MemBackend {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(self.map.read().map_err(poisoned)?.get(key).cloned())
    }

    fn range(&self, start: Option<&[u8]>, end: Option<&[u8]>, reverse: bool) -> Result<KvIter> {
        if empty_range(start, end) {
            return Ok(Box::new(std::iter::empty()));
        }
        let map = self.map.read().map_err(poisoned)?;
        let iter = map
            .range::<[u8], _>(bounds(start, end))
            .map(|(k, v)| Ok((k.clone(), v.clone())));
        let items: Vec<Result<KvPair>> = if reverse {
            iter.rev().collect()
        } else {
            iter.collect()
        };
        Ok(Box::new(items.into_iter()))
    }

    fn write(&self, batch: WriteBatch) -> Result<()> {
        let mut map = self.map.write().map_err(poisoned)?;
        for op in batch.ops {
            match op {
                Op::Put(k, v) => {
                    map.insert(k, v);
                }
                Op::Delete(k) => {
                    map.remove(&k);
                }
            }
        }
        Ok(())
    }

    fn sync(&self) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// redb backend
// ---------------------------------------------------------------------------

const TABLE: TableDefinition<'static, &'static [u8], &'static [u8]> = TableDefinition::new("kv");

type RedbItem = core::result::Result<
    (
        redb::AccessGuard<'static, &'static [u8]>,
        redb::AccessGuard<'static, &'static [u8]>,
    ),
    redb::StorageError,
>;

type ReadTable = redb::ReadOnlyTable<&'static [u8], &'static [u8]>;

/// Options for [`RedbBackend`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedbOptions {
    /// redb page-cache size in bytes.
    pub cache_size: usize,
    /// When true every write is fsynced before it returns
    /// (`Durability::Immediate`). When false writes are atomic but only
    /// become durable at the next [`KvBackend::sync`] (tests, benchmarks).
    pub sync_writes: bool,
}

impl Default for RedbOptions {
    fn default() -> Self {
        Self {
            cache_size: 256 << 20,
            sync_writes: true,
        }
    }
}

/// [`redb`]-backed persistent backend: one database file, one table.
pub struct RedbBackend {
    db: redb::Database,
    path: PathBuf,
    options: RedbOptions,
    /// Read handle reused across reads; dropped after every write so that
    /// readers see the new state and redb can reclaim freed pages.
    reader: Mutex<Option<ReadTable>>,
    writes: AtomicU64,
}

impl fmt::Debug for RedbBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RedbBackend")
            .field("path", &self.path)
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl RedbBackend {
    /// Opens (or creates) the database file at `path`.
    pub fn open(path: impl AsRef<Path>, options: RedbOptions) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(Error::new)?;
        }
        let db = redb::Builder::new()
            .set_cache_size(options.cache_size)
            .create(&path)
            .map_err(Error::new)?;
        // Make sure the table exists so read transactions can open it.
        let txn = db.begin_write().map_err(Error::new)?;
        txn.open_table(TABLE).map_err(Error::new)?;
        txn.commit().map_err(Error::new)?;
        Ok(Self {
            db,
            path,
            options,
            reader: Mutex::new(None),
            writes: AtomicU64::new(0),
        })
    }

    /// Database file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Number of write transactions committed through this handle.
    #[must_use]
    pub fn write_count(&self) -> u64 {
        self.writes.load(Ordering::Relaxed)
    }

    fn with_table<T>(&self, f: impl FnOnce(&ReadTable) -> Result<T>) -> Result<T> {
        let mut guard = self.reader.lock().map_err(poisoned)?;
        if guard.is_none() {
            let txn = self.db.begin_read().map_err(Error::new)?;
            *guard = Some(txn.open_table(TABLE).map_err(Error::new)?);
        }
        match guard.as_ref() {
            Some(table) => f(table),
            None => Err(Error::new("read table unavailable")),
        }
    }

    /// Begins a write transaction, applies `apply`, and **drops it without
    /// committing**: what a crash in the middle of a commit leaves behind.
    /// Test hook for crash-consistency tests.
    pub fn abort_write(&self, batch: &WriteBatch) -> Result<()> {
        let txn = self.db.begin_write().map_err(Error::new)?;
        {
            let mut table = txn.open_table(TABLE).map_err(Error::new)?;
            for op in &batch.ops {
                match op {
                    Op::Put(k, v) => {
                        table
                            .insert(k.as_slice(), v.as_slice())
                            .map_err(Error::new)?;
                    }
                    Op::Delete(k) => {
                        table.remove(k.as_slice()).map_err(Error::new)?;
                    }
                }
            }
        }
        txn.abort().map_err(Error::new)
    }
}

impl KvBackend for RedbBackend {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.with_table(|table| {
            Ok(table
                .get(key)
                .map_err(Error::new)?
                .map(|v| v.value().to_vec()))
        })
    }

    fn range(&self, start: Option<&[u8]>, end: Option<&[u8]>, reverse: bool) -> Result<KvIter> {
        if empty_range(start, end) {
            return Ok(Box::new(std::iter::empty()));
        }
        let range =
            self.with_table(|table| table.range::<&[u8]>(bounds(start, end)).map_err(Error::new))?;
        let map = |item: RedbItem| {
            item.map(|(k, v)| (k.value().to_vec(), v.value().to_vec()))
                .map_err(Error::new)
        };
        Ok(if reverse {
            Box::new(range.rev().map(map))
        } else {
            Box::new(range.map(map))
        })
    }

    fn write(&self, batch: WriteBatch) -> Result<()> {
        // Drop the cached reader first so it does not pin old pages.
        *self.reader.lock().map_err(poisoned)? = None;
        let mut txn = self.db.begin_write().map_err(Error::new)?;
        txn.set_durability(if self.options.sync_writes {
            redb::Durability::Immediate
        } else {
            redb::Durability::None
        });
        {
            let mut table = txn.open_table(TABLE).map_err(Error::new)?;
            for op in &batch.ops {
                match op {
                    Op::Put(k, v) => {
                        table
                            .insert(k.as_slice(), v.as_slice())
                            .map_err(Error::new)?;
                    }
                    Op::Delete(k) => {
                        table.remove(k.as_slice()).map_err(Error::new)?;
                    }
                }
            }
        }
        txn.commit().map_err(Error::new)?;
        self.writes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn sync(&self) -> Result<()> {
        if self.options.sync_writes {
            return Ok(());
        }
        *self.reader.lock().map_err(poisoned)? = None;
        let mut txn = self.db.begin_write().map_err(Error::new)?;
        txn.set_durability(redb::Durability::Immediate);
        txn.commit().map_err(Error::new)
    }

    fn disk_size(&self) -> Option<u64> {
        std::fs::metadata(&self.path).ok().map(|m| m.len())
    }
}

// ---------------------------------------------------------------------------
// Fault injection
// ---------------------------------------------------------------------------

/// Wraps a backend and fails writes on demand (crash simulation).
#[derive(Debug)]
pub struct FaultyBackend {
    inner: Arc<dyn KvBackend>,
    fail_writes: AtomicBool,
}

impl FaultyBackend {
    /// Wraps `inner`; writes succeed until [`Self::fail_writes`] is set.
    #[must_use]
    pub fn new(inner: Arc<dyn KvBackend>) -> Self {
        Self {
            inner,
            fail_writes: AtomicBool::new(false),
        }
    }

    /// Makes every subsequent write fail without applying anything.
    pub fn fail_writes(&self, fail: bool) {
        self.fail_writes.store(fail, Ordering::SeqCst);
    }
}

impl KvBackend for FaultyBackend {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.inner.get(key)
    }

    fn range(&self, start: Option<&[u8]>, end: Option<&[u8]>, reverse: bool) -> Result<KvIter> {
        self.inner.range(start, end, reverse)
    }

    fn write(&self, batch: WriteBatch) -> Result<()> {
        if self.fail_writes.load(Ordering::SeqCst) {
            return Err(Error::new("injected write failure (simulated crash)"));
        }
        self.inner.write(batch)
    }

    fn sync(&self) -> Result<()> {
        self.inner.sync()
    }

    fn disk_size(&self) -> Option<u64> {
        self.inner.disk_size()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn collect(iter: KvIter) -> Vec<KvPair> {
        iter.map(Result::unwrap).collect()
    }

    fn exercise(backend: &dyn KvBackend) {
        let mut batch = WriteBatch::new();
        for k in [b"a1", b"a2", b"b1", b"b2", b"c1"] {
            batch.put(k.to_vec(), k.to_vec());
        }
        batch.delete(b"b2".to_vec());
        assert_eq!(batch.len(), 6);
        backend.write(batch).unwrap();
        assert_eq!(backend.get(b"a1").unwrap(), Some(b"a1".to_vec()));
        assert_eq!(backend.get(b"b2").unwrap(), None);
        let keys = |v: Vec<KvPair>| v.into_iter().map(|(k, _)| k).collect::<Vec<_>>();
        assert_eq!(
            keys(collect(
                backend.range(Some(b"a2"), Some(b"c1"), false).unwrap()
            )),
            vec![b"a2".to_vec(), b"b1".to_vec()]
        );
        assert_eq!(
            keys(collect(backend.range(None, None, true).unwrap())),
            vec![
                b"c1".to_vec(),
                b"b1".to_vec(),
                b"a2".to_vec(),
                b"a1".to_vec()
            ]
        );
        assert_eq!(
            keys(collect(backend.prefix_range(b"a", false).unwrap())),
            vec![b"a1".to_vec(), b"a2".to_vec()]
        );
        assert!(collect(backend.range(Some(b"z"), Some(b"a"), false).unwrap()).is_empty());
        // Later ops on the same key win.
        let mut batch = WriteBatch::new();
        batch.put(b"x".to_vec(), b"1".to_vec());
        batch.delete(b"x".to_vec());
        batch.put(b"y".to_vec(), b"1".to_vec());
        batch.put(b"y".to_vec(), b"2".to_vec());
        backend.write(batch).unwrap();
        assert_eq!(backend.get(b"x").unwrap(), None);
        assert_eq!(backend.get(b"y").unwrap(), Some(b"2".to_vec()));
        backend.sync().unwrap();
    }

    #[test]
    fn mem_backend_contract() {
        let mem = MemBackend::new();
        exercise(&mem);
        assert_eq!(mem.len(), 5);
        assert!(!mem.is_empty());
        assert!(mem.byte_size() > 0);
    }

    #[test]
    fn redb_backend_contract_and_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.redb");
        {
            let db = RedbBackend::open(&path, RedbOptions::default()).unwrap();
            exercise(&db);
            assert!(db.disk_size().unwrap() > 0);
            assert_eq!(db.write_count(), 2);
        }
        let db = RedbBackend::open(&path, RedbOptions::default()).unwrap();
        assert_eq!(db.get(b"y").unwrap(), Some(b"2".to_vec()));
        assert_eq!(collect(db.range(None, None, false).unwrap()).len(), 5);
    }

    #[test]
    fn redb_aborted_transaction_leaves_no_trace() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.redb");
        {
            let db = RedbBackend::open(&path, RedbOptions::default()).unwrap();
            let mut committed = WriteBatch::new();
            committed.put(b"k".to_vec(), b"old".to_vec());
            db.write(committed).unwrap();
            let mut interrupted = WriteBatch::new();
            interrupted.put(b"k".to_vec(), b"new".to_vec());
            interrupted.put(b"other".to_vec(), b"v".to_vec());
            db.abort_write(&interrupted).unwrap();
            assert_eq!(db.get(b"k").unwrap(), Some(b"old".to_vec()));
        }
        let db = RedbBackend::open(&path, RedbOptions::default()).unwrap();
        assert_eq!(db.get(b"k").unwrap(), Some(b"old".to_vec()));
        assert_eq!(db.get(b"other").unwrap(), None);
    }

    #[test]
    fn redb_unsynced_writes_are_visible_and_sync_works() {
        let dir = tempfile::tempdir().unwrap();
        let db = RedbBackend::open(
            dir.path().join("db.redb"),
            RedbOptions {
                sync_writes: false,
                ..RedbOptions::default()
            },
        )
        .unwrap();
        exercise(&db);
        db.sync().unwrap();
    }

    #[test]
    fn faulty_backend_fails_without_applying() {
        let inner: Arc<dyn KvBackend> = Arc::new(MemBackend::new());
        let faulty = FaultyBackend::new(inner.clone());
        let mut batch = WriteBatch::new();
        batch.put(b"a".to_vec(), b"1".to_vec());
        faulty.write(batch.clone()).unwrap();
        faulty.fail_writes(true);
        batch.put(b"b".to_vec(), b"2".to_vec());
        assert!(faulty.write(batch).is_err());
        assert_eq!(inner.get(b"b").unwrap(), None);
        assert_eq!(faulty.get(b"a").unwrap(), Some(b"1".to_vec()));
    }

    #[test]
    fn prefix_end_edges() {
        assert_eq!(prefix_end(b"ab"), Some(b"ac".to_vec()));
        assert_eq!(prefix_end(&[1, 0xff]), Some(vec![2]));
        assert_eq!(prefix_end(&[0xff, 0xff]), None);
        assert_eq!(prefix_end(b""), None);
        assert_eq!(Error::new("x").to_string(), "kvdb: x");
    }
}
