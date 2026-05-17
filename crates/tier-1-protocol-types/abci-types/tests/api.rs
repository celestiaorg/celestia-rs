//! API tests for the abci-types crate.

#![allow(clippy::expect_used, clippy::too_many_lines)]

use abci_types::{
    CommitInfo, Error, Event, EventAttribute, ExecTxResult, ExtendedCommitInfo, ExtendedVoteInfo,
    Misbehavior, MisbehaviorType, Request, RequestValue, Response, ResponseValue, Snapshot,
    Validator, ValidatorUpdate, VoteInfo, proto,
};
use tendermint_types::{BlockIdFlag, PublicKey};

fn address(byte: u8) -> [u8; 20] {
    [byte; 20]
}

fn hash(byte: u8) -> Vec<u8> {
    vec![byte; 32]
}

fn timestamp() -> proto::google::Timestamp {
    proto::google::Timestamp {
        seconds: 1_700_000_000,
        nanos: 123_000_000,
    }
}

fn validator() -> Validator {
    Validator {
        address: address(1),
        power: 10,
    }
}

fn validator_update() -> ValidatorUpdate {
    ValidatorUpdate {
        public_key: PublicKey::Ed25519([2; 32]),
        power: 11,
    }
}

fn event() -> Event {
    Event {
        ty: "transfer".to_owned(),
        attributes: vec![EventAttribute {
            key: "sender".to_owned(),
            value: "celestia1sender".to_owned(),
            index: true,
        }],
    }
}

fn exec_result() -> ExecTxResult {
    ExecTxResult {
        code: 0,
        data: b"data".to_vec(),
        log: "log".to_owned(),
        info: "info".to_owned(),
        gas_wanted: -1,
        gas_used: -2,
        events: vec![event()],
        codespace: "bank".to_owned(),
    }
}

fn snapshot() -> Snapshot {
    Snapshot {
        height: 10,
        format: 1,
        chunks: 2,
        hash: b"snapshot-hash".to_vec(),
        metadata: b"metadata".to_vec(),
    }
}

fn vote_info() -> VoteInfo {
    VoteInfo {
        validator: validator(),
        block_id_flag: BlockIdFlag::Commit,
    }
}

fn extended_vote_info() -> ExtendedVoteInfo {
    ExtendedVoteInfo {
        validator: validator(),
        vote_extension: b"extension".to_vec(),
        extension_signature: vec![3; 64],
        block_id_flag: BlockIdFlag::Commit,
    }
}

fn commit_info() -> CommitInfo {
    CommitInfo {
        round: 1,
        votes: vec![vote_info()],
    }
}

fn extended_commit_info() -> ExtendedCommitInfo {
    ExtendedCommitInfo {
        round: 1,
        votes: vec![extended_vote_info()],
    }
}

fn misbehavior() -> Misbehavior {
    Misbehavior {
        ty: MisbehaviorType::DuplicateVote,
        validator: validator(),
        height: 9,
        time: timestamp(),
        total_voting_power: 100,
    }
}

#[test]
fn error_display_messages_are_stable() {
    let cases = [
        (
            Error::InvalidHashLength { actual: 3 },
            "invalid ABCI hash length: 3",
        ),
        (
            Error::InvalidAddressLength { actual: 3 },
            "invalid ABCI address length: 3",
        ),
        (Error::InvalidHeight, "invalid ABCI height"),
        (Error::InvalidRound, "invalid ABCI round"),
        (Error::InvalidEvent, "invalid ABCI event"),
        (
            Error::InvalidExecTxResult,
            "invalid ABCI tx execution result",
        ),
        (Error::InvalidValidator, "invalid ABCI validator"),
        (
            Error::InvalidValidatorUpdate,
            "invalid ABCI validator update",
        ),
        (Error::InvalidVoteInfo, "invalid ABCI vote info"),
        (Error::InvalidCommitInfo, "invalid ABCI commit info"),
        (Error::InvalidMisbehavior, "invalid ABCI misbehavior"),
        (Error::InvalidSnapshot, "invalid ABCI snapshot"),
        (Error::InvalidRequest, "invalid ABCI request"),
        (Error::InvalidResponse, "invalid ABCI response"),
        (Error::InvalidStatus, "invalid ABCI status"),
        (Error::InvalidPublicKey, "invalid ABCI public key"),
        (
            Error::Decode("boom".to_owned()),
            "decode ABCI protobuf: boom",
        ),
        (
            Error::Encode("boom".to_owned()),
            "encode ABCI protobuf: boom",
        ),
    ];

    for (error, message) in cases {
        assert_eq!(error.to_string(), message);
    }
}

