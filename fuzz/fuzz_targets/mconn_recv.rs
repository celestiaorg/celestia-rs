//! MConnection packet parsing: an authenticated peer's plaintext stream
//! (delimited `Packet`s: ping, pong, chunked channel messages) through the
//! receive routine, with the channels a celestia node registers.
#![no_main]

use libfuzzer_sys::fuzz_target;
use p2p::secret_connection::Plain;
use p2p::{ChannelDescriptor, MConnConfig, MConnection};

fuzz_target!(|data: &[u8]| {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime");
    let data = data.to_vec();
    rt.block_on(async move {
        let descs: Vec<ChannelDescriptor> = [0x00u8, 0x20, 0x21, 0x22, 0x23, 0x30, 0x38, 0x40, 0x50, 0x60, 0x61]
            .iter()
            .map(|&id| ChannelDescriptor::new(id, 1))
            .collect();
        let reader = Plain(std::io::Cursor::new(data));
        let writer = Plain(tokio::io::sink());
        let Ok((conn, mut rx)) = MConnection::start(reader, writer, &descs, MConnConfig::default())
        else {
            return;
        };
        while rx.recv().await.is_some() {}
        let _ = conn.closed().await;
        conn.stop();
    });
});
