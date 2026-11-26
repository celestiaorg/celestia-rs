//! Local shard store (celestia-app v11 `fibre/store*.go`, `local` backend).
//!
//! The store keeps, per accepted upload, the payment promise, the shard
//! payload and a prune-index entry, until the promise's prune time passes.
//!
//! # Layout
//!
//! Under [`StoreConfig::path`]:
//!
//! - `metadata.redb`: a [`redb`] database with Go's Pebble keys (see the
//!   `keys` module): `/pp/<hash>` promises, `/shard/<commitment>/<hash>` markers,
//!   `/prune/<YYYYMMDDHHmm>/<commitment>/<hash>` prune index and the
//!   `/health/probe` key;
//! - `shards/<commitment>-<hash>`: one payload per promise, in Go's binary
//!   shard format (byte-identical, see the `codec` module);
//! - `staging/`: payloads being written; emptied at open.
//!
//! # Blocking API
//!
//! Every method does blocking file and database I/O. Async callers run them
//! on a blocking thread (`tokio::task::spawn_blocking` with an
//! `Arc<Store>`); the methods take `&self` and the store is `Send + Sync`.
//! Where Go checks its `context.Context`, the methods take a
//! [`CancellationToken`].
//!
//! # Crash safety
//!
//! A put writes the payload to `staging/`, renames it into `shards/`, then
//! commits the three metadata keys in one redb transaction. A crash before
//! the rename leaves only a staging file (removed at the next open); a crash
//! between rename and commit leaves an orphan payload file without metadata
//! (accepted, as in Go). Metadata commits use redb's `Eventual` durability:
//! like Go's `pebble.NoSync` they survive a process crash, and a power loss
//! can roll back to an earlier commit. The health probe commits with
//! `Immediate` durability (fsync), like Go's `pebble.Sync`.

mod codec;
mod error;
mod keys;
mod local;
mod maintenance;
mod marker;
#[cfg(test)]
mod tests;

use std::fmt;
use std::fs;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Mutex, PoisonError, RwLock};
use std::time::{Instant, SystemTime};

use celestia_proto::celestia::fibre::v1::{BlobShard, PaymentPromise};
use prost::Message;
use redb::Durability;
use tokio_util::sync::CancellationToken;

pub use codec::shard_binary_size;
pub use error::StoreError;
pub use maintenance::{PruneSummary, prune_expired, seed_occupancy};

use error::db_err;
use local::LocalBackend;
use marker::{BackendTag, decode_marker, encode_marker};

/// A 32-byte blob commitment.
pub type Commitment = [u8; x_fibre::promise::COMMITMENT_SIZE];

/// Upper bound on the entries one [`Store::prune_batch`] deletes (one
/// metadata commit). [`prune_expired`] drains full batches in one pass.
pub const MAX_PRUNE_BATCH_SIZE: usize = 1000;

/// File name of the metadata database under [`StoreConfig::path`].
pub const METADATA_FILE: &str = "metadata.redb";

/// The single metadata table: Go's Pebble keyspace.
const TABLE: redb::TableDefinition<'static, &[u8], &[u8]> = redb::TableDefinition::new("fibre");

/// redb page cache: metadata values are small.
const METADATA_CACHE_BYTES: usize = 16 << 20;

/// Store result.
pub type Result<T, E = StoreError> = std::result::Result<T, E>;

/// Backend for new shard payloads (Go `storage_backend`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StorageBackend {
    /// Flat files under the store directory.
    #[default]
    Local,
    /// Go's S3-compatible object storage; [`Store::open`] rejects it with
    /// [`StoreError::Unsupported`].
    Object,
}

impl StorageBackend {
    /// The configuration spelling (`local` or `object`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Object => "object",
        }
    }
}

impl fmt::Display for StorageBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for StorageBackend {
    type Err = StoreError;

    /// Parses Go's `storage_backend`; empty means `local`.
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "" | "local" => Ok(Self::Local),
            "object" => Ok(Self::Object),
            _ => Err(StoreError::Config(
                "storage_backend must be local or object".into(),
            )),
        }
    }
}

/// Store configuration (Go `StoreConfig` without the object-storage part).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreConfig {
    /// Backend for new shards.
    pub storage_backend: StorageBackend,
    /// Store directory.
    pub path: PathBuf,
}

