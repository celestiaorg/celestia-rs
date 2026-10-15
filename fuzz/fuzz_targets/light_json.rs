//! Light-client / state-sync RPC client decoding of an untrusted server's
//! response: HTTP framing (chunked bodies) and every CometBFT JSON decoder.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = light::http::fuzzing::http_body(data);
    light::http::fuzzing::decode_json(data);
});
