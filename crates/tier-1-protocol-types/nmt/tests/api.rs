//! Public API coverage for the `nmt` crate.

#![allow(clippy::expect_used)]

use nmt::{
    Error, MAX_NAMESPACE, MIN_NAMESPACE, Namespace, NamespacedHash, NamespacedMerkleTree, Proof,
    ProofKind, TreeOptions, empty_root, hash_leaf, hash_node, subroot_paths,
};

fn namespace(byte: u8) -> Namespace {
    Namespace::new([byte; 8])
}

fn leaf(namespace_byte: u8, payload: &[u8]) -> Vec<u8> {
    let mut leaf = namespace(namespace_byte).into_bytes().to_vec();
    leaf.extend_from_slice(payload);
    leaf
}

fn build_tree() -> nmt::Result<NamespacedMerkleTree> {
    let mut tree = NamespacedMerkleTree::new();
    tree.extend([leaf(1, b"a"), leaf(1, b"b"), leaf(3, b"c"), leaf(5, b"d")])?;
    Ok(tree)
}

#[test]
fn namespace_parsing_and_formatting() -> nmt::Result<()> {
    let namespace = Namespace::from_slice(&[1, 2, 3, 4, 5, 6, 7, 8])?;
    assert_eq!(format!("{namespace:x}"), "0102030405060708");
    assert!(MIN_NAMESPACE.is_min());
    assert!(MAX_NAMESPACE.is_max());
    assert!(!namespace.is_min());
    assert!(!namespace.is_max());
    assert_eq!(
        Namespace::try_from(namespace.as_bytes().as_slice())?,
        namespace
    );
    assert_eq!(Namespace::from(namespace.into_bytes()), namespace);
    let bytes: [u8; 8] = namespace.into();
    assert_eq!(bytes, [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(
        Namespace::from_slice(&[1, 2]).expect_err("short namespace"),
        Error::InvalidNamespaceLength { actual: 2 }
    );
    Ok(())
}

#[test]
fn error_display_messages_are_stable() {
    let cases = [
        (
            Error::InvalidNamespaceLength { actual: 7 },
            "invalid namespace length: 7",
        ),
        (
            Error::InvalidLeafLength { actual: 1, min: 8 },
            "invalid NMT leaf length: got 1, want >= 8",
        ),
        (
            Error::InvalidNodeLength {
                actual: 47,
                expected: 48,
            },
            "invalid NMT node length: got 47, want 48",
        ),
        (
            Error::InvalidNodeNamespaceOrder,
            "invalid NMT node namespace order",
        ),
        (
            Error::UnorderedSiblings,
            "NMT sibling nodes are not ordered by namespace",
        ),
        (
            Error::UnorderedLeaves,
            "NMT leaves are not ordered by namespace",
        ),
        (Error::InvalidRange, "invalid NMT proof range"),
        (Error::InvalidProof, "invalid NMT proof"),
        (
            Error::FailedCompletenessCheck,
            "failed NMT completeness check",
        ),
        (Error::InvalidSubrootPath, "invalid NMT subroot path input"),
        (
            Error::Decode("bad bytes".to_owned()),
            "decode NMT data: bad bytes",
        ),
        (
            Error::Encode("bad value".to_owned()),
            "encode NMT data: bad value",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn hash_leaf_matches_nmt_formula() -> nmt::Result<()> {
    let leaf = leaf(7, b"payload");
    let hash = hash_leaf(&leaf)?;

    let mut expected_input = vec![0];
    expected_input.extend_from_slice(&leaf);
    assert_eq!(hash.min_namespace, namespace(7));
    assert_eq!(hash.max_namespace, namespace(7));
    assert_eq!(hash.digest, hash::sha256(&expected_input));
    assert_eq!(
        hash_leaf(&[1, 2]),
        Err(Error::InvalidLeafLength { actual: 2, min: 8 })
    );
    Ok(())
}

#[test]
fn hash_node_honors_max_namespace_ignore_option() -> nmt::Result<()> {
    let left = hash_leaf(&leaf(2, b"left"))?;
    let right = hash_leaf(&leaf(0xff, b"right"))?;

    let ignored = hash_node(&left, &right, TreeOptions::default())?;
    assert_eq!(ignored.max_namespace, namespace(2));

    let included = hash_node(
        &left,
        &right,
        TreeOptions {
            ignore_max_namespace: false,
        },
    )?;
    assert_eq!(included.max_namespace, MAX_NAMESPACE);

    assert_eq!(
        hash_node(&right, &left, TreeOptions::default()).expect_err("unordered siblings"),
        Error::UnorderedSiblings
    );
    Ok(())
}

#[test]
fn namespaced_hash_round_trips_bytes() -> nmt::Result<()> {
    let hash = hash_leaf(&leaf(4, b"leaf"))?;
    let encoded = hash.to_bytes();
    assert_eq!(NamespacedHash::from_bytes(&encoded)?, hash);
    let invalid = NamespacedHash::new(namespace(9), namespace(8), [0; 32]);
    assert_eq!(
        invalid
            .validate_basic()
            .expect_err("invalid namespace order"),
        Error::InvalidNodeNamespaceOrder
    );
    assert_eq!(
        NamespacedHash::from_bytes(&encoded[..47]).expect_err("short node"),
        Error::InvalidNodeLength {
            actual: 47,
            expected: 48,
        }
    );
    Ok(())
}

#[test]
fn tree_root_get_and_ordering() -> nmt::Result<()> {
    let mut tree = NamespacedMerkleTree::new();
    assert_eq!(NamespacedMerkleTree::default(), tree);
    assert!(tree.is_empty());
    assert_eq!(tree.len(), 0);
    assert_eq!(tree.options(), TreeOptions::default());
    assert!(tree.leaves().is_empty());
    assert_eq!(tree.root()?, empty_root());
    assert_eq!(tree.prove_namespace(namespace(1))?, Proof::empty(true));
    assert_eq!(
        tree.push(vec![1, 2]),
        Err(Error::InvalidLeafLength { actual: 2, min: 8 })
    );

    tree.push(leaf(1, b"a"))?;
    let single_root = tree.root()?;
    assert_eq!(single_root, hash_leaf(&leaf(1, b"a"))?);

    tree.push(leaf(2, b"b"))?;
    tree.push(leaf(2, b"c"))?;
    assert_eq!(tree.min_namespace()?, namespace(1));
    assert_eq!(tree.max_namespace()?, namespace(2));
    assert_eq!(tree.get(namespace(2)), vec![leaf(2, b"b"), leaf(2, b"c")]);
    assert!(tree.get(namespace(9)).is_empty());
    assert_eq!(tree.push(leaf(1, b"late")), Err(Error::UnorderedLeaves));
    assert_eq!(tree.prove_range(1, 1), Err(Error::InvalidRange));
    assert_eq!(tree.prove_range(0, 9), Err(Error::InvalidRange));
    tree.reset();
    assert!(tree.is_empty());
    assert_eq!(tree.root()?, empty_root());
    Ok(())
}

#[test]
fn tree_root_uses_go_split_point_for_unbalanced_trees() -> nmt::Result<()> {
    let mut tree = NamespacedMerkleTree::new();
    tree.extend([leaf(1, b"a"), leaf(2, b"b"), leaf(3, b"c")])?;

    let left = hash_node(
        &hash_leaf(&leaf(1, b"a"))?,
        &hash_leaf(&leaf(2, b"b"))?,
        TreeOptions::default(),
    )?;
    let expected = hash_node(&left, &hash_leaf(&leaf(3, b"c"))?, TreeOptions::default())?;
    assert_eq!(tree.root()?, expected);
    Ok(())
}

#[test]
fn range_proofs_verify_leaf_hashes() -> nmt::Result<()> {
    let tree = build_tree()?;
    let root = tree.root()?;
    let proof = tree.prove_range(0, 2)?;
    let hashes = [hash_leaf(&leaf(1, b"a"))?, hash_leaf(&leaf(1, b"b"))?];

    assert_eq!(proof.kind(), ProofKind::Inclusion);
    assert_eq!(proof.nodes_as_bytes().len(), proof.nodes.len());
    assert!(proof.verify_leaf_hashes(namespace(1), &hashes, &root, true)?);
    assert!(!proof.verify_leaf_hashes(namespace(1), &hashes, &empty_root(), true)?);
    assert_eq!(
        proof
            .verify_leaf_hashes(namespace(1), &hashes[..1], &root, true)
            .expect_err("wrong leaf hash count"),
        Error::InvalidProof
    );
    assert_eq!(
        proof
            .verify_leaf_hashes(namespace(9), &hashes, &root, false)
            .expect_err("wrong namespace"),
        Error::InvalidProof
    );
    Ok(())
}

#[test]
fn namespace_inclusion_and_absence_proofs_verify() -> nmt::Result<()> {
    let tree = build_tree()?;
    let root = tree.root()?;

    let inclusion = tree.prove_namespace(namespace(1))?;
    assert_eq!(inclusion.kind(), ProofKind::Inclusion);
    assert!(inclusion.verify_namespace(namespace(1), &tree.get(namespace(1)), &root)?);

    let absence = tree.prove_namespace(namespace(2))?;
    assert_eq!(absence.kind(), ProofKind::Absence);
    assert!(absence.verify_namespace(namespace(2), &[], &root)?);
    assert_eq!(
        absence
            .verify_namespace(namespace(2), &[leaf(2, b"unexpected")], &root)
            .expect_err("absence proof takes no leaves"),
        Error::InvalidProof
    );

    let empty = tree.prove_namespace(namespace(9))?;
    assert_eq!(empty.kind(), ProofKind::Empty);
    assert!(empty.verify_namespace(namespace(9), &[], &root)?);
    assert!(!empty.verify_namespace(namespace(9), &[leaf(9, b"x")], &root)?);
    Ok(())
}

#[test]
fn malformed_proofs_are_rejected() -> nmt::Result<()> {
    assert_eq!(
        Proof::inclusion(2, 2, Vec::new(), true),
        Err(Error::InvalidProof)
    );
    assert_eq!(
        Proof::inclusion(3, 2, Vec::new(), true),
        Err(Error::InvalidRange)
    );

    let root = build_tree()?.root()?;
    let bad_absence = Proof::absence(
        0,
        1,
        Vec::new(),
        hash_leaf(&leaf(1, b"same-or-lower"))?,
        true,
    )?;
    assert_eq!(
        bad_absence
            .verify_leaf_hashes(
                namespace(2),
                &[hash_leaf(&leaf(1, b"same-or-lower"))?],
                &root,
                false
            )
            .expect_err("absence leaf must be greater than namespace"),
        Error::InvalidProof
    );

    let proof = build_tree()?.prove_range(0, 1)?;
    assert_eq!(
        proof
            .verify_namespace(namespace(1), &[leaf(2, b"wrong")], &root)
            .expect_err("leaf namespace mismatch"),
        Error::InvalidProof
    );
    Ok(())
}

#[test]
fn namespace_verification_rejects_incomplete_ranges() -> nmt::Result<()> {
    let tree = build_tree()?;
    let root = tree.root()?;
    let proof = tree.prove_range(0, 1)?;
    let hashes = [hash_leaf(&leaf(1, b"a"))?];

    assert_eq!(
        proof
            .verify_leaf_hashes(namespace(1), &hashes, &root, true)
            .expect_err("incomplete range"),
        Error::FailedCompletenessCheck
    );
    assert!(proof.verify_leaf_hashes(namespace(1), &hashes, &root, false)?);
    Ok(())
}

#[test]
fn subtree_root_rejects_ranges_that_are_not_subtrees() -> nmt::Result<()> {
    let tree = build_tree()?;
    assert!(tree.subtree_root(0, 2).is_ok());
    assert_eq!(tree.subtree_root(1, 3), Err(Error::InvalidRange));
    Ok(())
}

#[test]
fn subroot_paths_match_go_examples() -> nmt::Result<()> {
    assert_eq!(
        subroot_paths(2, 0, 1)?,
        vec![vec![nmt::SubrootPath {
            row: 0,
            path: vec![false],
        }]]
    );
    assert_eq!(
        subroot_paths(2, 0, 2)?,
        vec![vec![nmt::SubrootPath {
            row: 0,
            path: Vec::new(),
        }]]
    );
    let paths = subroot_paths(4, 1, 2)?;
    assert_eq!(
        paths,
        vec![vec![
            nmt::SubrootPath {
                row: 0,
                path: vec![false, true],
            },
            nmt::SubrootPath {
                row: 0,
                path: vec![true, false],
            },
        ]]
    );

    let rows = subroot_paths(32, 0, 96)?;
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[0],
        vec![nmt::SubrootPath {
            row: 0,
            path: Vec::new(),
        }]
    );
    assert_eq!(
        rows[1],
        vec![nmt::SubrootPath {
            row: 1,
            path: Vec::new(),
        }]
    );
    assert_eq!(
        rows[2],
        vec![nmt::SubrootPath {
            row: 2,
            path: Vec::new(),
        }]
    );
    assert_eq!(subroot_paths(1, 0, 1), Err(Error::InvalidSubrootPath));
    assert_eq!(subroot_paths(4, 0, 0), Err(Error::InvalidSubrootPath));
    assert_eq!(subroot_paths(4, 0, 17), Err(Error::InvalidSubrootPath));
    assert_eq!(
        subroot_paths(2, usize::MAX, 1),
        Err(Error::InvalidSubrootPath)
    );
    assert_eq!(
        subroot_paths(1usize << (usize::BITS - 1), 0, 1),
        Err(Error::InvalidSubrootPath)
    );
    Ok(())
}

#[test]
fn subroot_paths_cover_complex_go_examples() -> nmt::Result<()> {
    assert_eq!(
        subroot_paths(8, 1, 6)?[0],
        vec![
            nmt::SubrootPath {
                row: 0,
                path: vec![false, false, true],
            },
            nmt::SubrootPath {
                row: 0,
                path: vec![true, true, false],
            },
            nmt::SubrootPath {
                row: 0,
                path: vec![false, true],
            },
            nmt::SubrootPath {
                row: 0,
                path: vec![true, false],
            },
        ]
    );
    assert_eq!(
        subroot_paths(32, 988, 32)?,
        vec![
            vec![nmt::SubrootPath {
                row: 30,
                path: vec![true, true, true],
            }],
            vec![
                nmt::SubrootPath {
                    row: 31,
                    path: vec![false],
                },
                nmt::SubrootPath {
                    row: 31,
                    path: vec![true, false],
                },
                nmt::SubrootPath {
                    row: 31,
                    path: vec![true, true, false],
                },
            ],
        ]
    );
    Ok(())
}
