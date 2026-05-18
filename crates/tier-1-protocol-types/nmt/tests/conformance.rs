//! Conformance tests for the nmt crate against committed fixture files.

#![allow(clippy::expect_used, clippy::panic)]

use nmt::{
    Error, HASH_SIZE, NAMESPACE_SIZE, NamespacedHash, NamespacedMerkleTree, Proof, SubrootPath,
    TreeOptions, empty_root, hash_leaf, hash_node, subroot_paths,
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
struct HashesFixture {
    namespace_size: usize,
    hash_size: usize,
    empty_root_hex: String,
    leaf_cases: Vec<LeafHashCase>,
    node_cases: Vec<NodeHashCase>,
    parse_cases: Vec<ParseNodeCase>,
}

#[derive(Deserialize)]
struct LeafHashCase {
    label: String,
    leaf_hex: String,
    hash_hex: String,
    namespace: String,
}

#[derive(Deserialize)]
struct NodeHashCase {
    label: String,
    ignore_max_namespace: bool,
    left_hex: String,
    right_hex: String,
    hash_hex: String,
    min_namespace: String,
    max_namespace: String,
}

#[derive(Deserialize)]
struct ParseNodeCase {
    label: String,
    node_hex: String,
    valid: bool,
    #[serde(default)]
    expected_error: String,
}

#[derive(Deserialize)]
struct RootsFixture {
    cases: Vec<RootCase>,
}

#[derive(Deserialize)]
struct RootCase {
    label: String,
    ignore_max_namespace: bool,
    leaves_hex: Vec<String>,
    root_hex: String,
    min_namespace: String,
    max_namespace: String,
}

#[derive(Deserialize)]
struct ProofsFixture {
    cases: Vec<ProofTreeCase>,
}

#[derive(Deserialize)]
struct ProofTreeCase {
    label: String,
    leaves_hex: Vec<String>,
    root_hex: String,
    namespace_cases: Vec<NamespaceProofCase>,
    range_cases: Vec<RangeProofCase>,
}

#[derive(Deserialize)]
struct NamespaceProofCase {
    label: String,
    namespace_hex: String,
    leaves_hex: Vec<String>,
    proof: ProofFixture,
}

#[derive(Deserialize)]
struct RangeProofCase {
    label: String,
    namespace_hex: String,
    start: usize,
    end: usize,
    leaf_hashes_hex: Vec<String>,
    proof: ProofFixture,
}

#[derive(Deserialize)]
struct ProofFixture {
    start: usize,
    end: usize,
    nodes_hex: Vec<String>,
    #[serde(default)]
    leaf_hash_hex: String,
    is_max_namespace_ignored: bool,
}

#[derive(Deserialize)]
struct SubrootPathsFixture {
    cases: Vec<SubrootPathCase>,
}

#[derive(Deserialize)]
struct SubrootPathCase {
    label: String,
    square_size: usize,
    share_start: usize,
    share_count: usize,
    paths: Option<Vec<Vec<Vec<u8>>>>,
    #[serde(default)]
    error: String,
}

#[derive(Deserialize)]
struct MalformedFixture {
    cases: Vec<MalformedCase>,
}

#[derive(Deserialize)]
struct MalformedCase {
    label: String,
    operation: String,
    #[serde(default)]
    namespace_hex: String,
    #[serde(default)]
    leaf_hex: String,
    #[serde(default)]
    node_hex: String,
    #[serde(default)]
    left_hex: String,
    #[serde(default)]
    right_hex: String,
    #[serde(default)]
    leaves_hex: Vec<String>,
    #[serde(default)]
    start: usize,
    #[serde(default)]
    end: usize,
    expected_error: String,
}

fn decode_bytes(hex: &str) -> Vec<u8> {
    hex::decode(hex).expect("valid fixture hex")
}

fn decode_namespace(hex: &str) -> nmt::Result<nmt::Namespace> {
    nmt::Namespace::from_slice(&decode_bytes(hex))
}

fn decode_node(hex: &str) -> nmt::Result<NamespacedHash> {
    NamespacedHash::from_bytes(&decode_bytes(hex))
}

fn decode_nodes(nodes_hex: &[String]) -> nmt::Result<Vec<NamespacedHash>> {
    nodes_hex.iter().map(|node| decode_node(node)).collect()
}

fn decode_leaves(leaves_hex: &[String]) -> Vec<Vec<u8>> {
    leaves_hex.iter().map(|leaf| decode_bytes(leaf)).collect()
}

fn decode_proof(fixture: &ProofFixture) -> nmt::Result<Proof> {
    let leaf_hash = if fixture.leaf_hash_hex.is_empty() {
        None
    } else {
        Some(decode_node(&fixture.leaf_hash_hex)?)
    };

    Ok(Proof {
        start: fixture.start,
        end: fixture.end,
        nodes: decode_nodes(&fixture.nodes_hex)?,
        leaf_hash,
        is_max_namespace_ignored: fixture.is_max_namespace_ignored,
    })
}

fn build_tree(
    leaves_hex: &[String],
    ignore_max_namespace: bool,
) -> nmt::Result<NamespacedMerkleTree> {
    let mut tree = NamespacedMerkleTree::with_options(TreeOptions {
        ignore_max_namespace,
    });
    for leaf in decode_leaves(leaves_hex) {
        tree.push(leaf)?;
    }
    Ok(tree)
}

fn error_name(error: &Error) -> &'static str {
    match error {
        Error::InvalidNamespaceLength { .. } => "invalid_namespace_length",
        Error::InvalidLeafLength { .. } => "invalid_leaf_length",
        Error::InvalidNodeLength { .. } => "invalid_node_length",
        Error::InvalidNodeNamespaceOrder => "invalid_node_namespace_order",
        Error::UnorderedSiblings => "unordered_siblings",
        Error::UnorderedLeaves => "unordered_leaves",
        Error::InvalidRange => "invalid_range",
        Error::InvalidProof => "invalid_proof",
        Error::FailedCompletenessCheck => "failed_completeness_check",
        Error::InvalidSubrootPath => "invalid_subroot_path",
        Error::Decode(_) => "decode",
        Error::Encode(_) => "encode",
    }
}