impl StoreConfig {
    /// A `local` store at `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            storage_backend: StorageBackend::Local,
            path: path.into(),
        }
    }

    /// Go `StoreConfig.Validate`, plus the rejection of `object`.
    pub fn validate(&self) -> Result<()> {
        if self.storage_backend == StorageBackend::Object {
            return Err(StoreError::Unsupported(
                "fibre storage_backend \"object\" is not supported by celestia-rs; use \"local\""
                    .into(),
            ));
        }
        if self.path.as_os_str().is_empty() {
            return Err(StoreError::Config("store path is required".into()));
        }
        Ok(())
    }
}

/// Presence of one shard (Go `shardStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShardStatus {
    /// The payload exists.
    pub has: bool,
    /// A versioned marker exists, so its size already counts in occupancy
    /// (a re-upload must not reserve again).
    pub accounted: bool,
}

/// Result of [`Store::size`].
#[derive(Debug)]
pub struct StoreSize {
    /// Marker-accounted encoded size of the stored shards.
    pub bytes: i64,
    /// Set when invalid metadata was skipped: `bytes` is then the usable
    /// partial total (Go returns it together with `ErrStoreIntegrity`).
    pub integrity_error: Option<StoreError>,
}

/// Result of one prune batch (Go `pruneBefore`).
#[derive(Debug)]
pub struct PruneBatch {
    /// Committed deletions.
    pub pruned: usize,
    /// Marker-accounted bytes of the committed deletions; release them from
    /// occupancy.
    pub freed: i64,
    /// Cursor to pass as `after` for the next batch; `None` when the walk
    /// reached the cutoff or the end of the index.
    pub next: Option<Vec<u8>>,
    /// Integrity errors (skipped corrupt markers), payload deletion failures
    /// ([`StoreError::PartialDelete`]), cancellation or a fatal error. The
    /// counts above are valid even when this is set.
    pub error: Option<StoreError>,
}

/// The local shard store. See the module documentation.
pub struct Store {
    /// `None` after [`Store::close`]. Operations hold the read lock, so
    /// close waits for them.
    db: RwLock<Option<redb::Database>>,
    local: LocalBackend,
    path: PathBuf,
    /// One health probe at a time.
    probe: Mutex<()>,
    #[cfg(test)]
    fail_commits: std::sync::atomic::AtomicBool,
}

// The service shares one store across blocking tasks.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Store>();
};

impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Store")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Store {
    /// Opens (or creates) the store at `cfg.path` and removes leftover
    /// staging files from a previous crash. Fails when another `Store` has
    /// the directory open (redb's file lock).
    pub fn open(cfg: &StoreConfig) -> Result<Self> {
        cfg.validate()?;
        fs::create_dir_all(&cfg.path).map_err(|e| StoreError::io("creating store directory", e))?;
        let db = redb::Builder::new()
            .set_cache_size(METADATA_CACHE_BYTES)
            .create(cfg.path.join(METADATA_FILE))
            .map_err(|e| StoreError::Db(format!("opening metadata database: {e}")))?;
        let txn = db.begin_write().map_err(db_err)?;
        txn.open_table(TABLE).map_err(db_err)?;
        txn.commit().map_err(db_err)?;
        let local = LocalBackend::open(&cfg.path)?;
        let store = Self {
            db: RwLock::new(Some(db)),
            local,
            path: cfg.path.clone(),
            probe: Mutex::new(()),
            #[cfg(test)]
            fail_commits: std::sync::atomic::AtomicBool::new(false),
        };
        store.reconcile()?;
        Ok(store)
    }

    /// Go `reconcile`: drops everything under `staging/`. Orphan markers and
    /// orphan payload files are left alone: markers are skipped by
    /// [`Self::get`] and removed at their prune time.
    fn reconcile(&self) -> Result<()> {
        let start = Instant::now();
        match self.local.reset_staging() {
            Ok(removed) => {
                tracing::info!(
                    staging_files_removed = removed,
                    elapsed_ms = start.elapsed().as_millis(),
                    "store reconcile complete"
                );
                Ok(())
            }
            Err(e) => {
                tracing::error!(error = %e, elapsed_ms = start.elapsed().as_millis(), "store reconcile failed");
                Err(e)
            }
        }
    }

    /// The store directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Stores a promise and its shard until `prune_at`: stage the payload,
    /// rename it into place, then commit the promise, marker and prune-index
    /// keys atomically. Puts for one commitment with different promises are
    /// stored independently; a re-put of the same promise overwrites the
    /// same keys and file (and is idempotent because `prune_at` is derived
    /// from the promise). `cancel` is honoured up to the payload rename.
    pub fn put(
        &self,
        promise: &PaymentPromise,
        shard: &BlobShard,
        prune_at: SystemTime,
        cancel: &CancellationToken,
    ) -> Result<()> {
        if cancel.is_cancelled() {
            return Err(StoreError::Cancelled);
        }
        let promise_hash = x_fibre::promise::promise_hash(promise)
            .map_err(|e| StoreError::Other(format!("getting promise hash: {e}")))?;
        let commitment: Commitment = promise
            .commitment
            .as_slice()
            .try_into()
            .map_err(|_| StoreError::Other("invalid promise commitment".into()))?;
        let marker = encode_marker(BackendTag::Local, shard_binary_size(shard));
        let promise_bytes = promise.encode_to_vec();
        let shard_key = keys::shard_key(&commitment, &promise_hash);
        let prune_key = keys::prune_key(prune_at, &commitment, &promise_hash);
        let promise_key = keys::promise_key(&promise_hash);

        self.with_db(|db| {
            if cancel.is_cancelled() {
                return Err(StoreError::Cancelled);
            }
            self.local.put(&commitment, &promise_hash, shard, cancel)?;
            let committed = self.commit(db, Durability::Eventual, |table| {
                table.insert(promise_key.as_slice(), promise_bytes.as_slice())?;
                table.insert(shard_key.as_slice(), marker.as_slice())?;
                table.insert(prune_key.as_slice(), &[][..])?;
                Ok(())
            });
            if let Err(e) = committed {
                // Remove the payload only when no marker refers to it (a
                // failed re-put keeps the earlier, committed one).
                if Self::marker_missing(db, &shard_key)
                    && let Err(rm) = self.local.delete(&commitment, &promise_hash)
                {
                    tracing::warn!(
                        commitment = %hex::encode(commitment),
                        error = %rm,
                        "failed to remove orphaned shard after commit failure"
                    );
                }
                return Err(match e {
                    StoreError::Db(msg) => StoreError::Db(format!("committing metadata: {msg}")),
                    other => other,
                });
            }
            Ok(())
        })
    }

    /// The first readable shard for `commitment`, in promise-hash order (the
    /// same choice on every validator, and the same as Go's). Markers whose
    /// payload is missing are skipped and kept: pruning releases their
    /// size. Returns [`StoreError::NotFound`] when no payload is readable,
    /// or the collected errors (integrity, decode, I/O) when some failed.
    pub fn get(&self, commitment: &Commitment) -> Result<BlobShard> {
        self.with_db(|db| {
            let prefix = keys::shard_prefix(commitment);
            let entries = Self::scan(db, &prefix)?;
            let mut errs = Vec::new();
            for (key, marker) in entries {
                let promise_hash = match hex::decode(&key[prefix.len()..]) {
                    Ok(hash) => hash,
                    Err(e) => {
                        errs.push(StoreError::Other(format!(
                            "decoding promise hash from shard key: {e}"
                        )));
                        continue;
                    }
                };
                match self.read_payload(&marker, commitment, &promise_hash) {
                    Ok(shard) => return Ok(shard),
                    Err(StoreError::NotFound) => {}
                    Err(e) => errs.push(e),
                }
            }
            Err(StoreError::join(errs.into_iter().map(Some)).unwrap_or(StoreError::NotFound))
        })
    }

    /// Whether the payload for (`commitment`, `promise_hash`) exists. An
    /// orphan marker (payload missing) reports `false`.
    pub fn has(&self, commitment: &Commitment, promise_hash: &[u8]) -> Result<bool> {
        self.shard_status(commitment, promise_hash).map(|s| s.has)
    }

    /// Whether the payload exists and whether its marker counts in
    /// occupancy (upload admission reserves only when not `accounted`).
    pub fn shard_status(
        &self,
        commitment: &Commitment,
        promise_hash: &[u8],
    ) -> Result<ShardStatus> {
        self.with_db(|db| {
            let Some(marker) = Self::get_raw(db, &keys::shard_key(commitment, promise_hash))?
            else {
                return Ok(ShardStatus {
                    has: false,
                    accounted: false,
                });
            };
            let has = match decode_marker(&marker)?.0 {
                BackendTag::Local => self.local.has(commitment, promise_hash)?,
                BackendTag::Object => return Err(object_unavailable()),
            };
            Ok(ShardStatus {
                has,
                accounted: !marker.is_empty(),
            })
        })
    }

    /// The stored promise with this hash, if any.
    pub fn get_payment_promise(&self, promise_hash: &[u8]) -> Result<Option<PaymentPromise>> {
        self.with_db(|db| {
            Self::get_raw(db, &keys::promise_key(promise_hash))?
                .map(|bytes| {
                    PaymentPromise::decode(bytes.as_slice()).map_err(|e| {
                        StoreError::Other(format!("unmarshaling payment promise: {e}"))
                    })
                })
                .transpose()
        })
    }

    /// Marker-accounted encoded size of all stored shards (legacy empty
    /// markers: the file size). Invalid entries are skipped and reported in
    /// [`StoreSize::integrity_error`] next to the usable partial total.
    pub fn size(&self, cancel: &CancellationToken) -> Result<StoreSize> {
        if cancel.is_cancelled() {
            return Err(StoreError::Cancelled);
        }
        self.with_db(|db| {
            let table = Self::read_table(db)?;
            let end = keys::prefix_end(keys::SHARD_KEY_PREFIX);
            let range = table
                .range::<&[u8]>((
                    Bound::Included(keys::SHARD_KEY_PREFIX),
                    Bound::Excluded(end.as_slice()),
                ))
                .map_err(db_err)?;
            let mut total: i64 = 0;
            let mut integrity: Option<StoreError> = None;
            let mut invalid = 0usize;
            for item in range {
                if cancel.is_cancelled() {
                    return Err(StoreError::Cancelled);
                }
                let (key, marker) =
                    item.map_err(|e| StoreError::Db(format!("iterating shards: {e}")))?;
                let key = key.value();
                let Some((commitment, promise_hash)) = keys::parse_shard_key(key) else {
                    invalid += 1;
                    integrity.get_or_insert_with(|| {
                        StoreError::Integrity(format!(
                            "invalid shard key {:?}",
                            String::from_utf8_lossy(key)
                        ))
                    });
                    continue;
                };
                let size = match self.marker_size(marker.value(), &commitment, &promise_hash) {
                    Ok(size) => size,
                    Err(e) if e.is_integrity() => {
                        invalid += 1;
                        if integrity.is_none() {
                            integrity = Some(integrity_context(
                                &format!(
                                    "decoding shard marker {:?}",
                                    String::from_utf8_lossy(key)
                                ),
                                e,
                            ));
                        }
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                total = total.checked_add(size).ok_or_else(|| {
                    StoreError::Integrity("total shard size overflows int64".into())
                })?;
            }
            if cancel.is_cancelled() {
                return Err(StoreError::Cancelled);
            }
            if invalid > 1 {
                integrity =
                    integrity.map(|e| with_count(e, invalid, "invalid shard metadata entries"));
            }
            Ok(StoreSize {
                bytes: total,
                integrity_error: integrity,
            })
        })
    }

    /// Free bytes on the file system backing the store.
    pub fn disk_available(&self) -> Result<i64> {
        self.local.disk_available()
    }

    /// Deletes up to [`MAX_PRUNE_BATCH_SIZE`] entries whose prune time is
    /// before `before` (minute precision, UTC). See [`Self::prune_batch`].
    pub fn prune_before(&self, before: SystemTime, cancel: &CancellationToken) -> PruneBatch {
        self.prune_batch(before, None, cancel)
    }

    /// One prune batch (Go `pruneBefore`), resuming after the prune-index
    /// key `after` (the previous batch's [`PruneBatch::next`]).
    ///
    /// Walks the prune index in time order up to the cutoff, deletes each
    /// candidate's payload, then commits the removal of the prune entry,
    /// marker and promise of every candidate whose payload is gone. Entries
    /// with a corrupt marker or a failed payload deletion stay for a later
    /// pass. Database errors, a size overflow or cancellation during the
    /// walk return zero counts; cancellation during deletion still commits
    /// the deletions done so far.
    pub fn prune_batch(
        &self,
        before: SystemTime,
        after: Option<&[u8]>,
        cancel: &CancellationToken,
    ) -> PruneBatch {
        match self.with_db(|db| self.prune_batch_in(db, before, after, cancel)) {
            Ok(batch) => batch,
            Err(error) => PruneBatch {
                pruned: 0,
                freed: 0,
                next: None,
                error: Some(error),
            },
        }
    }

    fn prune_batch_in(
        &self,
        db: &redb::Database,
        before: SystemTime,
        after: Option<&[u8]>,
        cancel: &CancellationToken,
    ) -> Result<PruneBatch> {
        let PruneSelection {
            candidates,
            mut integrity,
            corrupt,
        } = self.select_prune_candidates(db, before, after, cancel)?;

        let next = (candidates.len() == MAX_PRUNE_BATCH_SIZE)
            .then(|| candidates.last().map(|c| c.key.clone()))
            .flatten();
        let (successful, failure) = self.delete_payloads(&candidates, cancel);
        let mut pruned = 0;
        let mut freed = 0;
        if !successful.is_empty() {
            let committed = self.commit(db, Durability::Eventual, |table| {
                for &i in &successful {
                    let c = &candidates[i];
                    table.remove(c.key.as_slice())?;
                    table.remove(keys::shard_key(&c.commitment, &c.promise_hash).as_slice())?;
                    table.remove(keys::promise_key(&c.promise_hash).as_slice())?;
                }
                Ok(())
            });
            if let Err(e) = committed {
                let deletion = failure.map(|f| match f {
                    DeleteFailure::Partial(e) | DeleteFailure::Aborted(e) => e,
                });
                return Err(StoreError::join([
                    deletion,
                    Some(StoreError::Db(format!("committing batch: {e}"))),
                ])
                .unwrap_or_else(|| StoreError::Db("committing batch".into())));
            }
            pruned = successful.len();
            freed = successful.iter().map(|&i| candidates[i].size).sum();
        }
        if corrupt > 1 {
            integrity = integrity.map(|e| with_count(e, corrupt, "corrupt shard markers"));
        }
        let error = match failure {
            Some(DeleteFailure::Partial(deletion)) => Some(StoreError::PartialDelete {
                deletion: Box::new(deletion),
                integrity: integrity.map(Box::new),
            }),
            Some(DeleteFailure::Aborted(e)) => StoreError::join([Some(e), integrity]),
            None => integrity,
        };
        Ok(PruneBatch {
            pruned,
            freed,
            next,
            error,
        })
    }

    /// Walks the prune index from `after` up to the cutoff and collects up
    /// to [`MAX_PRUNE_BATCH_SIZE`] candidates with valid markers.
    fn select_prune_candidates(
        &self,
        db: &redb::Database,
        before: SystemTime,
        after: Option<&[u8]>,
        cancel: &CancellationToken,
    ) -> Result<PruneSelection> {
        let before = keys::format_timestamp(before);
        let mut selection = PruneSelection::default();
        let mut selected: i64 = 0;
        let table = Self::read_table(db)?;
        // Resume at the first key after the cursor.
        let start = after.map_or_else(
            || keys::PRUNE_KEY_PREFIX.to_vec(),
            |a| [a, &[0][..]].concat(),
        );
        let end = keys::prefix_end(keys::PRUNE_KEY_PREFIX);
        if start >= end {
            return Ok(selection);
        }
        let range = table
            .range::<&[u8]>((
                Bound::Included(start.as_slice()),
                Bound::Excluded(end.as_slice()),
            ))
            .map_err(db_err)?;
        for item in range {
            if selection.candidates.len() >= MAX_PRUNE_BATCH_SIZE {
                break;
            }
            if cancel.is_cancelled() {
                return Err(StoreError::Cancelled);
            }
            let (key, _) =
                item.map_err(|e| StoreError::Db(format!("iterating prune index: {e}")))?;
            let key = key.value().to_vec();
            // Keys sort by time: stop at the first one at or after the cutoff.
            let prefix_len = keys::PRUNE_KEY_PREFIX.len();
            let Some(timestamp) = key.get(prefix_len..prefix_len + keys::TIMESTAMP_LEN) else {
                continue;
            };
            if timestamp >= before.as_bytes() {
                break;
            }
            let Some((commitment, promise_hash)) = keys::parse_prune_key(&key) else {
                continue;
            };
            let marker = table
                .get(keys::shard_key(&commitment, &promise_hash).as_slice())
                .map_err(|e| StoreError::Db(format!("getting shard marker: {e}")))?
                .map(|v| v.value().to_vec())
                .unwrap_or_default();
            let size = match self.marker_size(&marker, &commitment, &promise_hash) {
                Ok(size) => size,
                Err(e) if e.is_integrity() => {
                    selection.corrupt += 1;
                    if selection.integrity.is_none() {
                        selection.integrity = Some(integrity_context(
                            &format!("decoding shard marker {:?}", String::from_utf8_lossy(&key)),
                            e,
                        ));
                    }
                    continue;
                }
                Err(e) => return Err(e),
            };
            selected = selected
                .checked_add(size)
                .ok_or_else(|| StoreError::Other("pruned shard size overflows int64".into()))?;
            selection.candidates.push(PruneCandidate {
                key,
                commitment,
                promise_hash,
                size,
            });
        }
        Ok(selection)
    }

    /// Go `routedStorage.DeleteBatch` for local payloads: the positions of
    /// the deleted candidates and the failure, if any.
    fn delete_payloads(
        &self,
        candidates: &[PruneCandidate],
        cancel: &CancellationToken,
    ) -> (Vec<usize>, Option<DeleteFailure>) {
        let mut successful = Vec::with_capacity(candidates.len());
        let mut first: Option<StoreError> = None;
        for (i, c) in candidates.iter().enumerate() {
            if cancel.is_cancelled() {
                let err = StoreError::join([first, Some(StoreError::Cancelled)]);
                return (successful, err.map(DeleteFailure::Aborted));
            }
            match self.local.delete(&c.commitment, &c.promise_hash) {
                Ok(()) => successful.push(i),
                Err(e) if cancel.is_cancelled() => {
                    let err = StoreError::join([first, Some(e), Some(StoreError::Cancelled)]);
                    return (successful, err.map(DeleteFailure::Aborted));
                }
                Err(e) => {
                    first.get_or_insert(e);
                }
            }
        }
        (successful, first.map(DeleteFailure::Partial))
    }

    /// Writes `value` under `/health/probe` with fsync and reads it back
    /// (Go `Probe`). Probes run one at a time; a second waits for the first.
    /// Callers bound it with a timeout around the blocking call.
    pub fn probe(&self, value: &[u8]) -> Result<()> {
        let _one = self.probe.lock().unwrap_or_else(PoisonError::into_inner);
        self.with_db(|db| {
            self.commit(db, Durability::Immediate, |table| {
                table.insert(keys::PROBE_KEY, value)?;
                Ok(())
            })
            .map_err(|e| StoreError::Other(format!("write: {e}")))?;
            let got = Self::get_raw(db, keys::PROBE_KEY)
                .map_err(|e| StoreError::Other(format!("read: {e}")))?;
            if got.as_deref() != Some(value) {
                return Err(StoreError::Other("read back a different value".into()));
            }
            Ok(())
        })
    }

    /// Closes the metadata database once running operations finish (a
    /// stuck health probe delays it, like Go's close waiting for the
    /// probe). Later calls fail with [`StoreError::Closed`]; closing twice
    /// is a no-op. Dropping the store also closes it.
    pub fn close(&self) -> Result<()> {
        let mut guard = self.db.write().unwrap_or_else(PoisonError::into_inner);
        drop(guard.take());
        Ok(())
    }

    // -- helpers ------------------------------------------------------------

    fn with_db<T>(&self, f: impl FnOnce(&redb::Database) -> Result<T>) -> Result<T> {
        let guard = self.db.read().unwrap_or_else(PoisonError::into_inner);
        let db = guard.as_ref().ok_or(StoreError::Closed)?;
        f(db)
    }

    fn read_table(
        db: &redb::Database,
    ) -> Result<redb::ReadOnlyTable<&'static [u8], &'static [u8]>> {
        db.begin_read()
            .map_err(db_err)?
            .open_table(TABLE)
            .map_err(db_err)
    }

    fn get_raw(db: &redb::Database, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(Self::read_table(db)?
            .get(key)
            .map_err(db_err)?
            .map(|v| v.value().to_vec()))
    }

    /// Every (key, value) under `prefix`, collected so no read transaction
    /// stays open across payload file I/O.
    fn scan(db: &redb::Database, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let table = Self::read_table(db)?;
        let end = keys::prefix_end(prefix);
        let range = table
            .range::<&[u8]>((Bound::Included(prefix), Bound::Excluded(end.as_slice())))
            .map_err(db_err)?;
        let mut out = Vec::new();
        for item in range {
            let (k, v) = item.map_err(|e| StoreError::Db(format!("iterating shards: {e}")))?;
            out.push((k.value().to_vec(), v.value().to_vec()));
        }
        Ok(out)
    }

    /// Go `shardMarkerMissing`: confirmed absence only; read errors never
    /// permit deletion.
    fn marker_missing(db: &redb::Database, shard_key: &[u8]) -> bool {
        match Self::get_raw(db, shard_key) {
            Ok(marker) => marker.is_none(),
            Err(e) => {
                tracing::warn!(error = %e, "failed to check shard marker");
                false
            }
        }
    }

    /// Commits one write transaction with the given durability.
    #[cfg_attr(not(test), allow(clippy::unused_self))]
    fn commit(
        &self,
        db: &redb::Database,
        durability: Durability,
        apply: impl FnOnce(
            &mut redb::Table<'_, &'static [u8], &'static [u8]>,
        ) -> Result<(), redb::StorageError>,
    ) -> Result<()> {
        let mut txn = db.begin_write().map_err(db_err)?;
        txn.set_durability(durability);
        {
            let mut table = txn.open_table(TABLE).map_err(db_err)?;
            apply(&mut table).map_err(db_err)?;
        }
        #[cfg(test)]
        if self.fail_commits.load(std::sync::atomic::Ordering::SeqCst) {
            txn.abort().map_err(db_err)?;
            return Err(StoreError::Db("injected commit failure".into()));
        }
        txn.commit().map_err(db_err)
    }

    /// Reads the payload a marker points to.
    fn read_payload(
        &self,
        marker: &[u8],
        commitment: &Commitment,
        promise_hash: &[u8],
    ) -> Result<BlobShard> {
        match decode_marker(marker)?.0 {
            BackendTag::Local => self.local.get(commitment, promise_hash),
            BackendTag::Object => Err(object_unavailable()),
        }
    }

    /// Go `routedStorage.size`: the marker's size, or the file size for a
    /// legacy empty marker (0 when the file is missing).
    fn marker_size(
        &self,
        marker: &[u8],
        commitment: &Commitment,
        promise_hash: &[u8],
    ) -> Result<i64> {
        match decode_marker(marker)? {
            (BackendTag::Object, _) => Err(object_unavailable()),
            (BackendTag::Local, size) if size > 0 => Ok(size),
            (BackendTag::Local, _) => self.local.size(commitment, promise_hash),
        }
    }
}

#[derive(Default)]
struct PruneSelection {
    candidates: Vec<PruneCandidate>,
    /// First integrity error (corrupt marker) skipped.
    integrity: Option<StoreError>,
    /// Number of corrupt markers skipped.
    corrupt: usize,
}

struct PruneCandidate {
    key: Vec<u8>,
    commitment: Commitment,
    promise_hash: Vec<u8>,
    size: i64,
}

enum DeleteFailure {
    /// Every candidate was attempted; some deletions failed.
    Partial(StoreError),
    /// Cancellation stopped the deletions.
    Aborted(StoreError),
}

/// Go's object backend is not configured here, so its markers are
/// unreadable metadata.
fn object_unavailable() -> StoreError {
    StoreError::Integrity(format!(
        "shard backend 0x{:02x} is unavailable",
        BackendTag::Object as u8
    ))
}

/// Prefixes an integrity message with context, keeping it an integrity
/// error.
fn integrity_context(context: &str, err: StoreError) -> StoreError {
    match err {
        StoreError::Integrity(msg) => StoreError::Integrity(format!("{context}: {msg}")),
        other => other,
    }
}

/// Appends Go's "(N ...)" count to the first integrity error.
fn with_count(err: StoreError, count: usize, what: &str) -> StoreError {
    match err {
        StoreError::Integrity(msg) => StoreError::Integrity(format!("{msg} ({count} {what})")),
        other => other,
    }
}
