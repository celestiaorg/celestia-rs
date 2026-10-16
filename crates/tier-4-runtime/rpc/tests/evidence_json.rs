//! Evidence in `/block` responses: the cmtjson rendering of duplicate vote
//! and light client attack evidence matches celestia-core v0.42.3
//! (`conformance/evidencevectors`, field `evidence_json`).

#![allow(clippy::unwrap_used, clippy::panic)]

use prost::Message as _;

#[test]
fn go_evidence_json() {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../conformance/fixtures/evidence/vectors.json"
    ))
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let mut n = 0;
    for c in v["cases"].as_array().unwrap() {
        let Some(want) = c["evidence_json"].as_str() else {
            continue;
        };
        let label = c["label"].as_str().unwrap();
        let bz = hex::decode(c["evidence"].as_str().unwrap()).unwrap();
        let p = ctypes::proto::types::Evidence::decode(bz.as_slice()).unwrap();
        let ev = ctypes::Evidence::from_proto(&p).unwrap_or_else(|e| panic!("{label}: {e}"));
        assert_eq!(rpc::encode::evidence(&ev).encode(), want, "{label}");
        n += 1;
    }
    assert!(n >= 30, "{n}");
}
