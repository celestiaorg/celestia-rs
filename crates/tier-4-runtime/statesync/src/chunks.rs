//! The chunk queue (`statesync/chunks.go`): chunks are stored in a
//! temporary directory, handed out to fetchers, returned in order to the
//! applier and can be retried, discarded or rejected by sender.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::snapshots::Snapshot;

/// A chunk received from a peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Snapshot height.
    pub height: u64,
    /// Snapshot format.
    pub format: u32,
    /// Chunk index.
    pub index: u32,
    /// Contents.
    pub chunk: Vec<u8>,
    /// Sender node ID.
    pub sender: String,
}

/// Chunk queue errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkError {
    /// `errDone`: every chunk was returned, or the queue is closed.
    Done,
    /// `errTimeout`.
    Timeout,
    /// Any other error (Go message).
    Msg(String),
}

impl std::fmt::Display for ChunkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Done => f.write_str("chunk queue has completed"),
            Self::Timeout => f.write_str("timed out waiting for chunk"),
            Self::Msg(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for ChunkError {}

/// Outcome of [`ChunkQueue::wait_for`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
    /// The chunk is in the queue.
    Arrived,
    /// The queue was closed or the index is invalid.
    Closed,
    /// The timeout elapsed.
    TimedOut,
    /// The cancel flag was set.
    Canceled,
}

#[derive(Debug)]
struct Inner {
    snapshot: Option<Snapshot>,
    dir: PathBuf,
    files: HashMap<u32, PathBuf>,
    senders: HashMap<u32, String>,
    allocated: HashSet<u32>,
    returned: HashSet<u32>,
}

/// `chunkQueue`.
#[derive(Debug)]
pub struct ChunkQueue {
    inner: Mutex<Inner>,
    cond: Condvar,
    chunk_timeout: Duration,
}

/// `chunkTimeout`.
pub const CHUNK_TIMEOUT: Duration = Duration::from_secs(120);

