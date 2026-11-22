//! Observe the lifetime of each transport connection and its substreams.
use anyhow::{Result, ensure};
use futures::{AsyncRead, AsyncWrite};
use libp2p::{
    PeerId,
    core::{
        ConnectedPoint,
        muxing::{StreamMuxer, StreamMuxerEvent},
    },
};
use serde_json::{Value, json};
use std::{
    io,
    pin::Pin,
    sync::{Arc, Mutex, Weak},
    task::{Context, Poll},
};

#[derive(Clone, Copy)]
pub(super) enum Transport {
    Tcp,
    Quic,
}

impl Transport {
    fn info(self) -> Value {
        let (transport, security, muxer) = match self {
            Self::Tcp => ("tcp", "/noise", "/yamux/1.0.0"),
            Self::Quic => ("quic-v1", "", ""),
        };
        json!({"Transport":transport,"Security":security,"StreamMultiplexer":muxer,"UsedEarlyMuxerNegotiation":false})
    }
}

struct Connection {
    transport: Transport,
    peer: PeerId,
    inbound: bool,
    opened: String,
    streams: Mutex<Vec<Weak<super::resources::Stream>>>,
    traffic: super::bandwidth::Traffic,
}

impl Connection {
    fn live_streams(&self) -> Vec<Arc<super::resources::Stream>> {
        self.streams
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter_map(Weak::upgrade)
            .collect()
    }

    fn open_stream(&self, outbound: bool) -> super::bandwidth::StreamTraffic {
        let live = Arc::new(super::resources::Stream::new(outbound));
        let mut streams = self
            .streams
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        streams.retain(|s| s.strong_count() > 0);
        streams.push(Arc::downgrade(&live));
        self.traffic.stream(live)
    }
}

#[derive(Clone, Default)]
pub(super) struct Connections {
    active: Arc<Mutex<Vec<Weak<Connection>>>>,
    pub(super) bandwidth: super::bandwidth::Bandwidth,
}

impl Connections {
    pub(super) fn observe<M>(
        &self,
        peer: PeerId,
        point: &ConnectedPoint,
        inner: M,
        transport: Transport,
    ) -> ObservedMuxer<M> {
        let now = light::verifier::now();
        let conn = Arc::new(Connection {
            transport,
            peer,
            inbound: point.is_listener(),
            opened: crate::util::rfc3339_nano(now.seconds, now.nanos.unsigned_abs()),
            streams: Mutex::default(),
            traffic: self.bandwidth.for_connection(peer),
        });
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        active.retain(|c| c.strong_count() > 0);
        active.push(Arc::downgrade(&conn));
        ObservedMuxer { inner, conn }
    }

    pub(super) fn resources(&self) -> Value {
        let mut snapshot = super::resources::Snapshot::default();
        let active = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for conn in active.iter().filter_map(Weak::upgrade) {
            snapshot.connection(
                conn.peer,
                !conn.inbound,
                matches!(conn.transport, Transport::Tcp),
            );
            for stream in conn.live_streams() {
                snapshot.stream(conn.peer, &stream);
            }
        }
        snapshot.json()
    }

    pub(super) fn state(&self, peer: PeerId) -> Result<Value> {
        let active = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let states: Vec<_> = active
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|c| c.peer == peer)
            .map(|c| {
                json!({
                    "Info":c.transport.info(),
                    "Direction":if c.inbound {1} else {2}, "Opened":c.opened,"Limited":false,
                    "NumStreams":c.live_streams().len()
                })
            })
            .collect();
        ensure!(!states.is_empty(), "no connections to peer {peer}");
        Ok(Value::Array(states))
    }
}

pub(super) struct ObservedMuxer<M> {
    inner: M,
    conn: Arc<Connection>,
}
pub(super) struct ObservedStream<S> {
    inner: S,
    traffic: super::bandwidth::StreamTraffic,
}

impl<M: StreamMuxer + Unpin> StreamMuxer for ObservedMuxer<M>
where
    M::Substream: Unpin,
{
    type Substream = ObservedStream<M::Substream>;
    type Error = M::Error;
    fn poll_inbound(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>> {
        Pin::new(&mut self.inner)
            .poll_inbound(cx)
            .map_ok(|inner| ObservedStream {
                inner,
                traffic: self.conn.open_stream(false),
            })
    }
    fn poll_outbound(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>> {
        Pin::new(&mut self.inner)
            .poll_outbound(cx)
            .map_ok(|inner| ObservedStream {
                inner,
                traffic: self.conn.open_stream(true),
            })
    }
    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.inner).poll_close(cx)
    }
    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<StreamMuxerEvent, Self::Error>> {
        Pin::new(&mut self.inner).poll(cx)
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for ObservedStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_read(cx, buf).map_ok(|n| {
            self.traffic.record(false, &buf[..n]);
            n
        })
    }
    fn poll_read_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &mut [io::IoSliceMut<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner)
            .poll_read_vectored(cx, bufs)
            .map_ok(|n| {
                self.traffic
                    .record_vectored(false, bufs.iter().map(|buf| &buf[..]), n);
                n
            })
    }
}
impl<S: AsyncWrite + Unpin> AsyncWrite for ObservedStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf).map_ok(|n| {
            self.traffic.record(true, &buf[..n]);
            n
        })
    }
    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner)
            .poll_write_vectored(cx, bufs)
            .map_ok(|n| {
                self.traffic
                    .record_vectored(true, bufs.iter().map(|buf| &buf[..]), n);
                n
            })
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_close(cx)
    }
}
