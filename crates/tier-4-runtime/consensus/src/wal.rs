//! The consensus write-ahead log (`consensus/wal.go`) in Go's on-disk
//! format: each record is `crc32c(data) (4 bytes BE) || len(data) (4 bytes
//! BE) || data`, with `data` a protobuf `tendermint.consensus.TimedWALMessage`.
//! `#ENDHEIGHT h` (an `EndHeight` record) marks that block `h` was saved
//! to the block store; crash recovery replays the records after the marker
//! of the last committed height.
//!
//! [`FileWal`] is `BaseWAL` over a port of `libs/autofile.Group`: the head
//! file (`data/cs.wal/wal`) is rotated into `wal.000`, `wal.001`, ... once
//! it reaches 10 MB (checked every 5 s), the oldest rotated files are
//! deleted once the group exceeds 1 GB, writes go through a 40 KiB buffer
//! that is flushed and fsynced by `WriteSync` / `FlushAndSync` and every
//! 2 s, and `SearchForEndHeight` reads the files newest to oldest as Go's
//! `GroupReader` + `WALDecoder` do. A Go node's WAL directory and a Rust
//! node's are interchangeable.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::time::{Duration, Instant};

use ctypes::Time;
use ctypes::proto::consensus as pbc;
use ctypes::proto::types as pb;
use prost::Message;

use crate::types::{ConsensusMessage, TimeoutInfo};

/// `maxMsgSizeBytes` (`maxMsgSize + 24`, `maxMsgSize = 512 KiB`, the
/// consensus reactor's receive capacity).
pub const MAX_MSG_SIZE_BYTES: u32 = 512 * 1024 + 24;

/// `defaultHeadSizeLimit`: the head file is rotated once it reaches this
/// size (10 MB).
pub const DEFAULT_HEAD_SIZE_LIMIT: u64 = 10 * 1024 * 1024;

/// `defaultTotalSizeLimit`: the oldest rotated files are deleted while the
/// group is at least this large (1 GB).
pub const DEFAULT_TOTAL_SIZE_LIMIT: u64 = 1024 * 1024 * 1024;

/// `defaultGroupCheckDuration`: how often the size limits are checked.
pub const DEFAULT_GROUP_CHECK_DURATION: Duration = Duration::from_millis(5000);

/// `walDefaultFlushInterval`: how often buffered records are flushed and
/// fsynced.
pub const WAL_DEFAULT_FLUSH_INTERVAL: Duration = Duration::from_secs(2);

/// `maxFilesToRemove`: files deleted per total-size check.
const MAX_FILES_TO_REMOVE: usize = 4;

/// `bufio.NewWriterSize(head, 4096*10)`.
const HEAD_BUF_SIZE: usize = 4096 * 10;

/// A WAL entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalMessage {
    /// `types.EventDataRoundState`.
    RoundState {
        /// Height.
        height: i64,
        /// Round.
        round: i32,
        /// Step name (`RoundStepPropose`, ...).
        step: String,
    },
    /// `msgInfo`: a proposal, block part or vote (`peer_id` empty = own).
    MsgInfo {
        /// Message.
        msg: ConsensusMessage,
        /// Sender.
        peer_id: String,
    },
    /// `timeoutInfo`.
    Timeout(TimeoutInfo),
    /// `EndHeightMessage`.
    EndHeight(i64),
}

/// `TimedWALMessage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedWalMessage {
    /// Write time.
    pub time: Time,
    /// Entry.
    pub msg: WalMessage,
}

/// WAL errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalError {
    /// `DataCorruptionError`.
    Corruption(String),
    /// I/O.
    Io(String),
}

