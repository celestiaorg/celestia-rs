//! The consensus write-ahead log (`consensus/wal.go`) in Go's on-disk
//! format: each record is `crc32c(data) (4 bytes BE) || len(data) (4 bytes
//! BE) || data`, with `data` a protobuf `tendermint.consensus.TimedWALMessage`.
//! `#ENDHEIGHT h` (an `EndHeight` record) marks that block `h` was saved
//! to the block store; crash recovery replays the records after the marker
//! of the last committed height.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ctypes::Time;
use ctypes::proto::consensus as pbc;
use ctypes::proto::types as pb;
use prost::Message;

use crate::types::{ConsensusMessage, TimeoutInfo};

/// `maxMsgSizeBytes` (`maxMsgSize + 24`, `maxMsgSize = 1 MiB`).
pub const MAX_MSG_SIZE_BYTES: u32 = 1024 * 1024 + 24;

/// Rotate (compact) the head file once it exceeds this size.
pub const DEFAULT_HEAD_SIZE_LIMIT: u64 = 10 * 1024 * 1024;

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
    /// All durable + buffered bytes of the log.
    fn read_all(&mut self) -> Result<Vec<u8>, WalError>;
    /// Replaces the log with `bz` (repair / compaction).
    fn replace(&mut self, bz: &[u8]) -> Result<(), WalError>;

    /// `SearchForEndHeight`: the records after `#ENDHEIGHT height`, or
    /// `None` when the marker is absent. Corruption after the marker is
    /// reported (corruption before it is skipped, as Go's
    /// `IgnoreDataCorruptionErrors`).
    fn search_for_end_height(
        &mut self,
        height: i64,
    ) -> Result<Option<Vec<TimedWalMessage>>, WalError> {
        self.flush_and_sync()?;
        let bz = self.read_all()?;
        let (msgs, _, err) = decode_all(&bz);
        let Some(pos) = msgs
            .iter()
            .position(|m| m.msg == WalMessage::EndHeight(height))
        else {
            return Ok(None);
        };
        if let Some(e) = err {
            // The records after the marker cannot all be read.
            return Err(e);
        }
        Ok(Some(msgs[pos + 1..].to_vec()))
    }

    /// `repairWalFile`: keeps the decodable prefix.
    fn repair(&mut self) -> Result<(), WalError> {
        self.flush_and_sync()?;
        let bz = self.read_all()?;
        let (_, good, _) = decode_all(&bz);
        self.replace(&bz[..good])
    }
}

/// A file-backed WAL (single head file `<dir>/wal`, compacted at an
/// `#ENDHEIGHT` once it grows past [`DEFAULT_HEAD_SIZE_LIMIT`]).
#[derive(Debug)]
pub struct FileWal {
    path: PathBuf,
    file: fs::File,
    buf: Vec<u8>,
    size: u64,
    head_size_limit: u64,
}

fn io(e: std::io::Error) -> WalError {
    WalError::Io(e.to_string())
}

impl FileWal {
    /// `NewWAL` + `OnStart`: opens (creating) the log; an empty log starts
    /// with `#ENDHEIGHT 0`.
    pub fn open(path: impl AsRef<Path>, now: Time) -> Result<Self, WalError> {
        let path = path.as_ref().to_path_buf();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(io)?;
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&path)
            .map_err(io)?;
        let size = file.metadata().map_err(io)?.len();
        let mut wal = Self {
            path,
            file,
            buf: Vec::new(),
            size,
            head_size_limit: DEFAULT_HEAD_SIZE_LIMIT,
        };
        if size == 0 {
            wal.write_sync(&TimedWalMessage {
                time: now,
                msg: WalMessage::EndHeight(0),
            })?;
        }
        Ok(wal)
    }

    /// Path of the head file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Sets the compaction threshold.
    pub fn set_head_size_limit(&mut self, limit: u64) {
        self.head_size_limit = limit;
    }
}

impl Wal for FileWal {
    fn write(&mut self, m: &TimedWalMessage) -> Result<(), WalError> {
        let rec = encode(m)?;
        self.buf.extend_from_slice(&rec);
        self.size += rec.len() as u64;
        if let WalMessage::EndHeight(_) = m.msg {
            if self.size > self.head_size_limit {
                // Everything before this marker is no longer needed.
                self.flush_and_sync()?;
                self.replace(&rec)?;
            }
        }
        Ok(())
    }

    fn flush_and_sync(&mut self) -> Result<(), WalError> {
        if !self.buf.is_empty() {
            self.file.write_all(&self.buf).map_err(io)?;
            self.buf.clear();
        }
        self.file.sync_data().map_err(io)
    }

    fn read_all(&mut self) -> Result<Vec<u8>, WalError> {
        self.flush_and_sync()?;
        fs::read(&self.path).map_err(io)
    }

    fn replace(&mut self, bz: &[u8]) -> Result<(), WalError> {
        let tmp = self.path.with_extension("tmp");
        {
            let mut f = fs::File::create(&tmp).map_err(io)?;
            f.write_all(bz).map_err(io)?;
            f.sync_all().map_err(io)?;
        }
        fs::rename(&tmp, &self.path).map_err(io)?;
        self.file = fs::OpenOptions::new()
            .append(true)
            .read(true)
            .open(&self.path)
            .map_err(io)?;
        self.buf.clear();
        self.size = bz.len() as u64;
        Ok(())
    }
}

/// The durable bytes of a [`MemWal`] (shared so they survive a simulated
/// crash of the node owning the WAL).
pub type MemWalDisk = Arc<Mutex<Vec<u8>>>;

/// In-memory WAL for simulations: unsynced writes are lost on "crash"
/// (when the [`MemWal`] is dropped without a flush).
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
}

impl Wal for MemWal {
    fn write(&mut self, m: &TimedWalMessage) -> Result<(), WalError> {
        self.buf.extend_from_slice(&encode(m)?);
        Ok(())
    }

    fn flush_and_sync(&mut self) -> Result<(), WalError> {
        let mut d = self
            .disk
            .lock()
            .map_err(|_| WalError::Io("poisoned".into()))?;
        d.extend_from_slice(&self.buf);
        self.buf.clear();
        Ok(())
    }

    fn read_all(&mut self) -> Result<Vec<u8>, WalError> {
        self.flush_and_sync()?;
        self.disk
            .lock()
            .map(|d| d.clone())
            .map_err(|_| WalError::Io("poisoned".into()))
    }

    fn replace(&mut self, bz: &[u8]) -> Result<(), WalError> {
        self.buf.clear();
        let mut d = self
            .disk
            .lock()
            .map_err(|_| WalError::Io("poisoned".into()))?;
        *d = bz.to_vec();
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
    fn read_all(&mut self) -> Result<Vec<u8>, WalError> {
        Ok(Vec::new())
    }
    fn replace(&mut self, _: &[u8]) -> Result<(), WalError> {
        Ok(())
    }
    fn search_for_end_height(&mut self, _: i64) -> Result<Option<Vec<TimedWalMessage>>, WalError> {
        Ok(None)
    }
}