#[test]
fn conformance_hashes() -> nmt::Result<()> {
    let fixture: HashesFixture = load_fixture("nmt/hashes.json");
    assert_eq!(fixture.namespace_size, NAMESPACE_SIZE);
    assert_eq!(fixture.hash_size, HASH_SIZE);
    assert_eq!(hex::encode(empty_root().to_bytes()), fixture.empty_root_hex);

    for case in &fixture.leaf_cases {
        let hash = hash_leaf(&decode_bytes(&case.leaf_hex))?;
        assert_eq!(
            hex::encode(hash.to_bytes()),
            case.hash_hex,
            "leaf hash mismatch for '{}'",
            case.label
        );
        assert_eq!(
            hex::encode(hash.min_namespace.into_bytes()),
            case.namespace,
            "leaf min namespace mismatch for '{}'",
            case.label
        );
        assert_eq!(
            hex::encode(hash.max_namespace.into_bytes()),
            case.namespace,
            "leaf max namespace mismatch for '{}'",
            case.label
        );
    }

    for case in &fixture.node_cases {
        let hash = hash_node(
            &decode_node(&case.left_hex)?,
            &decode_node(&case.right_hex)?,
            TreeOptions {
                ignore_max_namespace: case.ignore_max_namespace,
            },
        )?;
        assert_eq!(
            hex::encode(hash.to_bytes()),
            case.hash_hex,
            "node hash mismatch for '{}'",
            case.label
        );
        assert_eq!(
            hex::encode(hash.min_namespace.into_bytes()),
            case.min_namespace
        );
        assert_eq!(
            hex::encode(hash.max_namespace.into_bytes()),
            case.max_namespace
        );
    }

    for case in &fixture.parse_cases {
        let result = NamespacedHash::from_bytes(&decode_bytes(&case.node_hex));
        if case.valid {
            assert!(result.is_ok(), "parse node '{}' should succeed", case.label);
        } else {
            let error = result.expect_err("parse node should fail");
            assert_eq!(error_name(&error), case.expected_error, "{}", case.label);
        }
    }

    Ok(())
}

#[test]
fn conformance_roots() -> nmt::Result<()> {
    let fixture: RootsFixture = load_fixture("nmt/roots.json");

    for case in &fixture.cases {
        let tree = build_tree(&case.leaves_hex, case.ignore_max_namespace)?;
        let root = tree.root()?;
        assert_eq!(
            hex::encode(root.to_bytes()),
            case.root_hex,
            "root mismatch for '{}'",
            case.label
        );
        assert_eq!(
            hex::encode(root.min_namespace.into_bytes()),
            case.min_namespace
        );
        assert_eq!(
            hex::encode(root.max_namespace.into_bytes()),
            case.max_namespace
        );
    }

    Ok(())
}

