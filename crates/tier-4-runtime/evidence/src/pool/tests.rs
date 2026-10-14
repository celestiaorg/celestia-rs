//! Ports of `evidence/pool_test.go` and `evidence/reactor_test.go` on
//! in-memory stores.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::many_single_char_names
)]

use std::sync::{Arc, Mutex};

use ctypes::{BlockId, Commit, PartSetHeader, PubKey, Validator, ValidatorSet, Vote};
use kvdb::MemBackend;

use super::*;
use crate::reactor::{GossipPeer, Reactor};

const CHAIN: &str = "evidence-test";

struct Key {
    priv_key: crypto::ed25519::PrivateKeyBytes,
    pub_key: PubKey,
}

fn key(i: u8) -> Key {
    let pk = crypto::ed25519::private_key_from_seed(&[i + 7; 32]);
    let pubk = crypto::ed25519::public_key_from_private_key(&pk).unwrap();
    Key {
        priv_key: pk,
        pub_key: PubKey::Ed25519(pubk),
    }
}

fn t(h: i64) -> Time {
    Time::from_unix(1_700_000_000 + h * 60, 0)
}

fn vals() -> ValidatorSet {
    ValidatorSet::new(&[
        Validator::new(key(0).pub_key, 10),
        Validator::new(key(1).pub_key, 10),
    ])
    .unwrap()
}

fn state_at(h: i64, vs: &ValidatorSet) -> State {
    let mut st = nodestore::state::make_genesis_state(&nodestore::GenesisDoc {
        genesis_time: t(0),
        chain_id: CHAIN.into(),
        initial_height: 1,
        consensus_params: ctypes::ConsensusParams::default_params(),
        validators: Vec::new(),
        app_hash: Vec::new(),
        app_state: Vec::new(),
    })
    .unwrap();
    st.last_block_height = h;
    st.last_block_time = t(h);
    st.validators = vs.clone();
    st.next_validators = vs.clone();
    st.last_validators = vs.clone();
    st
}

struct Env {
    ss: StateStore,
    bs: BlockStore,
    db: Arc<dyn KvBackend>,
}

/// Stores with blocks 1..=n (block time t(h)) and validators.
fn env(n: i64) -> Env {
    let ss = StateStore::new(Arc::new(MemBackend::new()));
    let bs = BlockStore::new(Arc::new(MemBackend::new())).unwrap();
    let vs = vals();
    for h in 1..=n {
        ss.bootstrap(&state_at(h, &vs)).unwrap();
        let mut b = ctypes::Block::make(
            h,
            ctypes::Data::default(),
            Some(Commit::default()),
            Vec::new(),
        );
        b.header.time = t(h);
        b.header.chain_id = CHAIN.into();
        b.header.proposer_address = vals().validators[0].address.clone();
        b.header.validators_hash = vals().hash();
        b.header.next_validators_hash = vals().hash();
        let parts = b.make_part_set(ctypes::BLOCK_PART_SIZE_BYTES).unwrap();
        bs.save_block(
            &mut b,
            &parts,
            &Commit {
                height: h,
                ..Commit::default()
            },
        )
        .unwrap();
    }
    ss.save(&state_at(n, &vs)).unwrap();
    assert!(bs.load_block_meta(1).unwrap().is_some());
    Env {
        ss,
        bs,
        db: Arc::new(MemBackend::new()),
    }
}

fn pool(e: &Env) -> Pool {
    Pool::new(Arc::clone(&e.db), e.ss.clone(), e.bs.clone()).unwrap()
}

fn vote(k: &Key, idx: i32, height: i64, block: &[u8]) -> Vote {
    let mut v = Vote {
        vote_type: ctypes::vote::PREVOTE_TYPE,
        height,
        round: 0,
        block_id: BlockId {
            hash: hash::sha256(block).to_vec(),
            part_set_header: PartSetHeader {
                total: 1,
                hash: hash::sha256(b"parts").to_vec(),
            },
        },
        timestamp: t(height),
        validator_address: k.pub_key.address(),
        validator_index: idx,
        ..Vote::default()
    };
    let sb = ctypes::canonical::vote_sign_bytes(CHAIN, &v.to_proto()).unwrap();
    v.signature = crypto::ed25519::sign(&k.priv_key, &sb).unwrap().to_vec();
    v
}

