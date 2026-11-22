use super::stream::Stream;
use futures::{FutureExt, future::BoxFuture};
use libp2p::core::muxing::{StreamMuxer, StreamMuxerEvent};
use std::{
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, ready},
};
use tobi_webtransport::{session::Session, stream::WtBidirectionalStream};

pub(super) struct Mux {
    session: Arc<Session>,
    incoming: tokio::sync::mpsc::Receiver<WtBidirectionalStream>,
    opening: Option<BoxFuture<'static, io::Result<Stream>>>,
    closing: Option<BoxFuture<'static, io::Result<()>>>,
    finished: bool,
    closed: BoxFuture<'static, io::Error>,
}
impl Mux {
    pub(super) fn new(session: Session) -> Self {
        let (session, rx) = session.split_receivers();
        let parts = rx.into_parts();
        let mut closed = parts.closed;
        Self {
            session: Arc::new(session),
            incoming: parts.bidirectional,
            opening: None,
            closing: None,
            finished: false,
            closed: async move {
                match closed.wait_closed().await {
                    Ok(_) => io::ErrorKind::ConnectionAborted.into(),
                    Err(error) => io::Error::other(error),
                }
            }
            .boxed(),
        }
    }
}
impl StreamMuxer for Mux {
    type Substream = Stream;
    type Error = io::Error;
    fn poll_inbound(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<Stream>> {
        self.incoming.poll_recv(cx).map(|stream| {
            stream
                .map(Stream::new)
                .ok_or_else(|| io::ErrorKind::ConnectionAborted.into())
        })
    }
    fn poll_outbound(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<Stream>> {
        let this = self.as_mut().get_mut();
        let opening = this.opening.get_or_insert_with(|| {
            let session = this.session.clone();
            async move {
                session
                    .open_bidirectional_stream()
                    .await
                    .map(Stream::new)
                    .map_err(io::Error::other)
            }
            .boxed()
        });
        let result = ready!(opening.as_mut().poll(cx));
        this.opening = None;
        Poll::Ready(result)
    }
    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.as_mut().get_mut();
        if this.finished {
            return Poll::Ready(Ok(()));
        }
        let result = ready!(
            this.closing
                .get_or_insert_with(|| {
                    let session = this.session.clone();
                    async move { session.close(0, "").await.map_err(io::Error::other) }.boxed()
                })
                .as_mut()
                .poll(cx)
        );
        this.finished = true;
        Poll::Ready(result)
    }
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<StreamMuxerEvent>> {
        self.closed.as_mut().poll(cx).map(Err)
    }
}