#[test]
fn conformance_proofs() -> nmt::Result<()> {
    let fixture: ProofsFixture = load_fixture("nmt/proofs.json");

    for case in &fixture.cases {
        let tree = build_tree(&case.leaves_hex, true)?;
        let root = tree.root()?;
        assert_eq!(
            hex::encode(root.to_bytes()),
            case.root_hex,
            "proof tree root mismatch for '{}'",
            case.label
        );

        for proof_case in &case.namespace_cases {
            let namespace = decode_namespace(&proof_case.namespace_hex)?;
            let expected = decode_proof(&proof_case.proof)?;
            let generated = tree.prove_namespace(namespace)?;
            let leaves = decode_leaves(&proof_case.leaves_hex);
            assert_eq!(
                generated, expected,
                "namespace proof mismatch for '{}'/{}",
                case.label, proof_case.label
            );
            assert!(
                expected.verify_namespace(namespace, &leaves, &root)?,
                "namespace proof verifies for '{}'/{}",
                case.label,
                proof_case.label
            );
        }

        for proof_case in &case.range_cases {
            let namespace = decode_namespace(&proof_case.namespace_hex)?;
            let expected = decode_proof(&proof_case.proof)?;
            let generated = tree.prove_range(proof_case.start, proof_case.end)?;
            let leaf_hashes = decode_nodes(&proof_case.leaf_hashes_hex)?;
            assert_eq!(
                generated, expected,
                "range proof mismatch for '{}'/{}",
                case.label, proof_case.label
            );
            assert!(
                expected.verify_leaf_hashes(namespace, &leaf_hashes, &root, true)?,
                "range proof verifies for '{}'/{}",
                case.label,
                proof_case.label
            );
        }
    }

    Ok(())
}

#[test]
fn conformance_subroot_paths() -> nmt::Result<()> {
    let fixture: SubrootPathsFixture = load_fixture("nmt/subroot-paths.json");

    for case in &fixture.cases {
        let result = subroot_paths(case.square_size, case.share_start, case.share_count);
        if !case.error.is_empty() {
            let error = result.expect_err("subroot path case should fail");
            assert_eq!(error_name(&error), case.error, "{}", case.label);
            continue;
        }

        let actual = result?;
        assert_rows(case.share_start / case.square_size, &actual, &case.label);
        assert_eq!(
            &paths_as_fixture(&actual),
            case.paths.as_ref().expect("successful case has paths"),
            "subroot paths mismatch for '{}'",
            case.label
        );
    }

    Ok(())
}

#[test]
fn conformance_malformed() {
    let fixture: MalformedFixture = load_fixture("nmt/malformed.json");

    for case in &fixture.cases {
        let error = run_malformed_case(case).expect_err("malformed case should fail");
        assert_eq!(
            error_name(&error),
            case.expected_error,
            "malformed case '{}'",
            case.label
        );
    }
}

fn assert_rows(start_row: usize, actual: &[Vec<SubrootPath>], label: &str) {
    for (row_offset, row_paths) in actual.iter().enumerate() {
        for path in row_paths {
            assert_eq!(
                path.row,
                start_row + row_offset,
                "subroot row mismatch for '{label}'"
            );
        }
    }
}

fn paths_as_fixture(paths: &[Vec<SubrootPath>]) -> Vec<Vec<Vec<u8>>> {
    paths
        .iter()
        .map(|row| {
            row.iter()
                .map(|path| path.path.iter().map(|branch| u8::from(*branch)).collect())
                .collect()
        })
        .collect()
}

fn run_malformed_case(case: &MalformedCase) -> nmt::Result<()> {
    match case.operation.as_str() {
        "parse_namespace" => decode_namespace(&case.namespace_hex).map(|_| ()),
        "hash_leaf" => hash_leaf(&decode_bytes(&case.leaf_hex)).map(|_| ()),
        "parse_node" => NamespacedHash::from_bytes(&decode_bytes(&case.node_hex)).map(|_| ()),
        "push_leaves" => {
            let mut tree = NamespacedMerkleTree::new();
            for leaf in decode_leaves(&case.leaves_hex) {
                tree.push(leaf)?;
            }
            Ok(())
        }
        "hash_node" => hash_node(
            &decode_node(&case.left_hex)?,
            &decode_node(&case.right_hex)?,
            TreeOptions::default(),
        )
        .map(|_| ()),
        "prove_range" => {
            let tree = build_tree(&case.leaves_hex, true)?;
            tree.prove_range(case.start, case.end).map(|_| ())
        }
        "subroot_paths" => subroot_paths(1, 0, 1).map(|_| ()),
        other => panic!("unknown malformed operation {other}"),
    }
}
