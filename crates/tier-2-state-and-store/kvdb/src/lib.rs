//! Ordered key/value backends for persistent state (ADR 0002).
//!
//! [`KvBackend`] is the small storage contract the IAVL node store and the
//! multistore build on: point reads, ordered (forward/reverse) range scans and
//! atomic write batches. Implementations:
//!
//! - [`MemBackend`]: a `BTreeMap` behind a lock, for tests and for nodes that
//!   do not need durability;
//! - `RocksBackend` (`rocksdb` feature, the node default): a RocksDB LSM
//!   tree; every [`KvBackend::write`] is one atomic RocksDB `WriteBatch`;
//! - [`RedbBackend`]: a single-table [`redb`] database (pure Rust, ACID,
//!   copy-on-write B-tree). Every [`KvBackend::write`] is one redb write
//!   transaction, so a batch is either fully visible after a crash or not at
//!   all.
//!
//! [`open_backend`] opens the one selected by [`DbOptions`]; ADR 0002 has
//! the measurements behind the default.
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

#[cfg(feature = "rocksdb")]
mod rocks;
#[cfg(feature = "rocksdb")]
pub use rocks::{RocksBackend, RocksOptions};

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

    /// Space accounting, when the backend is file based (diagnostics).
    fn stats(&self) -> Option<StorageStats> {
        None
    }
}

/// Space accounting of a file-based backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StorageStats {
    /// Bytes of the database file(s) on disk.
    pub file_bytes: u64,
    /// Bytes holding data: redb allocated pages; RocksDB SST files plus
    /// memtables.
    pub allocated_bytes: u64,
    /// Bytes of backend caches in memory (RocksDB block cache and memtables;
    /// redb reports 0, its cache is bounded by `RedbOptions::cache_size`).
    pub cache_bytes: u64,
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
    /// Run `Database::compact()` when opening: relocates pages to the start
    /// of the file and truncates the free tail (slow on large files).
    pub compact_on_open: bool,
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
            compact_on_open: false,
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
        let mut db = redb::Builder::new()
            .set_cache_size(options.cache_size)
            .create(&path)
            .map_err(Error::new)?;
        if options.compact_on_open {
            while db.compact().map_err(Error::new)? {}
        }
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

    fn stats(&self) -> Option<StorageStats> {
        let txn = self.db.begin_write().ok()?;
        let stats = txn.stats().ok()?;
        txn.abort().ok()?;
        Some(StorageStats {
            file_bytes: self.disk_size()?,
            allocated_bytes: stats.allocated_pages() * stats.page_size() as u64,
            cache_bytes: 0,
        })
    }
}

// ---------------------------------------------------------------------------
// Backend selection
// ---------------------------------------------------------------------------

/// Persistent backend implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    /// `RocksBackend` (`rocksdb` feature): LSM tree, disk tracks live data.
    #[cfg(feature = "rocksdb")]
    RocksDb,
    /// [`RedbBackend`]: pure-Rust copy-on-write B-tree.
    Redb,
}

impl BackendKind {
    /// Every backend compiled in, default first.
    pub const ALL: &'static [Self] = &[
        #[cfg(feature = "rocksdb")]
        Self::RocksDb,
        Self::Redb,
    ];

    /// Name used in flags and file names (`rocksdb`, `redb`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            #[cfg(feature = "rocksdb")]
            Self::RocksDb => "rocksdb",
            Self::Redb => "redb",
        }
    }

    /// Path of database `name` of this backend inside `dir`:
    /// `<name>.rocksdb/` (a directory) or `<name>.redb` (a file). The
    /// extensions keep clear of Go's goleveldb `<name>.db` directories in a
    /// shared node home.
    #[must_use]
    pub fn db_path(self, dir: &Path, name: &str) -> PathBuf {
        dir.join(format!("{name}.{}", self.name()))
    }
}

impl Default for BackendKind {
    /// RocksDB when compiled in (ADR 0002 measurements), else redb.
    fn default() -> Self {
        Self::ALL[0]
    }
}

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl std::str::FromStr for BackendKind {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|k| k.name() == s)
            .ok_or_else(|| {
                let names: Vec<_> = Self::ALL.iter().map(|k| k.name()).collect();
                Error::new(format!(
                    "unknown db backend {s:?} (built with: {})",
                    names.join(", ")
                ))
            })
    }
}

/// Backend-neutral options of a node's state database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DbOptions {
    /// Implementation.
    pub backend: BackendKind,
    /// Backend cache in bytes (RocksDB block cache / redb page cache).
    pub cache_size: usize,
    /// Fsync every committed batch (see [`RedbOptions::sync_writes`]).
    pub sync_writes: bool,
}

impl Default for DbOptions {
    /// Validator defaults: default backend, 512 MiB cache, synced commits.
    fn default() -> Self {
        Self {
            backend: BackendKind::default(),
            cache_size: 512 << 20,
            sync_writes: true,
        }
    }
}