#[test]
fn events_and_exec_results_validate_and_round_trip_proto() {
    let event = event();
    assert_eq!(
        Event::from_proto(event.to_proto()).expect("event proto"),
        event
    );
    assert_eq!(
        EventAttribute {
            key: String::new(),
            value: String::new(),
            index: false,
        }
        .validate_basic(),
        Err(Error::InvalidEvent)
    );
    assert_eq!(
        Event {
            ty: String::new(),
            attributes: Vec::new(),
        }
        .validate_basic(),
        Err(Error::InvalidEvent)
    );

    let result = exec_result();
    assert_eq!(
        ExecTxResult::from_proto(result.to_proto()).expect("exec result proto"),
        result
    );
}

#[test]
fn validators_misbehavior_commits_and_snapshots_validate() {
    assert_eq!(
        Validator::from_proto(validator().to_proto()).expect("validator proto"),
        validator()
    );
    assert_eq!(
        Validator::from_proto(proto::abci::Validator {
            address: vec![1, 2, 3].into(),
            power: 1,
        }),
        Err(Error::InvalidAddressLength { actual: 3 })
    );
    assert_eq!(
        Validator {
            address: address(1),
            power: -1,
        }
        .validate_basic(),
        Err(Error::InvalidValidator)
    );

    assert_eq!(
        ValidatorUpdate::from_proto(validator_update().to_proto()).expect("validator update"),
        validator_update()
    );
    assert_eq!(
        ValidatorUpdate {
            public_key: PublicKey::Ed25519([2; 32]),
            power: -1,
        }
        .validate_basic(),
        Err(Error::InvalidValidatorUpdate)
    );

    assert_eq!(
        Misbehavior::from_proto(misbehavior().to_proto()).expect("misbehavior proto"),
        misbehavior()
    );
    assert_eq!(
        MisbehaviorType::try_from(99),
        Err(Error::InvalidMisbehavior)
    );
    assert_eq!(
        Misbehavior {
            height: 0,
            ..misbehavior()
        }
        .validate_basic(),
        Err(Error::InvalidMisbehavior)
    );

    assert_eq!(
        CommitInfo::from_proto(commit_info().to_proto()).expect("commit info proto"),
        commit_info()
    );
    assert_eq!(
        ExtendedCommitInfo::from_proto(extended_commit_info().to_proto())
            .expect("extended commit info proto"),
        extended_commit_info()
    );
    assert_eq!(
        CommitInfo {
            round: -1,
            votes: Vec::new(),
        }
        .validate_basic(),
        Err(Error::InvalidRound)
    );
    assert_eq!(
        ExtendedVoteInfo {
            extension_signature: vec![0; 65],
            ..extended_vote_info()
        }
        .validate_basic(),
        Err(Error::InvalidVoteInfo)
    );

    assert_eq!(
        Snapshot::from_proto(snapshot().to_proto()).expect("snapshot proto"),
        snapshot()
    );
    assert_eq!(
        Snapshot {
            height: 0,
            ..snapshot()
        }
        .validate_basic(),
        Err(Error::InvalidSnapshot)
    );
}

