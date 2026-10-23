//! ABCI socket framing: uvarint-length-prefixed protobuf `Request`s read from
//! a byte stream, then decoded.
#![no_main]

use libfuzzer_sys::fuzz_target;
use prost::Message as _;

fuzz_target!(|data: &[u8]| {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime");
    rt.block_on(async {
        let mut r = data;
        loop {
            let mut prefix = r;
            let Ok(Some(frame)) = abci_server::codec::read_frame(&mut r).await else {
                break;
            };
            // An accepted frame must agree with an independent u64 decoder.
            // Crash-only fuzzing cannot detect a silently truncated length.
            let len = prost::encoding::decode_varint(&mut prefix)
                .expect("accepted frame must have a valid u64 length");
            assert_eq!(frame.len() as u64, len);
            let _ = celestia_core_proto::abci::Request::decode(frame.as_slice());
            let _ = celestia_core_proto::abci::Response::decode(frame.as_slice());
        }
    });
});
