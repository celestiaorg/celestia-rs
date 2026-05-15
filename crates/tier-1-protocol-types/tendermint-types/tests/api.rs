//! API tests for the tendermint-types crate.

#![allow(clippy::expect_used)]

use tendermint_types::{
    BlockId, BlockIdFlag, Commit, CommitSig, ConsensusVersion, Error, Header, PartSetHeader,
    Proposal, PublicKey, SignedMsgType, Validator, ValidatorSet, Vote,
    canonical::{canonical_time, proposal_sign_bytes, vote_sign_bytes},
    hash::{address_from_bytes, optional_hash_from_bytes},
    proto,
};

fn hash(byte: u8) -> [u8; 32] {
    [byte; 32]
}

fn address(byte: u8) -> [u8; 20] {
    [byte; 20]
}

fn timestamp() -> proto::google::Timestamp {
    proto::google::Timestamp {
        seconds: 1_700_000_000,
        nanos: 123_000_000,
    }
}

fn complete_block_id() -> BlockId {
    BlockId {
        hash: Some(hash(1)),
        part_set_header: PartSetHeader {
            total: 2,
            hash: Some(hash(2)),
        },
    }
}

fn valid_vote() -> Vote {
    Vote {
        ty: SignedMsgType::Precommit,
        height: 10,
        round: 2,
        block_id: complete_block_id(),
        timestamp: Some(timestamp()),
        validator_address: address(14),
        validator_index: 0,
        signature: vec![15; 64],
        extension: b"extension".to_vec(),
        extension_signature: vec![16; 64],
    }
}

fn valid_proposal() -> Proposal {
    Proposal {
        height: 10,
        round: 2,
        pol_round: -1,
        block_id: complete_block_id(),
        timestamp: Some(timestamp()),
        signature: vec![19; 64],
    }
}

#[test]
fn error_display_messages_are_stable() {
    let cases = [
        (
            Error::InvalidHashLength { actual: 3 },
            "invalid CometBFT hash length: 3",
        ),
        (
            Error::InvalidAddressLength { actual: 3 },
            "invalid CometBFT address length: 3",
        ),
        (
            Error::InvalidChainIdLength {
                actual: 51,
                max: 50,
            },
            "invalid CometBFT chain ID length: 51 > 50",
        ),
        (Error::InvalidHeight, "invalid CometBFT height"),
        (Error::InvalidRound, "invalid CometBFT round"),
        (Error::InvalidBlockId, "invalid CometBFT block ID"),
        (
            Error::InvalidPartSetHeader,
            "invalid CometBFT part-set header",
        ),
        (Error::InvalidCommit, "invalid CometBFT commit"),
        (Error::InvalidCommitSig, "invalid CometBFT commit signature"),
        (Error::InvalidValidator, "invalid CometBFT validator"),
        (Error::InvalidValidatorSet, "invalid CometBFT validator set"),
        (Error::InvalidVote, "invalid CometBFT vote"),
        (Error::InvalidProposal, "invalid CometBFT proposal"),
        (Error::InvalidPublicKey, "invalid CometBFT public key"),
        (Error::InvalidTimestamp, "invalid CometBFT timestamp"),
        (
            Error::Decode("boom".to_owned()),
            "decode CometBFT protobuf: boom",
        ),
        (
            Error::Encode("boom".to_owned()),
            "encode CometBFT protobuf: boom",
        ),
    ];

    for (error, message) in cases {
        assert_eq!(error.to_string(), message);
    }
}

#[test]
fn canonical_time_formats_and_rejects_invalid_timestamps() {
    assert_eq!(
        canonical_time(&timestamp()).expect("canonical timestamp"),
        "2023-11-14T22:13:20.123Z"
    );
    assert_eq!(
        canonical_time(&proto::google::Timestamp {
            seconds: 1_700_000_000,
            nanos: 1_000_000_000,
        }),
        Err(Error::InvalidTimestamp)
    );
    assert_eq!(
        canonical_time(&proto::google::Timestamp {
            seconds: i64::MAX,
            nanos: 0,
        }),
        Err(Error::InvalidTimestamp)
    );
}

