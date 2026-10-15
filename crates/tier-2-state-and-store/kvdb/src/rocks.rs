//! RocksDB backend (`rocksdb` feature).
//!
//! An LSM tree: writes go to a WAL + memtable and are compacted into sorted
//! files in the background, so deleted and overwritten entries are reclaimed
//! by compaction and the on-disk size tracks live data (see ADR 0002,
//! "Backend measurements").

use std::collections::VecDeque;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rocksdb::{
    BlockBasedOptions, Cache, DB, DBCompressionType, Options, ReadOptions, WriteOptions,
};

use crate::{Error, KvBackend, KvIter, KvPair, Op, Result, StorageStats, WriteBatch, empty_range};

/// Options for [`RocksBackend`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RocksOptions {
    /// Shared LRU block cache in bytes (data, index and filter blocks).
    pub block_cache_size: usize,
    /// Size of one memtable in bytes (at most two are kept).
    pub write_buffer_size: usize,
    /// When true every write batch is fsynced (WAL `sync`) before it returns.
    pub sync_writes: bool,
    /// Upper bound on open SST files (`-1` = unlimited). Keep it below the
    /// process file-descriptor limit (macOS shells default to 256).
    pub max_open_files: i32,
}

impl Default for RocksOptions {
    fn default() -> Self {
        Self {
            block_cache_size: 256 << 20,
            write_buffer_size: 64 << 20,
            sync_writes: true,
            max_open_files: 512,
        }
    }
}

/// Sliding window (entries) of the compact-on-deletion collector: a file
/// with half of any window deleted is marked for compaction.
const DELETION_WINDOW: usize = 32 * 1024;

/// Entries fetched per RocksDB iterator refill (see [`RocksBackend::range`]).
const RANGE_CHUNK: usize = 1024;

/// [`rocksdb`]-backed persistent backend: one database directory, default
/// column family.
pub struct RocksBackend {
    db: Arc<DB>,
    path: PathBuf,
    options: RocksOptions,
    writes: AtomicU64,
}

impl fmt::Debug for RocksBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RocksBackend")
            .field("path", &self.path)
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl RocksBackend {
    /// Opens (or creates) the database directory at `path`.
    pub fn open(path: impl AsRef<Path>, options: RocksOptions) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&path).map_err(Error::new)?;
        let cache = Cache::new_lru_cache(options.block_cache_size);
        let mut table = BlockBasedOptions::default();
        table.set_block_cache(&cache);
        table.set_block_size(16 << 10);
        table.set_bloom_filter(10.0, false);
        // Index and filter blocks live in (and are bounded by) the block
        // cache, so RSS stays proportional to `block_cache_size`.
        table.set_cache_index_and_filter_blocks(true);
        table.set_pin_l0_filter_and_index_blocks_in_cache(true);
        table.set_format_version(5);

        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.set_block_based_table_factory(&table);
        opts.set_level_compaction_dynamic_level_bytes(true);
        opts.set_write_buffer_size(options.write_buffer_size);
        opts.set_max_write_buffer_number(2);
        opts.set_max_total_wal_size(2 * options.write_buffer_size as u64);
        opts.set_max_background_jobs(4);
        opts.set_bytes_per_sync(1 << 20);
        opts.set_max_open_files(options.max_open_files);
        // LZ4 above the bottommost level (cheap; keys, account and balance
        // protos, node keys and sizes compress even though hashes do not),
        // Zstd at the bottom, where most of the data lives. goleveldb
        // compresses every level (Snappy).
        opts.set_compression_type(DBCompressionType::Lz4);
        opts.set_bottommost_compression_type(DBCompressionType::Zstd);
        // IAVL pruning deletes old nodes in bulk: SST files whose entries are
        // mostly tombstones are compacted soon instead of when their level
        // happens to be picked, so the space comes back.
        opts.add_compact_on_deletion_collector_factory(DELETION_WINDOW, DELETION_WINDOW / 2, 0.5);
        opts.set_keep_log_file_num(4);
        opts.set_max_log_file_size(16 << 20);
        let db = DB::open(&opts, &path).map_err(Error::new)?;
        Ok(Self {
            db: Arc::new(db),
            path,
            options,
            writes: AtomicU64::new(0),
        })
    }

    /// Database directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Number of write batches committed through this handle.
    #[must_use]
    pub fn write_count(&self) -> u64 {
        self.writes.load(Ordering::Relaxed)
    }

    /// Integer RocksDB property (e.g. `rocksdb.estimate-live-data-size`).
    #[must_use]
    pub fn property(&self, name: &str) -> Option<u64> {
        self.db.property_int_value(name).ok().flatten()
    }

    /// Flushes memtables and compacts the whole key space (maintenance).
    pub fn compact(&self) -> Result<()> {
        self.db.flush().map_err(Error::new)?;
        self.db.compact_range::<&[u8], &[u8]>(None, None);
        Ok(())
    }
}

