//! Public API tests for ICS-23 proof operations.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use iavl::Tree;
use ics23_proofs::{
    CommitmentOp, CommitmentProof, CommitmentProofKind, Error, PROOF_OP_IAVL_COMMITMENT,
    PROOF_OP_SIMPLE_MERKLE_COMMITMENT, PROOF_OP_SMT_COMMITMENT, ProofOp, ProofOps, ProofSpec,
};
use prost::Message;

#[test]
fn iavl_membership_proof_round_trips_and_verifies() {
    let (root, proof) = proof_for(b"c");
    let value = b"carol";

    let commitment = CommitmentProof::from_iavl_proof(&proof).expect("wrap IAVL membership proof");
    assert_eq!(commitment.kind(), CommitmentProofKind::Existence);
    assert_eq!(commitment.key(), b"c");
    assert_eq!(commitment.value(), Some(value.as_slice()));
    assert_eq!(commitment.as_proto_bytes(), proof.ops()[0].data());
    assert_eq!(
        commitment
            .calculate_root(ProofSpec::Iavl)
            .expect("calculate root"),
        root
    );
    commitment
        .verify_membership(ProofSpec::Iavl, root, b"c", value)
        .expect("verify membership");

    let mut wrong_root = root;
    wrong_root[0] ^= 0xff;
    assert!(matches!(
        commitment.verify_membership(ProofSpec::Iavl, wrong_root, b"c", value),
        Err(Error::InvalidProof(_))
    ));

    let op = CommitmentOp::from_iavl_proof(&proof).expect("wrap commitment op");
    assert_eq!(op.field_type(), PROOF_OP_IAVL_COMMITMENT);
    assert_eq!(op.spec(), ProofSpec::Iavl);
    assert_eq!(op.key(), b"c");
    assert_eq!(op.to_proof_op().data, proof.ops()[0].data());
    assert_eq!(op.run(&[value.as_slice()]).expect("run op"), vec![root]);
}

#[test]
fn iavl_non_membership_proof_runs_without_args() {
    let (root, proof) = proof_for(b"b");

    let commitment = CommitmentProof::from_iavl_proof(&proof).expect("wrap IAVL absence proof");
    assert_eq!(commitment.kind(), CommitmentProofKind::NonExistence);
    assert_eq!(commitment.key(), b"b");
    assert_eq!(commitment.value(), None);
    assert_eq!(
        commitment
            .calculate_root(ProofSpec::Iavl)
            .expect("calculate root"),
        root
    );
    commitment
        .verify_non_membership(ProofSpec::Iavl, root, b"b")
        .expect("verify non-membership");

    let op = CommitmentOp::from_iavl_proof(&proof).expect("wrap commitment op");
    assert_eq!(op.run(&[]).expect("run op"), vec![root]);
    assert!(matches!(
        op.run(&[b"extra".as_slice(), b"arg".as_slice()]),
        Err(Error::InvalidProof(_))
    ));
}