#[test]
fn request_envelopes_round_trip_all_variants_and_reject_malformed_values() {
    let requests = vec![
        Request {
            value: RequestValue::Echo(proto::abci::RequestEcho {
                message: "hello".to_owned(),
            }),
        },
        Request {
            value: RequestValue::Flush,
        },
        Request {
            value: RequestValue::Info(proto::abci::RequestInfo {
                version: "v1".to_owned(),
                block_version: 11,
                p2p_version: 12,
                abci_version: "1.0".to_owned(),
            }),
        },
        Request {
            value: RequestValue::InitChain(proto::abci::RequestInitChain {
                time: Some(timestamp()),
                chain_id: "test-chain".to_owned(),
                consensus_params: None,
                validators: vec![validator_update().to_proto()],
                app_state_bytes: b"state".to_vec().into(),
                initial_height: 1,
            }),
        },
        Request {
            value: RequestValue::Query(proto::abci::RequestQuery {
                data: b"query".to_vec().into(),
                path: "/store".to_owned(),
                height: 0,
                prove: true,
            }),
        },
        Request {
            value: RequestValue::CheckTx(proto::abci::RequestCheckTx {
                tx: b"tx".to_vec().into(),
                r#type: proto::abci::CheckTxType::New.into(),
            }),
        },
        Request {
            value: RequestValue::Commit,
        },
        Request {
            value: RequestValue::ListSnapshots,
        },
        Request {
            value: RequestValue::OfferSnapshot(proto::abci::RequestOfferSnapshot {
                snapshot: Some(snapshot().to_proto()),
                app_hash: hash(8).into(),
            }),
        },
        Request {
            value: RequestValue::LoadSnapshotChunk(proto::abci::RequestLoadSnapshotChunk {
                height: 10,
                format: 1,
                chunk: 0,
            }),
        },
        Request {
            value: RequestValue::ApplySnapshotChunk(proto::abci::RequestApplySnapshotChunk {
                index: 0,
                chunk: b"chunk".to_vec().into(),
                sender: "peer".to_owned(),
            }),
        },
        Request {
            value: RequestValue::PrepareProposal(proto::abci::RequestPrepareProposal {
                max_tx_bytes: 1024,
                txs: vec![b"tx1".to_vec().into()],
                local_last_commit: Some(extended_commit_info().to_proto()),
                misbehavior: vec![misbehavior().to_proto()],
                height: 11,
                time: Some(timestamp()),
                next_validators_hash: hash(9).into(),
                proposer_address: address(1).to_vec().into(),
            }),
        },
        Request {
            value: RequestValue::ProcessProposal(proto::abci::RequestProcessProposal {
                txs: vec![b"tx1".to_vec().into()],
                proposed_last_commit: Some(commit_info().to_proto()),
                misbehavior: vec![misbehavior().to_proto()],
                hash: hash(10).into(),
                height: 11,
                time: Some(timestamp()),
                next_validators_hash: hash(11).into(),
                proposer_address: address(1).to_vec().into(),
            }),
        },
        Request {
            value: RequestValue::ExtendVote(proto::abci::RequestExtendVote {
                hash: hash(12).into(),
                height: 11,
                time: Some(timestamp()),
                txs: vec![b"tx1".to_vec().into()],
                proposed_last_commit: Some(commit_info().to_proto()),
                misbehavior: vec![misbehavior().to_proto()],
                next_validators_hash: hash(13).into(),
                proposer_address: address(1).to_vec().into(),
            }),
        },
        Request {
            value: RequestValue::VerifyVoteExtension(proto::abci::RequestVerifyVoteExtension {
                hash: hash(14).into(),
                validator_address: address(1).to_vec().into(),
                height: 11,
                vote_extension: b"extension".to_vec().into(),
            }),
        },
        Request {
            value: RequestValue::FinalizeBlock(proto::abci::RequestFinalizeBlock {
                txs: vec![b"tx1".to_vec().into()],
                decided_last_commit: Some(commit_info().to_proto()),
                misbehavior: vec![misbehavior().to_proto()],
                hash: hash(15).into(),
                height: 11,
                time: Some(timestamp()),
                next_validators_hash: hash(16).into(),
                proposer_address: address(1).to_vec().into(),
            }),
        },
    ];

    for request in requests {
        let encoded = request.encode_to_vec().expect("encode request");
        let decoded = Request::decode(&encoded).expect("decode request");
        assert_eq!(decoded, request);
    }

    assert_eq!(
        Request::from_proto(proto::abci::Request { value: None }),
        Err(Error::InvalidRequest)
    );
    assert!(matches!(Request::decode(&[0xff]), Err(Error::Decode(_))));
    assert_eq!(
        Request {
            value: RequestValue::CheckTx(proto::abci::RequestCheckTx {
                tx: Vec::new().into(),
                r#type: 99,
            }),
        }
        .validate_basic(),
        Err(Error::InvalidStatus)
    );
    assert_eq!(
        Request {
            value: RequestValue::VerifyVoteExtension(proto::abci::RequestVerifyVoteExtension {
                hash: vec![1, 2, 3].into(),
                validator_address: address(1).to_vec().into(),
                height: 1,
                vote_extension: Vec::new().into(),
            }),
        }
        .validate_basic(),
        Err(Error::InvalidHashLength { actual: 3 })
    );
}