impl Drop for RocksBackend {
    /// Writes the memtables to SST files on close: RocksDB keeps them only
    /// in the WAL otherwise, and the next open replays up to two memtables
    /// (128 MiB) before the node can start.
    fn drop(&mut self) {
        let _ = self.db.flush();
    }
}

impl KvBackend for RocksBackend {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .get_pinned(key)
            .map_err(Error::new)?
            .map(|v| v.to_vec()))
    }

    fn range(&self, start: Option<&[u8]>, end: Option<&[u8]>, reverse: bool) -> Result<KvIter> {
        if empty_range(start, end) {
            return Ok(Box::new(std::iter::empty()));
        }
        Ok(Box::new(ChunkedRange {
            db: Arc::clone(&self.db),
            lower: start.map(<[u8]>::to_vec),
            upper: end.map(<[u8]>::to_vec),
            reverse,
            buf: VecDeque::new(),
            done: false,
        }))
    }

    fn write(&self, batch: WriteBatch) -> Result<()> {
        let mut wb = rocksdb::WriteBatch::default();
        for op in batch.ops {
            match op {
                Op::Put(k, v) => wb.put(k, v),
                Op::Delete(k) => wb.delete(k),
            }
        }
        let mut wo = WriteOptions::default();
        wo.set_sync(self.options.sync_writes);
        self.db.write_opt(wb, &wo).map_err(Error::new)?;
        self.writes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn sync(&self) -> Result<()> {
        self.db.flush_wal(true).map_err(Error::new)
    }

    fn flush(&self) -> Result<()> {
        self.db.flush().map_err(Error::new)
    }

    fn disk_size(&self) -> Option<u64> {
        let mut total = 0;
        for entry in std::fs::read_dir(&self.path).ok()? {
            let meta = entry.ok()?.metadata().ok()?;
            if meta.is_file() {
                total += meta.len();
            }
        }
        Some(total)
    }

    fn stats(&self) -> Option<StorageStats> {
        Some(StorageStats {
            file_bytes: self.disk_size()?,
            allocated_bytes: self.property("rocksdb.total-sst-files-size")?
                + self.property("rocksdb.cur-size-all-mem-tables")?,
            cache_bytes: self.property("rocksdb.block-cache-usage")?
                + self.property("rocksdb.cur-size-all-mem-tables")?,
        })
    }
}

/// A range over an owned `Arc<DB>`: RocksDB iterators borrow the database,
/// so entries are fetched [`RANGE_CHUNK`] at a time with a fresh iterator
/// whose bounds resume after the last returned key. Each chunk is a
/// consistent view; writes committed between chunks may become visible
/// (callers iterate committed data from the single writer thread).
struct ChunkedRange {
    db: Arc<DB>,
    /// Inclusive lower bound (forward: advanced past each chunk).
    lower: Option<Vec<u8>>,
    /// Exclusive upper bound (reverse: lowered to each chunk's last key).
    upper: Option<Vec<u8>>,
    reverse: bool,
    buf: VecDeque<KvPair>,
    done: bool,
}

impl ChunkedRange {
    fn refill(&mut self) -> Result<()> {
        let mut ro = ReadOptions::default();
        if let Some(lower) = &self.lower {
            ro.set_iterate_lower_bound(lower.clone());
        }
        if let Some(upper) = &self.upper {
            ro.set_iterate_upper_bound(upper.clone());
        }
        let mut it = self.db.raw_iterator_opt(ro);
        if self.reverse {
            it.seek_to_last();
        } else {
            it.seek_to_first();
        }
        while it.valid() && self.buf.len() < RANGE_CHUNK {
            if let (Some(k), Some(v)) = (it.key(), it.value()) {
                self.buf.push_back((k.to_vec(), v.to_vec()));
            }
            if self.reverse {
                it.prev();
            } else {
                it.next();
            }
        }
        it.status().map_err(Error::new)?;
        if self.buf.len() < RANGE_CHUNK {
            self.done = true;
        } else if let Some((last, _)) = self.buf.back() {
            if self.reverse {
                self.upper = Some(last.clone());
            } else {
                let mut next = last.clone();
                next.push(0);
                self.lower = Some(next);
            }
        }
        Ok(())
    }
}

impl Iterator for ChunkedRange {
    type Item = Result<KvPair>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.buf.is_empty()
            && !self.done
            && let Err(e) = self.refill()
        {
            self.done = true;
            return Some(Err(e));
        }
        self.buf.pop_front().map(Ok)
    }
}