fn dve(height: i64) -> Evidence {
    let k = key(0);
    let idx = vals().get_by_address(&k.pub_key.address()).unwrap().0;
    Evidence::DuplicateVote(
        DuplicateVoteEvidence::new(
            vote(&k, idx, height, b"a"),
            vote(&k, idx, height, b"b"),
            t(height),
            &vals(),
        )
        .unwrap(),
    )
}

#[test]
fn evidence_pool_basic() {
    // TestEvidencePoolBasic
    let e = env(10);
    let p = pool(&e);
    assert_eq!(p.pending_evidence(-1), (Vec::new(), 0));
    let ev = dve(5);
    p.add_evidence(ev.clone()).unwrap();
    assert_eq!(p.size(), 1);
    let (evs, size) = p.pending_evidence(-1);
    assert_eq!(evs, vec![ev.clone()]);
    let enc = ev.to_proto().unwrap().encoded_len();
    assert_eq!(
        size,
        i64::try_from(1 + prost::encoding::encoded_len_varint(enc as u64) + enc).unwrap()
    );
    // max bytes too small
    assert_eq!(p.pending_evidence(size - 1).0.len(), 0);
    // adding it again is a no-op
    p.add_evidence(ev.clone()).unwrap();
    assert_eq!(p.size(), 1);
    assert_eq!(p.next_after(None).unwrap().1, ev);
    // committed in block 11
    p.update(&state_at(11, &vals()), std::slice::from_ref(&ev));
    assert_eq!(p.size(), 0);
    assert!(p.is_committed(&ev));
    assert!(!p.is_pending(&ev));
    assert!(p.next_after(None).is_none());
    // committed evidence is silently ignored by AddEvidence, refused by CheckEvidence
    p.add_evidence(ev.clone()).unwrap();
    assert_eq!(p.size(), 0);
    assert!(
        p.check_evidence(&[ev])
            .unwrap_err()
            .contains("evidence was already committed")
    );
}

#[test]
fn key_layout_matches_go() {
    let ev = dve(5);
    let k = key_pending(&ev);
    assert_eq!(k[0], 0x01);
    let suffix = std::str::from_utf8(&k[1..]).unwrap();
    assert_eq!(&suffix[..17], "0000000000000005/");
    assert_eq!(suffix[17..], hex::encode_upper(ev.hash()));
    assert_eq!(key_committed(&ev)[0], 0x00);
    assert_eq!(encode_int64_value(5), vec![0x08, 0x05]);
}

#[test]
fn add_expired_evidence() {
    // TestAddExpiredEvidence: expired only when both the block age and
    // the time age exceed the params.
    let e = env(10);
    let p = pool(&e);
    let mut st = state_at(10, &vals());
    st.consensus_params.evidence.max_age_num_blocks = 3;
    st.consensus_params.evidence.max_age_duration = 5 * 60 * 1_000_000_000;
    // re-seat the pool on these params
    let p2 = Pool::with_state(Arc::new(MemBackend::new()), e.ss.clone(), e.bs.clone(), st).unwrap();
    // height 7: 3 blocks old (not > 3) -> fine
    p2.add_evidence(dve(7)).unwrap();
    // height 6: 4 blocks old, 4 minutes old (not > 5) -> fine
    p2.add_evidence(dve(6)).unwrap();
    // height 4: 6 blocks and 6 minutes -> expired
    let err = p2.add_evidence(dve(4)).unwrap_err();
    assert!(
        matches!(&err, AddError::Invalid(s) if s.contains("is too old")),
        "{err}"
    );
    // unknown height
    let err = p2.add_evidence(dve(20)).unwrap_err();
    assert!(
        matches!(&err, AddError::Invalid(s) if s.contains("don't have header #20")),
        "{err}"
    );
    drop(p);
}

