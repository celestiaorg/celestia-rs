//! Go Cosmos SDK ICS-23 proof operation conformance tests.

#![allow(clippy::expect_used, clippy::panic, dead_code)]

use std::{fs, path::PathBuf};

use ics23_proofs::{
    CommitmentOp, CommitmentProof, CommitmentProofKind, Error, Hash, ProofOp, ProofOps, ProofSpec,
};
use prost::Message;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct IavlFixture {
    cases: Vec<ProofOpCase>,
}

#[derive(Debug, Deserialize)]
struct SimpleFixture {
    cases: Vec<ProofOpCase>,
}

#[derive(Debug, Deserialize)]
struct OpsFixture {
    proof_ops_proto_hex: String,
    ops: Vec<ProofOpCase>,
}

#[derive(Debug, Deserialize)]
struct MalformedFixture {
    cases: Vec<MalformedCase>,
}

#[derive(Debug, Deserialize)]
struct ProofOpCase {
    label: String,
    proof_type: String,
    proof_op_type: String,
    key_hex: String,
    #[serde(default)]
    value_hex: String,
    root_hash_hex: String,
    proof_proto_hex: String,
    proof_op_key_hex: String,
    proof_op_data_hex: String,
    run_args_hex: Vec<String>,
    run_root_hex: String,
    verified: bool,
}

#[derive(Debug, Deserialize)]
struct MalformedCase {
    label: String,
    api: String,
    #[serde(default)]
    proof_op_type: String,
    #[serde(default)]
    key_hex: String,
    #[serde(default)]
    data_hex: String,
    #[serde(default)]
    run_args_hex: Vec<String>,
    expected_error: String,
}

#[test]
fn iavl_fixtures_match_go_commitment_ops() {
    let fixture: IavlFixture = read_fixture("iavl.json");

    for case in fixture.cases {
        assert_commitment_case(&case);
    }
}

#[test]
fn simple_fixtures_match_go_commitment_ops() {
    let fixture: SimpleFixture = read_fixture("simple.json");

    for case in fixture.cases {
        assert_commitment_case(&case);
    }
}

#[test]
fn proof_ops_fixture_matches_cometbft_protobuf() {
    let fixture: OpsFixture = read_fixture("ops.json");
    let decoded = ProofOps::decode(decode_hex(&fixture.proof_ops_proto_hex).as_slice())
        .expect("decode ProofOps fixture");
    assert_eq!(decoded.ops.len(), fixture.ops.len());

    for (op, case) in decoded.ops.iter().zip(&fixture.ops) {
        assert_eq!(op.field_type, case.proof_op_type, "{}", case.label);
        assert_eq!(
            hex::encode(&op.key),
            case.proof_op_key_hex,
            "{}",
            case.label
        );
        assert_eq!(
            hex::encode(&op.data),
            case.proof_op_data_hex,
            "{}",
            case.label
        );
        assert_commitment_case(case);
    }
}

#[test]
fn malformed_fixtures_match_expected_errors() {
    let fixture: MalformedFixture = read_fixture("malformed.json");

    for case in fixture.cases {
        let err = match case.api.as_str() {
            "proof_decode" => expect_proof_decode_error(&case),
            "op_decode" => expect_op_decode_error(&case),
            "run" => expect_run_error(&case),
            other => panic!("{} unknown malformed api {other}", case.label),
        };
        assert_eq!(error_code(&err), case.expected_error, "{}", case.label);
    }
}

