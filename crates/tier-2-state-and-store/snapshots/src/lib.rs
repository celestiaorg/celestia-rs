//! Cosmos SDK state-sync snapshots (`cosmossdk.io/store/snapshots`, format 3).
//!
//! A snapshot of height `h` is the stream of length-delimited
//! `SnapshotItem` protobuf messages — for every IAVL store, sorted by name,
//! one `SnapshotStoreItem` followed by the store's IAVL export nodes in
//! post-order — compressed with zlib (level 7, like Go) and cut into chunks
//! of 10 MB. The snapshot hash is the SHA-256 of all chunk bytes and the
//! metadata lists the SHA-256 of every chunk. Rust and Go nodes can
//! therefore state-sync from each other: chunk bytes differ (different zlib
//! encoders) but every snapshot is self-consistent and decodes to the same
//! nodes.
//!
//! [`Manager`] stores snapshots under `<dir>/<height>/<format>/<chunk>`
//! (Go's layout) with the metadata in `<dir>/<height>/<format>/metadata`,
//! and drives restores chunk by chunk as ABCI `ApplySnapshotChunk` delivers
//! them. [`Manager::snapshot_in_background`] writes a snapshot on its own
//! thread, as Go's `SnapshotIfApplicable` does in a goroutine, so Commit
//! never waits for it.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::fs;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use prost::Message;

/// Snapshot format produced and accepted (Go `types.CurrentFormat`).
pub const CURRENT_FORMAT: u32 = 3;
/// Chunk size in bytes (Go `snapshotChunkSize`).
pub const CHUNK_SIZE: usize = 10_000_000;
/// zlib level (Go `snapshotCompressionLevel`).
pub const COMPRESSION_LEVEL: u32 = 7;
/// Largest accepted item (Go `snapshotMaxItemSize`, 64 MiB).
pub const MAX_ITEM_SIZE: usize = 64 << 20;

/// Protobuf types of `cosmos.store.snapshots.v1`.
pub mod proto {
    /// `Snapshot`.
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Snapshot {
        /// Height.
        #[prost(uint64, tag = "1")]
        pub height: u64,
        /// Format.
        #[prost(uint32, tag = "2")]
        pub format: u32,
        /// Number of chunks.
        #[prost(uint32, tag = "3")]
        pub chunks: u32,
        /// SHA-256 of all chunks.
        #[prost(bytes = "vec", tag = "4")]
        pub hash: Vec<u8>,
        /// Chunk hashes.
        #[prost(message, optional, tag = "5")]
        pub metadata: Option<Metadata>,
    }

    /// `Metadata`.
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Metadata {
        /// SHA-256 of each chunk.
        #[prost(bytes = "vec", repeated, tag = "1")]
        pub chunk_hashes: Vec<Vec<u8>>,
    }

    /// `SnapshotItem`.
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct SnapshotItem {
        /// The item.
        #[prost(oneof = "snapshot_item::Item", tags = "1, 2, 3, 4")]
        pub item: Option<snapshot_item::Item>,
    }

    /// Nested types of [`SnapshotItem`].
    pub mod snapshot_item {
        /// `SnapshotItem.item`.
        #[derive(Clone, PartialEq, ::prost::Oneof)]
        pub enum Item {
            /// Store header.
            #[prost(message, tag = "1")]
            Store(super::SnapshotStoreItem),
            /// IAVL export node.
            #[prost(message, tag = "2")]
            Iavl(super::SnapshotIavlItem),
            /// Extension header.
            #[prost(message, tag = "3")]
            Extension(super::SnapshotExtensionMeta),
            /// Extension payload.
            #[prost(message, tag = "4")]
            ExtensionPayload(super::SnapshotExtensionPayload),
        }
    }

    /// `SnapshotStoreItem`.
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct SnapshotStoreItem {
        /// Store name.
        #[prost(string, tag = "1")]
        pub name: String,
    }

    /// `SnapshotIAVLItem`.
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct SnapshotIavlItem {
        /// Key.
        #[prost(bytes = "vec", tag = "1")]
        pub key: Vec<u8>,
        /// Value (leaves only).
        #[prost(bytes = "vec", tag = "2")]
        pub value: Vec<u8>,
        /// Node version.
        #[prost(int64, tag = "3")]
        pub version: i64,
        /// Node height.
        #[prost(int32, tag = "4")]
        pub height: i32,
    }

    /// `SnapshotExtensionMeta`.
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct SnapshotExtensionMeta {
        /// Extension name.
        #[prost(string, tag = "1")]
        pub name: String,
        /// Extension format.
        #[prost(uint32, tag = "2")]
        pub format: u32,
    }

    /// `SnapshotExtensionPayload`.
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct SnapshotExtensionPayload {
        /// Payload.
        #[prost(bytes = "vec", tag = "1")]
        pub payload: Vec<u8>,
    }
}

use proto::snapshot_item::Item;

