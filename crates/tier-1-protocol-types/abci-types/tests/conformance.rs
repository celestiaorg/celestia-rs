//! Conformance tests for abci-types against committed fixtures.

#![allow(clippy::expect_used, clippy::panic)]

use abci_types::{
    CommitInfo, Error, Event, EventAttribute, ExecTxResult, ExtendedCommitInfo, ExtendedVoteInfo,
    Misbehavior, Request, RequestValue, Response, ResponseValue, Snapshot, Validator,
    ValidatorUpdate, VoteInfo, proto,
};
use prost::Message;
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
struct EnvelopeFixture {
    cases: Vec<EnvelopeCase>,
}

#[derive(Deserialize)]
struct EnvelopeCase {
    label: String,
    variant: String,
    proto_hex: String,
}

#[derive(Deserialize)]
struct VoteExtensionFixture {
    cases: Vec<VoteExtensionCase>,
}

#[derive(Deserialize)]
struct VoteExtensionCase {
    label: String,
    kind: String,
    proto_hex: String,
    vote_extension_hex: Option<String>,
    status: Option<String>,
}

#[derive(Deserialize)]
struct EventFixture {
    cases: Vec<EventCase>,
}

#[derive(Deserialize)]
struct EventCase {
    label: String,
    kind: String,
    proto_hex: String,
}

#[derive(Deserialize)]
struct MalformedFixture {
    cases: Vec<MalformedCase>,
}

#[derive(Deserialize)]
struct MalformedCase {
    label: String,
    kind: String,
    proto_hex: String,
    expected_error: String,
}

#[test]
fn conformance_requests() {
    let fixture: EnvelopeFixture = load_fixture("abci-types/requests.json");

    for case in &fixture.cases {
        let encoded = decode_fixture_hex(&case.proto_hex);
        let request = Request::decode(&encoded)
            .unwrap_or_else(|e| panic!("decode request for '{}': {e}", case.label));
        assert_eq!(
            request_variant(&request),
            case.variant,
            "request variant mismatch for '{}'",
            case.label
        );
        assert_eq!(
            request.encode_to_vec().expect("encode request"),
            encoded,
            "request protobuf round-trip mismatch for '{}'",
            case.label
        );
    }
}

#[test]
fn conformance_responses() {
    let fixture: EnvelopeFixture = load_fixture("abci-types/responses.json");

    for case in &fixture.cases {
        let encoded = decode_fixture_hex(&case.proto_hex);
        let response = Response::decode(&encoded)
            .unwrap_or_else(|e| panic!("decode response for '{}': {e}", case.label));
        assert_eq!(
            response_variant(&response),
            case.variant,
            "response variant mismatch for '{}'",
            case.label
        );
        assert_eq!(
            response.encode_to_vec().expect("encode response"),
            encoded,
            "response protobuf round-trip mismatch for '{}'",
            case.label
        );
    }
}

