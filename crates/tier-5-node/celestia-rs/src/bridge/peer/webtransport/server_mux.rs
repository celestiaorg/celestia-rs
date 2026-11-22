//! Drive one accepted WebTransport session and expose its bidirectional streams to libp2p.
use super::stream::Stream;
use futures::{
    FutureExt, StreamExt,
    future::{BoxFuture, Either},
    stream::FuturesUnordered,
};
use libp2p::core::muxing::{StreamMuxer, StreamMuxerEvent};
use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll, ready},
};
use tobi_webtransport::server::WtSession;
use tokio::io::AsyncReadExt;
use tokio::sync::{mpsc, oneshot};
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt};

type Inbound = Compat<tokio::io::Join<tobi_quic::ReceiveStream, tobi_quic::SendStream>>;
pub(super) type Substream = Either<Stream, Inbound>;

/// Consume exactly the two QUIC variable-length integers in the WebTransport
/// stream header. Bytes after the session ID belong to the libp2p protocol.
pub(super) async fn incoming(
    stream: tobi_quic::BidirectionalStream,
    session_id: u64,
) -> io::Result<Inbound> {
    use shiguredo_http3::webtransport::stream::{StreamHeader, StreamHeaderDecodeError};
    let (send, mut recv) = stream.split();
    let mut header = [0u8; 16];
    for length in 1..=header.len() {
        recv.read_exact(&mut header[length - 1..length]).await?;
        match StreamHeader::decode_bidirectional_checked(&header[..length]) {
            Ok((header, _)) if header.session_id() == session_id => {
                return Ok(tokio::io::join(recv, send).compat());
            }
            Err(StreamHeaderDecodeError::BufferTooShort) => {}
            _ => break,
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid WebTransport stream header or session ID",
    ))
}

pub(super) struct Mux {
    // Keep the receive half until disconnect so closing the Noise stream does not
    // race the browser's FIN with a QUIC STOP_SENDING.
    _auth: libp2p::noise::Output<Inbound>,
    incoming: mpsc::Receiver<Inbound>,
    open: Option<mpsc::Sender<oneshot::Sender<io::Result<Stream>>>>,
    opening: Option<BoxFuture<'static, io::Result<Stream>>>,
    driver: tokio::task::JoinHandle<io::Result<()>>,
    finished: bool,
}

impl Mux {
    pub(super) fn new(mut session: WtSession, auth: libp2p::noise::Output<Inbound>) -> Self {
        let (in_tx, incoming) = mpsc::channel(32);
        let (open, mut requests) = mpsc::channel::<oneshot::Sender<io::Result<Stream>>>(1);
        let connection = session.connection();
        let id = session.session_id();
        let driver = tokio::spawn(async move {
            let mut headers = FuturesUnordered::new();
            loop {
                tokio::select! {
                    request = requests.recv() => {
                        let Some(reply) = request else {
                            return session.close(0, "").await.map_err(io::Error::other);
                        };
                        let result = session.open_bi_stream().await.map(Stream::incoming).map_err(io::Error::other);
                        let _ = reply.send(result);
                    }
                    accepted = connection.accept_bidirectional_stream(), if headers.len()<32 => {
                        let stream=accepted.map_err(io::Error::other)?;
                        headers.push(self::incoming(stream,id));
                    }
                    Some(parsed) = headers.next(), if !headers.is_empty() => {
                        if let Ok(stream)=parsed { let _=in_tx.try_send(stream); }
                    }
                    result = session.wait_closed() => return result.map(|_|()).map_err(io::Error::other),

                }
            }
        });
        Self {
            _auth: auth,
            incoming,
            open: Some(open),
            opening: None,
            driver,
            finished: false,
        }
    }
    fn poll_driver(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.finished {
            return Poll::Ready(Ok(()));
        }
        let result = ready!(Pin::new(&mut self.driver).poll(cx));
        self.finished = true;
        Poll::Ready(result.map_err(io::Error::other)?)
    }
}
impl Drop for Mux {
    fn drop(&mut self) {
        self.driver.abort();
    }
}
impl StreamMuxer for Mux {
    type Substream = Substream;
    type Error = io::Error;
    fn poll_inbound(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<Substream>> {
        self.incoming.poll_recv(cx).map(|stream| {
            stream
                .map(Either::Right)
                .ok_or_else(|| io::ErrorKind::ConnectionAborted.into())
        })
    }
    fn poll_outbound(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<Substream>> {
        let this = self.as_mut().get_mut();
        let opening = this.opening.get_or_insert_with(|| {
            let open = this.open.clone();
            async move {
                let tx = open.ok_or(io::ErrorKind::BrokenPipe)?;
                let (reply, rx) = oneshot::channel();
                tx.send(reply)
                    .await
                    .map_err(|_| io::ErrorKind::BrokenPipe)?;
                rx.await.map_err(io::Error::other)?
            }
            .boxed()
        });
        let result = ready!(opening.as_mut().poll(cx));
        this.opening = None;
        Poll::Ready(result.map(Either::Left))
    }
    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.opening = None;
        self.open = None;
        self.poll_driver(cx)
    }
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<StreamMuxerEvent>> {
        ready!(self.poll_driver(cx))?;
        Poll::Ready(Err(io::ErrorKind::ConnectionAborted.into()))
    }
}