/// Snapshot errors, mapped to ABCI results by the application.
#[derive(Debug)]
pub enum Error {
    /// Unsupported snapshot format (`REJECT_FORMAT`).
    UnknownFormat(u32),
    /// Malformed snapshot metadata (`REJECT`).
    InvalidMetadata(String),
    /// A chunk does not match its hash (`RETRY` + refetch).
    ChunkHashMismatch(u32),
    /// No restore is in progress.
    NoRestore,
    /// Malformed snapshot stream.
    Decode(String),
    /// Filesystem error.
    Io(io::Error),
    /// Store error.
    Store(multistore::Error),
    /// Another snapshot operation is in progress, or a snapshot of this or
    /// a later height exists (Go `ErrConflict`).
    Conflict(String),
    /// The snapshot was abandoned (shutdown).
    Cancelled,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownFormat(format) => write!(f, "unknown snapshot format {format}"),
            Self::InvalidMetadata(m) => write!(f, "invalid snapshot metadata: {m}"),
            Self::ChunkHashMismatch(i) => write!(f, "chunk {i} hash mismatch"),
            Self::NoRestore => f.write_str("no snapshot restore in progress"),
            Self::Decode(m) => write!(f, "snapshot decode: {m}"),
            Self::Io(e) => write!(f, "snapshot io: {e}"),
            Self::Store(e) => write!(f, "snapshot store: {e}"),
            Self::Conflict(m) => write!(f, "snapshot conflict: {m}"),
            Self::Cancelled => f.write_str("snapshot cancelled"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<multistore::Error> for Error {
    fn from(e: multistore::Error) -> Self {
        Self::Store(e)
    }
}

impl From<iavl::Error> for Error {
    fn from(e: iavl::Error) -> Self {
        Self::Store(multistore::Error::Iavl(e))
    }
}

/// Result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Snapshot descriptor (Go `types.Snapshot`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Height.
    pub height: u64,
    /// Format.
    pub format: u32,
    /// Number of chunks.
    pub chunks: u32,
    /// SHA-256 over all chunk bytes.
    pub hash: Vec<u8>,
    /// SHA-256 of each chunk.
    pub chunk_hashes: Vec<Vec<u8>>,
}

impl Snapshot {
    /// ABCI `Snapshot.metadata`: the protobuf `Metadata`.
    #[must_use]
    pub fn metadata_bytes(&self) -> Vec<u8> {
        proto::Metadata {
            chunk_hashes: self.chunk_hashes.clone(),
        }
        .encode_to_vec()
    }

    /// From the ABCI snapshot fields (Go `SnapshotFromABCI`).
    pub fn from_abci(
        height: u64,
        format: u32,
        chunks: u32,
        hash: Vec<u8>,
        metadata: &[u8],
    ) -> Result<Self> {
        let metadata =
            proto::Metadata::decode(metadata).map_err(|e| Error::InvalidMetadata(e.to_string()))?;
        Ok(Self {
            height,
            format,
            chunks,
            hash,
            chunk_hashes: metadata.chunk_hashes,
        })
    }

    fn to_proto(&self) -> proto::Snapshot {
        proto::Snapshot {
            height: self.height,
            format: self.format,
            chunks: self.chunks,
            hash: self.hash.clone(),
            metadata: Some(proto::Metadata {
                chunk_hashes: self.chunk_hashes.clone(),
            }),
        }
    }