impl std::fmt::Display for WalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Corruption(s) => write!(f, "DataCorruptionError[{s}]"),
            Self::Io(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for WalError {}

impl TimedWalMessage {
    /// `WALToProto` + `TimedWALMessage`.
    #[must_use]
    pub fn to_proto(&self) -> pbc::TimedWalMessage {
        use pbc::wal_message::Sum;
        let sum = match &self.msg {
            WalMessage::RoundState {
                height,
                round,
                step,
            } => Sum::EventDataRoundState(pb::EventDataRoundState {
                height: *height,
                round: *round,
                step: step.clone(),
            }),
            WalMessage::MsgInfo { msg, peer_id } => Sum::MsgInfo(pbc::MsgInfo {
                msg: Some(msg.to_proto()),
                peer_id: peer_id.clone(),
            }),
            WalMessage::Timeout(ti) => Sum::TimeoutInfo(ti.to_proto()),
            WalMessage::EndHeight(h) => Sum::EndHeight(pbc::EndHeight { height: *h }),
        };
        pbc::TimedWalMessage {
            time: Some(self.time.to_proto()),
            msg: Some(pbc::WalMessage { sum: Some(sum) }),
        }
    }

    /// `WALFromProto`.
    pub fn from_proto(p: &pbc::TimedWalMessage) -> Result<Self, String> {
        use pbc::wal_message::Sum;
        let time = Time::from_proto(p.time.as_ref()).map_err(|e| e.to_string())?;
        let sum = p
            .msg
            .as_ref()
            .and_then(|m| m.sum.as_ref())
            .ok_or("nil WAL message")?;
        let msg = match sum {
            Sum::EventDataRoundState(e) => WalMessage::RoundState {
                height: e.height,
                round: e.round,
                step: e.step.clone(),
            },
            Sum::MsgInfo(m) => WalMessage::MsgInfo {
                msg: ConsensusMessage::from_proto(m.msg.as_ref().ok_or("nil msg")?)?,
                peer_id: m.peer_id.clone(),
            },
            Sum::TimeoutInfo(t) => {
                WalMessage::Timeout(TimeoutInfo::from_proto(t).ok_or("invalid timeout info")?)
            }
            Sum::EndHeight(e) => WalMessage::EndHeight(e.height),
        };
        Ok(Self { time, msg })
    }
}

/// CRC-32C (Castagnoli), as `crc32.MakeTable(crc32.Castagnoli)`.
#[must_use]
pub fn crc32c(data: &[u8]) -> u32 {
    const POLY: u32 = 0x82F6_3B78;
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ POLY
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// `WALEncoder.Encode`.
pub fn encode(m: &TimedWalMessage) -> Result<Vec<u8>, WalError> {
    let data = m.to_proto().encode_to_vec();
    let len = u32::try_from(data.len()).map_err(|e| WalError::Io(e.to_string()))?;
    if len > MAX_MSG_SIZE_BYTES {
        return Err(WalError::Io(format!(
            "msg is too big: {len} bytes, max: {MAX_MSG_SIZE_BYTES} bytes"
        )));
    }
    let mut out = Vec::with_capacity(8 + data.len());
    out.extend_from_slice(&crc32c(&data).to_be_bytes());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&data);
    Ok(out)
}

/// `WALDecoder`: every decodable record, the byte length they span and the
/// corruption (if any) that stopped decoding.
#[must_use]
pub fn decode_all(bz: &[u8]) -> (Vec<TimedWalMessage>, usize, Option<WalError>) {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bz.len() {
        if bz.len() - i < 8 {
            return (
                out,
                i,
                Some(WalError::Corruption(
                    "failed to read length: unexpected EOF".into(),
                )),
            );
        }
        let crc = u32::from_be_bytes([bz[i], bz[i + 1], bz[i + 2], bz[i + 3]]);
        let len = u32::from_be_bytes([bz[i + 4], bz[i + 5], bz[i + 6], bz[i + 7]]);
        if len > MAX_MSG_SIZE_BYTES {
            return (
                out,
                i,
                Some(WalError::Corruption(format!(
                    "length {len} exceeded maximum possible value of {MAX_MSG_SIZE_BYTES} bytes"
                ))),
            );
        }
        let start = i + 8;
        let end = start + len as usize;
        if end > bz.len() {
            return (
                out,
                i,
                Some(WalError::Corruption(format!(
                    "failed to read data: unexpected EOF (read: {}, wanted: {len})",
                    bz.len() - start
                ))),
            );
        }
        let data = &bz[start..end];
        let actual = crc32c(data);
        if actual != crc {
            return (
                out,
                i,
                Some(WalError::Corruption(format!(
                    "checksums do not match: read: {crc}, actual: {actual}"
                ))),
            );
        }
        let decoded = pbc::TimedWalMessage::decode(data)
            .map_err(|e| e.to_string())
            .and_then(|p| TimedWalMessage::from_proto(&p));
        match decoded {
            Ok(m) => out.push(m),
            Err(e) => {
                return (
                    out,
                    i,
                    Some(WalError::Corruption(format!("failed to decode data: {e}"))),
                );
            }
        }
        i = end;
    }
    (out, i, None)
}

// ---------------------------------------------------------------------------
// Reading: `GroupReader` + `WALDecoder`

/// One file of a group as the reader sees it.
#[derive(Debug, Clone)]
enum Source<'a> {
    /// A file on disk. A missing file reads as empty (Go's `GroupReader`
    /// opens it with `O_CREATE`).
    Path(PathBuf),
    /// In-memory bytes ([`MemWal`]).
    Bytes(&'a [u8]),
}

/// `GroupReader`: the files from a start index to the head, read as one
/// stream (a record may continue in the next file).
struct GroupReader<'a> {
    srcs: &'a [Source<'a>],
    next: usize,
    cur: Option<Box<dyn Read + 'a>>,
}