#[test]
fn report_conflicting_votes() {
    // TestReportConflictingVotes: votes become evidence at the next Update
    // with the block time and validator set of their height.
    let e = env(10);
    let p = pool(&e);
    let k = key(1);
    let idx = vals().get_by_address(&k.pub_key.address()).unwrap().0;
    let (a, b) = (vote(&k, idx, 11, b"x"), vote(&k, idx, 11, b"y"));
    p.report_conflicting_votes(a.clone(), b.clone());
    assert_eq!(p.size(), 0);
    let mut st = state_at(11, &vals());
    st.last_block_time = t(11);
    p.update(&st, &[]);
    assert_eq!(p.size(), 1);
    let (evs, _) = p.pending_evidence(-1);
    let Evidence::DuplicateVote(d) = &evs[0] else {
        panic!()
    };
    assert_eq!(d.timestamp, t(11));
    assert_eq!(d.total_voting_power, 20);
    assert_eq!(d.validator_power, 10);
    // the same votes again are not duplicated; votes from an older height
    // use the stored block time
    p.report_conflicting_votes(b, a);
    let (a5, b5) = (vote(&k, idx, 5, b"x"), vote(&k, idx, 5, b"y"));
    p.report_conflicting_votes(a5, b5);
    // votes from the future are dropped
    p.report_conflicting_votes(vote(&k, idx, 50, b"x"), vote(&k, idx, 50, b"y"));
    p.update(&state_at(12, &vals()), &[]);
    assert_eq!(p.size(), 2);
    let (evs, _) = p.pending_evidence(-1);
    assert_eq!(evs[0].height(), 5, "pending evidence is listed by height");
    assert_eq!(evs[0].time(), t(5));
    // an Update with a lower height is ignored (Go panics)
    p.update(&state_at(12, &vals()), &[]);
    assert_eq!(p.state().last_block_height, 12);
}

#[test]
fn expired_pending_evidence_is_pruned() {
    // TestEvidencePoolUpdate: pending evidence expires.
    let e = env(10);
    let p = pool(&e);
    p.add_evidence(dve(2)).unwrap();
    p.add_evidence(dve(9)).unwrap();
    let mut st = state_at(11, &vals());
    p.update(&st, &[]);
    assert_eq!(p.size(), 2);
    let params = st.consensus_params.evidence;
    // the pool prunes once past the expiry point it computed (height
    // ev + maxAge + 1, time ev + maxAge + 1s), strictly
    st.last_block_height = 2 + params.max_age_num_blocks + 1;
    st.last_block_time =
        nodestore::state::add_duration(t(2), params.max_age_duration + 2_000_000_000);
    p.update(&st, &[]);
    assert_eq!(p.size(), 2);
    st.last_block_height += 1;
    p.update(&st, &[]);
    assert_eq!(p.size(), 1);
    assert_eq!(p.pending_evidence(-1).0[0].height(), 9);
    assert_eq!(p.next_after(None).unwrap().1.height(), 9);
}

#[test]
fn recover_pending_evidence() {
    // TestRecoverPendingEvidence: pending evidence survives a restart,
    // expired evidence is dropped on startup.
    let e = env(10);
    let p = pool(&e);
    p.add_evidence(dve(3)).unwrap();
    p.add_evidence(dve(8)).unwrap();
    drop(p);
    let p = pool(&e);
    assert_eq!(p.size(), 2);
    assert_eq!(p.next_after(None).unwrap().1.height(), 3);
    drop(p);
    let mut st = state_at(10, &vals());
    st.consensus_params.evidence.max_age_num_blocks = 5;
    st.consensus_params.evidence.max_age_duration = 6 * 60 * 1_000_000_000;
    let p = Pool::with_state(Arc::clone(&e.db), e.ss.clone(), e.bs.clone(), st).unwrap();
    assert_eq!(p.size(), 1);
    assert_eq!(p.pending_evidence(-1).0[0].height(), 8);
}

#[test]
fn check_evidence_rules() {
    // TestVerifyPendingEvidencePasses / TestVerifyDuplicatedEvidenceFails
    let e = env(10);
    let p = pool(&e);
    let ev = dve(4);
    p.add_evidence(ev.clone()).unwrap();
    p.check_evidence(std::slice::from_ref(&ev)).unwrap();
    assert!(
        p.check_evidence(&[ev.clone(), ev.clone()])
            .unwrap_err()
            .contains("duplicate evidence")
    );
    // not yet pending: verified and added
    let ev2 = dve(6);
    p.check_evidence(std::slice::from_ref(&ev2)).unwrap();
    assert!(p.is_pending(&ev2));
    // invalid evidence (bad signature) is rejected with the verify error
    let Evidence::DuplicateVote(mut d) = dve(7) else {
        panic!()
    };
    d.vote_b.as_mut().unwrap().signature[0] ^= 1;
    assert_eq!(
        p.check_evidence(&[Evidence::DuplicateVote(d)]).unwrap_err(),
        "verifying VoteB: invalid signature"
    );
}

