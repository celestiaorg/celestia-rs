//! Observe the lifetime of each negotiated Yamux connection and its substreams.
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

struct Connection {
    peer: PeerId,
    inbound: bool,
    opened: String,
    streams: Arc<()>,
}

#[derive(Clone, Default)]
pub(super) struct Connections(Arc<Mutex<Vec<Weak<Connection>>>>);

impl Connections {
    pub(super) fn observe<M>(
        &self,
        peer: PeerId,
        point: &ConnectedPoint,
        inner: M,
    ) -> ObservedMuxer<M> {
        let now = light::verifier::now();
        let conn = Arc::new(Connection {
            peer,
            inbound: point.is_listener(),
            opened: crate::util::rfc3339_nano(now.seconds, now.nanos.unsigned_abs()),
            streams: Arc::new(()),
        });
        let mut active = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        active.retain(|c| c.strong_count() > 0);
        active.push(Arc::downgrade(&conn));
        ObservedMuxer { inner, conn }
    }

    pub(super) fn state(&self, peer: PeerId) -> Result<Value> {
        let active = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let states:Vec<_>=active.iter().filter_map(Weak::upgrade).filter(|c|c.peer==peer).map(|c|json!({
            "Info":{"StreamMultiplexer":"/yamux/1.0.0","Security":"/noise","Transport":"tcp","UsedEarlyMuxerNegotiation":false},
            "Direction":if c.inbound {1} else {2}, "Opened":c.opened,"Limited":false,
            "NumStreams":Arc::strong_count(&c.streams)-1
        })).collect();
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
    _live: Arc<()>,
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
                _live: self.conn.streams.clone(),
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
                _live: self.conn.streams.clone(),
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
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
    fn poll_read_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &mut [io::IoSliceMut<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_read_vectored(cx, bufs)
    }
}
impl<S: AsyncWrite + Unpin> AsyncWrite for ObservedStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_close(cx)
    }
}