impl<'a> GroupReader<'a> {
    fn new(srcs: &'a [Source<'a>]) -> Self {
        Self {
            srcs,
            next: 0,
            cur: None,
        }
    }

    /// `GroupReader.Read`: fills `buf` unless the last file ends first;
    /// returns the number of bytes read.
    fn read_full(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut n = 0;
        while n < buf.len() {
            if self.cur.is_none() {
                let Some(src) = self.srcs.get(self.next) else {
                    break;
                };
                self.next += 1;
                let rd: Box<dyn Read + 'a> = match src {
                    Source::Path(p) => match fs::File::open(p) {
                        Ok(f) => Box::new(std::io::BufReader::new(f)),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                            Box::new(std::io::empty())
                        }
                        Err(e) => return Err(e),
                    },
                    Source::Bytes(b) => Box::new(*b),
                };
                self.cur = Some(rd);
            }
            let Some(cur) = self.cur.as_mut() else {
                break;
            };
            match cur.read(&mut buf[n..]) {
                Ok(0) => self.cur = None,
                Ok(k) => n += k,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(n)
    }
}

/// `WALDecoder` over a [`GroupReader`].
struct WalDecoder<'a> {
    rd: GroupReader<'a>,
}

impl WalDecoder<'_> {
    /// `WALDecoder.Decode`: the next record, `Ok(None)` at the end of the
    /// stream (`io.EOF`), [`WalError::Corruption`] for a
    /// `DataCorruptionError`. Like Go, a failed record consumes what was
    /// read of it and decoding continues from there.
    fn decode(&mut self) -> Result<Option<TimedWalMessage>, WalError> {
        let mut b = [0u8; 4];
        // A short checksum read is io.EOF in Go (`errors.Is(err, io.EOF)`),
        // so a torn tail of 1-3 bytes ends the stream cleanly.
        if self.rd.read_full(&mut b).map_err(io)? < 4 {
            return Ok(None);
        }
        let crc = u32::from_be_bytes(b);
        if self.rd.read_full(&mut b).map_err(io)? < 4 {
            return Err(WalError::Corruption("failed to read length: EOF".into()));
        }
        let len = u32::from_be_bytes(b);
        if len > MAX_MSG_SIZE_BYTES {
            return Err(WalError::Corruption(format!(
                "length {len} exceeded maximum possible value of {MAX_MSG_SIZE_BYTES} bytes"
            )));
        }
        if len == 0 {
            // `GroupReader.Read` refuses an empty slice.
            return Err(WalError::Corruption(
                "failed to read data: given empty slice (read: 0, wanted: 0)".into(),
            ));
        }
        let mut data = vec![0u8; len as usize];
        let n = self.rd.read_full(&mut data).map_err(io)?;
        if n < data.len() {
            return Err(WalError::Corruption(format!(
                "failed to read data: EOF (read: {n}, wanted: {len})"
            )));
        }
        let actual = crc32c(&data);
        if actual != crc {
            return Err(WalError::Corruption(format!(
                "checksums do not match: read: {crc}, actual: {actual}"
            )));
        }
        let p = pbc::TimedWalMessage::decode(data.as_slice())
            .map_err(|e| WalError::Corruption(format!("failed to decode data: {e}")))?;
        TimedWalMessage::from_proto(&p)
            .map(Some)
            .map_err(|e| WalError::Corruption(format!("failed to convert from proto: {e}")))
    }
}

/// What `catchupReplay` reads after a found `#ENDHEIGHT`: the records up
/// to the end of the head, or up to the corruption that stopped it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Found {
    msgs: Vec<TimedWalMessage>,
    err: Option<WalError>,
}

impl Found {
    /// The trait's view: corruption after the marker is an error.
    fn into_result(self) -> Result<Vec<TimedWalMessage>, WalError> {
        match self.err {
            Some(e) => Err(e),
            None => Ok(self.msgs),
        }
    }
}

