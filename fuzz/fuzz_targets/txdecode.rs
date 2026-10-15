//! Block tx decoding at both app versions: the full decoder chain (BlobTx,
//! IndexWrapper, ADR-027, unknownproto, gogoproto, Any unpacking) and the
//! wrapper parsers on their own.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = txdecode::decode(data);
    let _ = txdecode::decode_v9(data);
    let _ = txdecode::unmarshal_blob_tx(data);
    let _ = txdecode::unmarshal_blob_tx_v9(data);
    let _ = txdecode::unmarshal_index_wrapper(data);
    let _ = txdecode::has_duplicate_tx_raw_field(data);
});
