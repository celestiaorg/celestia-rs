//! Unit tests ported from Go `types/vote_set_test.go`,
//! `consensus/types/height_vote_set_test.go`, `consensus/wal_test.go` and
//! `consensus/ticker` semantics.

use ctypes::vote::{PRECOMMIT_TYPE, PREVOTE_TYPE};
use ctypes::{BlockId, PartSetHeader, PubKey, Time, Validator, ValidatorSet, Vote};

use crate::height_vote_set::HeightVoteSet;
use crate::ticker::TimeoutTicker;
use crate::types::{ConsensusMessage, RoundStep, TimeoutInfo};
use crate::vote_set::{BitArray, VoteError, VoteSet};
use crate::wal::{MemWal, TimedWalMessage, Wal, WalError, WalMessage, crc32c, decode_all, encode};

const CHAIN: &str = "test_chain_id";

struct Keys(Vec<crypto::ed25519::PrivateKeyBytes>);

fn val_set(n: u8, power: i64) -> (ValidatorSet, Keys) {
    let keys: Vec<_> = (0..n)
        .map(|i| crypto::ed25519::private_key_from_seed(&[i + 1; 32]))
        .collect();
    let vals: Vec<Validator> = keys
        .iter()
        .map(|k| {
            Validator::new(
                PubKey::Ed25519(crypto::ed25519::public_key_from_private_key(k).unwrap()),
                power,
            )
        })
        .collect();
    let vs = ValidatorSet::new(&vals).unwrap();
    // Keys in validator-set order.
    let ordered = vs
        .validators
        .iter()
        .map(|v| {
            *keys
                .iter()
                .find(|k| {
                    crypto::ed25519::address(
                        &crypto::ed25519::public_key_from_private_key(k).unwrap(),
                    )
                    .as_slice()
                        == v.address.as_slice()
                })
                .unwrap()
        })
        .collect();
    (vs, Keys(ordered))
}

fn block_id(hash: &[u8]) -> BlockId {
    if hash.is_empty() {
        return BlockId::default();
    }
    BlockId {
        hash: hash.to_vec(),
        part_set_header: PartSetHeader {
            total: 1,
            hash: hash::sha256(hash).to_vec(),
        },
    }
}

fn signed_vote(
    vs: &ValidatorSet,
    keys: &Keys,
    idx: usize,
    t: i32,
    h: i64,
    r: i32,
    hash: &[u8],
) -> Vote {
    let mut v = Vote {
        vote_type: t,
        height: h,
        round: r,
        block_id: block_id(hash),
        timestamp: Time::from_unix(1_700_000_000, 0),
        validator_address: vs.validators[idx].address.clone(),
        validator_index: i32::try_from(idx).unwrap(),
        ..Vote::default()
    };
    let sb = ctypes::canonical::vote_sign_bytes(CHAIN, &v.to_proto()).unwrap();
    v.signature = crypto::ed25519::sign(&keys.0[idx], &sb).unwrap().to_vec();
    v
}

#[test]
fn vote_set_add_vote_good_and_bad() {
    let (vs, keys) = val_set(10, 1);
    let mut set = VoteSet::new(CHAIN, 1, 0, PREVOTE_TYPE, vs.clone());
    assert!(set.get_by_index(0).is_none());
    assert!(!set.has_two_thirds_any());
    let v = signed_vote(&vs, &keys, 0, PREVOTE_TYPE, 1, 0, &[]);
    assert_eq!(set.add_vote(&v), Ok(true));
    assert!(set.get_by_index(0).is_some());
    assert!(set.bit_array().get(0));
    // Duplicate.
    assert_eq!(set.add_vote(&v), Ok(false));
    // Wrong height / round / type.
    for bad in [
        signed_vote(&vs, &keys, 1, PREVOTE_TYPE, 2, 0, &[]),
        signed_vote(&vs, &keys, 1, PREVOTE_TYPE, 1, 1, &[]),
        signed_vote(&vs, &keys, 1, PRECOMMIT_TYPE, 1, 0, &[]),
    ] {
        assert!(matches!(
            set.add_vote(&bad),
            Err((false, VoteError::UnexpectedStep(_)))
        ));
    }
    // Bad signature.
    let mut bad = signed_vote(&vs, &keys, 2, PREVOTE_TYPE, 1, 0, &[]);
    bad.signature[0] ^= 1;
    assert!(matches!(
        set.add_vote(&bad),
        Err((false, VoteError::InvalidSignature(_)))
    ));
    // Index / address mismatch.
    let mut bad = signed_vote(&vs, &keys, 2, PREVOTE_TYPE, 1, 0, &[]);
    bad.validator_index = 3;
    assert!(matches!(
        set.add_vote(&bad),
        Err((false, VoteError::InvalidValidatorAddress(_)))
    ));
    bad.validator_index = 99;
    assert!(matches!(
        set.add_vote(&bad),
        Err((false, VoteError::InvalidValidatorIndex(_)))
    ));
}

