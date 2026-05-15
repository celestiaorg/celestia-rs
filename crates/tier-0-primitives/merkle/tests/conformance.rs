//! Conformance tests for the merkle crate against committed fixture files.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use merkle::{
    Error, HASH_LENGTH, Hash, MAX_AUNTS, Proof, empty_hash, inner_hash, leaf_hash,
    proofs_from_byte_slices, root_from_byte_slices,
};
use serde::Deserialize;
use std::path::PathBuf;

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
struct RootsFixture {
    primitives: PrimitiveCases,
    root_cases: Vec<RootCase>,
}

#[derive(Deserialize)]
struct PrimitiveCases {
    empty_hash: String,
    empty_leaf_hash: String,
    sample_leaf_hex: String,
    sample_leaf_hash: String,
    inner_left_hash: String,
    inner_right_hash: String,
    inner_hash: String,
}

#[derive(Deserialize)]
struct RootCase {
    label: String,
    items_hex: Vec<String>,
    root_hex: String,
}

#[derive(Deserialize)]
struct ProofsFixture {
    cases: Vec<ProofListCase>,
}

#[derive(Deserialize)]
struct ProofListCase {
    label: String,
    items_hex: Vec<String>,
    root_hex: String,
    proofs: Vec<ProofCase>,
}

#[derive(Deserialize)]
struct ProofCase {
    total: u64,
    index: u64,
    leaf_hash: String,
    aunts: Vec<String>,
}

#[derive(Deserialize)]
struct MalformedFixture {
    cases: Vec<MalformedCase>,
}

#[derive(Deserialize)]
struct MalformedCase {
    label: String,
    root_hex: String,
    leaf_hex: String,
    proof: ProofCase,
    expected_error: String,
}

fn decode_bytes(hex: &str) -> Vec<u8> {
    hex::decode(hex).expect("valid fixture hex")
}

fn decode_hash(hex: &str) -> Hash {
    let bytes = decode_bytes(hex);
    assert_eq!(bytes.len(), HASH_LENGTH, "fixture hash length");
    bytes.try_into().expect("32-byte hash")
}

fn decode_items(items_hex: &[String]) -> Vec<Vec<u8>> {
    items_hex.iter().map(|item| decode_bytes(item)).collect()
}

fn decode_proof(case: &ProofCase) -> Proof {
    Proof {
        total: case.total,
        index: case.index,
        leaf_hash: decode_hash(&case.leaf_hash),
        aunts: case.aunts.iter().map(|aunt| decode_hash(aunt)).collect(),
    }
}

fn expected_error(name: &str) -> Error {
    match name {
        "invalid_total" => Error::InvalidTotal,
        "invalid_index" => Error::InvalidIndex,
        "too_many_aunts" => Error::TooManyAunts {
            max: MAX_AUNTS,
            actual: MAX_AUNTS + 1,
        },
        "unexpected_aunt_count" => Error::UnexpectedAuntCount,
        "leaf_hash_mismatch" => Error::LeafHashMismatch,
        "root_mismatch" => Error::RootMismatch,
        other => panic!("unknown expected error {other}"),
    }
}

#[test]
fn conformance_roots() {
    let fixture: RootsFixture = load_fixture("merkle/roots.json");

    assert_eq!(empty_hash(), decode_hash(&fixture.primitives.empty_hash));
    assert_eq!(
        leaf_hash(&[]),
        decode_hash(&fixture.primitives.empty_leaf_hash)
    );
    assert_eq!(
        leaf_hash(&decode_bytes(&fixture.primitives.sample_leaf_hex)),
        decode_hash(&fixture.primitives.sample_leaf_hash)
    );
    assert_eq!(
        inner_hash(
            &decode_hash(&fixture.primitives.inner_left_hash),
            &decode_hash(&fixture.primitives.inner_right_hash)
        ),
        decode_hash(&fixture.primitives.inner_hash)
    );

    for case in &fixture.root_cases {
        let items = decode_items(&case.items_hex);
        assert_eq!(
            root_from_byte_slices(&items),
            decode_hash(&case.root_hex),
            "root mismatch for '{}'",
            case.label
        );
    }
}

#[test]
fn conformance_proofs() {
    let fixture: ProofsFixture = load_fixture("merkle/proofs.json");

    for case in &fixture.cases {
        let items = decode_items(&case.items_hex);
        let expected_root = decode_hash(&case.root_hex);
        let (root, generated) = proofs_from_byte_slices(&items);
        assert_eq!(root, expected_root, "root mismatch for '{}'", case.label);
        assert_eq!(
            generated.len(),
            case.proofs.len(),
            "proof count mismatch for '{}'",
            case.label
        );

        for (index, proof_case) in case.proofs.iter().enumerate() {
            let expected = decode_proof(proof_case);
            assert_eq!(
                generated[index], expected,
                "proof mismatch for '{}' index {}",
                case.label, index
            );
            generated[index]
                .verify(&expected_root, &items[index])
                .unwrap_or_else(|e| {
                    panic!("proof verifies for '{}' index {index}: {e}", case.label)
                });
        }
    }
}

#[test]
fn conformance_malformed_proofs() {
    let fixture: MalformedFixture = load_fixture("merkle/malformed-proofs.json");

    for case in &fixture.cases {
        let proof = decode_proof(&case.proof);
        let root = decode_hash(&case.root_hex);
        let leaf = decode_bytes(&case.leaf_hex);
        let result = proof.verify(&root, &leaf);
        assert_eq!(
            result,
            Err(expected_error(&case.expected_error)),
            "malformed proof mismatch for '{}'",
            case.label
        );
    }
}