/// Opens (or creates) database `name` in directory `dir`
/// ([`BackendKind::db_path`]).
///
/// Refuses to create a fresh database when `dir` already holds database
/// `name` written by another backend, so a misconfigured node does not
/// silently restart from an empty state.
pub fn open_backend(dir: &Path, name: &str, options: &DbOptions) -> Result<Arc<dyn KvBackend>> {
    let path = options.backend.db_path(dir, name);
    if !path.exists() {
        for other in BackendKind::ALL {
            let existing = other.db_path(dir, name);
            if *other != options.backend && existing.exists() {
                return Err(Error::new(format!(
                    "{} is a {other} database; select the {other} backend to open it",
                    existing.display()
                )));
            }
        }
    }
    Ok(match options.backend {
        #[cfg(feature = "rocksdb")]
        BackendKind::RocksDb => Arc::new(RocksBackend::open(
            path,
            RocksOptions {
                block_cache_size: options.cache_size,
                sync_writes: options.sync_writes,
                ..RocksOptions::default()
            },
        )?),
        BackendKind::Redb => Arc::new(RedbBackend::open(
            path,
            RedbOptions {
                cache_size: options.cache_size,
                sync_writes: options.sync_writes,
                ..RedbOptions::default()
            },
        )?),
    })
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

    fn stats(&self) -> Option<StorageStats> {
        self.inner.stats()
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
    fn redb_stats_and_compaction_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.redb");
        {
            let db = RedbBackend::open(&path, RedbOptions::default()).unwrap();
            let mut batch = WriteBatch::new();
            for i in 0u32..2_000 {
                batch.put(i.to_be_bytes().to_vec(), vec![7; 200]);
            }
            db.write(batch).unwrap();
            let mut batch = WriteBatch::new();
            for i in 0u32..1_900 {
                batch.delete(i.to_be_bytes().to_vec());
            }
            db.write(batch).unwrap();
            let stats = db.stats().unwrap();
            assert!(stats.file_bytes >= stats.allocated_bytes);
            assert!(stats.allocated_bytes > 0);
        }
        let db = RedbBackend::open(
            &path,
            RedbOptions {
                compact_on_open: true,
                ..RedbOptions::default()
            },
        )
        .unwrap();
        assert_eq!(collect(db.range(None, None, false).unwrap()).len(), 100);
    }

    #[test]
    fn backend_kind_names_and_mismatch_guard() {
        for kind in BackendKind::ALL {
            assert_eq!(kind.name().parse::<BackendKind>().unwrap(), *kind);
            assert_eq!(kind.to_string(), kind.name());
        }
        assert_eq!(BackendKind::default(), BackendKind::ALL[0]);
        assert!("leveldb".parse::<BackendKind>().is_err());
        let dir = tempfile::tempdir().unwrap();
        for kind in BackendKind::ALL {
            let options = DbOptions {
                backend: *kind,
                sync_writes: false,
                cache_size: 1 << 20,
            };
            let sub = dir.path().join(kind.name());
            let db = open_backend(&sub, "state", &options).unwrap();
            exercise(db.as_ref());
            assert!(db.stats().unwrap().file_bytes > 0);
            drop(db);
            assert!(kind.db_path(&sub, "state").exists());
            for other in BackendKind::ALL.iter().filter(|o| *o != kind) {
                let other = DbOptions {
                    backend: *other,
                    ..options
                };
                assert!(open_backend(&sub, "state", &other).is_err());
                drop(open_backend(&sub, "other", &other).unwrap());
            }
        }
        assert!(DbOptions::default().sync_writes);
    }

    #[cfg(feature = "rocksdb")]
    #[test]
    fn rocks_backend_contract_reopen_and_chunked_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rocks");
        {
            let db = RocksBackend::open(&path, RocksOptions::default()).unwrap();
            exercise(&db);
            assert_eq!(db.write_count(), 2);
            assert_eq!(db.path(), path.as_path());
            // More entries than one iterator chunk, in both directions.
            let mut batch = WriteBatch::new();
            for i in 0u32..2_500 {
                batch.put([b"p".as_slice(), &i.to_be_bytes()].concat(), vec![1; 10]);
            }
            db.write(batch).unwrap();
            let fwd = collect(db.prefix_range(b"p", false).unwrap());
            assert_eq!(fwd.len(), 2_500);
            assert!(fwd.windows(2).all(|w| w[0].0 < w[1].0));
            let rev = collect(db.prefix_range(b"p", true).unwrap());
            assert_eq!(rev.len(), 2_500);
            assert!(rev.windows(2).all(|w| w[0].0 > w[1].0));
            let mid = collect(
                db.range(
                    Some(&[b"p".as_slice(), &100u32.to_be_bytes()].concat()),
                    Some(&[b"p".as_slice(), &1_300u32.to_be_bytes()].concat()),
                    true,
                )
                .unwrap(),
            );
            assert_eq!(mid.len(), 1_200);
            db.compact().unwrap();
            assert!(db.stats().unwrap().allocated_bytes > 0);
            assert!(db.property("rocksdb.estimate-num-keys").is_some());
            db.sync().unwrap();
        }
        let db = RocksBackend::open(&path, RocksOptions::default()).unwrap();
        assert_eq!(db.get(b"y").unwrap(), Some(b"2".to_vec()));
        assert_eq!(collect(db.range(None, None, false).unwrap()).len(), 2_505);
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
