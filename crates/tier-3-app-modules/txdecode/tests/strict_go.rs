//! Acceptance fixtures from the proposed Go SDK StrictTxDecoder.
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixtures {
    sdk_commit: String,
    vectors: Vec<Vector>,
}

#[derive(Deserialize)]
struct Vector {
    name: String,
    raw_hex: String,
    strict_ok: bool,
    legacy_ok: bool,
}

#[test]
fn strict_and_historical_decoders_match_go() {
    let fixtures: Fixtures = serde_json::from_str(include_str!(
        "../../../../conformance/fixtures/strict-txdecode.json"
    ))
    .unwrap();
    assert_eq!(
        fixtures.sdk_commit,
        "aeb605c12be19efae4cf614000c7dbb5fc15785a"
    );
    assert!(fixtures.vectors.len() >= 40);
    for v in fixtures.vectors {
        let raw = hex::decode(&v.raw_hex).unwrap();
        for (version, expected) in [(10, v.legacy_ok), (11, v.legacy_ok), (12, v.strict_ok)] {
            let result = txdecode::decode_for(version, &raw);
            assert_eq!(result.is_ok(), expected, "{} at version {version}", v.name);
            if version == 12
                && let Err(err) = result
            {
                assert_eq!((err.codespace, err.code), ("sdk", 2), "{}", v.name);
            }
        }
    }
}
