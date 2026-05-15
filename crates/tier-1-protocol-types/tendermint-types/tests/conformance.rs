//! Conformance tests for tendermint-types against committed fixtures.

#![allow(clippy::expect_used, clippy::panic)]

use serde::Deserialize;
use std::path::PathBuf;
use tendermint_types::Header;

fn workspace_root() -> PathBuf {
    let mut dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for _ in 0..10 {
        let manifest = dir.join("Cargo.toml");
        if manifest.exists() {
            let body =
                std::fs::read_to_string(&manifest).expect("read candidate workspace Cargo.toml");
            if body.contains("[workspace]") {
                return dir;
            }
        }
        if !dir.pop() {
            break;
        }
    }
    panic!("no workspace Cargo.toml with [workspace] found above CARGO_MANIFEST_DIR")
}

fn load_fixture<T>(relative_path: &str) -> T
where
    T: for<'de> Deserialize<'de>,
{
    let path = workspace_root()
        .join("conformance/fixtures")
        .join(relative_path);
    let body =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&body).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

#[derive(Deserialize)]
struct HeaderFixture {
    cases: Vec<HeaderCase>,
}

#[derive(Deserialize)]
struct HeaderCase {
    label: String,
    header_proto_hex: String,
    header_hash_hex: String,
}

#[test]
fn conformance_headers() {
    let fixture: HeaderFixture = load_fixture("tendermint-types/headers.json");

    for case in &fixture.cases {
        let encoded = hex::decode(&case.header_proto_hex).expect("valid header protobuf hex");
        let header = Header::decode(&encoded)
            .unwrap_or_else(|e| panic!("decode header for '{}': {e}", case.label));
        assert_eq!(
            hex::encode(header.encode_to_vec().expect("encode header")),
            case.header_proto_hex,
            "header protobuf round-trip mismatch for '{}'",
            case.label
        );
        assert_eq!(
            header.hash().map(hex::encode).unwrap_or_default(),
            case.header_hash_hex,
            "header hash mismatch for '{}'",
            case.label
        );
    }
}