#[test]
fn conformance_vote_extensions() {
    let fixture: VoteExtensionFixture = load_fixture("abci-types/vote-extensions.json");

    for case in &fixture.cases {
        let encoded = decode_fixture_hex(&case.proto_hex);
        match case.kind.as_str() {
            "commit_info" => {
                let commit = CommitInfo::from_proto(decode_proto(&encoded))
                    .unwrap_or_else(|e| panic!("commit info for '{}': {e}", case.label));
                assert_eq!(commit.round, 1);
                assert_eq!(commit.votes.len(), 1);
            }
            "extended_commit_info" => {
                let commit = ExtendedCommitInfo::from_proto(decode_proto(&encoded))
                    .unwrap_or_else(|e| panic!("extended commit info for '{}': {e}", case.label));
                assert_eq!(commit.round, 1);
                assert_eq!(
                    hex::encode(&commit.votes[0].vote_extension),
                    case.vote_extension_hex
                        .as_deref()
                        .unwrap_or_else(|| panic!("missing vote extension for '{}'", case.label))
                );
            }
            "request_extend_vote" => {
                let proto = decode_proto::<proto::abci::RequestExtendVote>(&encoded);
                Request {
                    value: RequestValue::ExtendVote(proto),
                }
                .validate_basic()
                .unwrap_or_else(|e| panic!("request extend vote for '{}': {e}", case.label));
            }
            "request_verify_vote_extension" => {
                let proto = decode_proto::<proto::abci::RequestVerifyVoteExtension>(&encoded);
                assert_eq!(
                    hex::encode(&proto.vote_extension),
                    case.vote_extension_hex.as_deref().unwrap_or_else(|| {
                        panic!("missing vote extension for '{}'", case.label)
                    })
                );
                Request {
                    value: RequestValue::VerifyVoteExtension(proto),
                }
                .validate_basic()
                .unwrap_or_else(|e| {
                    panic!("request verify vote extension for '{}': {e}", case.label)
                });
            }
            "response_extend_vote" => {
                let proto = decode_proto::<proto::abci::ResponseExtendVote>(&encoded);
                assert_eq!(
                    hex::encode(&proto.vote_extension),
                    case.vote_extension_hex.as_deref().unwrap_or_else(|| {
                        panic!("missing vote extension for '{}'", case.label)
                    })
                );
                Response {
                    value: ResponseValue::ExtendVote(proto),
                }
                .validate_basic()
                .unwrap_or_else(|e| panic!("response extend vote for '{}': {e}", case.label));
            }
            "response_verify_vote_extension" => {
                let proto = decode_proto::<proto::abci::ResponseVerifyVoteExtension>(&encoded);
                let expected = case
                    .status
                    .as_deref()
                    .unwrap_or_else(|| panic!("missing status for '{}'", case.label));
                assert_eq!(expected, "accept");
                Response {
                    value: ResponseValue::VerifyVoteExtension(proto),
                }
                .validate_basic()
                .unwrap_or_else(|e| {
                    panic!("response verify vote extension for '{}': {e}", case.label)
                });
            }
            other => panic!("unknown vote-extension kind '{other}'"),
        }
    }
}

#[test]
fn conformance_events() {
    let fixture: EventFixture = load_fixture("abci-types/events.json");

    for case in &fixture.cases {
        let encoded = decode_fixture_hex(&case.proto_hex);
        match case.kind.as_str() {
            "event" => {
                let event = Event::from_proto(decode_proto(&encoded))
                    .unwrap_or_else(|e| panic!("event for '{}': {e}", case.label));
                assert_eq!(event.ty, "transfer");
                assert_eq!(event.attributes.len(), 1);
            }
            "event_attribute" => {
                let attribute = EventAttribute::from_proto(decode_proto(&encoded))
                    .unwrap_or_else(|e| panic!("event attribute for '{}': {e}", case.label));
                assert_eq!(attribute.key, "sender");
                assert!(attribute.index);
            }
            "exec_tx_result" => {
                let result = ExecTxResult::from_proto(decode_proto(&encoded))
                    .unwrap_or_else(|e| panic!("exec tx result for '{}': {e}", case.label));
                assert_eq!(result.gas_wanted, -1);
                assert_eq!(result.gas_used, -2);
                assert_eq!(result.events.len(), 1);
            }
            "response_check_tx" => {
                let proto = decode_proto::<proto::abci::ResponseCheckTx>(&encoded);
                Response {
                    value: ResponseValue::CheckTx(proto),
                }
                .validate_basic()
                .unwrap_or_else(|e| panic!("response check tx for '{}': {e}", case.label));
            }
            "response_finalize_block" => {
                let proto = decode_proto::<proto::abci::ResponseFinalizeBlock>(&encoded);
                Response {
                    value: ResponseValue::FinalizeBlock(proto),
                }
                .validate_basic()
                .unwrap_or_else(|e| panic!("response finalize block for '{}': {e}", case.label));
            }
            other => panic!("unknown event kind '{other}'"),
        }
    }
}

#[test]
fn conformance_malformed() {
    let fixture: MalformedFixture = load_fixture("abci-types/malformed.json");

    for case in &fixture.cases {
        let result = malformed_result(case);
        let Err(error) = result else {
            panic!("malformed case '{}' unexpectedly succeeded", case.label);
        };
        assert_eq!(
            error_code(&error),
            case.expected_error,
            "malformed error mismatch for '{}'",
            case.label
        );
    }
}

