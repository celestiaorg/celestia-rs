//! Conformance tests for tendermint-types against committed fixtures.

#![allow(clippy::expect_used, clippy::panic)]

use serde::Deserialize;
use std::path::PathBuf;
use tendermint_types::{
    Commit, Header, Proposal, PublicKey, Validator, ValidatorSet, Vote,
    proto::{self},
};

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

#[derive(Deserialize)]
struct ValidatorFixture {
    cases: Vec<ValidatorCase>,
    set: ValidatorSetCase,
}

#[derive(Deserialize)]
struct ValidatorCase {
    label: String,
    public_key_hex: String,
    address_hex: String,
    voting_power: i64,
    proposer_priority: i64,
    validator_bytes_hex: String,
}

#[derive(Deserialize)]
struct ValidatorSetCase {
    #[serde(rename = "validator_addresses_hex")]
    addresses: Vec<String>,
    #[serde(rename = "proposer_address_hex")]
    proposer_address: String,
    #[serde(rename = "validator_set_hash_hex")]
    hash: String,
}

#[derive(Deserialize)]
struct CommitFixture {
    cases: Vec<CommitCase>,
}

#[derive(Deserialize)]
struct CommitCase {
    label: String,
    chain_id: String,
    commit_proto_hex: String,
    commit_hash_hex: String,
    vote_sign_bytes_hex: String,
}

#[derive(Deserialize)]
struct CanonicalFixture {
    cases: Vec<CanonicalCase>,
}

#[derive(Deserialize)]
struct CanonicalCase {
    label: String,
    chain_id: String,
    vote_proto_hex: Option<String>,
    vote_sign_bytes_hex: Option<String>,
    vote_extension_sign_bytes_hex: Option<String>,
    proposal_proto_hex: Option<String>,
    proposal_sign_bytes_hex: Option<String>,
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

#[test]
fn conformance_validators() {
    let fixture: ValidatorFixture = load_fixture("tendermint-types/validators.json");
    let validators = fixture
        .cases
        .iter()
        .map(|case| {
            let public_key = decode_array::<32>(&case.public_key_hex);
            let address = decode_array::<20>(&case.address_hex);
            let validator = Validator {
                address,
                public_key: PublicKey::Ed25519(public_key),
                voting_power: case.voting_power,
                proposer_priority: case.proposer_priority,
            };
            validator
                .validate_basic()
                .unwrap_or_else(|e| panic!("validator '{}' validates: {e}", case.label));
            assert_eq!(
                hex::encode(validator.bytes().expect("validator bytes")),
                case.validator_bytes_hex,
                "validator bytes mismatch for '{}'",
                case.label
            );
            validator
        })
        .collect::<Vec<_>>();

    let set = ValidatorSet::new(validators).expect("validator set");
    assert_eq!(
        set.validators
            .iter()
            .map(|validator| hex::encode(validator.address))
            .collect::<Vec<_>>(),
        fixture.set.addresses
    );
    assert_eq!(
        set.proposer
            .as_ref()
            .map(|validator| hex::encode(validator.address))
            .unwrap_or_default(),
        fixture.set.proposer_address
    );
    assert_eq!(
        hex::encode(set.hash().expect("validator set hash")),
        fixture.set.hash
    );
}

#[test]
fn conformance_commits() {
    let fixture: CommitFixture = load_fixture("tendermint-types/commits.json");

    for case in &fixture.cases {
        let encoded = hex::decode(&case.commit_proto_hex).expect("valid commit protobuf hex");
        let proto = <proto::types::Commit as prost::Message>::decode(encoded.as_slice())
            .unwrap_or_else(|e| panic!("decode commit for '{}': {e}", case.label));
        let commit = Commit::from_proto(proto)
            .unwrap_or_else(|e| panic!("commit from proto for '{}': {e}", case.label));
        assert_eq!(
            hex::encode(commit.hash()),
            case.commit_hash_hex,
            "commit hash mismatch for '{}'",
            case.label
        );
        assert_eq!(
            hex::encode(
                commit
                    .vote_sign_bytes(&case.chain_id, 0)
                    .expect("vote sign bytes")
            ),
            case.vote_sign_bytes_hex,
            "commit vote sign bytes mismatch for '{}'",
            case.label
        );
    }
}

#[test]
fn conformance_canonical_sign_bytes() {
    let fixture: CanonicalFixture = load_fixture("tendermint-types/canonical-sign-bytes.json");

    for case in &fixture.cases {
        if let Some(vote_proto_hex) = &case.vote_proto_hex {
            let encoded = hex::decode(vote_proto_hex).expect("valid vote protobuf hex");
            let proto = <proto::types::Vote as prost::Message>::decode(encoded.as_slice())
                .unwrap_or_else(|e| panic!("decode vote for '{}': {e}", case.label));
            let vote = Vote::from_proto(proto)
                .unwrap_or_else(|e| panic!("vote from proto for '{}': {e}", case.label));
            assert_eq!(
                hex::encode(vote.sign_bytes(&case.chain_id).expect("vote sign bytes")),
                case.vote_sign_bytes_hex.as_deref().unwrap_or_default(),
                "vote sign bytes mismatch for '{}'",
                case.label
            );
            assert_eq!(
                hex::encode(
                    vote.extension_sign_bytes(&case.chain_id)
                        .expect("vote extension sign bytes")
                ),
                case.vote_extension_sign_bytes_hex
                    .as_deref()
                    .unwrap_or_default(),
                "vote extension sign bytes mismatch for '{}'",
                case.label
            );
        }

        if let Some(proposal_proto_hex) = &case.proposal_proto_hex {
            let encoded = hex::decode(proposal_proto_hex).expect("valid proposal protobuf hex");
            let proto = <proto::types::Proposal as prost::Message>::decode(encoded.as_slice())
                .unwrap_or_else(|e| panic!("decode proposal for '{}': {e}", case.label));
            let proposal = Proposal::from_proto(proto)
                .unwrap_or_else(|e| panic!("proposal from proto for '{}': {e}", case.label));
            assert_eq!(
                hex::encode(
                    proposal
                        .sign_bytes(&case.chain_id)
                        .expect("proposal sign bytes")
                ),
                case.proposal_sign_bytes_hex.as_deref().unwrap_or_default(),
                "proposal sign bytes mismatch for '{}'",
                case.label
            );
        }
    }
}

fn decode_array<const N: usize>(hex: &str) -> [u8; N] {
    let bytes = hex::decode(hex).expect("valid fixture hex");
    assert_eq!(bytes.len(), N, "fixture byte length");
    bytes
        .try_into()
        .unwrap_or_else(|_| panic!("{N}-byte fixture"))
}
