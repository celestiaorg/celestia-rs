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
        while let Ok(Some(frame)) = abci_server::codec::read_frame(&mut r).await {
            let _ = celestia_core_proto::abci::Request::decode(frame.as_slice());
            let _ = celestia_core_proto::abci::Response::decode(frame.as_slice());
        }
    });
});