#[test]
fn hash_and_address_helpers_enforce_lengths() {
    assert_eq!(optional_hash_from_bytes(&[]).expect("empty hash"), None);
    assert_eq!(
        optional_hash_from_bytes(&hash(7)).expect("valid hash"),
        Some(hash(7))
    );
    assert!(optional_hash_from_bytes(&[1, 2, 3]).is_err());

    assert_eq!(
        address_from_bytes(&address(9)).expect("valid address"),
        address(9)
    );
    assert!(address_from_bytes(&[1, 2, 3]).is_err());
}

#[test]
fn block_id_zero_and_complete_match_cometbft_shape() {
    let zero = BlockId::default();
    assert!(zero.is_zero());
    assert!(!zero.is_complete());
    zero.validate_basic().expect("zero block ID is valid");

    let complete = complete_block_id();
    assert!(!complete.is_zero());
    assert!(complete.is_complete());
    complete
        .validate_basic()
        .expect("complete block ID is valid");

    let malformed = BlockId {
        hash: None,
        part_set_header: PartSetHeader {
            total: 1,
            hash: Some(hash(1)),
        },
    };
    assert!(malformed.validate_basic().is_err());
}

#[test]
fn header_round_trips_proto_and_hash_requires_validators_hash() {
    let header = Header {
        version: ConsensusVersion { block: 11, app: 1 },
        chain_id: "celestia-test".to_owned(),
        height: 10,
        time: timestamp(),
        last_block_id: complete_block_id(),
        last_commit_hash: Some(hash(3)),
        data_hash: Some(hash(4)),
        validators_hash: Some(hash(5)),
        next_validators_hash: Some(hash(6)),
        consensus_hash: Some(hash(7)),
        app_hash: vec![8; 32],
        last_results_hash: Some(hash(9)),
        evidence_hash: Some(hash(10)),
        proposer_address: address(11),
    };

    let encoded = header.encode_to_vec().expect("encode header");
    let decoded = Header::decode(&encoded).expect("decode header");
    assert_eq!(decoded, header);
    assert!(header.hash().is_some());

    let mut no_validators_hash = header;
    no_validators_hash.validators_hash = None;
    assert_eq!(no_validators_hash.hash(), None);
}

#[test]
fn commit_hash_and_vote_sign_bytes_are_deterministic() {
    let commit = Commit {
        height: 10,
        round: 1,
        block_id: complete_block_id(),
        signatures: vec![CommitSig {
            block_id_flag: BlockIdFlag::Commit,
            validator_address: Some(address(12)),
            timestamp: Some(timestamp()),
            signature: vec![13; 64],
        }],
    };

    commit.validate_basic().expect("valid commit");
    assert_eq!(commit.size(), 1);
    assert_eq!(commit.hash(), commit.hash());

    let vote = commit
        .get_vote(0)
        .expect("vote conversion")
        .expect("present vote");
    assert_eq!(vote.ty, SignedMsgType::Precommit);
    assert_eq!(
        commit
            .vote_sign_bytes("test-chain", 0)
            .expect("commit vote sign bytes"),
        vote.sign_bytes("test-chain").expect("vote sign bytes")
    );

    let absent = CommitSig::absent();
    absent.validate_basic().expect("absent commit sig");
}

#[test]
fn validators_sort_deduplicate_hash_and_round_trip_proto() {
    let public_key_a = PublicKey::Ed25519([1; 32]);
    let public_key_b = PublicKey::Ed25519([2; 32]);
    let validator_b = Validator::new(public_key_b, 20);
    let validator_a = Validator::new(public_key_a, 10);

    let set = ValidatorSet::new(vec![validator_b.clone(), validator_a.clone()])
        .expect("valid validator set");
    assert_eq!(set.validators[0].address, validator_b.address);
    assert_eq!(set.total_voting_power().expect("total power"), 30);
    assert_eq!(
        set.hash().expect("validator set hash"),
        set.hash().expect("again")
    );

    let proto = set.to_proto();
    let decoded = ValidatorSet::from_proto(proto).expect("validator set from proto");
    assert_eq!(decoded.validators, set.validators);

    assert!(ValidatorSet::new(vec![validator_b.clone(), validator_b]).is_err());
}

