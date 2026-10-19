//! Multiplexed connection (port of `p2p/conn/connection.go`).
//!
//! One `MConnection` per peer carries messages for several channels over a
//! byte stream of length-delimited `tendermint.p2p.Packet`s. Outgoing
//! messages are split into `PacketMsg`s of at most
//! `max_packet_msg_payload_size` bytes; the send routine picks the channel
//! with the lowest `recently_sent / priority`, batches up to 10 packets,
//! flushes at most every `flush_throttle`, pings every `ping_interval` and
//! fails the connection if no pong arrives within `pong_timeout`. Both
//! directions are rate-limited to `send_rate` / `recv_rate`.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

use celestia_core_proto::tendermint::p2p as pb;
use prost::Message as _;
use tokio::sync::{Notify, Semaphore, mpsc, watch};
use tokio::time::Instant;

use crate::flowrate::{Monitor, MonitorStatus};
use crate::secret_connection::{ByteRead, ByteWrite, encode_delimited, read_delimited};

/// Default maximum payload per `PacketMsg`.
pub const DEFAULT_MAX_PACKET_MSG_PAYLOAD_SIZE: usize = 1024;
const NUM_BATCH_PACKET_MSGS: usize = 10;
const MIN_WRITE_BUFFER_SIZE: usize = 65536;
const UPDATE_STATS: Duration = Duration::from_secs(2);
/// Default channel send queue capacity.
pub const DEFAULT_SEND_QUEUE_CAPACITY: usize = 1;
/// Default channel receive buffer capacity.
pub const DEFAULT_RECV_BUFFER_CAPACITY: usize = 4096;
/// Default maximum message size per channel (21 MB).
pub const DEFAULT_RECV_MESSAGE_CAPACITY: usize = 22_020_096;
/// How long `send` waits for queue space.
pub const DEFAULT_SEND_TIMEOUT: Duration = Duration::from_secs(10);

/// MConnection configuration (`MConnConfig`).
#[derive(Debug, Clone)]
pub struct MConnConfig {
    /// Send rate limit, bytes/s (<= 0: unlimited).
    pub send_rate: i64,
    /// Receive rate limit, bytes/s (<= 0: unlimited).
    pub recv_rate: i64,
    /// Maximum payload per `PacketMsg`.
    pub max_packet_msg_payload_size: usize,
    /// Minimum interval between flushes of buffered packets.
    pub flush_throttle: Duration,
    /// Interval between pings.
    pub ping_interval: Duration,
    /// Time to wait for a pong before failing the connection.
    pub pong_timeout: Duration,
}

impl Default for MConnConfig {
    /// Go's `DefaultMConnConfig`.
    fn default() -> Self {
        Self {
            send_rate: 512_000,
            recv_rate: 512_000,
            max_packet_msg_payload_size: DEFAULT_MAX_PACKET_MSG_PAYLOAD_SIZE,
            flush_throttle: Duration::from_millis(100),
            ping_interval: Duration::from_secs(60),
            pong_timeout: Duration::from_secs(45),
        }
    }
}

/// A channel's configuration (`ChannelDescriptor`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelDescriptor {
    /// Channel ID.
    pub id: u8,
    /// Relative send priority (> 0).
    pub priority: i32,
    /// Messages that may wait in the send queue (0: default 1).
    pub send_queue_capacity: usize,
    /// Initial receive buffer capacity (0: default 4096).
    pub recv_buffer_capacity: usize,
    /// Maximum size of one received message (0: default 21 MB).
    pub recv_message_capacity: usize,
}

impl ChannelDescriptor {
    /// A descriptor with default capacities.
    #[must_use]
    pub fn new(id: u8, priority: i32) -> Self {
        Self {
            id,
            priority,
            send_queue_capacity: 0,
            recv_buffer_capacity: 0,
            recv_message_capacity: 0,
        }
    }

    /// `FillDefaults`.
    #[must_use]
    pub fn fill_defaults(mut self) -> Self {
        if self.send_queue_capacity == 0 {
            self.send_queue_capacity = DEFAULT_SEND_QUEUE_CAPACITY;
        }
        if self.recv_buffer_capacity == 0 {
            self.recv_buffer_capacity = DEFAULT_RECV_BUFFER_CAPACITY;
        }
        if self.recv_message_capacity == 0 {
            self.recv_message_capacity = DEFAULT_RECV_MESSAGE_CAPACITY;
        }
        self
    }
}