struct FakePeer {
    id: String,
    height: Mutex<Option<i64>>,
    sent: Mutex<Vec<Vec<u8>>>,
}

impl GossipPeer for FakePeer {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn is_running(&self) -> bool {
        true
    }
    fn consensus_height(&self) -> Option<i64> {
        *self.height.lock().unwrap()
    }
    fn send(&self, msg: Vec<u8>) -> bool {
        self.sent.lock().unwrap().push(msg);
        true
    }
}

#[test]
fn reactor_gossips_to_ready_peers() {
    // TestReactorBroadcastEvidence / TestReactorSelectiveBroadcast
    let e = env(10);
    let p = pool(&e);
    let r = Reactor::new(p.clone());
    let peer = Arc::new(FakePeer {
        id: "a".into(),
        height: Mutex::new(None),
        sent: Mutex::new(Vec::new()),
    });
    r.add_gossip_peer(peer.clone());
    let ev1 = dve(5);
    let ev2 = dve(7);
    p.add_evidence(ev1.clone()).unwrap();
    p.add_evidence(ev2.clone()).unwrap();
    let now = std::time::Instant::now();
    // no consensus state yet: nothing sent
    r.gossip_step(now);
    assert!(peer.sent.lock().unwrap().is_empty());
    // peer at height 6: only evidence below its height (after the 10 s
    // rescan, as in Go)
    *peer.height.lock().unwrap() = Some(6);
    r.gossip_step(now + crate::reactor::BROADCAST_EVIDENCE_INTERVAL);
    r.gossip_step(now + crate::reactor::BROADCAST_EVIDENCE_INTERVAL * 2);
    let sent: Vec<Evidence> = peer
        .sent
        .lock()
        .unwrap()
        .iter()
        .flat_map(|m| {
            crate::reactor::evidence_list_from_proto(
                &pb::EvidenceList::decode(m.as_slice()).unwrap(),
            )
            .unwrap()
        })
        .collect();
    assert!(sent.contains(&ev1));
    assert!(!sent.contains(&ev2));
    // a second node receives it
    let e2 = env(10);
    let p2 = pool(&e2);
    let r2 = Reactor::new(p2.clone());
    let msg = peer.sent.lock().unwrap()[0].clone();
    r2.receive_bytes("a", &msg).unwrap();
    assert!(p2.is_pending(&ev1));
    // invalid evidence disconnects the sender
    let Evidence::DuplicateVote(mut d) = dve(8) else {
        panic!()
    };
    d.total_voting_power = 1;
    let bad = crate::reactor::evidence_list_to_proto(&[Evidence::DuplicateVote(d)]).unwrap();
    assert!(
        r2.receive_bytes("a", &bad.encode_to_vec())
            .unwrap_err()
            .starts_with("Invalid evidence: ")
    );
    // undecodable lists too
    assert!(r2.receive_bytes("a", &[0x0a, 0x02, 0x0a, 0x00]).is_err());
}

#[test]
fn reactor_does_not_gossip_committed_or_old_evidence() {
    // TestReactorsGossipNoCommittedEvidence
    let e = env(10);
    let p = pool(&e);
    let r = Reactor::new(p.clone());
    let ev = dve(5);
    p.add_evidence(ev.clone()).unwrap();
    p.update(&state_at(11, &vals()), &[ev]);
    let peer = Arc::new(FakePeer {
        id: "a".into(),
        height: Mutex::new(Some(100_000_000)),
        sent: Mutex::new(Vec::new()),
    });
    r.add_gossip_peer(peer.clone());
    // pending but too old for the peer
    p.add_evidence(dve(6)).unwrap();
    r.gossip_step(std::time::Instant::now());
    assert!(peer.sent.lock().unwrap().is_empty());
}