#[test]
fn validator_set_rejects_empty_bad_proposer_duplicates_and_overflow() {
    assert!(ValidatorSet::new(Vec::new()).is_err());

    let validator_a_high = Validator::new(PublicKey::Ed25519([1; 32]), 30);
    let validator_b = Validator::new(PublicKey::Ed25519([2; 32]), 20);
    let validator_a_low = Validator::new(PublicKey::Ed25519([1; 32]), 10);
    assert!(
        ValidatorSet::new(vec![
            validator_a_high.clone(),
            validator_b.clone(),
            validator_a_low,
        ])
        .is_err()
    );

    let mut set = ValidatorSet::new(vec![validator_a_high, validator_b]).expect("validator set");
    set.proposer = None;
    assert_eq!(set.validate_basic(), Err(Error::InvalidValidatorSet));

    set.proposer = Some(Validator::new(PublicKey::Ed25519([3; 32]), 1));
    assert_eq!(set.validate_basic(), Err(Error::InvalidValidatorSet));

    let overflow = ValidatorSet::new(vec![
        Validator::new(PublicKey::Ed25519([4; 32]), i64::MAX),
        Validator::new(PublicKey::Ed25519([5; 32]), 1),
    ])
    .expect("validator set");
    assert_eq!(
        overflow.total_voting_power(),
        Err(Error::InvalidValidatorSet)
    );
}

#[test]
fn vote_proto_and_canonical_sign_bytes_work() {
    let vote = valid_vote();
    vote.validate_basic().expect("valid vote");
    let proto = vote.to_proto();
    assert_eq!(Vote::from_proto(proto).expect("vote from proto"), vote);
    assert_eq!(
        vote.sign_bytes("test-chain").expect("vote sign bytes"),
        vote_sign_bytes("test-chain", &vote).expect("free sign bytes")
    );
    assert_eq!(
        vote.commit_sig().expect("commit sig").block_id_flag,
        BlockIdFlag::Commit
    );
    assert!(!vote.verify("test-chain", &PublicKey::Ed25519([3; 32])));
}

#[test]
fn vote_validation_rejects_incomplete_blocks_and_bad_extensions() {
    let mut vote = valid_vote();
    vote.block_id = BlockId {
        hash: Some(hash(1)),
        part_set_header: PartSetHeader::default(),
    };
    assert_eq!(vote.validate_basic(), Err(Error::InvalidBlockId));

    vote = valid_vote();
    vote.validator_index = -1;
    assert_eq!(vote.validate_basic(), Err(Error::InvalidVote));

    vote = valid_vote();
    vote.ty = SignedMsgType::Prevote;
    assert_eq!(vote.validate_basic(), Err(Error::InvalidVote));

    vote = valid_vote();
    vote.block_id = BlockId::default();
    assert_eq!(vote.validate_basic(), Err(Error::InvalidVote));

    vote = valid_vote();
    vote.extension_signature.clear();
    assert_eq!(vote.validate_basic(), Err(Error::InvalidVote));

    vote = valid_vote();
    vote.extension_signature = vec![16; 65];
    assert_eq!(vote.validate_basic(), Err(Error::InvalidVote));
}

#[test]
fn proposal_proto_canonical_sign_bytes_and_validation_work() {
    let proposal = valid_proposal();
    proposal.validate_basic().expect("valid proposal");
    let proto = proposal.to_proto();
    assert_eq!(
        Proposal::from_proto(proto).expect("proposal from proto"),
        proposal
    );
    assert_eq!(
        proposal
            .sign_bytes("test-chain")
            .expect("proposal sign bytes"),
        proposal_sign_bytes("test-chain", &proposal).expect("free proposal sign bytes")
    );
    assert!(!proposal.verify("test-chain", &PublicKey::Ed25519([3; 32])));

    let mut zero_height = proposal.clone();
    zero_height.height = 0;
    zero_height
        .validate_basic()
        .expect("CometBFT allows zero-height proposals");

    let mut negative_round = proposal.clone();
    negative_round.round = -1;
    assert_eq!(negative_round.validate_basic(), Err(Error::InvalidRound));

    let mut negative_pol_round = proposal.clone();
    negative_pol_round.pol_round = -2;
    assert_eq!(
        negative_pol_round.validate_basic(),
        Err(Error::InvalidRound)
    );
}