/// Status of one channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelStatus {
    /// Channel ID.
    pub id: u8,
    /// Send queue capacity.
    pub send_queue_capacity: usize,
    /// Messages queued or partially sent.
    pub send_queue_size: i32,
    /// Priority.
    pub priority: i32,
    /// Exponentially decayed bytes sent.
    pub recently_sent: i64,
}

/// Status of a connection (`ConnectionStatus`).
#[derive(Debug, Clone)]
pub struct ConnectionStatus {
    /// Age of the connection.
    pub duration: Duration,
    /// Send-side monitor.
    pub send_monitor: MonitorStatus,
    /// Receive-side monitor.
    pub recv_monitor: MonitorStatus,
    /// Pings received from the peer.
    pub pings_received: u64,
    /// Pongs received from the peer.
    pub pongs_received: u64,
    /// Per-channel status.
    pub channels: Vec<ChannelStatus>,
}

/// Lifecycle state, observable with [`MConnection::closed`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnState {
    /// Running.
    Running,
    /// Stopped by the owner.
    Stopped,
    /// Failed with an error (Go's `onError`).
    Errored(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quit {
    Running,
    Stop,
    FlushStop,
}

#[derive(Debug)]
struct Channel {
    desc: ChannelDescriptor,
    queue: Mutex<VecDeque<Vec<u8>>>,
    space: Semaphore,
    queue_size: AtomicI32,
    recently_sent: AtomicI64,
}

#[derive(Debug)]
struct Shared {
    config: MConnConfig,
    channels: Vec<Channel>,
    index: HashMap<u8, usize>,
    wake: Notify,
    running: AtomicBool,
    quit: watch::Sender<Quit>,
    state: watch::Sender<ConnState>,
    send_monitor: Monitor,
    recv_monitor: Monitor,
    pings_received: AtomicU64,
    pongs_received: AtomicU64,
    created: Instant,
    max_packet_msg_size: usize,
}

impl Shared {
    fn stop_with(&self, mode: Quit, state: ConnState) {
        if self.running.swap(false, Ordering::SeqCst) {
            self.quit.send_replace(mode);
            self.state.send_replace(state);
        }
    }

    fn stop_for_error(&self, err: String) {
        tracing::debug!(%err, "MConnection failed");
        self.stop_with(Quit::Stop, ConnState::Errored(err));
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Encoded size of the largest possible `PacketMsg` packet for a payload
/// limit (Go's `maxPacketMsgSize`).
#[must_use]
pub fn max_packet_msg_size(max_payload: usize) -> usize {
    wrap_msg(1, true, vec![0; max_payload]).encoded_len()
}

fn wrap_msg(channel_id: u8, eof: bool, data: Vec<u8>) -> pb::Packet {
    pb::Packet {
        sum: Some(pb::packet::Sum::PacketMsg(pb::PacketMsg {
            channel_id: i32::from(channel_id),
            eof,
            data,
        })),
    }
}

/// Delimited bytes of a ping packet.
#[must_use]
pub fn ping_packet() -> Vec<u8> {
    encode_delimited(&pb::Packet {
        sum: Some(pb::packet::Sum::PacketPing(pb::PacketPing {})),
    })
}

/// Delimited bytes of a pong packet.
#[must_use]
pub fn pong_packet() -> Vec<u8> {
    encode_delimited(&pb::Packet {
        sum: Some(pb::packet::Sum::PacketPong(pb::PacketPong {})),
    })
}

/// Delimited bytes of a `PacketMsg` packet.
#[must_use]
pub fn msg_packet(channel_id: u8, eof: bool, data: Vec<u8>) -> Vec<u8> {
    encode_delimited(&wrap_msg(channel_id, eof, data))
}

/// A received, reassembled message.
pub type Received = (u8, Vec<u8>);

/// Handle to a running multiplexed connection. Cloning shares it.
#[derive(Debug, Clone)]
pub struct MConnection {
    shared: Arc<Shared>,
}

impl MConnection {
    /// Start the send and receive routines over `reader` / `writer`.
    /// Reassembled messages arrive on the returned receiver, which closes
    /// when the connection stops; [`Self::closed`] then reports why.
    pub fn start<R, W>(
        reader: R,
        writer: W,
        descs: &[ChannelDescriptor],
        config: MConnConfig,
    ) -> Result<(Self, mpsc::Receiver<Received>), String>
    where
        R: ByteRead + 'static,
        W: ByteWrite + 'static,
    {
        if config.pong_timeout >= config.ping_interval {
            return Err("pongTimeout must be less than pingInterval".into());
        }
        let mut channels = Vec::new();
        let mut index = HashMap::new();
        for d in descs {
            let desc = d.clone().fill_defaults();
            if desc.priority <= 0 {
                return Err("Channel default priority must be a positive integer".into());
            }
            index.insert(desc.id, channels.len());
            channels.push(Channel {
                space: Semaphore::new(desc.send_queue_capacity),
                desc,
                queue: Mutex::new(VecDeque::new()),
                queue_size: AtomicI32::new(0),
                recently_sent: AtomicI64::new(0),
            });
        }
        let (quit, _) = watch::channel(Quit::Running);
        let (state, _) = watch::channel(ConnState::Running);
        let shared = Arc::new(Shared {
            max_packet_msg_size: max_packet_msg_size(config.max_packet_msg_payload_size),
            config,
            channels,
            index,
            wake: Notify::new(),
            running: AtomicBool::new(true),
            quit,
            state,
            send_monitor: Monitor::new(),
            recv_monitor: Monitor::new(),
            pings_received: AtomicU64::new(0),
            pongs_received: AtomicU64::new(0),
            created: Instant::now(),
        });
        let (tx, rx) = mpsc::channel(16);
        let sender = SendRoutine {
            sending: vec![None; shared.channels.len()],
            shared: shared.clone(),
            writer,
            buf: Vec::with_capacity(MIN_WRITE_BUFFER_SIZE),
            flush_at: None,
        };
        let (pong_req_tx, pong_req_rx) = mpsc::channel(1);
        let (pong_got_tx, pong_got_rx) = mpsc::channel(1);
        tokio::spawn(sender.run(pong_req_rx, pong_got_rx));
        let recv = RecvRoutine {
            recving: shared
                .channels
                .iter()
                .map(|c| Vec::with_capacity(c.desc.recv_buffer_capacity))
                .collect(),
            shared: shared.clone(),
            reader,
            out: tx,
            pong_req: pong_req_tx,
            pong_got: pong_got_tx,
        };
        tokio::spawn(recv.run());
        Ok((Self { shared }, rx))
    }

    /// Whether the connection is running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.shared.running.load(Ordering::SeqCst)
    }

    /// Queue `msg` on channel `ch`, waiting up to 10s for queue space
    /// (`Send`). Returns false if not running, unknown channel or timeout.
    pub async fn send(&self, ch: u8, msg: Vec<u8>) -> bool {
        if !self.is_running() {
            return false;
        }
        let Some(&i) = self.shared.index.get(&ch) else {
            tracing::error!("Cannot send bytes, unknown channel {ch:X}");
            return false;
        };
        let c = &self.shared.channels[i];
        match tokio::time::timeout(DEFAULT_SEND_TIMEOUT, c.space.acquire()).await {
            Ok(Ok(p)) => p.forget(),
            _ => return false,
        }
        self.enqueue(i, msg);
        true
    }

    /// Queue `msg` without waiting (`TrySend`).
    #[must_use]
    pub fn try_send(&self, ch: u8, msg: Vec<u8>) -> bool {
        if !self.is_running() {
            return false;
        }
        let Some(&i) = self.shared.index.get(&ch) else {
            tracing::error!("Cannot send bytes, unknown channel {ch:X}");
            return false;
        };
        match self.shared.channels[i].space.try_acquire() {
            Ok(p) => p.forget(),
            Err(_) => return false,
        }
        self.enqueue(i, msg);
        true
    }

    fn enqueue(&self, i: usize, msg: Vec<u8>) {
        let c = &self.shared.channels[i];
        lock(&c.queue).push_back(msg);
        c.queue_size.fetch_add(1, Ordering::SeqCst);
        self.shared.wake.notify_one();
    }

    /// Heuristic: whether a send on `ch` would not block (`CanSend`; Go
    /// compares against the default queue capacity).
    #[must_use]
    pub fn can_send(&self, ch: u8) -> bool {
        if !self.is_running() {
            return false;
        }
        self.shared.index.get(&ch).is_some_and(|&i| {
            usize::try_from(self.shared.channels[i].queue_size.load(Ordering::SeqCst))
                .is_ok_and(|n| n < DEFAULT_SEND_QUEUE_CAPACITY)
        })
    }

    /// Stop without flushing (`Stop`).
    pub fn stop(&self) {
        self.shared.stop_with(Quit::Stop, ConnState::Stopped);
    }

    /// Stop after sending everything queued (`FlushStop`).
    pub fn flush_stop(&self) {
        self.shared.stop_with(Quit::FlushStop, ConnState::Stopped);
    }

    /// Wait until the connection stops; returns the final state.
    pub async fn closed(&self) -> ConnState {
        let mut rx = self.shared.state.subscribe();
        loop {
            let s = rx.borrow_and_update().clone();
            if s != ConnState::Running {
                return s;
            }
            if rx.changed().await.is_err() {
                return ConnState::Stopped;
            }
        }
    }

    /// The current state.
    #[must_use]
    pub fn state(&self) -> ConnState {
        self.shared.state.borrow().clone()
    }

    /// A status snapshot.
    #[must_use]
    pub fn status(&self) -> ConnectionStatus {
        ConnectionStatus {
            duration: self.shared.created.elapsed(),
            send_monitor: self.shared.send_monitor.status(),
            recv_monitor: self.shared.recv_monitor.status(),
            pings_received: self.shared.pings_received.load(Ordering::Relaxed),
            pongs_received: self.shared.pongs_received.load(Ordering::Relaxed),
            channels: self
                .shared
                .channels
                .iter()
                .map(|c| ChannelStatus {
                    id: c.desc.id,
                    send_queue_capacity: c.desc.send_queue_capacity,
                    send_queue_size: c.queue_size.load(Ordering::SeqCst),
                    priority: c.desc.priority,
                    recently_sent: c.recently_sent.load(Ordering::SeqCst),
                })
                .collect(),
        }
    }
}

async fn sleep_until_opt(at: Option<Instant>) {
    match at {
        Some(t) => tokio::time::sleep_until(t).await,
        None => std::future::pending().await,
    }
}

struct SendRoutine<W> {
    shared: Arc<Shared>,
    writer: W,
    buf: Vec<u8>,
    /// Per channel: message being sent and offset.
    sending: Vec<Option<(Vec<u8>, usize)>>,
    flush_at: Option<Instant>,
}

impl<W: ByteWrite> SendRoutine<W> {
    async fn run(mut self, mut pong_req: mpsc::Receiver<()>, mut pong_got: mpsc::Receiver<()>) {
        let cfg = self.shared.config.clone();
        let mut quit = self.shared.quit.subscribe();
        let mut ping =
            tokio::time::interval_at(Instant::now() + cfg.ping_interval, cfg.ping_interval);
        let mut stats = tokio::time::interval_at(Instant::now() + UPDATE_STATS, UPDATE_STATS);
        let mut pong_deadline: Option<Instant> = None;
        loop {
            let res: Result<(), String> = tokio::select! {
                r = quit.changed() => {
                    let mode = if r.is_err() { Quit::Stop } else { *quit.borrow_and_update() };
                    match mode {
                        Quit::Running => Ok(()),
                        Quit::Stop => break,
                        Quit::FlushStop => {
                            while let Ok(false) = self.send_some().await {}
                            let _ = self.flush().await;
                            break;
                        }
                    }
                }
                () = sleep_until_opt(self.flush_at) => self.flush().await,
                _ = stats.tick() => {
                    for c in &self.shared.channels {
                        let v = c.recently_sent.load(Ordering::SeqCst);
                        #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
                        c.recently_sent.store((v as f64 * 0.8) as i64, Ordering::SeqCst);
                    }
                    Ok(())
                }
                _ = ping.tick() => {
                    let p = ping_packet();
                    self.buf.extend_from_slice(&p);
                    self.shared.send_monitor.update(p.len(), cfg.send_rate);
                    pong_deadline = Some(Instant::now() + cfg.pong_timeout);
                    self.flush().await
                }
                () = sleep_until_opt(pong_deadline) => Err("pong timeout".to_string()),
                Some(()) = pong_req.recv() => {
                    let p = pong_packet();
                    self.buf.extend_from_slice(&p);
                    self.shared.send_monitor.update(p.len(), cfg.send_rate);
                    self.flush().await
                }
                Some(()) = pong_got.recv() => {
                    pong_deadline = None;
                    Ok(())
                }
                () = self.shared.wake.notified() => {
                    match self.send_some().await {
                        Ok(true) => Ok(()),
                        Ok(false) => {
                            self.shared.wake.notify_one();
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                }
            };
            if let Err(e) = res {
                self.shared.stop_for_error(e);
                break;
            }
            if !self.shared.running.load(Ordering::SeqCst) && *quit.borrow() != Quit::FlushStop {
                break;
            }
        }
        let _ = self.writer.shutdown_bytes().await;
    }

    async fn flush(&mut self) -> Result<(), String> {
        self.flush_at = None;
        if self.buf.is_empty() {
            return Ok(());
        }
        let r = self.writer.write_all_bytes(&self.buf).await;
        self.buf.clear();
        r.map_err(|e| e.to_string())?;
        self.writer.flush_bytes().await.map_err(|e| e.to_string())
    }

    /// `sendSomePacketMsgs`: returns true if all channels are exhausted.
    async fn send_some(&mut self) -> Result<bool, String> {
        let cfg = &self.shared.config;
        self.shared
            .send_monitor
            .limit(self.shared.max_packet_msg_size, cfg.send_rate)
            .await;
        let mut total = 0;
        let mut exhausted = false;
        for _ in 0..NUM_BATCH_PACKET_MSGS {
            let Some(i) = self.select_channel() else {
                exhausted = true;
                break;
            };
            total += self.write_packet(i).await?;
        }
        if total > 0 {
            self.shared
                .send_monitor
                .update(total, self.shared.config.send_rate);
        }
        Ok(exhausted)
    }

    /// `isSendPending` for channel `i`: pulls the next queued message.
    fn is_send_pending(&mut self, i: usize) -> bool {
        if self.sending[i].is_some() {
            return true;
        }
        let c = &self.shared.channels[i];
        let next = lock(&c.queue).pop_front();
        match next {
            Some(m) => {
                c.space.add_permits(1);
                self.sending[i] = Some((m, 0));
                true
            }
            None => false,
        }
    }

    /// `selectChannelToGossipOn`.
    fn select_channel(&mut self) -> Option<usize> {
        let mut least = f32::MAX;
        let mut best = None;
        for i in 0..self.shared.channels.len() {
            if !self.is_send_pending(i) {
                continue;
            }
            let c = &self.shared.channels[i];
            #[allow(clippy::cast_precision_loss)]
            let ratio = c.recently_sent.load(Ordering::SeqCst) as f32 / c.desc.priority as f32;
            if ratio < least {
                least = ratio;
                best = Some(i);
            }
        }
        best
    }

    async fn write_packet(&mut self, i: usize) -> Result<usize, String> {
        let max = self.shared.config.max_packet_msg_payload_size;
        let c = &self.shared.channels[i];
        let Some((msg, off)) = self.sending[i].take() else {
            return Ok(0);
        };
        let rest = msg.len() - off;
        let packet = if rest <= max {
            c.queue_size.fetch_sub(1, Ordering::SeqCst);
            msg_packet(c.desc.id, true, msg[off..].to_vec())
        } else {
            let p = msg_packet(c.desc.id, false, msg[off..off + max].to_vec());
            self.sending[i] = Some((msg, off + max));
            p
        };
        let n = packet.len();
        self.buf.extend_from_slice(&packet);
        c.recently_sent
            .fetch_add(i64::try_from(n).unwrap_or(i64::MAX), Ordering::SeqCst);
        if self.buf.len() >= MIN_WRITE_BUFFER_SIZE {
            let r = self.writer.write_all_bytes(&self.buf).await;
            self.buf.clear();
            r.map_err(|e| e.to_string())?;
        }
        if self.flush_at.is_none() {
            self.flush_at = Some(Instant::now() + self.shared.config.flush_throttle);
        }
        Ok(n)
    }
}

struct RecvRoutine<R> {
    shared: Arc<Shared>,
    reader: R,
    recving: Vec<Vec<u8>>,
    out: mpsc::Sender<Received>,
    pong_req: mpsc::Sender<()>,
    pong_got: mpsc::Sender<()>,
}

impl<R: ByteRead> RecvRoutine<R> {
    async fn run(mut self) {
        let mut quit = self.shared.quit.subscribe();
        loop {
            if *quit.borrow_and_update() != Quit::Running {
                break;
            }
            tokio::select! {
                _ = quit.changed() => break,
                r = self.recv_one() => {
                    if let Err(e) = r {
                        if self.shared.running.load(Ordering::SeqCst) {
                            self.shared.stop_for_error(e);
                        }
                        break;
                    }
                }
            }
        }
    }

    async fn recv_one(&mut self) -> Result<(), String> {
        let max = self.shared.max_packet_msg_size;
        let rate = self.shared.config.recv_rate;
        self.shared.recv_monitor.limit(max, rate).await;
        let (body, n) = read_delimited(&mut self.reader, max).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                "EOF".to_string()
            } else {
                e.to_string()
            }
        })?;
        self.shared.recv_monitor.update(n, rate);
        let packet = celestia_core_proto::gogo_decode::<pb::Packet>(body.as_slice())
            .map_err(|e| e.to_string())?;
        match packet.sum {
            Some(pb::packet::Sum::PacketPing(_)) => {
                self.shared.pings_received.fetch_add(1, Ordering::Relaxed);
                let _ = self.pong_req.try_send(());
            }
            Some(pb::packet::Sum::PacketPong(_)) => {
                self.shared.pongs_received.fetch_add(1, Ordering::Relaxed);
                let _ = self.pong_got.try_send(());
            }
            Some(pb::packet::Sum::PacketMsg(m)) => {
                let idx = u8::try_from(m.channel_id)
                    .ok()
                    .and_then(|id| self.shared.index.get(&id).copied());
                let Some(i) = idx else {
                    return Err(format!("unknown channel {:X}", m.channel_id));
                };
                let desc = &self.shared.channels[i].desc;
                let received = self.recving[i].len() + m.data.len();
                if desc.recv_message_capacity < received {
                    return Err(format!(
                        "received message exceeds available capacity: {} < {received}",
                        desc.recv_message_capacity
                    ));
                }
                self.recving[i].extend_from_slice(&m.data);
                if m.eof {
                    let msg = std::mem::replace(
                        &mut self.recving[i],
                        Vec::with_capacity(desc.recv_buffer_capacity),
                    );
                    if self.out.send((desc.id, msg)).await.is_err() {
                        return Err("receiver dropped".into());
                    }
                }
            }
            None => return Err("unknown message type <nil>".into()),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret_connection::Plain;

    fn cfg() -> MConnConfig {
        MConnConfig {
            flush_throttle: Duration::from_millis(5),
            ..MConnConfig::default()
        }
    }

    #[tokio::test]
    async fn roundtrip_and_priorities() {
        let (a, b) = tokio::io::duplex(1 << 20);
        let (ar, aw) = tokio::io::split(a);
        let (br, bw) = tokio::io::split(b);
        let descs = vec![
            ChannelDescriptor {
                send_queue_capacity: 10,
                ..ChannelDescriptor::new(1, 1)
            },
            ChannelDescriptor {
                send_queue_capacity: 10,
                ..ChannelDescriptor::new(2, 5)
            },
        ];
        let (ca, _ra) = MConnection::start(Plain(ar), Plain(aw), &descs, cfg()).unwrap();
        let (_cb, mut rb) = MConnection::start(Plain(br), Plain(bw), &descs, cfg()).unwrap();
        let big: Vec<u8> = (0..5000u32).map(|i| (i % 13) as u8).collect();
        assert!(ca.send(1, big.clone()).await);
        assert!(ca.send(2, b"hi".to_vec()).await);
        let mut got = HashMap::new();
        for _ in 0..2 {
            let (ch, m) = rb.recv().await.unwrap();
            got.insert(ch, m);
        }
        assert_eq!(got[&1], big);
        assert_eq!(got[&2], b"hi");
        ca.stop();
        assert_eq!(ca.closed().await, ConnState::Stopped);
    }

    #[tokio::test]
    async fn unknown_channel_errors() {
        let (a, b) = tokio::io::duplex(1 << 16);
        let (br, bw) = tokio::io::split(b);
        let descs = vec![ChannelDescriptor::new(1, 1)];
        let (cb, mut rb) = MConnection::start(Plain(br), Plain(bw), &descs, cfg()).unwrap();
        let (_ar, aw) = tokio::io::split(a);
        let mut aw = Plain(aw);
        aw.write_all_bytes(&msg_packet(9, true, vec![1]))
            .await
            .unwrap();
        assert!(rb.recv().await.is_none());
        assert!(
            matches!(cb.closed().await, ConnState::Errored(e) if e.contains("unknown channel"))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn pong_timeout_fails_connection() {
        let (a, _b) = tokio::io::duplex(1 << 16);
        let (ar, aw) = tokio::io::split(a);
        let c = MConnConfig {
            ping_interval: Duration::from_secs(2),
            pong_timeout: Duration::from_secs(1),
            ..cfg()
        };
        let (ca, _r) =
            MConnection::start(Plain(ar), Plain(aw), &[ChannelDescriptor::new(1, 1)], c).unwrap();
        assert_eq!(ca.closed().await, ConnState::Errored("pong timeout".into()));
    }
}
