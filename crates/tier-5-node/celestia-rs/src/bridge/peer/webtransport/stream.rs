//! Adapt the session's flow-controlled, message-sized reads and writes to libp2p I/O.
use futures::{AsyncRead, AsyncWrite, FutureExt, StreamExt, TryStreamExt, future::BoxFuture};
use std::{
    io,
    pin::Pin,
    task::{Context, Poll, ready},
};
use tobi_webtransport::stream::WtBidirectionalStream;
use tokio::io::AsyncReadExt as _;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

pub(super) struct Stream {
    read: Pin<Box<dyn AsyncRead + Send>>,
    write: Compat<tokio::io::WriteHalf<tokio::io::SimplexStream>>,
    finished: BoxFuture<'static, io::Result<()>>,
    closed: bool,
}

impl Stream {
    pub(super) fn new(stream: WtBidirectionalStream) -> Self {
        let (mut send, recv) = stream.split();
        let read = futures::stream::try_unfold(recv, |mut recv| async move {
            Ok::<_, io::Error>(
                recv.recv()
                    .await
                    .map_err(io::Error::other)?
                    .map(|bytes| (bytes, recv)),
            )
        })
        .boxed()
        .into_async_read();
        // One bounded writer per stream. On drop the worker drains accepted bytes
        // before sending FIN. Closing explicitly also propagates the peer's error.
        let (mut input, output) = tokio::io::simplex(16 * 1024);
        let writer = tokio::spawn(async move {
            let mut buf = vec![0; 16 * 1024];
            loop {
                let n = input.read(&mut buf).await?;
                if n == 0 {
                    return send.finish().await.map_err(io::Error::other);
                }
                send.send(&buf[..n]).await.map_err(io::Error::other)?;
            }
        });
        Self {
            read: Box::pin(read),
            write: output.compat_write(),
            finished: async move { writer.await.map_err(io::Error::other)? }.boxed(),
            closed: false,
        }
    }
}

impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        self.read.as_mut().poll_read(cx, buf)
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.write).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.write).poll_flush(cx)
    }
    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.closed {
            return Poll::Ready(Ok(()));
        }
        ready!(Pin::new(&mut self.write).poll_close(cx))?;
        let result = ready!(self.finished.as_mut().poll(cx));
        self.closed = true;
        Poll::Ready(result)
    }
}