fn assert_commitment_case(case: &ProofOpCase) {
    assert!(case.verified, "{}", case.label);
    let proof_bytes = decode_hex(&case.proof_proto_hex);
    let commitment = CommitmentProof::from_proto_bytes(proof_bytes.clone())
        .unwrap_or_else(|err| panic!("{} decode commitment proof: {err}", case.label));
    assert_eq!(commitment.as_proto_bytes(), proof_bytes, "{}", case.label);
    assert_eq!(
        hex::encode(commitment.key()),
        case.key_hex,
        "{}",
        case.label
    );
    assert_eq!(
        commitment.value().map(hex::encode).unwrap_or_default(),
        case.value_hex,
        "{}",
        case.label
    );

    let root = decode_hash(&case.root_hash_hex);
    assert_eq!(
        commitment
            .calculate_root(spec_for(&case.proof_op_type))
            .unwrap_or_else(|err| panic!("{} calculate root: {err}", case.label)),
        root,
        "{}",
        case.label
    );

    match case.proof_type.as_str() {
        "membership" => {
            assert_eq!(
                commitment.kind(),
                CommitmentProofKind::Existence,
                "{}",
                case.label
            );
            let value = decode_hex(&case.value_hex);
            commitment
                .verify_membership(
                    spec_for(&case.proof_op_type),
                    root,
                    commitment.key(),
                    &value,
                )
                .unwrap_or_else(|err| panic!("{} verify membership: {err}", case.label));
        }
        "non_membership" => {
            assert_eq!(
                commitment.kind(),
                CommitmentProofKind::NonExistence,
                "{}",
                case.label
            );
            commitment
                .verify_non_membership(spec_for(&case.proof_op_type), root, commitment.key())
                .unwrap_or_else(|err| panic!("{} verify non-membership: {err}", case.label));
        }
        other => panic!("{} unknown proof type {other}", case.label),
    }

    let op = CommitmentOp::from_proof_op(proof_op(case))
        .unwrap_or_else(|err| panic!("{} decode commitment op: {err}", case.label));
    assert_eq!(op.field_type(), case.proof_op_type, "{}", case.label);
    assert_eq!(
        hex::encode(op.key()),
        case.proof_op_key_hex,
        "{}",
        case.label
    );
    assert_eq!(
        hex::encode(op.proof().as_proto_bytes()),
        case.proof_op_data_hex,
        "{}",
        case.label
    );
    let encoded = op.to_proof_op();
    assert_eq!(encoded.field_type, case.proof_op_type, "{}", case.label);
    assert_eq!(
        hex::encode(encoded.key),
        case.proof_op_key_hex,
        "{}",
        case.label
    );
    assert_eq!(
        hex::encode(encoded.data),
        case.proof_op_data_hex,
        "{}",
        case.label
    );

    let args = decode_args(&case.run_args_hex);
    let arg_refs = args.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let roots = op
        .run(&arg_refs)
        .unwrap_or_else(|err| panic!("{} run commitment op: {err}", case.label));
    assert_eq!(roots.len(), 1, "{}", case.label);
    assert_eq!(hex::encode(roots[0]), case.run_root_hex, "{}", case.label);
}

fn expect_proof_decode_error(case: &MalformedCase) -> Error {
    match CommitmentProof::from_proto_bytes(decode_hex(&case.data_hex)) {
        Ok(_) => panic!("{} expected proof decode error", case.label),
        Err(err) => err,
    }
}

fn expect_op_decode_error(case: &MalformedCase) -> Error {
    match CommitmentOp::from_proof_op(ProofOp {
        field_type: case.proof_op_type.clone(),
        key: decode_hex(&case.key_hex),
        data: decode_hex(&case.data_hex),
    }) {
        Ok(_) => panic!("{} expected op decode error", case.label),
        Err(err) => err,
    }
}

fn expect_run_error(case: &MalformedCase) -> Error {
    let op = CommitmentOp::from_proof_op(ProofOp {
        field_type: case.proof_op_type.clone(),
        key: decode_hex(&case.key_hex),
        data: decode_hex(&case.data_hex),
    })
    .unwrap_or_else(|err| panic!("{} decode op before run: {err}", case.label));
    let args = decode_args(&case.run_args_hex);
    let arg_refs = args.iter().map(Vec::as_slice).collect::<Vec<_>>();
    match op.run(&arg_refs) {
        Ok(_) => panic!("{} expected run error", case.label),
        Err(err) => err,
    }
}

fn proof_op(case: &ProofOpCase) -> ProofOp {
    ProofOp {
        field_type: case.proof_op_type.clone(),
        key: decode_hex(&case.proof_op_key_hex),
        data: decode_hex(&case.proof_op_data_hex),
    }
}

fn spec_for(proof_op_type: &str) -> ProofSpec {
    match proof_op_type {
        "ics23:iavl" => ProofSpec::Iavl,
        "ics23:simple" => ProofSpec::Tendermint,
        "ics23:smt" => ProofSpec::Smt,
        other => panic!("unknown proof op type {other}"),
    }
}

fn error_code(err: &Error) -> &'static str {
    match err {
        Error::MalformedProto(_) => "malformed_proto",
        Error::InvalidProofOpType(_) => "invalid_type",
        Error::EmptyKey => "empty_key",
        Error::InvalidProof(_) => "invalid_proof",
        Error::UnsupportedProofKind(_) => "unsupported_kind",
        Error::Iavl(_) => "iavl",
    }
}

fn decode_args(hexes: &[String]) -> Vec<Vec<u8>> {
    hexes.iter().map(|hex| decode_hex(hex)).collect()
}

fn decode_hash(hex: &str) -> Hash {
    let bytes = decode_hex(hex);
    bytes
        .try_into()
        .unwrap_or_else(|bytes: Vec<u8>| panic!("expected 32-byte hash, got {}", bytes.len()))
}

fn decode_hex(hex: &str) -> Vec<u8> {
    hex::decode(hex).expect("valid fixture hex")
}

fn read_fixture<T>(name: &str) -> T
where
    T: for<'de> Deserialize<'de>,
{
    let path = fixture_path(name);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read fixture {}: {err}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("parse fixture {}: {err}", path.display()))
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../conformance/fixtures/ics23-proofs")
        .join(name)
}