#[test]
fn vote_set_strings_like_go() {
    // `VoteSet.VoteStrings` / `BitArrayString` as /consensus_state lists
    // them in `height_vote_set` (Go: `BA{4:x_x_} 2/4 = 0.50`, `nil-Vote`).
    let (vs, keys) = val_set(4, 1);
    let mut set = VoteSet::new(CHAIN, 7, 1, PREVOTE_TYPE, vs.clone());
    assert_eq!(set.bit_array_string(), "BA{4:____} 0/4 = 0.00");
    for idx in [0, 2] {
        let v = signed_vote(
            &vs,
            &keys,
            idx,
            PREVOTE_TYPE,
            7,
            1,
            b"some-block-hash-0000000000000000",
        );
        assert_eq!(set.add_vote(&v), Ok(true));
    }
    assert_eq!(set.bit_array_string(), "BA{4:x_x_} 2/4 = 0.50");
    let s = set.vote_strings();
    assert_eq!(s.len(), 4);
    assert_eq!(s[1], "nil-Vote");
    assert_eq!(s[3], "nil-Vote");
    let v = set.get_by_index(2).unwrap();
    assert_eq!(
        s[2],
        format!(
            "Vote{{2:{} 7/01/SIGNED_MSG_TYPE_PREVOTE(Prevote) {} {} 000000000000 @ 2023-11-14T22:13:20Z}}",
            ctypes::gofmt::fingerprint_hex(&v.validator_address),
            ctypes::gofmt::fingerprint_hex(&v.block_id.hash),
            ctypes::gofmt::fingerprint_hex(&v.signature),
        )
    );
}

#[test]
fn vote_set_two_thirds_majority() {
    let (vs, keys) = val_set(10, 1);
    let mut set = VoteSet::new(CHAIN, 1, 0, PREVOTE_TYPE, vs.clone());
    for i in 0..6 {
        set.add_vote(&signed_vote(&vs, &keys, i, PREVOTE_TYPE, 1, 0, &[]))
            .unwrap();
    }
    assert!(!set.has_two_thirds_majority());
    set.add_vote(&signed_vote(
        &vs,
        &keys,
        6,
        PREVOTE_TYPE,
        1,
        0,
        b"some-block-hash-0000000000000000",
    ))
    .unwrap();
    assert!(!set.has_two_thirds_majority());
    assert!(set.has_two_thirds_any());
    set.add_vote(&signed_vote(&vs, &keys, 7, PREVOTE_TYPE, 1, 0, &[]))
        .unwrap();
    assert_eq!(set.two_thirds_majority(), Some(BlockId::default()));
}

/// `TestVoteSet_Conflicts`.
#[test]
fn vote_set_conflicts() {
    let (vs, keys) = val_set(4, 1);
    let mut set = VoteSet::new(CHAIN, 1, 0, PREVOTE_TYPE, vs.clone());
    let h1 = [1u8; 32];
    let h2 = [2u8; 32];
    assert_eq!(
        set.add_vote(&signed_vote(&vs, &keys, 0, PREVOTE_TYPE, 1, 0, &[])),
        Ok(true)
    );
    // val0 votes again for blockHash1: conflict, not added.
    let r = set.add_vote(&signed_vote(&vs, &keys, 0, PREVOTE_TYPE, 1, 0, &h1));
    assert!(matches!(r, Err((false, VoteError::Conflicting { .. }))));
    // Start tracking blockHash1.
    set.set_peer_maj23("peerA", block_id(&h1)).unwrap();
    let r = set.add_vote(&signed_vote(&vs, &keys, 0, PREVOTE_TYPE, 1, 0, &h1));
    assert!(matches!(r, Err((true, VoteError::Conflicting { .. }))));
    assert!(set.set_peer_maj23("peerA", block_id(&h2)).is_err());
    let r = set.add_vote(&signed_vote(&vs, &keys, 0, PREVOTE_TYPE, 1, 0, &h2));
    assert!(matches!(r, Err((false, VoteError::Conflicting { .. }))));
    assert_eq!(
        set.add_vote(&signed_vote(&vs, &keys, 1, PREVOTE_TYPE, 1, 0, &h1)),
        Ok(true)
    );
    assert!(!set.has_two_thirds_majority());
    assert!(!set.has_two_thirds_any());
    assert_eq!(
        set.add_vote(&signed_vote(&vs, &keys, 2, PREVOTE_TYPE, 1, 0, &h2)),
        Ok(true)
    );
    assert!(!set.has_two_thirds_majority());
    assert!(set.has_two_thirds_any());
    set.set_peer_maj23("peerB", block_id(&h1)).unwrap();
    let r = set.add_vote(&signed_vote(&vs, &keys, 2, PREVOTE_TYPE, 1, 0, &h1));
    assert!(matches!(r, Err((true, VoteError::Conflicting { .. }))));
    assert!(set.has_two_thirds_majority());
    assert_eq!(set.two_thirds_majority().unwrap().hash, h1.to_vec());
    assert!(set.has_two_thirds_any());
}

