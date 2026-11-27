//! Classifying a node's implementation from its P2P `NodeInfo.version`.

use node_census::classify::{Implementation, classify};

#[test]
fn celestia_rs_suffix_is_rust() {
    assert_eq!(classify("0.38.17-celestia-rs.0.4.2"), Implementation::Rust);
    assert_eq!(
        classify("0.38.17-celestia-rs.1.0.0-rc.1"),
        Implementation::Rust
    );
}

#[test]
fn plain_comet_semver_is_go() {
    assert_eq!(classify("0.38.17"), Implementation::Go);
    assert_eq!(classify("0.34.29"), Implementation::Go);
    assert_eq!(classify("1.0.0-rc.1"), Implementation::Go);
}

#[test]
fn anything_else_is_unknown() {
    assert_eq!(classify(""), Implementation::Unknown);
    assert_eq!(classify("metadata-probe/1"), Implementation::Unknown);
    assert_eq!(classify("v"), Implementation::Unknown);
    // Seed-only crawlers report their own versions (mocha-5-seed: "0.0.1").
    assert_eq!(classify("0.0.1"), Implementation::Unknown);
    assert_eq!(classify("0.33.9"), Implementation::Unknown);
}

#[test]
fn implementation_serializes_lowercase() {
    assert_eq!(
        serde_json::to_string(&Implementation::Rust).unwrap(),
        "\"rust\""
    );
    assert_eq!(
        serde_json::to_string(&Implementation::Go).unwrap(),
        "\"go\""
    );
    assert_eq!(
        serde_json::to_string(&Implementation::Unknown).unwrap(),
        "\"unknown\""
    );
}
