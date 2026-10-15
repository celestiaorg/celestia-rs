//! CometBFT RPC request parsing: JSON-RPC bodies (single and batch), their
//! params for every argument type, URI query arguments and event queries.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rpc::params::ArgType;

const TYPES: &[ArgType] = &[
    ArgType::I64Ptr,
    ArgType::IntPtr,
    ArgType::I64,
    ArgType::U64,
    ArgType::Uint,
    ArgType::Bool,
    ArgType::Str,
    ArgType::HexBytes,
    ArgType::Bytes,
    ArgType::BytesList,
    ArgType::Evidence,
];

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(reqs) = rpc::server::parse_requests(s) {
        for r in reqs {
            let _ = rpc::params::json_args(&["a", "b", "c"], &TYPES[..3], &r.params);
            let _ = rpc::params::json_args(&["h", "p"], &TYPES[7..9], &r.params);
        }
    }
    for &ty in TYPES {
        let _ = rpc::params::uri_arg(ty, s);
        let _ = rpc::params::cmtjson_decode(s, ty);
    }
    let _ = rpc::query::parse(s);
    let _ = rpc::json::compact(s);
});
