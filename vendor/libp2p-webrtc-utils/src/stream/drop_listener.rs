// Copyright 2022 Parity Technologies (UK) Ltd.
//
// Permission is hereby granted, free of charge, to any person obtaining a
// copy of this software and associated documentation files (the "Software"),
// to deal in the Software without restriction, including without limitation
// the rights to use, copy, modify, merge, publish, distribute, sublicense,
// and/or sell copies of the Software, and to permit persons to whom the
// Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
// OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
// FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use crate::{
    proto::{Flag, Message},
    stream::framed_dc::FramedDc,
};
use futures::{
    AsyncRead, AsyncWrite, FutureExt, SinkExt, StreamExt, channel::oneshot, future::BoxFuture,
};

/// Finish the SCTP channel after the application releases its stream.
#[must_use]
pub struct DropListener(BoxFuture<'static, io::Result<()>>);

#[derive(Default)]
pub(super) struct GracefullyClosed {
    pub(super) peer_fin_acked: bool,
    pub(super) local_fin_acked: bool,
}

impl DropListener {
    pub(super) fn new<T: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
        mut stream: FramedDc<T>,
        receiver: oneshot::Receiver<GracefullyClosed>,
    ) -> Self {
        Self(
            async move {
                let Ok(mut closed) = receiver.await else {
                    stream.send(control(Flag::Reset)).await?;
                    return Ok(());
                };
                // A responder can finish its work before a peer half-closes. Keep
                // processing the peer's FIN after dropping the application stream.
                // Browser libp2p waits for this ACK before completing closeWrite.
                {
                    let finish = async {
                        while !(closed.peer_fin_acked && closed.local_fin_acked) {
                            let Some(message) = stream.next().await.transpose()? else {
                                break;
                            };
                            match message.flag.and_then(|value| Flag::try_from(value).ok()) {
                                Some(Flag::Fin) => {
                                    stream.send(control(Flag::FinAck)).await?;
                                    closed.peer_fin_acked = true;
                                }
                                Some(Flag::FinAck) => closed.local_fin_acked = true,
                                Some(Flag::Reset) => break,
                                _ => {}
                            }
                        }
                        Ok::<(), io::Error>(())
                    };
                    futures::pin_mut!(finish);
                    let deadline = futures_timer::Delay::new(Duration::from_secs(10));
                    if let futures::future::Either::Left((result, _)) =
                        futures::future::select(finish, deadline).await
                    {
                        result?;
                    }
                }
                stream.close().await?;
                Ok(())
            }
            .boxed(),
        )
    }
}

fn control(flag: Flag) -> Message {
    Message {
        flag: Some(flag as i32),
        message: None,
    }
}

impl Future for DropListener {
    type Output = io::Result<()>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.0.as_mut().poll(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{AsyncWriteExt, executor::block_on, io::AllowStdIo};
    use std::{
        io::{Cursor, Read, Write},
        sync::{Arc, Mutex},
    };

    #[derive(Clone, Default)]
    struct Channel {
        input: Arc<Mutex<Cursor<Vec<u8>>>>,
        output: Arc<Mutex<Vec<u8>>>,
    }
    impl Read for Channel {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.input.lock().unwrap().read(buf)
        }
    }
    impl Write for Channel {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Write::write(&mut *self.output.lock().unwrap(), buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn finishes_a_browser_half_close_after_the_handler_drops_its_stream() {
        block_on(async {
            let channel = Channel::default();
            let (mut stream, cleanup) = crate::Stream::new(AllowStdIo::new(channel.clone()));
            stream.close().await.unwrap();
            drop(stream);
            let fin = [2, 8, Flag::Fin as u8];
            let ack = [2, 8, Flag::FinAck as u8];
            *channel.input.lock().unwrap() = Cursor::new([fin, ack].concat());
            cleanup.await.unwrap();
            assert_eq!(*channel.output.lock().unwrap(), [fin, ack].concat());
        });
    }
}