#[test]
fn vote_set_make_commit_and_back() {
    let (vs, keys) = val_set(10, 1);
    let mut set = VoteSet::new(CHAIN, 1, 0, PRECOMMIT_TYPE, vs.clone());
    let h = [7u8; 32];
    for i in 0..6 {
        set.add_vote(&signed_vote(&vs, &keys, i, PRECOMMIT_TYPE, 1, 0, &h))
            .unwrap();
    }
    assert!(
        set.make_extended_commit(&ctypes::params::AbciParams::default())
            .is_err()
    );
    set.add_vote(&signed_vote(
        &vs,
        &keys,
        6,
        PRECOMMIT_TYPE,
        1,
        0,
        &[9u8; 32],
    ))
    .unwrap();
    set.add_vote(&signed_vote(&vs, &keys, 7, PRECOMMIT_TYPE, 1, 0, &h))
        .unwrap();
    set.add_vote(&signed_vote(&vs, &keys, 8, PRECOMMIT_TYPE, 1, 0, &[]))
        .unwrap();
    let ec = set
        .make_extended_commit(&ctypes::params::AbciParams::default())
        .unwrap();
    let c = ec.to_commit();
    assert_eq!(c.signatures.len(), 10);
    // Vote for another block -> absent; nil vote kept; missing -> absent.
    assert_eq!(c.signatures[6].block_id_flag, ctypes::BLOCK_ID_FLAG_ABSENT);
    assert_eq!(c.signatures[8].block_id_flag, ctypes::BLOCK_ID_FLAG_NIL);
    assert_eq!(c.signatures[9].block_id_flag, ctypes::BLOCK_ID_FLAG_ABSENT);
    ctypes::verify_commit(CHAIN, &vs, &block_id(&h), 1, &c).unwrap();
    let back = VoteSet::from_commit(CHAIN, &c, vs).unwrap();
    assert_eq!(back.two_thirds_majority(), Some(block_id(&h)));
}

/// `TestPeerCatchupRounds`.
#[test]
fn peer_catchup_rounds() {
    let (vs, keys) = val_set(10, 1);
    let mut hvs = HeightVoteSet::new(CHAIN, 1, vs.clone(), false);
    let v999 = signed_vote(&vs, &keys, 0, PREVOTE_TYPE, 1, 999, &[]);
    assert_eq!(hvs.add_vote(&v999, "peer1"), Ok(true));
    let v1000 = signed_vote(&vs, &keys, 0, PREVOTE_TYPE, 1, 1000, &[]);
    assert_eq!(hvs.add_vote(&v1000, "peer1"), Ok(true));
    let v1001 = signed_vote(&vs, &keys, 0, PREVOTE_TYPE, 1, 1001, &[]);
    assert_eq!(
        hvs.add_vote(&v1001, "peer1"),
        Err((false, VoteError::UnwantedRound))
    );
    assert_eq!(hvs.add_vote(&v1001, "peer2"), Ok(true));
}

#[test]
fn pol_info_finds_latest_polka() {
    let (vs, keys) = val_set(4, 1);
    let mut hvs = HeightVoteSet::new(CHAIN, 1, vs.clone(), false);
    hvs.set_round(3);
    assert_eq!(hvs.pol_info().0, -1);
    for i in 0..3 {
        hvs.add_vote(
            &signed_vote(&vs, &keys, i, PREVOTE_TYPE, 1, 1, &[5; 32]),
            "p",
        )
        .unwrap();
    }
    assert_eq!(hvs.pol_info(), (1, block_id(&[5; 32])));
    for i in 0..3 {
        hvs.add_vote(&signed_vote(&vs, &keys, i, PREVOTE_TYPE, 1, 2, &[]), "p")
            .unwrap();
    }
    assert_eq!(hvs.pol_info(), (2, BlockId::default()));
}

