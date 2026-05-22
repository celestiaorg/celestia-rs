//! Go IAVL conformance fixture tests.

#![allow(clippy::expect_used, clippy::panic)]

use std::{fs, path::PathBuf};

use iavl::{ProofOpType, Tree};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct RootFixture {
    cases: Vec<RootCase>,
}

#[derive(Debug, Deserialize)]
struct RootCase {
    label: String,
    commits: Vec<CommitCase>,
}

#[derive(Debug, Deserialize)]
struct CommitCase {
    label: String,
    mutations: Vec<Mutation>,
    version: i64,
    size: usize,
    root_hash_hex: String,
}

#[derive(Debug, Deserialize)]
struct Mutation {
    action: String,
    key_hex: String,
    #[serde(default)]
    value_hex: String,
}

#[derive(Debug, Deserialize)]
struct IteratorFixture {
    cases: Vec<IteratorCase>,
}

#[derive(Debug, Deserialize)]
struct IteratorCase {
    label: String,
    entries: Vec<Kv>,
    ranges: Vec<RangeCase>,
    root_hash_hex: String,
    tree_size: usize,
    tree_version: i64,
}

#[derive(Debug, Deserialize)]
struct RangeCase {
    label: String,
    #[serde(default)]
    start_hex: String,
    #[serde(default)]
    end_hex: String,
    ascending: bool,
    entries: Vec<Kv>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct Kv {
    key_hex: String,
    value_hex: String,
}

#[derive(Debug, Deserialize)]
struct ProofFixture {
    cases: Vec<ProofCase>,
}

#[derive(Debug, Deserialize)]
struct ProofCase {
    label: String,
    proof_type: String,
    key_hex: String,
    #[serde(default)]
    value_hex: String,
    root_hash_hex: String,
    proof_proto_hex: String,
    verified: bool,
}

#[test]
fn root_fixtures_match_go_iavl() {
    let fixture: RootFixture = read_fixture("roots.json");

    for case in fixture.cases {
        let mut tree = Tree::new();
        for commit in case.commits {
            for mutation in &commit.mutations {
                apply_mutation(&mut tree, mutation);
            }
            let info = tree.save_version().unwrap_or_else(|err| {
                panic!("{}:{} save_version: {err}", case.label, commit.label)
            });
            assert_eq!(
                info.version, commit.version,
                "{}:{}",
                case.label, commit.label
            );
            assert_eq!(info.size, commit.size, "{}:{}", case.label, commit.label);
            assert_eq!(
                hex::encode(info.root_hash),
                commit.root_hash_hex,
                "{}:{}",
                case.label,
                commit.label
            );
            assert_eq!(
                hex::encode(tree.root_hash()),
                commit.root_hash_hex,
                "{}:{} root_hash",
                case.label,
                commit.label
            );
        }
    }
}

#[test]
fn iterator_fixtures_match_go_iavl() {
    let fixture: IteratorFixture = read_fixture("iterators.json");

    for case in fixture.cases {
        let mut tree = Tree::new();
        for entry in &case.entries {
            tree.set(decode_hex(&entry.key_hex), decode_hex(&entry.value_hex))
                .unwrap_or_else(|err| panic!("{} set {}: {err}", case.label, entry.key_hex));
        }
        let info = tree
            .save_version()
            .unwrap_or_else(|err| panic!("{} save_version: {err}", case.label));
        assert_eq!(info.version, case.tree_version, "{}", case.label);
        assert_eq!(info.size, case.tree_size, "{}", case.label);
        assert_eq!(
            hex::encode(info.root_hash),
            case.root_hash_hex,
            "{}",
            case.label
        );

        for range in case.ranges {
            let start = optional_hex(&range.start_hex);
            let end = optional_hex(&range.end_hex);
            let iter = if range.ascending {
                tree.iter(start.as_deref(), end.as_deref())
            } else {
                tree.reverse_iter(start.as_deref(), end.as_deref())
            };
            let entries = iter
                .map(|entry| Kv {
                    key_hex: hex::encode(entry.key),
                    value_hex: hex::encode(entry.value),
                })
                .collect::<Vec<_>>();
            assert_eq!(entries, range.entries, "{}:{}", case.label, range.label);
        }
    }
}

#[test]
fn proof_fixtures_match_go_iavl() {
    let fixture: ProofFixture = read_fixture("proofs.json");
    let mut tree = Tree::new();
    for key in ["0a", "11", "2e", "32", "50", "72", "99", "a1", "e4", "f7"] {
        let bytes = decode_hex(key);
        tree.set(bytes.clone(), bytes)
            .unwrap_or_else(|err| panic!("set proof fixture key {key}: {err}"));
    }
    let root = tree
        .save_version()
        .unwrap_or_else(|err| panic!("save proof fixture tree: {err}"))
        .root_hash;

    for case in fixture.cases {
        assert_eq!(hex::encode(root), case.root_hash_hex, "{}", case.label);
        let key = decode_hex(&case.key_hex);
        let proof = tree
            .get_with_proof(&key)
            .unwrap_or_else(|err| panic!("{} get proof: {err}", case.label))
            .unwrap_or_else(|| panic!("{} missing proof", case.label));
        assert_eq!(
            hex::encode(proof.ops()[0].data()),
            case.proof_proto_hex,
            "{}",
            case.label
        );
        assert!(case.verified, "{}", case.label);

        match case.proof_type.as_str() {
            "membership" => {
                let value = decode_hex(&case.value_hex);
                assert_eq!(proof.ops()[0].field_type(), ProofOpType::IavlValue);
                assert_eq!(proof.value(), Some(value.as_slice()));
                Tree::verify_membership(root, &proof, &key, &value)
                    .unwrap_or_else(|err| panic!("{} verify membership: {err}", case.label));
            }
            "non_membership" => {
                assert_eq!(proof.ops()[0].field_type(), ProofOpType::IavlAbsence);
                assert_eq!(proof.value(), None);
                Tree::verify_non_membership(root, &proof, &key)
                    .unwrap_or_else(|err| panic!("{} verify non-membership: {err}", case.label));
            }
            other => panic!("unknown proof type {other}"),
        }
    }
}

fn apply_mutation(tree: &mut Tree, mutation: &Mutation) {
    match mutation.action.as_str() {
        "set" => {
            tree.set(
                decode_hex(&mutation.key_hex),
                decode_hex(&mutation.value_hex),
            )
            .unwrap_or_else(|err| panic!("set {}: {err}", mutation.key_hex));
        }
        "remove" => {
            tree.remove(&decode_hex(&mutation.key_hex))
                .unwrap_or_else(|err| panic!("remove {}: {err}", mutation.key_hex));
        }
        other => panic!("unknown mutation action {other}"),
    }
}

fn read_fixture<T: for<'de> Deserialize<'de>>(name: &str) -> T {
    let path = workspace_root()
        .join("conformance/fixtures/iavl")
        .join(name);
    let body =
        fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    serde_json::from_str(&body).unwrap_or_else(|err| panic!("parse {}: {err}", path.display()))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("crate lives under crates/tier-2-state-and-store/iavl")
        .to_path_buf()
}

fn optional_hex(input: &str) -> Option<Vec<u8>> {
    if input.is_empty() {
        None
    } else {
        Some(decode_hex(input))
    }
}

fn decode_hex(input: &str) -> Vec<u8> {
    hex::decode(input).unwrap_or_else(|err| panic!("decode hex {input}: {err}"))
}