fn malformed_result(case: &MalformedCase) -> Result<(), Error> {
    let encoded = decode_fixture_hex(&case.proto_hex);
    match case.kind.as_str() {
        "request" => Request::decode(&encoded).map(|_| ()),
        "response" => Response::decode(&encoded).map(|_| ()),
        "event" => Event::from_proto(decode_proto(&encoded)).map(|_| ()),
        "event_attribute" => EventAttribute::from_proto(decode_proto(&encoded)).map(|_| ()),
        "validator" => Validator::from_proto(decode_proto(&encoded)).map(|_| ()),
        "validator_update" => ValidatorUpdate::from_proto(decode_proto(&encoded)).map(|_| ()),
        "commit_info" => CommitInfo::from_proto(decode_proto(&encoded)).map(|_| ()),
        "vote_info" => VoteInfo::from_proto(decode_proto(&encoded)).map(|_| ()),
        "extended_vote_info" => ExtendedVoteInfo::from_proto(decode_proto(&encoded)).map(|_| ()),
        "misbehavior" => Misbehavior::from_proto(decode_proto(&encoded)).map(|_| ()),
        "snapshot" => Snapshot::from_proto(decode_proto(&encoded)).map(|_| ()),
        other => panic!("unknown malformed kind '{other}'"),
    }
}

fn request_variant(request: &Request) -> &'static str {
    match &request.value {
        RequestValue::Echo(_) => "echo",
        RequestValue::Flush => "flush",
        RequestValue::Info(_) => "info",
        RequestValue::InitChain(_) => "init_chain",
        RequestValue::Query(_) => "query",
        RequestValue::CheckTx(_) => "check_tx",
        RequestValue::Commit => "commit",
        RequestValue::ListSnapshots => "list_snapshots",
        RequestValue::OfferSnapshot(_) => "offer_snapshot",
        RequestValue::LoadSnapshotChunk(_) => "load_snapshot_chunk",
        RequestValue::ApplySnapshotChunk(_) => "apply_snapshot_chunk",
        RequestValue::PrepareProposal(_) => "prepare_proposal",
        RequestValue::ProcessProposal(_) => "process_proposal",
        RequestValue::ExtendVote(_) => "extend_vote",
        RequestValue::VerifyVoteExtension(_) => "verify_vote_extension",
        RequestValue::FinalizeBlock(_) => "finalize_block",
    }
}

fn response_variant(response: &Response) -> &'static str {
    match &response.value {
        ResponseValue::Exception(_) => "exception",
        ResponseValue::Echo(_) => "echo",
        ResponseValue::Flush => "flush",
        ResponseValue::Info(_) => "info",
        ResponseValue::InitChain(_) => "init_chain",
        ResponseValue::Query(_) => "query",
        ResponseValue::CheckTx(_) => "check_tx",
        ResponseValue::Commit(_) => "commit",
        ResponseValue::ListSnapshots(_) => "list_snapshots",
        ResponseValue::OfferSnapshot(_) => "offer_snapshot",
        ResponseValue::LoadSnapshotChunk(_) => "load_snapshot_chunk",
        ResponseValue::ApplySnapshotChunk(_) => "apply_snapshot_chunk",
        ResponseValue::PrepareProposal(_) => "prepare_proposal",
        ResponseValue::ProcessProposal(_) => "process_proposal",
        ResponseValue::ExtendVote(_) => "extend_vote",
        ResponseValue::VerifyVoteExtension(_) => "verify_vote_extension",
        ResponseValue::FinalizeBlock(_) => "finalize_block",
    }
}

fn decode_fixture_hex(hex_value: &str) -> Vec<u8> {
    hex::decode(hex_value).expect("valid fixture hex")
}

fn decode_proto<T>(bytes: &[u8]) -> T
where
    T: Message + Default,
{
    <T as Message>::decode(bytes).expect("valid protobuf fixture")
}

fn error_code(error: &Error) -> &'static str {
    match error {
        Error::InvalidHashLength { .. } => "invalid_hash_length",
        Error::InvalidAddressLength { .. } => "invalid_address_length",
        Error::InvalidHeight => "invalid_height",
        Error::InvalidRound => "invalid_round",
        Error::InvalidEvent => "invalid_event",
        Error::InvalidExecTxResult => "invalid_exec_tx_result",
        Error::InvalidValidator => "invalid_validator",
        Error::InvalidValidatorUpdate => "invalid_validator_update",
        Error::InvalidVoteInfo => "invalid_vote_info",
        Error::InvalidCommitInfo => "invalid_commit_info",
        Error::InvalidMisbehavior => "invalid_misbehavior",
        Error::InvalidSnapshot => "invalid_snapshot",
        Error::InvalidRequest => "invalid_request",
        Error::InvalidResponse => "invalid_response",
        Error::InvalidStatus => "invalid_status",
        Error::InvalidPublicKey => "invalid_public_key",
        Error::Decode(_) => "decode",
        Error::Encode(_) => "encode",
    }
}