/// `SearchForEndHeight(height, {IgnoreDataCorruptionErrors: true})` over
/// the group files `srcs` (oldest first, head last), then `catchupReplay`'s
/// decode loop over the rest of the stream.
fn search_group(
    srcs: &[Source<'_>],
    min_index: usize,
    height: i64,
) -> Result<Option<Found>, WalError> {
    let mut last_height_found = -1i64;
    // Starting from the last file: the search is usually for the last height.
    for start in (0..srcs.len()).rev() {
        let mut dec = WalDecoder {
            rd: GroupReader::new(&srcs[start..]),
        };
        loop {
            match dec.decode() {
                Ok(None) => {
                    // No need to look at older files after seeing h < height.
                    if last_height_found > 0 && last_height_found < height {
                        return Ok(None);
                    }
                    break;
                }
                Err(WalError::Corruption(e)) => {
                    tracing::error!(err = %e, "Corrupted entry. Skipping...");
                }
                Err(e) => return Err(e),
                Ok(Some(m)) => {
                    if let WalMessage::EndHeight(h) = m.msg {
                        last_height_found = h;
                        if h == height {
                            tracing::info!(height, index = min_index + start, "Found");
                            let mut msgs = Vec::new();
                            loop {
                                match dec.decode() {
                                    Ok(Some(m)) => msgs.push(m),
                                    Ok(None) => return Ok(Some(Found { msgs, err: None })),
                                    Err(e @ WalError::Corruption(_)) => {
                                        return Ok(Some(Found { msgs, err: Some(e) }));
                                    }
                                    Err(e) => return Err(e),
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------

/// The WAL as consensus uses it.
pub trait Wal: Send {
    /// `Write` (buffered).
    fn write(&mut self, m: &TimedWalMessage) -> Result<(), WalError>;
    /// `WriteSync`: write, flush and fsync.
    fn write_sync(&mut self, m: &TimedWalMessage) -> Result<(), WalError> {
        self.write(m)?;
        self.flush_and_sync()
    }
    /// `FlushAndSync`.
    fn flush_and_sync(&mut self) -> Result<(), WalError>;

    /// `SearchForEndHeight` as `catchupReplay` uses it: the records after
    /// `#ENDHEIGHT height` (to the end of the head file), or `None` when
    /// the marker is absent. Corruption before the marker is skipped
    /// (`IgnoreDataCorruptionErrors`); corruption after it is returned,
    /// as `catchupReplay`'s decode loop does.
    fn search_for_end_height(
        &mut self,
        height: i64,
    ) -> Result<Option<Vec<TimedWalMessage>>, WalError>;

    /// `OnStart`'s repair of a corrupted WAL (`repairWalFile`): keeps the
    /// decodable prefix of the head file.
    fn repair(&mut self) -> Result<(), WalError>;
}

fn io(e: std::io::Error) -> WalError {
    WalError::Io(e.to_string())
}

/// `GroupInfo`: the group's files as found on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GroupInfo {
    /// Index of the first file in the group, including the head.
    pub min_index: usize,
    /// Index of the last file in the group (the head).
    pub max_index: usize,
    /// Total size of the group (every file whose name starts with the
    /// head's name).
    pub total_size: u64,
    /// Size of the head.
    pub head_size: u64,
}

/// [`FileWal`] settings: Go's `autofile.Group` options plus the WAL flush
/// interval. Go's `config.toml` sets none of them; [`Default`] is Go's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupOptions {
    /// `GroupHeadSizeLimit` (0: never rotate).
    pub head_size_limit: u64,
    /// `GroupTotalSizeLimit` (0: never delete).
    pub total_size_limit: u64,
    /// `GroupCheckDuration` (zero: no background checks; tests call
    /// [`FileWal::check_head_size_limit`] / [`FileWal::check_total_size_limit`]).
    pub group_check_duration: Duration,
    /// `SetFlushInterval` (zero: no periodic flush).
    pub flush_interval: Duration,
}

impl Default for GroupOptions {
    fn default() -> Self {
        Self {
            head_size_limit: DEFAULT_HEAD_SIZE_LIMIT,
            total_size_limit: DEFAULT_TOTAL_SIZE_LIMIT,
            group_check_duration: DEFAULT_GROUP_CHECK_DURATION,
            flush_interval: WAL_DEFAULT_FLUSH_INTERVAL,
        }
    }
}

/// `filePathForIndex`: `<head>.NNN`, or the head itself for `max_index`.
#[must_use]
pub fn file_path_for_index(head: &Path, index: usize, max_index: usize) -> PathBuf {
    if index == max_index {
        return head.to_path_buf();
    }
    let mut s = head.as_os_str().to_owned();
    s.push(format!(".{index:03}"));
    PathBuf::from(s)
}

/// The index of a rotated file name (`^.+\.([0-9]{3,})$`).
fn indexed_file(name: &str) -> Option<usize> {
    let (pre, digits) = name.rsplit_once('.')?;
    if pre.is_empty() || digits.len() < 3 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// `os.OpenFile(path, O_RDWR|O_CREATE|O_APPEND, 0600)`.
fn open_append(path: &Path) -> std::io::Result<fs::File> {
    let mut o = fs::OpenOptions::new();
    o.read(true).append(true).create(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(path)
}

/// `cmtos.EnsureDir(dir, 0700)`.
fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    let mut b = fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)
}

/// fsyncs a directory so a rename or a new file in it is durable (Go does
/// not; it only makes rotation more robust to power loss).
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Err(e) = fs::File::open(dir).and_then(|d| d.sync_all()) {
        tracing::warn!(dir = %dir.display(), err = %e, "failed to sync WAL directory");
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// `autofile.Group` (with its `AutoFile` head).
#[derive(Debug)]
struct Group {
    head_path: PathBuf,
    /// The open head (`nil` after a rotation until the next write).
    head: Option<fs::File>,
    /// `headBuf`.
    buf: Vec<u8>,
    head_size_limit: u64,
    total_size_limit: u64,
    /// Set from the directory at open (Go never updates it afterwards).
    min_index: usize,
    /// The head's index; incremented by every rotation.
    max_index: usize,
    /// A failed rotation (Go panics): every later write fails.
    broken: Option<String>,
}

impl Group {
    /// `OpenGroup`.
    fn open(head_path: PathBuf, opts: &GroupOptions) -> Result<Self, WalError> {
        let head = open_append(&head_path).map_err(io)?;
        let mut g = Self {
            head_path,
            head: Some(head),
            buf: Vec::with_capacity(HEAD_BUF_SIZE),
            head_size_limit: opts.head_size_limit,
            total_size_limit: opts.total_size_limit,
            min_index: 0,
            max_index: 0,
            broken: None,
        };
        let info = g.read_group_info().map_err(io)?;
        g.min_index = info.min_index;
        g.max_index = info.max_index;
        Ok(g)
    }

    fn check_broken(&self) -> Result<(), WalError> {
        match &self.broken {
            Some(e) => Err(WalError::Io(format!("WAL group is unusable: {e}"))),
            None => Ok(()),
        }
    }

    /// The head file, reopened (created) if a rotation closed it.
    fn head(&mut self) -> std::io::Result<&mut fs::File> {
        let f = if let Some(f) = self.head.take() {
            f
        } else {
            let f = open_append(&self.head_path)?;
            if let Some(dir) = self.head_path.parent() {
                sync_dir(dir);
            }
            f
        };
        Ok(self.head.insert(f))
    }

    /// `bufio.Writer.Write`.
    fn write(&mut self, mut p: &[u8]) -> std::io::Result<()> {
        while p.len() > HEAD_BUF_SIZE - self.buf.len() {
            if self.buf.is_empty() {
                // Large write, empty buffer: write directly.
                self.head()?.write_all(p)?;
                return Ok(());
            }
            let n = HEAD_BUF_SIZE - self.buf.len();
            self.buf.extend_from_slice(&p[..n]);
            p = &p[n..];
            self.flush()?;
        }
        self.buf.extend_from_slice(p);
        Ok(())
    }

    /// `headBuf.Flush`.
    fn flush(&mut self) -> std::io::Result<()> {
        if !self.buf.is_empty() {
            let buf = std::mem::take(&mut self.buf);
            let res = self.head().and_then(|f| f.write_all(&buf));
            self.buf = buf;
            res?;
            self.buf.clear();
        }
        Ok(())
    }

    /// `FlushAndSync`: flush the buffer and fsync the head.
    fn flush_and_sync(&mut self) -> std::io::Result<()> {
        self.flush()?;
        self.head()?.sync_all()
    }

    /// `Head.Size()`.
    fn head_size(&mut self) -> std::io::Result<u64> {
        Ok(self.head()?.metadata()?.len())
    }

    /// `checkHeadSizeLimit`.
    fn check_head_size_limit(&mut self) {
        let limit = self.head_size_limit;
        if limit == 0 {
            return;
        }
        match self.head_size() {
            Err(e) => tracing::error!(
                head = %self.head_path.display(), err = %e,
                "Group's head may grow without bound"
            ),
            Ok(size) if size >= limit => {
                if let Err(e) = self.rotate_file() {
                    tracing::error!(err = %e, "WAL rotation failed");
                }
            }
            Ok(_) => {}
        }
    }

    /// `checkTotalSizeLimit`: deletes up to four of the oldest files while
    /// the group is at or above the limit (never the head).
    fn check_total_size_limit(&mut self) {
        let limit = self.total_size_limit;
        if limit == 0 {
            return;
        }
        let info = match self.read_group_info() {
            Ok(i) => i,
            Err(e) => {
                tracing::error!(err = %e, "failed to read WAL group info");
                return;
            }
        };
        let mut total = info.total_size;
        for i in 0..MAX_FILES_TO_REMOVE {
            let index = info.min_index + i;
            if total < limit {
                return;
            }
            if index == info.max_index {
                // Special degenerate case, just do nothing.
                tracing::error!(head = %self.head_path.display(), "Group's head may grow without bound");
                return;
            }
            let path = file_path_for_index(&self.head_path, index, info.max_index);
            let Ok(meta) = fs::metadata(&path) else {
                tracing::error!(file = %path.display(), "Failed to fetch info for file");
                continue;
            };
            let size = meta.len();
            if let Err(e) = fs::remove_file(&path) {
                tracing::error!(path = %path.display(), err = %e, "Failed to remove path");
                return;
            }
            tracing::info!(path = %path.display(), size, "removed old WAL file");
            total = total.saturating_sub(size);
        }
    }

    /// `RotateFile`: flush, fsync and close the head, rename it to
    /// `<head>.NNN` (NNN = the head's index); the next write creates a new
    /// head.
    fn rotate_file(&mut self) -> Result<(), WalError> {
        self.check_broken()?;
        let res = (|| -> std::io::Result<PathBuf> {
            self.flush_and_sync()?;
            self.head = None;
            let to = file_path_for_index(&self.head_path, self.max_index, self.max_index + 1);
            fs::rename(&self.head_path, &to)?;
            if let Some(dir) = self.head_path.parent() {
                sync_dir(dir);
            }
            Ok(to)
        })();
        match res {
            Ok(to) => {
                tracing::debug!(to = %to.display(), "rotated WAL head");
                self.max_index += 1;
                Ok(())
            }
            Err(e) => {
                let msg = format!("rotate {}: {e}", self.head_path.display());
                self.broken = Some(msg.clone());
                Err(WalError::Io(msg))
            }
        }
    }

    /// `readGroupInfo`: scans the head's directory.
    fn read_group_info(&self) -> std::io::Result<GroupInfo> {
        let dir = self.head_path.parent().unwrap_or_else(|| Path::new("."));
        let head_base = self
            .head_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (mut min, mut max): (Option<usize>, Option<usize>) = (None, None);
        let (mut total_size, mut head_size) = (0u64, 0u64);
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == head_base {
                let size = entry.metadata()?.len();
                total_size += size;
                head_size = size;
            } else if name.starts_with(&head_base) {
                total_size += entry.metadata()?.len();
                if let Some(index) = indexed_file(&name) {
                    max = Some(max.map_or(index, |m| m.max(index)));
                    min = Some(min.map_or(index, |m| m.min(index)));
                }
            }
        }
        let (min_index, max_index) = match (min, max) {
            // No numbered files: the head is index 0.
            (Some(min), Some(max)) => (min, max + 1),
            _ => (0, 0),
        };
        Ok(GroupInfo {
            min_index,
            max_index,
            total_size,
            head_size,
        })
    }

    /// The group files from `min_index` to the head.
    fn sources(&self) -> Vec<Source<'static>> {
        (self.min_index..=self.max_index)
            .map(|i| Source::Path(file_path_for_index(&self.head_path, i, self.max_index)))
            .collect()
    }
}

/// The background routines: `BaseWAL.processFlushTicks` and
/// `Group.processTicks`.
#[derive(Debug)]
struct Ticker {
    stop: mpsc::Sender<()>,
    handle: std::thread::JoinHandle<()>,
}

fn spawn_ticker(group: Arc<Mutex<Group>>, opts: GroupOptions) -> Result<Option<Ticker>, WalError> {
    let flush = opts.flush_interval;
    let check = opts.group_check_duration;
    if flush.is_zero() && check.is_zero() {
        return Ok(None);
    }
    let (stop, rx) = mpsc::channel::<()>();
    let handle = std::thread::Builder::new()
        .name("cs-wal".into())
        .spawn(move || {
            let start = Instant::now();
            let mut next_flush = (!flush.is_zero()).then(|| start + flush);
            let mut next_check = (!check.is_zero()).then(|| start + check);
            loop {
                let Some(next) = next_flush.into_iter().chain(next_check).min() else {
                    return;
                };
                match rx.recv_timeout(next.saturating_duration_since(Instant::now())) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    _ => return,
                }
                let now = Instant::now();
                let Ok(mut g) = group.lock() else {
                    return;
                };
                if let Some(t) = next_flush
                    && now >= t
                {
                    if g.broken.is_none()
                        && let Err(e) = g.flush_and_sync()
                    {
                        tracing::error!(err = %e, "Periodic WAL flush failed");
                    }
                    next_flush = Some(now + flush);
                }
                if let Some(t) = next_check
                    && now >= t
                {
                    if g.broken.is_none() {
                        g.check_head_size_limit();
                        g.check_total_size_limit();
                    }
                    next_check = Some(now + check);
                }
            }
        })
        .map_err(io)?;
    Ok(Some(Ticker { stop, handle }))
}

/// The file-backed WAL: Go's `BaseWAL` over an `autofile.Group` in the
/// head file's directory (`<dir>/wal`, `<dir>/wal.000`, ...).
#[derive(Debug)]
pub struct FileWal {
    path: PathBuf,
    opts: GroupOptions,
    group: Arc<Mutex<Group>>,
    ticker: Option<Ticker>,
}

impl FileWal {
    /// `NewWAL` + `OnStart` with Go's defaults (10 MB head, 1 GB group,
    /// 5 s checks, 2 s flushes): opens (creating) the group; an empty head
    /// starts with `#ENDHEIGHT 0`. A head-only directory written by an
    /// older build opens as a group of one file.
    pub fn open(path: impl AsRef<Path>, now: Time) -> Result<Self, WalError> {
        Self::open_with(path, now, GroupOptions::default())
    }

    /// [`FileWal::open`] with explicit group options.
    pub fn open_with(
        path: impl AsRef<Path>,
        now: Time,
        opts: GroupOptions,
    ) -> Result<Self, WalError> {
        let path = std::path::absolute(path.as_ref()).map_err(io)?;
        if let Some(dir) = path.parent() {
            ensure_dir(dir).map_err(|e| {
                WalError::Io(format!("failed to ensure WAL directory is in place: {e}"))
            })?;
        }
        let group = Group::open(path.clone(), &opts)?;
        let mut wal = Self {
            path,
            opts,
            group: Arc::new(Mutex::new(group)),
            ticker: None,
        };
        wal.start(now)?;
        Ok(wal)
    }

    /// `OnStart`: `#ENDHEIGHT 0` into an empty head, then the tickers.
    fn start(&mut self, now: Time) -> Result<(), WalError> {
        let empty = self.lock()?.head_size().map_err(io)? == 0;
        if empty {
            self.write_sync(&TimedWalMessage {
                time: now,
                msg: WalMessage::EndHeight(0),
            })?;
        }
        if self.ticker.is_none() {
            self.ticker = spawn_ticker(self.group.clone(), self.opts)?;
        }
        Ok(())
    }

    fn lock(&self) -> Result<MutexGuard<'_, Group>, WalError> {
        self.group
            .lock()
            .map_err(|_| WalError::Io("WAL lock poisoned".into()))
    }

    /// Path of the head file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The group options.
    #[must_use]
    pub fn options(&self) -> GroupOptions {
        self.opts
    }

    /// `ReadGroupInfo`.
    pub fn group_info(&self) -> Result<GroupInfo, WalError> {
        self.lock()?.read_group_info().map_err(io)
    }

    /// `MinIndex` (as read at open).
    pub fn min_index(&self) -> Result<usize, WalError> {
        Ok(self.lock()?.min_index)
    }

    /// `MaxIndex`: the head's index.
    pub fn max_index(&self) -> Result<usize, WalError> {
        Ok(self.lock()?.max_index)
    }

    /// `Buffered`: bytes not yet written to the head file.
    pub fn buffered(&self) -> Result<usize, WalError> {
        Ok(self.lock()?.buf.len())
    }

    /// `checkHeadSizeLimit` (run by the background ticker).
    pub fn check_head_size_limit(&self) -> Result<(), WalError> {
        let mut g = self.lock()?;
        g.check_broken()?;
        g.check_head_size_limit();
        g.check_broken()
    }

    /// `checkTotalSizeLimit` (run by the background ticker).
    pub fn check_total_size_limit(&self) -> Result<(), WalError> {
        let mut g = self.lock()?;
        g.check_broken()?;
        g.check_total_size_limit();
        Ok(())
    }

    /// `RotateFile`.
    pub fn rotate_file(&self) -> Result<(), WalError> {
        self.lock()?.rotate_file()
    }

    /// `SearchForEndHeight` + `catchupReplay`'s decode loop.
    fn search(&mut self, height: i64) -> Result<Option<Found>, WalError> {
        let mut g = self.lock()?;
        g.check_broken()?;
        g.flush_and_sync().map_err(io)?;
        tracing::info!(
            height,
            min = g.min_index,
            max = g.max_index,
            "Searching for height"
        );
        // Holding the lock: no rotation while the files are read.
        let srcs = g.sources();
        search_group(&srcs, g.min_index, height)
    }
}

impl Drop for FileWal {
    /// `OnStop` + `Close`: stop the tickers, flush and fsync.
    fn drop(&mut self) {
        if let Some(t) = self.ticker.take() {
            drop(t.stop);
            let _ = t.handle.join();
        }
        if let Ok(mut g) = self.group.lock()
            && g.broken.is_none()
            && let Err(e) = g.flush_and_sync()
        {
            tracing::error!(err = %e, "error on flush data to disk");
        }
    }
}

impl Wal for FileWal {
    fn write(&mut self, m: &TimedWalMessage) -> Result<(), WalError> {
        let rec = encode(m)?;
        let mut g = self.lock()?;
        g.check_broken()?;
        g.write(&rec).map_err(io)
    }

    fn flush_and_sync(&mut self) -> Result<(), WalError> {
        let mut g = self.lock()?;
        g.check_broken()?;
        g.flush_and_sync().map_err(io)
    }

    fn search_for_end_height(
        &mut self,
        height: i64,
    ) -> Result<Option<Vec<TimedWalMessage>>, WalError> {
        self.search(height)?.map(Found::into_result).transpose()
    }

    /// `OnStart`'s repair: stop the WAL, copy the head to
    /// `<head>.CORRUPTED`, rewrite the head with the records decodable
    /// before the first error (`repairWalFile`), reopen the WAL.
    fn repair(&mut self) -> Result<(), WalError> {
        {
            let mut g = self.lock()?;
            if let Err(e) = g.flush_and_sync() {
                tracing::error!(err = %e, "error on flush data to disk");
            }
            g.head = None;
            let mut corrupted = self.path.as_os_str().to_owned();
            corrupted.push(".CORRUPTED");
            let corrupted = PathBuf::from(corrupted);
            fs::copy(&self.path, &corrupted).map_err(io)?;
            tracing::debug!(src = %self.path.display(), dst = %corrupted.display(), "backed up WAL file");
            let bz = fs::read(&corrupted).map_err(io)?;
            let (_, good, _) = decode_all(&bz);
            {
                let mut f = fs::OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(&self.path)
                    .map_err(io)?;
                f.write_all(&bz[..good]).map_err(io)?;
                f.sync_all().map_err(io)?;
            }
            // loadWalFile: a fresh group over the repaired files.
            *g = Group::open(self.path.clone(), &self.opts)?;
        }
        self.start(crate::ticker::system_now())
    }
}

/// The durable bytes of a [`MemWal`] (shared so they survive a simulated
/// crash of the node owning the WAL).
pub type MemWalDisk = Arc<Mutex<Vec<u8>>>;

/// In-memory WAL for simulations (a group of one file): unsynced writes
/// are lost on "crash" (when the [`MemWal`] is dropped without a flush).
#[derive(Debug)]
pub struct MemWal {
    disk: MemWalDisk,
    buf: Vec<u8>,
}

impl MemWal {
    /// Opens a WAL over `disk` (writing `#ENDHEIGHT 0` when empty).
    pub fn open(disk: MemWalDisk, now: Time) -> Result<Self, WalError> {
        let empty = disk.lock().map_or(true, |d| d.is_empty());
        let mut w = Self {
            disk,
            buf: Vec::new(),
        };
        if empty {
            w.write_sync(&TimedWalMessage {
                time: now,
                msg: WalMessage::EndHeight(0),
            })?;
        }
        Ok(w)
    }

    fn disk(&self) -> Result<MutexGuard<'_, Vec<u8>>, WalError> {
        self.disk
            .lock()
            .map_err(|_| WalError::Io("poisoned".into()))
    }
}

impl Wal for MemWal {
    fn write(&mut self, m: &TimedWalMessage) -> Result<(), WalError> {
        self.buf.extend_from_slice(&encode(m)?);
        Ok(())
    }

    fn flush_and_sync(&mut self) -> Result<(), WalError> {
        let buf = std::mem::take(&mut self.buf);
        self.disk()?.extend_from_slice(&buf);
        Ok(())
    }

    fn search_for_end_height(
        &mut self,
        height: i64,
    ) -> Result<Option<Vec<TimedWalMessage>>, WalError> {
        self.flush_and_sync()?;
        let bz = self.disk()?.clone();
        search_group(&[Source::Bytes(&bz)], 0, height)?
            .map(Found::into_result)
            .transpose()
    }

    fn repair(&mut self) -> Result<(), WalError> {
        self.flush_and_sync()?;
        let mut d = self.disk()?;
        let (_, good, _) = decode_all(&d);
        d.truncate(good);
        Ok(())
    }
}

/// A WAL that records nothing (`nilWAL`).
#[derive(Debug, Default, Clone, Copy)]
pub struct NilWal;

impl Wal for NilWal {
    fn write(&mut self, _: &TimedWalMessage) -> Result<(), WalError> {
        Ok(())
    }
    fn flush_and_sync(&mut self) -> Result<(), WalError> {
        Ok(())
    }
    fn search_for_end_height(&mut self, _: i64) -> Result<Option<Vec<TimedWalMessage>>, WalError> {
        Ok(None)
    }
    fn repair(&mut self) -> Result<(), WalError> {
        Ok(())
    }
}

#[cfg(test)]
#[path = "wal_tests.rs"]
mod tests;
