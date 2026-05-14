//! Conformance tests for the hash crate against committed fixture files.
//!
//! Fixture files live at `conformance/fixtures/hash/` (workspace root).
//! The path is derived by walking up from `CARGO_MANIFEST_DIR`.

// Tests parse fixtures and use hex / serde utilities freely.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use hash::{ripemd160, sha256, sha512, tmhash_len_prefixed, Sha256Hasher};
use serde::Deserialize;
use std::path::{Path, PathBuf};

// --------------------------------------------------------------------------
// Fixture path helper
// --------------------------------------------------------------------------

/// Return the workspace root by walking up from `CARGO_MANIFEST_DIR`
/// until we find a `Cargo.toml` that contains `[workspace]`.
fn workspace_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    // crates/tier-0-primitives/hash -> go up 3 levels to workspace root
    manifest
        .parent() // tier-0-primitives
        .and_then(|p| p.parent()) // crates
        .and_then(|p| p.parent()) // workspace root
        .expect("CARGO_MANIFEST_DIR must be at least 3 levels deep")
        .to_path_buf()
}

fn fixture_path(name: &str) -> PathBuf {
    workspace_root()
        .join("conformance")
        .join("fixtures")
        .join("hash")
        .join(name)
}

// --------------------------------------------------------------------------
// Fixture types
// --------------------------------------------------------------------------

#[derive(Deserialize)]
struct HashAllCase {
    label: String,
    input_hex: String,
    sha256: String,
    sha512: String,
    ripemd160: String,
    tmhash: String,
}

#[derive(Deserialize)]
struct EmptyFixture {
    cases: Vec<HashAllCase>,
}

#[derive(Deserialize)]
struct ShortVectorsFixture {
    cases: Vec<HashAllCase>,
}

#[derive(Deserialize)]
struct StreamingCase {
    label: String,
    input_hex: String,
    chunks_hex: Vec<String>,
    sha256: String,
}

#[derive(Deserialize)]
struct StreamingFixture {
    cases: Vec<StreamingCase>,
}

#[derive(Deserialize)]
struct TmhashCase {
    label: String,
    input_hex: Option<String>,
    fill_byte: Option<u8>,
    fill_len: Option<usize>,
    input_len: usize,
    tmhash: String,
}

#[derive(Deserialize)]
struct TmhashFixture {
    cases: Vec<TmhashCase>,
}

// --------------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------------

fn decode_hex(s: &str) -> Vec<u8> {
    hex::decode(s).expect("valid hex in fixture")
}

/// Reconstruct the input bytes from a `TmhashCase`.
fn tmhash_case_input(case: &TmhashCase) -> Vec<u8> {
    if let Some(ref h) = case.input_hex {
        let bytes = decode_hex(h);
        assert_eq!(
            bytes.len(),
            case.input_len,
            "fixture input_hex length mismatch for '{}'",
            case.label
        );
        bytes
    } else {
        let fill = case.fill_byte.expect("fill_byte must be present when input_hex is absent");
        let len = case.fill_len.expect("fill_len must be present when input_hex is absent");
        assert_eq!(len, case.input_len);
        vec![fill; len]
    }
}

// --------------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------------

#[test]
fn conformance_empty() {
    let path = fixture_path("empty.json");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let fixture: EmptyFixture = serde_json::from_str(&content).expect("parse empty.json");

    for case in &fixture.cases {
        let input = decode_hex(&case.input_hex);

        assert_eq!(
            hex::encode(sha256(&input)),
            case.sha256,
            "sha256 mismatch for '{}'",
            case.label
        );
        assert_eq!(
            hex::encode(sha512(&input)),
            case.sha512,
            "sha512 mismatch for '{}'",
            case.label
        );
        assert_eq!(
            hex::encode(ripemd160(&input)),
            case.ripemd160,
            "ripemd160 mismatch for '{}'",
            case.label
        );
        assert_eq!(
            hex::encode(tmhash_len_prefixed(&input)),
            case.tmhash,
            "tmhash mismatch for '{}'",
            case.label
        );
    }
}

#[test]
fn conformance_short_vectors() {
    let path = fixture_path("short-vectors.json");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let fixture: ShortVectorsFixture =
        serde_json::from_str(&content).expect("parse short-vectors.json");

    for case in &fixture.cases {
        let input = decode_hex(&case.input_hex);

        assert_eq!(
            hex::encode(sha256(&input)),
            case.sha256,
            "sha256 mismatch for '{}'",
            case.label
        );
        assert_eq!(
            hex::encode(sha512(&input)),
            case.sha512,
            "sha512 mismatch for '{}'",
            case.label
        );
        assert_eq!(
            hex::encode(ripemd160(&input)),
            case.ripemd160,
            "ripemd160 mismatch for '{}'",
            case.label
        );
        assert_eq!(
            hex::encode(tmhash_len_prefixed(&input)),
            case.tmhash,
            "tmhash mismatch for '{}'",
            case.label
        );
    }
}

#[test]
fn conformance_streaming() {
    let path = fixture_path("streaming.json");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let fixture: StreamingFixture = serde_json::from_str(&content).expect("parse streaming.json");

    for case in &fixture.cases {
        // 1. One-shot sha256 of the full input must match.
        let input = decode_hex(&case.input_hex);
        assert_eq!(
            hex::encode(sha256(&input)),
            case.sha256,
            "sha256(full input) mismatch for '{}'",
            case.label
        );

        // 2. Streaming Sha256Hasher fed chunk-by-chunk must also match.
        let mut hasher = Sha256Hasher::new();
        for chunk_hex in &case.chunks_hex {
            let chunk = decode_hex(chunk_hex);
            hasher.update(&chunk);
        }
        assert_eq!(
            hex::encode(hasher.finalize()),
            case.sha256,
            "streaming sha256 mismatch for '{}'",
            case.label
        );
    }
}

#[test]
fn conformance_tmhash() {
    let path = fixture_path("tmhash.json");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let fixture: TmhashFixture = serde_json::from_str(&content).expect("parse tmhash.json");

    for case in &fixture.cases {
        let input = tmhash_case_input(case);
        assert_eq!(
            hex::encode(tmhash_len_prefixed(&input)),
            case.tmhash,
            "tmhash mismatch for '{}' (len {})",
            case.label,
            case.input_len
        );
    }
}