    fn from_proto(p: proto::Snapshot) -> Self {
        Self {
            height: p.height,
            format: p.format,
            chunks: p.chunks,
            hash: p.hash,
            chunk_hashes: p.metadata.map(|m| m.chunk_hashes).unwrap_or_default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Stream encoding
// ---------------------------------------------------------------------------

fn write_item(out: &mut impl Write, item: Item) -> io::Result<()> {
    let bytes = proto::SnapshotItem { item: Some(item) }.encode_to_vec();
    let mut len = Vec::with_capacity(10);
    let mut n = bytes.len() as u64;
    while n >= 0x80 {
        len.push(u8::try_from(n & 0x7f).unwrap_or(0) | 0x80);
        n >>= 7;
    }
    len.push(u8::try_from(n).unwrap_or(0));
    out.write_all(&len)?;
    out.write_all(&bytes)
}

/// Reads one length-delimited item; `None` at a clean end of stream.
fn read_item(input: &mut impl Read) -> Result<Option<proto::SnapshotItem>> {
    let mut len = 0u64;
    let mut shift = 0;
    loop {
        let mut b = [0u8; 1];
        let n = input.read(&mut b)?;
        if n == 0 {
            if shift == 0 {
                return Ok(None);
            }
            return Err(Error::Decode("truncated length prefix".into()));
        }
        len |= u64::from(b[0] & 0x7f) << shift;
        if b[0] < 0x80 {
            break;
        }
        shift += 7;
        if shift > 63 {
            return Err(Error::Decode("length prefix overflow".into()));
        }
    }
    let len = usize::try_from(len).map_err(|_| Error::Decode("item too large".into()))?;
    if len > MAX_ITEM_SIZE {
        return Err(Error::Decode(format!("item of {len} bytes exceeds limit")));
    }
    let mut buf = vec![0u8; len];
    input.read_exact(&mut buf)?;
    proto::SnapshotItem::decode(buf.as_slice())
        .map(Some)
        .map_err(|e| Error::Decode(e.to_string()))
}

/// Writes the uncompressed item stream of `store` at `height` (Go
/// `rootmulti.Store.Snapshot`). Returns the number of IAVL nodes written.
pub fn write_items(store: &multistore::Store, height: u64, out: &mut impl Write) -> Result<u64> {
    let version = i64::try_from(height).map_err(|_| Error::InvalidMetadata("height".into()))?;
    let mut nodes = 0u64;
    for (name, exporter) in store.export(version)? {
        write_item(out, Item::Store(proto::SnapshotStoreItem { name }))?;
        for node in exporter {
            let node = node?;
            write_item(
                out,
                Item::Iavl(proto::SnapshotIavlItem {
                    key: node.key,
                    value: node.value,
                    version: node.version,
                    height: i32::from(node.height),
                }),
            )?;
            nodes += 1;
        }
    }
    Ok(nodes)
}

/// Restores an uncompressed item stream into `store` at `height` (Go
/// `rootmulti.Store.Restore`) and returns the restored commit ID.
pub fn read_items(
    store: &mut multistore::Store,
    height: u64,
    input: &mut impl Read,
) -> Result<multistore::CommitId> {
    let version = i64::try_from(height).map_err(|_| Error::InvalidMetadata("height".into()))?;
    let mut restorer = store.restore(version)?;
    while let Some(item) = read_item(input)? {
        match item.item {
            Some(Item::Store(s)) => restorer.start_store(&s.name)?,
            Some(Item::Iavl(node)) => {
                let height = i8::try_from(node.height)
                    .map_err(|_| Error::Decode(format!("node height {} too large", node.height)))?;
                if node.version < 0 {
                    return Err(Error::Decode(format!(
                        "negative node version {}",
                        node.version
                    )));
                }
                restorer.add_node(iavl::ExportNode {
                    key: node.key,
                    value: node.value,
                    version: node.version,
                    height,
                })?;
            }
            Some(Item::Extension(ext)) => {
                return Err(Error::Decode(format!(
                    "unknown extension snapshotter {}",
                    ext.name
                )));
            }
            Some(Item::ExtensionPayload(_)) | None => {
                return Err(Error::Decode("unexpected snapshot item".into()));
            }
        }
    }
    Ok(restorer.finish()?)
}

/// `io::Write` that cuts its input into chunk files, hashing as it goes.
struct ChunkWriter {
    dir: PathBuf,
    chunk_size: usize,
    current: Option<(BufWriter<fs::File>, hash::Sha256Hasher)>,
    written: usize,
    chunk_hashes: Vec<Vec<u8>>,
    total: hash::Sha256Hasher,
    cancel: Arc<AtomicBool>,
}

impl ChunkWriter {
    fn new(dir: PathBuf, chunk_size: usize, cancel: Arc<AtomicBool>) -> Self {
        Self {
            dir,
            chunk_size: chunk_size.max(1),
            current: None,
            written: 0,
            chunk_hashes: Vec::new(),
            total: hash::Sha256Hasher::new(),
            cancel,
        }
    }

    fn close_chunk(&mut self) -> io::Result<()> {
        if let Some((mut file, hasher)) = self.current.take() {
            file.flush()?;
            self.chunk_hashes.push(hasher.finalize().to_vec());
        }
        Ok(())
    }

    fn finish(mut self) -> io::Result<(Vec<Vec<u8>>, Vec<u8>)> {
        self.close_chunk()?;
        Ok((self.chunk_hashes, self.total.finalize().to_vec()))
    }
}

impl Write for ChunkWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::other("snapshot cancelled"));
        }
        if self.current.is_none() || self.written >= self.chunk_size {
            self.close_chunk()?;
            let path = self.dir.join(self.chunk_hashes.len().to_string());
            self.current = Some((
                BufWriter::new(fs::File::create(path)?),
                hash::Sha256Hasher::new(),
            ));
            self.written = 0;
        }
        let n = data.len().min(self.chunk_size - self.written);
        if let Some((file, hasher)) = &mut self.current {
            file.write_all(&data[..n])?;
            hasher.update(&data[..n]);
        }
        self.total.update(&data[..n]);
        self.written += n;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some((file, _)) = &mut self.current {
            file.flush()?;
        }
        Ok(())
    }
}

/// `io::Read` over chunk files in order.
struct ChunkReader {
    paths: std::vec::IntoIter<PathBuf>,
    current: Option<BufReader<fs::File>>,
}

impl Read for ChunkReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.current.is_none() {
                match self.paths.next() {
                    Some(path) => self.current = Some(BufReader::new(fs::File::open(path)?)),
                    None => return Ok(0),
                }
            }
            if let Some(reader) = &mut self.current {
                let n = reader.read(buf)?;
                if n > 0 || buf.is_empty() {
                    return Ok(n);
                }
                self.current = None;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

/// Snapshot policy (app.toml `state-sync.snapshot-interval` /
/// `snapshot-keep-recent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotOptions {
    /// Take a snapshot every `interval` heights (0 = never).
    pub interval: u64,
    /// Snapshots kept (0 = all).
    pub keep_recent: u32,
}

impl Default for SnapshotOptions {
    fn default() -> Self {
        Self {
            interval: 0,
            keep_recent: 2,
        }
    }
}

#[derive(Debug)]
struct Restore {
    snapshot: Snapshot,
    next_chunk: u32,
    dir: PathBuf,
}

/// Called on the snapshot thread before a background snapshot starts
/// writing (tests use it to hold a snapshot in flight).
type SnapshotHook = Arc<dyn Fn(u64) + Send + Sync>;

/// A snapshot being written on its own thread.
struct InFlight {
    height: u64,
    cancel: Arc<AtomicBool>,
    handle: JoinHandle<Result<Snapshot>>,
}