#[test]
fn proof_ops_encode_as_cometbft_protobuf() {
    let op = ProofOp {
        field_type: PROOF_OP_IAVL_COMMITMENT.to_owned(),
        key: b"k".to_vec(),
        data: b"proof".to_vec(),
    };
    let decoded = tendermint_proto::v0_38::crypto::ProofOp::decode(op.encode_to_vec().as_slice())
        .expect("decode proof op");
    assert_eq!(decoded.r#type, PROOF_OP_IAVL_COMMITMENT);
    assert_eq!(decoded.key, b"k");
    assert_eq!(decoded.data, b"proof");

    let ops = ProofOps {
        ops: vec![op.clone()],
    };
    let decoded = tendermint_proto::v0_38::crypto::ProofOps::decode(ops.encode_to_vec().as_slice())
        .expect("decode proof ops");
    assert_eq!(decoded.ops.len(), 1);
    assert_eq!(decoded.ops[0].r#type, op.field_type);
}

#[test]
fn malformed_inputs_return_typed_errors() {
    assert!(matches!(
        CommitmentProof::from_proto_bytes(vec![0xff]),
        Err(Error::MalformedProto(_))
    ));

    assert!(matches!(
        CommitmentOp::from_proof_op(ProofOp {
            field_type: "unknown".to_owned(),
            key: b"k".to_vec(),
            data: vec![],
        }),
        Err(Error::InvalidProofOpType(field_type)) if field_type == "unknown"
    ));

    let (_, proof) = proof_for(b"c");
    let commitment = CommitmentProof::from_iavl_proof(&proof).expect("wrap proof");
    assert!(matches!(
        CommitmentOp::from_proof_op(ProofOp {
            field_type: PROOF_OP_IAVL_COMMITMENT.to_owned(),
            key: vec![],
            data: commitment.as_proto_bytes().to_vec(),
        }),
        Err(Error::EmptyKey)
    ));
}

#[test]
fn proof_op_decode_and_constructor_variants_are_typed() {
    let (_, proof) = proof_for(b"c");
    let commitment = CommitmentProof::from_iavl_proof(&proof).expect("wrap proof");
    let op = ProofOp {
        field_type: PROOF_OP_IAVL_COMMITMENT.to_owned(),
        key: b"c".to_vec(),
        data: commitment.as_proto_bytes().to_vec(),
    };

    let decoded = CommitmentOp::from_proof_op(op).expect("decode commitment op");
    assert_eq!(decoded.field_type(), PROOF_OP_IAVL_COMMITMENT);
    assert_eq!(decoded.spec(), ProofSpec::Iavl);
    assert_eq!(decoded.proof().key(), b"c");

    let empty_key = CommitmentOp::new_iavl(Vec::new(), commitment.clone());
    assert!(matches!(empty_key.run(&[]), Err(Error::EmptyKey)));

    let simple = CommitmentOp::new_simple_merkle(b"store".to_vec(), commitment.clone());
    assert_eq!(simple.field_type(), PROOF_OP_SIMPLE_MERKLE_COMMITMENT);
    assert_eq!(simple.spec(), ProofSpec::Tendermint);
    assert!(matches!(
        simple
            .proof()
            .verify_membership(ProofSpec::Tendermint, [0; 32], b"c", b"carol"),
        Err(Error::InvalidProof(_))
    ));

    let smt = CommitmentOp::new_smt(b"store".to_vec(), commitment);
    assert_eq!(smt.field_type(), PROOF_OP_SMT_COMMITMENT);
    assert_eq!(smt.spec(), ProofSpec::Smt);
    assert!(matches!(
        smt.proof()
            .verify_membership(ProofSpec::Smt, [0; 32], b"c", b"carol"),
        Err(Error::InvalidProof(_))
    ));
}

#[test]
fn batch_and_compressed_proofs_calculate_and_verify() {
    let (root, proof_a) = proof_for(b"a");
    let (_, proof_c) = proof_for(b"c");
    let raw = batch_raw(&[proof_a, proof_c]);
    let batch = CommitmentProof::from_proto_bytes(raw.encode_to_vec()).expect("decode batch");

    assert_eq!(batch.kind(), CommitmentProofKind::Batch);
    assert_eq!(batch.key(), b"");
    assert_eq!(batch.value(), None);
    assert_eq!(
        batch.calculate_root(ProofSpec::Iavl).expect("batch root"),
        root
    );
    batch
        .verify_membership(ProofSpec::Iavl, root, b"a", b"alice")
        .expect("batch verifies a");
    batch
        .verify_membership(ProofSpec::Iavl, root, b"c", b"carol")
        .expect("batch verifies c");

    let compressed_raw = ics23::compress(&raw).expect("compress batch");
    let compressed = CommitmentProof::from_proto_bytes(compressed_raw.encode_to_vec())
        .expect("decode compressed");
    assert_eq!(compressed.kind(), CommitmentProofKind::Compressed);
    assert_eq!(
        compressed
            .calculate_root(ProofSpec::Iavl)
            .expect("compressed root"),
        root
    );
    compressed
        .verify_membership(ProofSpec::Iavl, root, b"a", b"alice")
        .expect("compressed verifies a");
}

#[test]
fn malformed_proof_shapes_report_errors() {
    assert!(matches!(
        CommitmentProof::from_proto_bytes(Vec::new()),
        Err(Error::MalformedProto("missing commitment proof variant"))
    ));

    let empty_batch = ics23::CommitmentProof {
        proof: Some(ics23::commitment_proof::Proof::Batch(ics23::BatchProof {
            entries: vec![],
        })),
    };
    let proof =
        CommitmentProof::from_proto_bytes(empty_batch.encode_to_vec()).expect("decode empty batch");
    assert!(matches!(
        proof.calculate_root(ProofSpec::Iavl),
        Err(Error::InvalidProof("empty batch proof"))
    ));

    let empty_entry = ics23::CommitmentProof {
        proof: Some(ics23::commitment_proof::Proof::Batch(ics23::BatchProof {
            entries: vec![ics23::BatchEntry { proof: None }],
        })),
    };
    let proof = CommitmentProof::from_proto_bytes(empty_entry.encode_to_vec())
        .expect("decode empty batch entry");
    assert!(matches!(
        proof.calculate_root(ProofSpec::Iavl),
        Err(Error::InvalidProof("empty batch proof entry"))
    ));

    let no_neighbors = ics23::CommitmentProof {
        proof: Some(ics23::commitment_proof::Proof::Nonexist(
            ics23::NonExistenceProof {
                key: b"missing".to_vec(),
                left: None,
                right: None,
            },
        )),
    };
    let proof =
        CommitmentProof::from_proto_bytes(no_neighbors.encode_to_vec()).expect("decode nonexist");
    assert!(matches!(
        proof.calculate_root(ProofSpec::Iavl),
        Err(Error::InvalidProof("missing non-existence neighbors"))
    ));

    let (root_a, proof_a) = one_key_proof(b"a", b"alice");
    let (root_z, proof_z) = one_key_proof(b"z", b"zara");
    assert_ne!(root_a, root_z);
    let mismatched = batch_raw(&[proof_a, proof_z]);
    let proof =
        CommitmentProof::from_proto_bytes(mismatched.encode_to_vec()).expect("decode mismatch");
    assert!(matches!(
        proof.calculate_root(ProofSpec::Iavl),
        Err(Error::InvalidProof("batch proof roots mismatch"))
    ));
}

#[test]
fn error_display_messages_are_stable() {
    assert_eq!(Error::EmptyKey.to_string(), "empty key");
    assert_eq!(
        Error::InvalidProofOpType("bad".to_owned()).to_string(),
        "invalid proof operation type: bad"
    );
    assert_eq!(
        Error::InvalidProof("bad proof").to_string(),
        "invalid proof: bad proof"
    );
    assert_eq!(
        Error::MalformedProto("bad proto").to_string(),
        "malformed protobuf: bad proto"
    );
    assert_eq!(
        Error::UnsupportedProofKind("batch").to_string(),
        "unsupported proof kind: batch"
    );
    assert_eq!(
        Error::from(iavl::Error::EmptyKey).to_string(),
        "iavl: empty key"
    );
}

fn proof_for(key: &[u8]) -> (iavl::Hash, iavl::Proof) {
    let mut tree = Tree::new();
    tree.set(b"a".to_vec(), b"alice".to_vec()).expect("set a");
    tree.set(b"c".to_vec(), b"carol".to_vec()).expect("set c");
    tree.set(b"e".to_vec(), b"eve".to_vec()).expect("set e");
    let root = tree.save_version().expect("save version").root_hash;
    let proof = tree
        .get_with_proof(key)
        .expect("get proof")
        .expect("non-empty tree returns proof");
    (root, proof)
}

fn one_key_proof(key: &[u8], value: &[u8]) -> (iavl::Hash, iavl::Proof) {
    let mut tree = Tree::new();
    tree.set(key.to_vec(), value.to_vec()).expect("set key");
    let root = tree.save_version().expect("save version").root_hash;
    let proof = tree
        .get_with_proof(key)
        .expect("get proof")
        .expect("non-empty tree returns proof");
    (root, proof)
}

fn batch_raw(proofs: &[iavl::Proof]) -> ics23::CommitmentProof {
    let entries = proofs
        .iter()
        .map(|proof| {
            let raw = raw_proof(proof);
            let proof = match raw.proof.expect("proof variant") {
                ics23::commitment_proof::Proof::Exist(existence) => {
                    ics23::batch_entry::Proof::Exist(existence)
                }
                ics23::commitment_proof::Proof::Nonexist(nonexistence) => {
                    ics23::batch_entry::Proof::Nonexist(nonexistence)
                }
                _ => panic!("unexpected proof kind"),
            };
            ics23::BatchEntry { proof: Some(proof) }
        })
        .collect();
    ics23::CommitmentProof {
        proof: Some(ics23::commitment_proof::Proof::Batch(ics23::BatchProof {
            entries,
        })),
    }
}

fn raw_proof(proof: &iavl::Proof) -> ics23::CommitmentProof {
    ics23::CommitmentProof::decode(proof.ops()[0].data()).expect("decode raw commitment proof")
}