fn make_temp_dir(parent: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(parent)?;
    for _ in 0..100 {
        let p = parent.join(format!("tm-statesync{}", rand::random::<u32>()));
        match std::fs::create_dir(&p) {
            Ok(()) => return Ok(p),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("could not create a unique temp dir"))
}

impl ChunkQueue {
    /// `newChunkQueue`: `temp_dir` is the parent (Go `os.MkdirTemp`, empty =
    /// the system temp directory).
    pub fn new(snapshot: &Snapshot, temp_dir: &Path) -> Result<Self, ChunkError> {
        let parent = if temp_dir.as_os_str().is_empty() {
            std::env::temp_dir()
        } else {
            temp_dir.to_path_buf()
        };
        let dir = make_temp_dir(&parent).map_err(|e| {
            ChunkError::Msg(format!(
                "unable to create temp dir for state sync chunks: {e}"
            ))
        })?;
        if snapshot.chunks == 0 {
            return Err(ChunkError::Msg("snapshot has no chunks".into()));
        }
        Ok(Self {
            inner: Mutex::new(Inner {
                snapshot: Some(snapshot.clone()),
                dir,
                files: HashMap::new(),
                senders: HashMap::new(),
                allocated: HashSet::new(),
                returned: HashSet::new(),
            }),
            cond: Condvar::new(),
            chunk_timeout: CHUNK_TIMEOUT,
        })
    }

    /// Overrides `chunkTimeout` (tests).
    #[must_use]
    pub fn with_chunk_timeout(mut self, t: Duration) -> Self {
        self.chunk_timeout = t;
        self
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `Add`: false if the chunk is already queued or the queue is closed.
    pub fn add(&self, chunk: &Chunk) -> Result<bool, ChunkError> {
        // A decoded proto3 `bytes` field is empty exactly when Go's is nil
        // (missing chunks carry no contents).
        if chunk.chunk.is_empty() {
            return Err(ChunkError::Msg("cannot add nil chunk".into()));
        }
        let mut q = self.lock();
        let Some(s) = &q.snapshot else {
            return Ok(false);
        };
        if chunk.height != s.height {
            return Err(ChunkError::Msg(format!(
                "invalid chunk height {}, expected {}",
                chunk.height, s.height
            )));
        }
        if chunk.format != s.format {
            return Err(ChunkError::Msg(format!(
                "invalid chunk format {}, expected {}",
                chunk.format, s.format
            )));
        }
        if chunk.index >= s.chunks {
            return Err(ChunkError::Msg(format!(
                "received unexpected chunk {}",
                chunk.index
            )));
        }
        if q.files.contains_key(&chunk.index) {
            return Ok(false);
        }
        let path = q.dir.join(chunk.index.to_string());
        std::fs::write(&path, &chunk.chunk).map_err(|e| {
            ChunkError::Msg(format!(
                "failed to save chunk {} to file {}: {e}",
                chunk.index,
                path.display()
            ))
        })?;
        q.files.insert(chunk.index, path);
        q.senders.insert(chunk.index, chunk.sender.clone());
        drop(q);
        self.cond.notify_all();
        Ok(true)
    }

    /// `Allocate`: the next unallocated chunk index, or `Done`.
    pub fn allocate(&self) -> Result<u32, ChunkError> {
        let mut q = self.lock();
        let Some(chunks) = q.snapshot.as_ref().map(|s| s.chunks) else {
            return Err(ChunkError::Done);
        };
        if u32::try_from(q.allocated.len()).unwrap_or(u32::MAX) >= chunks {
            return Err(ChunkError::Done);
        }
        for i in 0..chunks {
            if q.allocated.insert(i) {
                return Ok(i);
            }
        }
        Err(ChunkError::Done)
    }

    /// `Close`: wakes all waiters and removes the temporary files.
    pub fn close(&self) -> Result<(), ChunkError> {
        let mut q = self.lock();
        if q.snapshot.is_none() {
            return Ok(());
        }
        q.snapshot = None;
        let dir = q.dir.clone();
        drop(q);
        self.cond.notify_all();
        std::fs::remove_dir_all(&dir).map_err(|e| {
            ChunkError::Msg(format!(
                "failed to clean up state sync tempdir {}: {e}",
                dir.display()
            ))
        })
    }

    /// `Discard`: removes a chunk so it is fetched and returned again.
    pub fn discard(&self, index: u32) -> Result<(), ChunkError> {
        Self::discard_locked(&mut self.lock(), index)
    }

    fn discard_locked(q: &mut Inner, index: u32) -> Result<(), ChunkError> {
        if q.snapshot.is_none() {
            return Ok(());
        }
        let Some(path) = q.files.get(&index).cloned() else {
            return Ok(());
        };
        std::fs::remove_file(&path)
            .map_err(|e| ChunkError::Msg(format!("failed to remove chunk {index}: {e}")))?;
        q.files.remove(&index);
        q.returned.remove(&index);
        q.allocated.remove(&index);
        Ok(())
    }

    /// `DiscardSender`: discards every unreturned chunk from `peer_id`.
    pub fn discard_sender(&self, peer_id: &str) -> Result<(), ChunkError> {
        let mut q = self.lock();
        let indexes: Vec<u32> = q
            .senders
            .iter()
            .filter(|(i, s)| s.as_str() == peer_id && !q.returned.contains(i))
            .map(|(i, _)| *i)
            .collect();
        for index in indexes {
            Self::discard_locked(&mut q, index)?;
            q.senders.remove(&index);
        }
        Ok(())
    }

    /// `GetSender`.
    #[must_use]
    pub fn get_sender(&self, index: u32) -> String {
        self.lock().senders.get(&index).cloned().unwrap_or_default()
    }

    /// `Has`.
    #[must_use]
    pub fn has(&self, index: u32) -> bool {
        self.lock().files.contains_key(&index)
    }

    fn load(q: &Inner, index: u32) -> Result<Option<Chunk>, ChunkError> {
        let Some(path) = q.files.get(&index) else {
            return Ok(None);
        };
        let body = std::fs::read(path)
            .map_err(|e| ChunkError::Msg(format!("failed to load chunk {index}: {e}")))?;
        let s = q.snapshot.as_ref();
        Ok(Some(Chunk {
            height: s.map_or(0, |s| s.height),
            format: s.map_or(0, |s| s.format),
            index,
            chunk: body,
            sender: q.senders.get(&index).cloned().unwrap_or_default(),
        }))
    }

    fn next_up(q: &Inner) -> Result<u32, ChunkError> {
        let Some(s) = &q.snapshot else {
            return Err(ChunkError::Done);
        };
        (0..s.chunks)
            .find(|i| !q.returned.contains(i))
            .ok_or(ChunkError::Done)
    }

    /// `Next`: the next chunk in order, blocking until it arrives (up to
    /// the chunk timeout).
    pub fn next(&self) -> Result<Chunk, ChunkError> {
        self.next_until(None)
    }

    /// [`ChunkQueue::next`], ending early (`Done`) once `cancel` is set.
    pub fn next_until(&self, cancel: Option<&AtomicBool>) -> Result<Chunk, ChunkError> {
        let mut q = self.lock();
        let index = Self::next_up(&q)?;
        if let Some(c) = Self::load(&q, index)? {
            q.returned.insert(index);
            return Ok(c);
        }
        drop(q);
        match self.wait_for(index, self.chunk_timeout, cancel) {
            Wait::Arrived => {}
            Wait::Closed | Wait::Canceled => return Err(ChunkError::Done),
            Wait::TimedOut => return Err(ChunkError::Timeout),
        }
        let mut q = self.lock();
        let c = Self::load(&q, index)?
            .ok_or_else(|| ChunkError::Msg(format!("chunk {index} disappeared")))?;
        q.returned.insert(index);
        Ok(c)
    }

    /// `Retry`: return the chunk again without refetching it.
    pub fn retry(&self, index: u32) {
        self.lock().returned.remove(&index);
    }

    /// `RetryAll`.
    pub fn retry_all(&self) {
        self.lock().returned.clear();
    }

    /// `Size`: the chunk count, 0 when closed.
    #[must_use]
    pub fn size(&self) -> u32 {
        self.lock().snapshot.as_ref().map_or(0, |s| s.chunks)
    }

    /// `WaitFor`: blocks until chunk `index` is queued, the queue closes,
    /// `timeout` elapses or `cancel` is set.
    pub fn wait_for(&self, index: u32, timeout: Duration, cancel: Option<&AtomicBool>) -> Wait {
        let deadline = Instant::now() + timeout;
        let mut q = self.lock();
        loop {
            match &q.snapshot {
                None => return Wait::Closed,
                Some(s) if index >= s.chunks => return Wait::Closed,
                Some(_) if q.files.contains_key(&index) => return Wait::Arrived,
                Some(_) => {}
            }
            if cancel.is_some_and(|c| c.load(Ordering::SeqCst)) {
                return Wait::Canceled;
            }
            let now = Instant::now();
            if now >= deadline {
                return Wait::TimedOut;
            }
            // wake periodically to observe the cancel flag
            let step = (deadline - now).min(Duration::from_millis(200));
            q = self
                .cond
                .wait_timeout(q, step)
                .map_or_else(|e| e.into_inner().0, |(g, _)| g);
        }
    }

    /// Wakes all waiters (used when fetchers are canceled).
    pub fn notify(&self) {
        self.cond.notify_all();
    }
}

impl Drop for ChunkQueue {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, ChunkQueue) {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let s = Snapshot {
            height: 3,
            format: 1,
            chunks: 5,
            hash: vec![7],
            ..Snapshot::default()
        };
        let q = ChunkQueue::new(&s, dir.path()).unwrap_or_else(|e| panic!("{e}"));
        (dir, q)
    }

    fn chunk(index: u32, sender: &str) -> Chunk {
        Chunk {
            height: 3,
            format: 1,
            index,
            chunk: vec![3, 1, u8::try_from(index).unwrap_or(0)],
            sender: sender.into(),
        }
    }

    /// Ports of Go's `chunks_test.go` (`TestChunkQueue_*`).
    #[test]
    fn add_allocate_next() {
        let (_d, q) = setup();
        assert_eq!(q.add(&chunk(0, "a")), Ok(true));
        assert_eq!(q.add(&chunk(0, "b")), Ok(false));
        assert!(
            q.add(&Chunk {
                height: 9,
                ..chunk(1, "a")
            })
            .is_err()
        );
        assert!(
            q.add(&Chunk {
                format: 9,
                ..chunk(1, "a")
            })
            .is_err()
        );
        assert!(q.add(&chunk(5, "a")).is_err());
        for i in 0..5 {
            assert_eq!(q.allocate(), Ok(i));
        }
        assert_eq!(q.allocate(), Err(ChunkError::Done));
        for i in 1..5 {
            q.add(&chunk(i, "b")).unwrap_or_else(|e| panic!("{e}"));
        }
        for i in 0..5 {
            assert_eq!(q.next().map(|c| c.index), Ok(i));
        }
        assert_eq!(q.next(), Err(ChunkError::Done));
        q.retry(2);
        assert_eq!(q.next().map(|c| c.index), Ok(2));
        q.retry_all();
        assert_eq!(q.next().map(|c| (c.index, c.sender)), Ok((0, "a".into())));
        q.discard(1).unwrap_or_else(|e| panic!("{e}"));
        assert!(!q.has(1));
        assert_eq!(q.allocate(), Ok(1));
        assert_eq!(q.size(), 5);
        q.close().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(q.size(), 0);
        assert_eq!(q.next(), Err(ChunkError::Done));
        assert_eq!(q.add(&chunk(1, "a")), Ok(false));
    }

    #[test]
    fn discard_sender() {
        let (_d, q) = setup();
        for i in 0..5 {
            q.add(&chunk(i, if i % 2 == 0 { "a" } else { "b" }))
                .unwrap_or_else(|e| panic!("{e}"));
        }
        assert_eq!(q.next().map(|c| c.index), Ok(0));
        q.discard_sender("a").unwrap_or_else(|e| panic!("{e}"));
        // chunk 0 was returned already and stays
        assert!(q.has(0) && q.has(1) && !q.has(2) && q.has(3) && !q.has(4));
        assert_eq!(q.get_sender(0), "a");
        assert_eq!(q.get_sender(2), "");
    }

    #[test]
    fn next_waits_and_times_out() {
        let (_d, q) = setup();
        let q = std::sync::Arc::new(q.with_chunk_timeout(Duration::from_millis(300)));
        let q2 = std::sync::Arc::clone(&q);
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            q2.add(&chunk(0, "a"))
        });
        assert_eq!(q.next().map(|c| c.index), Ok(0));
        assert_eq!(t.join().ok(), Some(Ok(true)));
        assert_eq!(q.next(), Err(ChunkError::Timeout));
        assert_eq!(q.wait_for(9, Duration::from_secs(1), None), Wait::Closed);
        let cancel = AtomicBool::new(true);
        assert_eq!(
            q.wait_for(1, Duration::from_secs(1), Some(&cancel)),
            Wait::Canceled
        );
    }
}