impl std::fmt::Debug for InFlight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InFlight")
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// Creates, lists, serves and restores snapshots (Go `snapshots.Manager`).
pub struct Manager {
    dir: PathBuf,
    options: SnapshotOptions,
    chunk_size: usize,
    restore: Option<Restore>,
    in_flight: Option<InFlight>,
    hook: Option<SnapshotHook>,
}

impl std::fmt::Debug for Manager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Manager")
            .field("dir", &self.dir)
            .field("options", &self.options)
            .field("chunk_size", &self.chunk_size)
            .field("restore", &self.restore)
            .field("in_flight", &self.in_flight)
            .finish_non_exhaustive()
    }
}

const METADATA_FILE: &str = "metadata";
/// Present in a snapshot directory while it is being written.
const WRITING_FILE: &str = "writing";

impl Manager {
    /// Manager storing snapshots under `dir`. Snapshots this node was
    /// writing when it crashed are removed: no snapshot can be in flight
    /// before the manager exists. (Directories without the marker, such as
    /// a migrated Go home's snapshots indexed by `metadata.db`, are left
    /// alone.)
    pub fn new(dir: impl Into<PathBuf>, options: SnapshotOptions) -> Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        remove_incomplete(&dir)?;
        Ok(Self {
            dir,
            options,
            chunk_size: CHUNK_SIZE,
            restore: None,
            in_flight: None,
            hook: None,
        })
    }

    /// Installs a hook run on the snapshot thread before a background
    /// snapshot starts writing (tests).
    pub fn set_snapshot_hook(&mut self, hook: impl Fn(u64) + Send + Sync + 'static) {
        self.hook = Some(Arc::new(hook));
    }

    /// Overrides the chunk size (tests; Go always uses [`CHUNK_SIZE`]).
    pub fn set_chunk_size(&mut self, chunk_size: usize) {
        self.chunk_size = chunk_size;
    }

    /// The policy.
    #[must_use]
    pub fn options(&self) -> SnapshotOptions {
        self.options
    }

    fn snapshot_dir(&self, height: u64, format: u32) -> PathBuf {
        self.dir.join(height.to_string()).join(format.to_string())
    }

    /// True when a snapshot should be taken after committing `height`.
    #[must_use]
    pub fn should_snapshot(&self, height: u64) -> bool {
        self.options.interval > 0 && height > 0 && height.is_multiple_of(self.options.interval)
    }

    /// Takes a snapshot of `store` at committed `height`, then prunes old
    /// snapshots per the policy. Blocks until done; the node uses
    /// [`Self::snapshot_in_background`].
    pub fn create(&self, store: &multistore::Store, height: u64) -> Result<Snapshot> {
        let snapshot = write_snapshot(
            &self.snapshot_dir(height, CURRENT_FORMAT),
            self.chunk_size,
            store,
            height,
            &Arc::new(AtomicBool::new(false)),
        )?;
        if self.options.keep_recent > 0 {
            prune_dir(&self.dir, self.options.keep_recent)?;
        }
        Ok(snapshot)
    }

    /// Go `Manager.SnapshotIfApplicable` → `go m.snapshot(height)`: starts
    /// writing a snapshot of committed `height` of `store` on its own
    /// thread and returns at once. The thread reads a detached view of the
    /// version and holds a [`multistore::VersionPin`] so pruning keeps it
    /// until the snapshot is done (Go defers `PruneSnapshotHeight` to the
    /// end of `Manager.Create`). Afterwards old snapshots are pruned per
    /// `keep_recent` (Go `snapshot`). A failure is logged on that thread and
    /// never reaches the caller.
    ///
    /// Fails with [`Error::Conflict`] like Go's `Manager.Create` when
    /// another snapshot or a restore is in progress (`begin(opSnapshot)`)
    /// or a snapshot of this or a later height exists.
    pub fn snapshot_in_background(&mut self, store: &multistore::Store, height: u64) -> Result<()> {
        self.reap_finished();
        if let Some(f) = &self.in_flight {
            return Err(Error::Conflict(format!(
                "a snapshot operation is in progress (height {})",
                f.height
            )));
        }
        if self.restore.is_some() {
            return Err(Error::Conflict("a restore operation is in progress".into()));
        }
        if let Some(latest) = self.list()?.first()
            && latest.height >= height
        {
            return Err(Error::Conflict(format!(
                "a more recent snapshot already exists at height {}",
                latest.height
            )));
        }
        let version = i64::try_from(height).map_err(|_| Error::InvalidMetadata("height".into()))?;
        let pin = store.pin_version(version);
        let view = store.view_at(version)?;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker = Worker {
            root: self.dir.clone(),
            dir: self.snapshot_dir(height, CURRENT_FORMAT),
            chunk_size: self.chunk_size,
            keep_recent: self.options.keep_recent,
            cancel: Arc::clone(&cancel),
            hook: self.hook.clone(),
        };
        let handle = std::thread::Builder::new()
            .name(format!("snapshot-{height}"))
            .spawn(move || {
                let result = worker.run(&view, height);
                drop(pin);
                result
            })?;
        self.in_flight = Some(InFlight {
            height,
            cancel,
            handle,
        });
        Ok(())
    }

    /// True while a background snapshot is being written.
    #[must_use]
    pub fn snapshot_in_progress(&self) -> bool {
        self.in_flight
            .as_ref()
            .is_some_and(|f| !f.handle.is_finished())
    }

    /// Waits for the background snapshot, if any, and returns its result.
    pub fn wait_for_snapshot(&mut self) -> Option<Result<Snapshot>> {
        let f = self.in_flight.take()?;
        Some(join(f))
    }

    /// Stops the background snapshot, if any, at its next write and waits
    /// for its thread to exit; its partial directory is removed and never
    /// listed (shutdown).
    pub fn abandon_snapshot(&mut self) {
        if let Some(f) = self.in_flight.take() {
            f.cancel.store(true, Ordering::Relaxed);
            let height = f.height;
            match join(f) {
                Ok(_) | Err(Error::Cancelled) => {}
                Err(e) => tracing::warn!(height, error = %e, "abandoned state snapshot"),
            }
        }
    }

    /// Drops the handle of a finished background snapshot (its result was
    /// already logged by its thread).
    fn reap_finished(&mut self) {
        if self
            .in_flight
            .as_ref()
            .is_some_and(|f| f.handle.is_finished())
            && let Some(f) = self.in_flight.take()
        {
            let _ = join(f);
        }
    }

    /// Complete snapshots, newest first.
    pub fn list(&self) -> Result<Vec<Snapshot>> {
        list_dir(&self.dir)
    }

    /// Chunk bytes, or `None` when the snapshot/chunk does not exist.
    pub fn load_chunk(&self, height: u64, format: u32, chunk: u32) -> Result<Option<Vec<u8>>> {
        let dir = self.snapshot_dir(height, format);
        if !dir.join(METADATA_FILE).exists() {
            return Ok(None);
        }
        match fs::read(dir.join(chunk.to_string())) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Deletes all but the `retain` newest snapshots; returns how many were
    /// deleted.
    pub fn prune(&self, retain: u32) -> Result<u64> {
        prune_dir(&self.dir, retain)
    }

    /// Starts restoring `snapshot` (ABCI `OfferSnapshot`). Replaces any
    /// restore in progress.
    pub fn begin_restore(&mut self, snapshot: Snapshot) -> Result<()> {
        if snapshot.format != CURRENT_FORMAT {
            return Err(Error::UnknownFormat(snapshot.format));
        }
        if snapshot.chunks == 0 {
            return Err(Error::InvalidMetadata("no chunks".into()));
        }
        if usize::try_from(snapshot.chunks).ok() != Some(snapshot.chunk_hashes.len()) {
            return Err(Error::InvalidMetadata(format!(
                "snapshot has {} chunk hashes, but {} chunks",
                snapshot.chunk_hashes.len(),
                snapshot.chunks
            )));
        }
        let dir = self.dir.join("restore");
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        fs::create_dir_all(&dir)?;
        self.restore = Some(Restore {
            snapshot,
            next_chunk: 0,
            dir,
        });
        Ok(())
    }

    /// True while a restore is in progress.
    #[must_use]
    pub fn restoring(&self) -> bool {
        self.restore.is_some()
    }

    /// Applies the next chunk (ABCI `ApplySnapshotChunk`). After the last
    /// chunk the snapshot is imported into `store` and its commit ID is
    /// returned; earlier chunks return `None`.
    pub fn restore_chunk(
        &mut self,
        store: &mut multistore::Store,
        chunk: &[u8],
    ) -> Result<Option<multistore::CommitId>> {
        let restore = self.restore.as_mut().ok_or(Error::NoRestore)?;
        let index = restore.next_chunk;
        let expected = usize::try_from(index)
            .ok()
            .and_then(|i| restore.snapshot.chunk_hashes.get(i))
            .ok_or(Error::NoRestore)?;
        if hash::sha256(chunk).as_slice() != expected.as_slice() {
            return Err(Error::ChunkHashMismatch(index));
        }
        fs::write(restore.dir.join(index.to_string()), chunk)?;
        restore.next_chunk += 1;
        if restore.next_chunk < restore.snapshot.chunks {
            return Ok(None);
        }
        let Some(restore) = self.restore.take() else {
            return Err(Error::NoRestore);
        };
        let paths: Vec<PathBuf> = (0..restore.snapshot.chunks)
            .map(|i| restore.dir.join(i.to_string()))
            .collect();
        let result = restore_from(store, restore.snapshot.height, paths);
        fs::remove_dir_all(&restore.dir)?;
        result.map(Some)
    }

    /// Restores a snapshot held by this manager (Go `RestoreLocalSnapshot`).
    pub fn restore_local(
        &self,
        store: &mut multistore::Store,
        height: u64,
        format: u32,
    ) -> Result<multistore::CommitId> {
        let snapshot = self
            .list()?
            .into_iter()
            .find(|s| s.height == height && s.format == format)
            .ok_or_else(|| Error::InvalidMetadata(format!("no snapshot {height}/{format}")))?;
        let dir = self.snapshot_dir(height, format);
        let paths = (0..snapshot.chunks)
            .map(|i| dir.join(i.to_string()))
            .collect();
        restore_from(store, height, paths)
    }

    /// Snapshot directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

fn restore_from(
    store: &mut multistore::Store,
    height: u64,
    paths: Vec<PathBuf>,
) -> Result<multistore::CommitId> {
    let reader = ChunkReader {
        paths: paths.into_iter(),
        current: None,
    };
    let mut decoder = BufReader::new(ZlibDecoder::new(reader));
    read_items(store, height, &mut decoder)
}

/// Complete snapshots under `root`, newest first. A directory pruned by
/// the snapshot thread while it is being listed is skipped.
fn list_dir(root: &Path) -> Result<Vec<Snapshot>> {
    let mut out = Vec::new();
    for height_dir in fs::read_dir(root)? {
        let height_dir = height_dir?;
        if height_dir
            .file_name()
            .to_string_lossy()
            .parse::<u64>()
            .is_err()
        {
            continue;
        }
        let format_dirs = match fs::read_dir(height_dir.path()) {
            Ok(dirs) => dirs,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        for format_dir in format_dirs {
            let Ok(format_dir) = format_dir else {
                continue;
            };
            if let Ok(bytes) = fs::read(format_dir.path().join(METADATA_FILE)) {
                let snapshot = proto::Snapshot::decode(bytes.as_slice())
                    .map_err(|e| Error::InvalidMetadata(e.to_string()))?;
                out.push(Snapshot::from_proto(snapshot));
            }
        }
    }
    out.sort_by(|a, b| b.height.cmp(&a.height).then(b.format.cmp(&a.format)));
    Ok(out)
}

/// Deletes all but the `retain` newest snapshots under `root`; returns how
/// many were deleted.
fn prune_dir(root: &Path, retain: u32) -> Result<u64> {
    let mut deleted = 0;
    let keep = usize::try_from(retain).unwrap_or(usize::MAX);
    for snapshot in list_dir(root)?.into_iter().skip(keep) {
        let height_dir = root.join(snapshot.height.to_string());
        fs::remove_dir_all(height_dir.join(snapshot.format.to_string()))?;
        if fs::read_dir(&height_dir)?.next().is_none() {
            fs::remove_dir(height_dir)?;
        }
        deleted += 1;
    }
    Ok(deleted)
}

/// Removes `<height>/<format>` directories still marked as being written,
/// and height directories this leaves empty.
fn remove_incomplete(root: &Path) -> Result<()> {
    for height_dir in fs::read_dir(root)? {
        let height_dir = height_dir?;
        if height_dir
            .file_name()
            .to_string_lossy()
            .parse::<u64>()
            .is_err()
            || !height_dir.file_type()?.is_dir()
        {
            continue;
        }
        let mut removed = false;
        for format_dir in fs::read_dir(height_dir.path())? {
            let format_dir = format_dir?.path();
            if format_dir.join(WRITING_FILE).exists() {
                fs::remove_dir_all(format_dir)?;
                removed = true;
            }
        }
        if removed && fs::read_dir(height_dir.path())?.next().is_none() {
            fs::remove_dir(height_dir.path())?;
        }
    }
    Ok(())
}

/// Writes the snapshot of `store` at `height` into `dir` (Go
/// `Manager.Create` + `Store.Save`): chunks first, metadata last, so an
/// interrupted snapshot is never listed. On failure the directory is
/// removed (Go `Store.Save` deletes the snapshot on error).
fn write_snapshot(
    dir: &Path,
    chunk_size: usize,
    store: &multistore::Store,
    height: u64,
    cancel: &Arc<AtomicBool>,
) -> Result<Snapshot> {
    let result = (|| {
        if dir.exists() {
            fs::remove_dir_all(dir)?;
        }
        fs::create_dir_all(dir)?;
        fs::write(dir.join(WRITING_FILE), [])?;
        let mut encoder = ZlibEncoder::new(
            ChunkWriter::new(dir.to_path_buf(), chunk_size, Arc::clone(cancel)),
            Compression::new(COMPRESSION_LEVEL),
        );
        write_items(store, height, &mut encoder)?;
        let (chunk_hashes, hash) = encoder.finish()?.finish()?;
        let snapshot = Snapshot {
            height,
            format: CURRENT_FORMAT,
            chunks: u32::try_from(chunk_hashes.len())
                .map_err(|_| Error::InvalidMetadata("too many chunks".into()))?,
            hash,
            chunk_hashes,
        };
        let tmp = dir.join(format!("{METADATA_FILE}.tmp"));
        fs::write(&tmp, snapshot.to_proto().encode_to_vec())?;
        fs::rename(tmp, dir.join(METADATA_FILE))?;
        fs::remove_file(dir.join(WRITING_FILE))?;
        Ok(snapshot)
    })();
    result.map_err(|e| {
        let _ = fs::remove_dir_all(dir);
        if let Some(height_dir) = dir.parent()
            && fs::read_dir(height_dir).is_ok_and(|mut d| d.next().is_none())
        {
            let _ = fs::remove_dir(height_dir);
        }
        if cancel.load(Ordering::Relaxed) {
            Error::Cancelled
        } else {
            e
        }
    })
}

/// The work of one background snapshot (Go `Manager.snapshot`).
struct Worker {
    root: PathBuf,
    dir: PathBuf,
    chunk_size: usize,
    keep_recent: u32,
    cancel: Arc<AtomicBool>,
    hook: Option<SnapshotHook>,
}

impl Worker {
    fn run(self, view: &multistore::Store, height: u64) -> Result<Snapshot> {
        tracing::info!(height, "creating state snapshot");
        if let Some(hook) = &self.hook {
            hook(height);
        }
        let snapshot = match write_snapshot(&self.dir, self.chunk_size, view, height, &self.cancel)
        {
            Ok(snapshot) => snapshot,
            Err(Error::Cancelled) => {
                tracing::info!(height, "state snapshot abandoned");
                return Err(Error::Cancelled);
            }
            Err(e) => {
                tracing::error!(height, error = %e, "failed to create state snapshot");
                return Err(e);
            }
        };
        tracing::info!(
            height,
            format = snapshot.format,
            chunks = snapshot.chunks,
            "completed state snapshot"
        );
        if self.keep_recent > 0 {
            match prune_dir(&self.root, self.keep_recent) {
                Ok(pruned) => tracing::debug!(pruned, "pruned state snapshots"),
                Err(e) => tracing::error!(error = %e, "failed to prune state snapshots"),
            }
        }
        Ok(snapshot)
    }
}

fn join(f: InFlight) -> Result<Snapshot> {
    f.handle
        .join()
        .unwrap_or_else(|_| Err(Error::Io(io::Error::other("snapshot thread panicked"))))
}

impl Drop for Manager {
    /// Shutdown abandons an in-flight snapshot cleanly (Go leaves its
    /// goroutine to die with the process; its unsaved snapshot is never
    /// listed).
    fn drop(&mut self) {
        self.abandon_snapshot();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use std::sync::Arc;

    fn store(db: Arc<dyn kvdb::KvBackend>) -> multistore::Store {
        let mut s = multistore::Store::open(
            db,
            multistore::StoreOptions {
                pruning: multistore::PruningOptions::nothing(),
                node_cache_size: 128,
            },
        );
        for name in ["acc", "bank", "zempty"] {
            s.mount_store(name, multistore::StoreKind::Iavl).unwrap();
        }
        s.load_latest_version().unwrap();
        s
    }

    fn fill(s: &mut multistore::Store, heights: u64) {
        for h in 0..heights {
            for i in 0..200u64 {
                let x = (h * 7919 + i * 104_729) % 1000;
                let st = if x % 2 == 0 { "acc" } else { "bank" };
                s.get_store_mut(st)
                    .unwrap()
                    .set(
                        format!("k{x:04}").into_bytes(),
                        vec![u8::try_from(h).unwrap(); 64],
                    )
                    .unwrap();
            }
            s.commit().unwrap();
        }
    }

    #[test]
    fn create_list_load_and_restore_round_trip() {
        let src_db: Arc<dyn kvdb::KvBackend> = Arc::new(kvdb::MemBackend::new());
        let mut src = store(src_db);
        fill(&mut src, 6);
        let dir = tempfile::tempdir().unwrap();
        let manager = Manager::new(
            dir.path().join("snapshots"),
            SnapshotOptions {
                interval: 3,
                keep_recent: 1,
            },
        )
        .unwrap();
        assert!(manager.should_snapshot(3) && !manager.should_snapshot(4));
        manager.create(&src, 3).unwrap();
        let snap = manager.create(&src, 6).unwrap();
        // keep_recent = 1 pruned height 3.
        assert_eq!(manager.list().unwrap(), vec![snap.clone()]);
        assert!(manager.load_chunk(3, CURRENT_FORMAT, 0).unwrap().is_none());

        // Offer + apply through the ABCI path, with the metadata round
        // tripping through its protobuf form.
        let offered = Snapshot::from_abci(
            snap.height,
            snap.format,
            snap.chunks,
            snap.hash.clone(),
            &snap.metadata_bytes(),
        )
        .unwrap();
        assert_eq!(offered, snap);
        let dst_dir = tempfile::tempdir().unwrap();
        let mut dst_manager = Manager::new(dst_dir.path(), SnapshotOptions::default()).unwrap();
        let mut dst = store(Arc::new(kvdb::MemBackend::new()));
        dst_manager.begin_restore(offered).unwrap();
        let mut result = None;
        for i in 0..snap.chunks {
            let chunk = manager.load_chunk(6, CURRENT_FORMAT, i).unwrap().unwrap();
            // A corrupted chunk is refused and can be retried.
            let mut bad = chunk.clone();
            bad[0] ^= 1;
            assert!(matches!(
                dst_manager.restore_chunk(&mut dst, &bad),
                Err(Error::ChunkHashMismatch(_))
            ));
            result = dst_manager.restore_chunk(&mut dst, &chunk).unwrap();
        }
        assert_eq!(result.unwrap(), src.last_commit_id().unwrap());
        assert_eq!(dst.last_commit_id().unwrap(), src.last_commit_id().unwrap());
        assert!(!dst_manager.restoring());

        // Local restore too.
        let mut local = store(Arc::new(kvdb::MemBackend::new()));
        assert_eq!(
            manager
                .restore_local(&mut local, 6, CURRENT_FORMAT)
                .unwrap(),
            src.last_commit_id().unwrap()
        );
    }

    #[test]
    fn multi_chunk_streams_and_metadata_validation() {
        // Incompressible values force several chunks.
        let mut src = store(Arc::new(kvdb::MemBackend::new()));
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        for i in 0..100u32 {
            let value: Vec<u8> = (0..2_000)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    x.to_le_bytes()[0]
                })
                .collect();
            src.get_store_mut("acc")
                .unwrap()
                .set(i.to_be_bytes().to_vec(), value)
                .unwrap();
        }
        src.commit().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut manager = Manager::new(dir.path(), SnapshotOptions::default()).unwrap();
        manager.set_chunk_size(64 * 1024);
        let snap = manager.create(&src, 1).unwrap();
        assert!(snap.chunks >= 3, "{}", snap.chunks);
        let mut dst = store(Arc::new(kvdb::MemBackend::new()));
        assert_eq!(
            manager.restore_local(&mut dst, 1, CURRENT_FORMAT).unwrap(),
            src.last_commit_id().unwrap()
        );

        let mut bad = snap.clone();
        bad.format = 2;
        assert!(matches!(
            manager.begin_restore(bad),
            Err(Error::UnknownFormat(2))
        ));
        let mut bad = snap.clone();
        bad.chunk_hashes.pop();
        assert!(matches!(
            manager.begin_restore(bad),
            Err(Error::InvalidMetadata(_))
        ));
        assert!(matches!(
            manager.restore_chunk(&mut dst, b"x"),
            Err(Error::NoRestore)
        ));
    }

    /// A store pruned after every commit down to the latest version (the
    /// harshest policy for an in-flight snapshot).
    fn pruned_store() -> multistore::Store {
        let mut s = store(Arc::new(kvdb::MemBackend::new()));
        s.set_pruning(multistore::PruningOptions::latest_only());
        s
    }

    /// A hook that reports the snapshot start and blocks it until released.
    fn blocking_hook(
        manager: &mut Manager,
    ) -> (std::sync::mpsc::Receiver<u64>, std::sync::mpsc::Sender<()>) {
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let release_rx = std::sync::Mutex::new(release_rx);
        manager.set_snapshot_hook(move |height| {
            started_tx.send(height).unwrap();
            let _ = release_rx.lock().unwrap().recv();
        });
        (started_rx, release_tx)
    }

    /// Go `Manager.SnapshotIfApplicable` runs the snapshot in a goroutine:
    /// the caller (Commit) returns at once, later commits prune freely
    /// except the pinned snapshot height, a second snapshot is skipped while
    /// one is running, and the result is a valid snapshot of that height.
    #[test]
    fn background_snapshot_does_not_block_commits_and_pins_its_height() {
        let mut src = pruned_store();
        fill(&mut src, 3);
        let at3 = src.last_commit_id().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut manager = Manager::new(
            dir.path(),
            SnapshotOptions {
                interval: 3,
                keep_recent: 1,
            },
        )
        .unwrap();
        let (started, release) = blocking_hook(&mut manager);

        manager.snapshot_in_background(&src, 3).unwrap();
        assert_eq!(started.recv().unwrap(), 3);
        assert!(manager.snapshot_in_progress());
        assert!(manager.list().unwrap().is_empty());

        // Commits continue while the snapshot is blocked; version 3 is
        // pinned against pruning, every other old version goes.
        fill(&mut src, 4);
        assert_eq!(src.available_versions(), vec![3, 7]);
        // Go: "a snapshot operation is in progress".
        assert!(matches!(
            manager.snapshot_in_background(&src, 6),
            Err(Error::Conflict(_))
        ));

        release.send(()).unwrap();
        let snap = manager.wait_for_snapshot().unwrap().unwrap();
        assert_eq!(snap.height, 3);
        assert!(!manager.snapshot_in_progress());
        assert_eq!(manager.list().unwrap(), vec![snap]);
        let mut dst = store(Arc::new(kvdb::MemBackend::new()));
        assert_eq!(
            manager.restore_local(&mut dst, 3, CURRENT_FORMAT).unwrap(),
            at3
        );

        // The pin is released: the next commit prunes version 3.
        fill(&mut src, 1);
        assert_eq!(src.available_versions(), vec![8]);
        // Go: "a more recent snapshot already exists".
        assert!(matches!(
            manager.snapshot_in_background(&src, 3),
            Err(Error::Conflict(_))
        ));
    }

    /// Shutdown abandons an in-flight snapshot: nothing partial is left on
    /// disk or listed, and the pin is released.
    #[test]
    fn abandoned_snapshot_leaves_no_partial_directory() {
        let mut src = pruned_store();
        fill(&mut src, 3);
        let dir = tempfile::tempdir().unwrap();
        let mut manager = Manager::new(
            dir.path(),
            SnapshotOptions {
                interval: 3,
                keep_recent: 0,
            },
        )
        .unwrap();
        let (started, release) = blocking_hook(&mut manager);
        manager.snapshot_in_background(&src, 3).unwrap();
        assert_eq!(started.recv().unwrap(), 3);
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            release.send(()).unwrap();
        });
        manager.abandon_snapshot();
        releaser.join().unwrap();
        assert!(!manager.snapshot_in_progress());
        assert!(manager.list().unwrap().is_empty());
        assert!(!dir.path().join("3").exists());
        fill(&mut src, 1);
        assert_eq!(src.available_versions(), vec![4]);
    }

    /// A snapshot interrupted by a crash (still marked as being written)
    /// is removed when the manager opens; a Go snapshot directory (chunks
    /// only, metadata in Go's `metadata.db`) is kept.
    #[test]
    fn interrupted_snapshot_directories_are_removed_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let ours = dir.path().join("9").join("3");
        fs::create_dir_all(&ours).unwrap();
        fs::write(ours.join("0"), b"partial").unwrap();
        fs::write(ours.join(WRITING_FILE), b"").unwrap();
        let go = dir.path().join("6").join("3");
        fs::create_dir_all(&go).unwrap();
        fs::write(go.join("0"), b"go chunk").unwrap();
        let manager = Manager::new(dir.path(), SnapshotOptions::default()).unwrap();
        assert!(manager.list().unwrap().is_empty());
        assert!(!dir.path().join("9").exists());
        assert!(go.join("0").exists());
    }
}
