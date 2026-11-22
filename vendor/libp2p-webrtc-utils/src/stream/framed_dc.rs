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

use asynchronous_codec::Framed;
use futures::{AsyncRead, AsyncWrite};

use crate::{
    proto::Message,
    stream::{MAX_MSG_LEN, VARINT_LEN},
};

pub(crate) type FramedDc<T> = Framed<T, prost_codec::Codec<Message>>;
pub(crate) fn new<T>(inner: T) -> FramedDc<T>
where
    T: AsyncRead + AsyncWrite,
{
    let mut framed = Framed::new(inner, codec());
    // Flush each encoded message before accepting another. A small control or
    // SHREX frame followed by a full data frame can otherwise exceed the SCTP
    // message limit even when each individual frame fits.
    framed.set_send_high_water_mark(1);
    framed
}

pub(crate) fn codec() -> prost_codec::Codec<Message, Message> {
    prost_codec::Codec::new(MAX_MSG_LEN - VARINT_LEN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{SinkExt, executor::block_on, io::AllowStdIo};
    use std::io::{self, Read, Write};

    struct DataChannel;

    impl Read for DataChannel {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Ok(0)
        }
    }

    impl Write for DataChannel {
        fn write(&mut self, packet: &[u8]) -> io::Result<usize> {
            assert!(packet.len() <= MAX_MSG_LEN);
            Ok(packet.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn bounds_each_sctp_message_when_a_square_is_written_in_shares() {
        block_on(async {
            let mut channel = new(AllowStdIo::new(DataChannel));
            for len in std::iter::once(3).chain(std::iter::repeat_n(512, 64)) {
                channel
                    .feed(Message {
                        flag: None,
                        message: Some(vec![0; len]),
                    })
                    .await
                    .unwrap();
            }
            channel.flush().await.unwrap();
        });
    }
}