#[test]
fn bit_array_proto_round_trip() {
    let mut b = BitArray::new(70);
    b.set(0, true);
    b.set(64, true);
    b.set(69, true);
    let p = b.to_proto();
    assert_eq!(p.bits, 70);
    assert_eq!(p.elems, vec![1, 1 | (1 << 5)]);
    assert_eq!(BitArray::from_proto(&p), b);
    assert_eq!(b.true_indices(), vec![0, 64, 69]);
}

#[test]
fn crc32c_known_values() {
    // RFC 3720 test vectors.
    assert_eq!(crc32c(b""), 0);
    assert_eq!(crc32c(&[0u8; 32]), 0x8A91_36AA);
    assert_eq!(crc32c(b"123456789"), 0xE306_9283);
}

fn sample_messages() -> Vec<TimedWalMessage> {
    let (vs, keys) = val_set(4, 1);
    let t = |s: i64| Time::from_unix(1_700_000_000 + s, 7);
    let vote = signed_vote(&vs, &keys, 1, PREVOTE_TYPE, 1, 0, &[3; 32]);
    vec![
        TimedWalMessage {
            time: t(0),
            msg: WalMessage::EndHeight(0),
        },
        TimedWalMessage {
            time: t(1),
            msg: WalMessage::RoundState {
                height: 1,
                round: 0,
                step: RoundStep::Propose.as_str().into(),
            },
        },
        TimedWalMessage {
            time: t(2),
            msg: WalMessage::MsgInfo {
                msg: ConsensusMessage::Vote(vote),
                peer_id: String::new(),
            },
        },
        TimedWalMessage {
            time: t(3),
            msg: WalMessage::Timeout(TimeoutInfo {
                duration: 3_000_000_000,
                height: 1,
                round: 0,
                step: RoundStep::Propose,
            }),
        },
        TimedWalMessage {
            time: t(4),
            msg: WalMessage::EndHeight(1),
        },
        TimedWalMessage {
            time: t(5),
            msg: WalMessage::MsgInfo {
                msg: ConsensusMessage::Vote(signed_vote(&vs, &keys, 2, PRECOMMIT_TYPE, 2, 0, &[])),
                peer_id: "peer".into(),
            },
        },
    ]
}

/// `TestWALEncoderDecoder` / `TestWALSearchForEndHeight`.
#[test]
fn wal_encode_decode_and_search() {
    let msgs = sample_messages();
    let mut bz = Vec::new();
    for m in &msgs {
        bz.extend(encode(m).unwrap());
    }
    let (back, n, err) = decode_all(&bz);
    assert_eq!(back, msgs);
    assert_eq!(n, bz.len());
    assert!(err.is_none());

    let disk = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut wal = MemWal::open(disk.clone(), Time::ZERO).unwrap();
    for m in &msgs[1..] {
        wal.write(m).unwrap();
    }
    let after1 = wal.search_for_end_height(1).unwrap().unwrap();
    assert_eq!(after1, msgs[5..].to_vec());
    assert!(wal.search_for_end_height(2).unwrap().is_none());
}

/// Unsynced writes are lost on a crash; synced ones survive.
#[test]
fn wal_crash_loses_only_unsynced() {
    let msgs = sample_messages();
    let disk = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    {
        let mut wal = MemWal::open(disk.clone(), Time::ZERO).unwrap();
        wal.write_sync(&msgs[2]).unwrap();
        wal.write(&msgs[3]).unwrap();
    }
    let mut wal = MemWal::open(disk, Time::ZERO).unwrap();
    let after0 = wal.search_for_end_height(0).unwrap().unwrap();
    assert_eq!(after0, vec![msgs[2].clone()]);
}

