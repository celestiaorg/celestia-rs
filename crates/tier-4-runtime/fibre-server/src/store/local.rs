//! Local shard payloads as flat files (Go `store_local.go`).
//!
//! `<store>/shards/<commitment-hex>-<promise-hash-hex>` holds one shard in
//! the [`super::codec`] format. A put writes `<store>/staging/<unique>` and
//! renames it into place, so readers never see a partial file; the staging
//! directory is emptied when the store opens.

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use celestia_proto::celestia::fibre::v1::BlobShard;
use tokio_util::sync::CancellationToken;

use super::codec::{read_shard, write_shard};
use super::{Commitment, StoreError};

const SHARDS_SUBDIR: &str = "shards";
const STAGING_SUBDIR: &str = "staging";
/// Buffer for the small header reads and writes around row payloads.
const IO_BUFFER: usize = 1 << 20;

/// Test hook run before a payload deletion; an error fails the deletion.
#[cfg(test)]
pub(crate) type DeleteHook = Box<dyn Fn(&[u8]) -> io::Result<()> + Send + Sync>;

/// Flat-file payload storage.
pub(crate) struct LocalBackend {
    root: PathBuf,
    shards: PathBuf,
    staging: PathBuf,
    next_tmp: AtomicU64,
    #[cfg(test)]
    pub(crate) delete_hook: std::sync::Mutex<Option<DeleteHook>>,
}

impl std::fmt::Debug for LocalBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalBackend")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

/// Removes a staged file unless it was published.
struct Staged<'a>(Option<&'a Path>);

impl Drop for Staged<'_> {
    fn drop(&mut self) {
        if let Some(path) = self.0 {
            let _ = fs::remove_file(path);
        }
    }
}

impl LocalBackend {
    /// Creates `shards/` and `staging/` under `root`.
    pub(crate) fn open(root: &Path) -> Result<Self, StoreError> {
        let shards = root.join(SHARDS_SUBDIR);
        let staging = root.join(STAGING_SUBDIR);
        for dir in [&shards, &staging] {
            fs::create_dir_all(dir)
                .map_err(|e| StoreError::io(format!("creating {}", dir.display()), e))?;
        }
        Ok(Self {
            root: root.to_path_buf(),
            shards,
            staging,
            next_tmp: AtomicU64::new(0),
            #[cfg(test)]
            delete_hook: std::sync::Mutex::new(None),
        })
    }

    pub(crate) fn shard_path(&self, commitment: &Commitment, promise_hash: &[u8]) -> PathBuf {
        self.shards.join(format!(
            "{}-{}",
            hex::encode(commitment),
            hex::encode(promise_hash)
        ))
    }

    /// Stages and publishes a payload. Honours `cancel` before the staging
    /// write and before the rename; the staged file never outlives the call.
    pub(crate) fn put(
        &self,
        commitment: &Commitment,
        promise_hash: &[u8],
        shard: &BlobShard,
        cancel: &CancellationToken,
    ) -> Result<(), StoreError> {
        if cancel.is_cancelled() {
            return Err(StoreError::Cancelled);
        }
        // Unique per writer: concurrent puts of one key never share a staged
        // file; the last rename wins. The store holds an exclusive lock on
        // its directory and empties staging at open, so a counter suffices.
        let tmp = self.staging.join(format!(
            "{}-{}",
            std::process::id(),
            self.next_tmp.fetch_add(1, Ordering::Relaxed)
        ));
        let mut staged = Staged(Some(&tmp));
        Self::write_tmp(&tmp, shard).map_err(|e| StoreError::io("writing shard tmp", e))?;
        if cancel.is_cancelled() {
            return Err(StoreError::Cancelled);
        }
        fs::rename(&tmp, self.shard_path(commitment, promise_hash))
            .map_err(|e| StoreError::io("renaming shard tmp to final", e))?;
        staged.0 = None;
        Ok(())
    }

    fn write_tmp(tmp: &Path, shard: &BlobShard) -> io::Result<()> {
        let file = File::create(tmp)?;
        let mut w = BufWriter::with_capacity(IO_BUFFER, file);
        write_shard(&mut w, shard)?;
        w.flush()?;
        // Like Go, no fsync: a power loss may lose the file, which then reads
        // as a missing payload. `into_inner` surfaces a late flush error.
        drop(w.into_inner().map_err(io::IntoInnerError::into_error)?);
        Ok(())
    }

    /// Reads a payload; a missing file is [`StoreError::NotFound`].
    pub(crate) fn get(
        &self,
        commitment: &Commitment,
        promise_hash: &[u8],
    ) -> Result<BlobShard, StoreError> {
        let file = match File::open(self.shard_path(commitment, promise_hash)) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(StoreError::NotFound),
            Err(e) => return Err(StoreError::io("opening shard file", e)),
        };
        read_shard(&mut BufReader::with_capacity(IO_BUFFER, file)).map_err(StoreError::Codec)
    }

    pub(crate) fn has(
        &self,
        commitment: &Commitment,
        promise_hash: &[u8],
    ) -> Result<bool, StoreError> {
        match fs::metadata(self.shard_path(commitment, promise_hash)) {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(StoreError::io("stat shard file", e)),
        }
    }

    /// Removes a payload; a missing file is not an error.
    pub(crate) fn delete(
        &self,
        commitment: &Commitment,
        promise_hash: &[u8],
    ) -> Result<(), StoreError> {
        #[cfg(test)]
        if let Some(hook) = self
            .delete_hook
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            hook(promise_hash).map_err(|e| StoreError::io("removing shard file", e))?;
        }
        match fs::remove_file(self.shard_path(commitment, promise_hash)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => {
                Err(StoreError::io("removing shard file", e))
            }
            _ => Ok(()),
        }
    }

    /// Size of a payload file; 0 when it is missing.
    pub(crate) fn size(
        &self,
        commitment: &Commitment,
        promise_hash: &[u8],
    ) -> Result<i64, StoreError> {
        match fs::metadata(self.shard_path(commitment, promise_hash)) {
            Ok(meta) => Ok(i64::try_from(meta.len()).unwrap_or(i64::MAX)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(0),
            Err(e) => Err(StoreError::io("stat legacy shard file", e)),
        }
    }

    /// Free bytes for unprivileged users on the store's file system.
    pub(crate) fn disk_available(&self) -> Result<i64, StoreError> {
        let stat = rustix::fs::statvfs(&self.root)
            .map_err(|e| StoreError::io("getting disk usage", e.into()))?;
        Ok(i64::try_from(stat.f_bavail.saturating_mul(stat.f_frsize)).unwrap_or(i64::MAX))
    }

    /// Empties `staging/` (leftovers of puts that crashed before their
    /// rename) and returns how many entries it removed.
    pub(crate) fn reset_staging(&self) -> Result<usize, StoreError> {
        let removed = match fs::read_dir(&self.staging) {
            Ok(entries) => entries.count(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => 0,
            Err(e) => return Err(StoreError::io("reading staging", e)),
        };
        match fs::remove_dir_all(&self.staging) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => {
                return Err(StoreError::io("removing staging", e));
            }
            _ => {}
        }
        fs::create_dir_all(&self.staging).map_err(|e| StoreError::io("recreating staging", e))?;
        Ok(removed)
    }
}