#[test]
fn response_envelopes_round_trip_all_variants_and_reject_malformed_values() {
    let responses = vec![
        Response {
            value: ResponseValue::Exception(proto::abci::ResponseException {
                error: "boom".to_owned(),
            }),
        },
        Response {
            value: ResponseValue::Echo(proto::abci::ResponseEcho {
                message: "hello".to_owned(),
            }),
        },
        Response {
            value: ResponseValue::Flush,
        },
        Response {
            value: ResponseValue::Info(proto::abci::ResponseInfo {
                data: "data".to_owned(),
                version: "v1".to_owned(),
                app_version: 1,
                last_block_height: 10,
                last_block_app_hash: hash(17).into(),
            }),
        },
        Response {
            value: ResponseValue::InitChain(proto::abci::ResponseInitChain {
                consensus_params: None,
                validators: vec![validator_update().to_proto()],
                app_hash: hash(18).into(),
            }),
        },
        Response {
            value: ResponseValue::Query(proto::abci::ResponseQuery {
                code: 0,
                log: "log".to_owned(),
                info: "info".to_owned(),
                index: 1,
                key: b"key".to_vec().into(),
                value: b"value".to_vec().into(),
                proof_ops: None,
                height: 10,
                codespace: "bank".to_owned(),
            }),
        },
        Response {
            value: ResponseValue::CheckTx(proto::abci::ResponseCheckTx {
                code: 0,
                data: b"data".to_vec().into(),
                log: "log".to_owned(),
                info: "info".to_owned(),
                gas_wanted: 1,
                gas_used: 1,
                events: vec![event().to_proto()],
                codespace: "bank".to_owned(),
            }),
        },
        Response {
            value: ResponseValue::Commit(proto::abci::ResponseCommit { retain_height: 1 }),
        },
        Response {
            value: ResponseValue::ListSnapshots(proto::abci::ResponseListSnapshots {
                snapshots: vec![snapshot().to_proto()],
            }),
        },
        Response {
            value: ResponseValue::OfferSnapshot(proto::abci::ResponseOfferSnapshot {
                result: proto::abci::response_offer_snapshot::Result::Accept.into(),
            }),
        },
        Response {
            value: ResponseValue::LoadSnapshotChunk(proto::abci::ResponseLoadSnapshotChunk {
                chunk: b"chunk".to_vec().into(),
            }),
        },
        Response {
            value: ResponseValue::ApplySnapshotChunk(proto::abci::ResponseApplySnapshotChunk {
                result: proto::abci::response_apply_snapshot_chunk::Result::Accept.into(),
                refetch_chunks: vec![1, 2],
                reject_senders: vec!["peer".to_owned()],
            }),
        },
        Response {
            value: ResponseValue::PrepareProposal(proto::abci::ResponsePrepareProposal {
                txs: vec![b"tx1".to_vec().into()],
            }),
        },
        Response {
            value: ResponseValue::ProcessProposal(proto::abci::ResponseProcessProposal {
                status: proto::abci::response_process_proposal::ProposalStatus::Accept.into(),
            }),
        },
        Response {
            value: ResponseValue::ExtendVote(proto::abci::ResponseExtendVote {
                vote_extension: b"extension".to_vec().into(),
            }),
        },
        Response {
            value: ResponseValue::VerifyVoteExtension(proto::abci::ResponseVerifyVoteExtension {
                status: proto::abci::response_verify_vote_extension::VerifyStatus::Accept.into(),
            }),
        },
        Response {
            value: ResponseValue::FinalizeBlock(proto::abci::ResponseFinalizeBlock {
                events: vec![event().to_proto()],
                tx_results: vec![exec_result().to_proto()],
                validator_updates: vec![validator_update().to_proto()],
                consensus_param_updates: None,
                app_hash: hash(19).into(),
            }),
        },
    ];

    for response in responses {
        let encoded = response.encode_to_vec().expect("encode response");
        let decoded = Response::decode(&encoded).expect("decode response");
        assert_eq!(decoded, response);
    }

    assert_eq!(
        Response::from_proto(proto::abci::Response { value: None }),
        Err(Error::InvalidResponse)
    );
    assert!(matches!(Response::decode(&[0xff]), Err(Error::Decode(_))));
    assert_eq!(
        Response {
            value: ResponseValue::VerifyVoteExtension(proto::abci::ResponseVerifyVoteExtension {
                status: 99
            },),
        }
        .validate_basic(),
        Err(Error::InvalidStatus)
    );
    assert_eq!(
        Response {
            value: ResponseValue::ProcessProposal(proto::abci::ResponseProcessProposal {
                status: proto::abci::response_process_proposal::ProposalStatus::Unknown.into(),
            }),
        }
        .validate_basic(),
        Err(Error::InvalidStatus)
    );
}