/// `TestWALTruncate`-like: a torn tail is corruption; repair keeps the
/// decodable prefix.
#[test]
fn wal_corruption_and_repair() {
    let msgs = sample_messages();
    let disk = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut wal = MemWal::open(disk.clone(), Time::ZERO).unwrap();
    for m in &msgs[1..] {
        wal.write_sync(m).unwrap();
    }
    {
        let mut d = disk.lock().unwrap();
        let n = d.len();
        d.truncate(n - 3);
    }
    assert!(matches!(
        wal.search_for_end_height(1),
        Err(WalError::Corruption(_))
    ));
    wal.repair().unwrap();
    assert_eq!(wal.search_for_end_height(1).unwrap().unwrap(), vec![]);
    // A flipped bit is a checksum failure.
    let mut bz = encode(&msgs[0]).unwrap();
    let last = bz.len() - 1;
    bz[last] ^= 1;
    assert!(matches!(decode_all(&bz).2, Some(WalError::Corruption(_))));
}

#[test]
fn file_wal_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cs.wal").join("wal");
    let msgs = sample_messages();
    {
        let mut wal = crate::wal::FileWal::open(&path, Time::ZERO).unwrap();
        for m in &msgs[1..] {
            wal.write(m).unwrap();
        }
        wal.flush_and_sync().unwrap();
    }
    let mut wal = crate::wal::FileWal::open(&path, Time::ZERO).unwrap();
    assert_eq!(
        wal.search_for_end_height(1).unwrap().unwrap(),
        msgs[5..].to_vec()
    );
    // Rotation keeps every height (Go's autofile group).
    wal.write_sync(&TimedWalMessage {
        time: Time::ZERO,
        msg: WalMessage::EndHeight(2),
    })
    .unwrap();
    wal.rotate_file().unwrap();
    assert_eq!(
        wal.search_for_end_height(1).unwrap().unwrap(),
        [
            &msgs[5..],
            &[TimedWalMessage {
                time: Time::ZERO,
                msg: WalMessage::EndHeight(2),
            }]
        ]
        .concat()
    );
    assert_eq!(wal.search_for_end_height(2).unwrap().unwrap(), vec![]);
    assert!(path.with_extension("000").exists());
}

#[test]
fn ticker_ignores_older_timeouts() {
    let mut t = TimeoutTicker::default();
    let now = Time::from_unix(100, 0);
    let ti = |h, r, s, d| TimeoutInfo {
        duration: d,
        height: h,
        round: r,
        step: s,
    };
    t.schedule(ti(1, 0, RoundStep::Propose, 1_000), now);
    // Older step at the same round: ignored.
    t.schedule(ti(1, 0, RoundStep::NewRound, 5), now);
    assert_eq!(t.deadline(), Some(Time::from_unix(100, 1_000)));
    // Newer step replaces.
    t.schedule(ti(1, 0, RoundStep::PrevoteWait, 2_000), now);
    assert!(t.poll(Time::from_unix(100, 1_999)).is_none());
    assert_eq!(
        t.poll(Time::from_unix(100, 2_000)).unwrap().step,
        RoundStep::PrevoteWait
    );
    assert!(t.poll(Time::from_unix(200, 0)).is_none());
    // Lower height after firing: still ignored.
    t.schedule(ti(0, 5, RoundStep::Commit, 0), now);
    assert!(t.deadline().is_none());
}

#[test]
fn system_clock_wait_ends_at_stop() {
    use crate::ticker::{Clock, SystemClock, system_now};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let soon = nodestore::state::add_duration(system_now(), 1_000_000);
    assert!(SystemClock::default().sleep_until(soon));

    let stop = Arc::new(AtomicBool::new(false));
    let mut clock = SystemClock::with_stop(stop.clone());
    stop.store(true, Ordering::SeqCst);
    let started = std::time::Instant::now();
    let far = nodestore::state::add_duration(system_now(), 10_000_000_000);
    assert!(!clock.sleep_until(far));
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

#[test]
fn consensus_message_wire_round_trip() {
    let (vs, keys) = val_set(4, 1);
    let msgs = vec![
        ConsensusMessage::NewRoundStep {
            height: 5,
            round: 2,
            step: RoundStep::Prevote,
            seconds_since_start_time: 3,
            last_commit_round: 0,
        },
        ConsensusMessage::Vote(signed_vote(&vs, &keys, 3, PRECOMMIT_TYPE, 9, 1, &[4; 32])),
        ConsensusMessage::HasVote {
            height: 9,
            round: 1,
            vote_type: PRECOMMIT_TYPE,
            index: 3,
        },
        ConsensusMessage::VoteSetBits {
            height: 9,
            round: 1,
            vote_type: PREVOTE_TYPE,
            block_id: block_id(&[4; 32]),
            votes: BitArray(vec![true, false, true]),
        },
    ];
    for m in msgs {
        assert_eq!(ConsensusMessage::decode(&m.encode()).unwrap(), m);
    }
}
