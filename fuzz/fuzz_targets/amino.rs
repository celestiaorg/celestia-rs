//! The legacy amino `StdTx` codec (`TxDecodeAmino` / `TxEncodeAmino`, reachable
//! through ABCI `Query` and the gRPC/REST tx service).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = legacy_amino::tx_decode_amino(data);
    let _ = legacy_amino::tx_encode_amino(data);
});
