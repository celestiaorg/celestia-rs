//! Ports of `blocksync/pool_test.go` on a simulated clock.

use std::sync::mpsc::{Receiver, channel};

use super::*;

fn block(h: i64) -> Block {
    let mut b = Block::default();
    b.header.height = h;
    b
}

struct Harness {
    pool: BlockPool,
    reqs: Receiver<BlockRequest>,
    errs: Receiver<PeerError>,
    now: Instant,
}

impl Harness {
    fn new(start: i64) -> Self {
        let (rt, reqs) = channel();
        let (et, errs) = channel();
        let now = Instant::now();
        let pool = BlockPool::new(start, rt, et, now);
        pool.start(now);
        Self {
            pool,
            reqs,
            errs,
            now,
        }
    }

    fn advance(&mut self, d: Duration) {
        self.now += d;
        self.pool.tick(self.now);
    }

    fn requests(&self) -> Vec<BlockRequest> {
        self.reqs.try_iter().collect()
    }

    fn errors(&self) -> Vec<PeerError> {
        self.errs.try_iter().collect()
    }
}

#[test]
fn block_pool_basic() {
    // TestBlockPoolBasic: 10 honest peers serve every request; the pool
    // advances from 42 past 300.
    let start = 42;
    let mut h = Harness::new(start);
    let peers: Vec<(String, i64, i64)> = (0..10)
        .map(|i| (format!("peer{i}"), start, 900 + i64::from(i) * 10))
        .collect();
    for (id, base, height) in &peers {
        h.pool.set_peer_range(id, *base, *height, h.now);
    }
    let mut asked = HashSet::new();
    for _ in 0..20_000 {
        h.advance(REQUEST_INTERVAL);
        for r in h.requests() {
            assert!(
                asked.insert(r.height),
                "height {} requested twice",
                r.height
            );
            let (_, base, height) = peers.iter().find(|p| p.0 == r.peer_id).unwrap();
            assert!(r.height >= *base && r.height <= *height);
            h.pool
                .add_block(&r.peer_id, block(r.height), None, 1000, h.now)
                .unwrap();
        }
        assert!(h.errors().is_empty());
        if h.pool.has_two_blocks() {
            let (first, second, _) = h.pool.peek_two_blocks();
            assert_eq!(first.unwrap().header.height, h.pool.height());
            assert_eq!(second.unwrap().header.height, h.pool.height() + 1);
            h.pool.pop_request();
        }
        if h.pool.height() > 300 {
            break;
        }
    }
    assert!(h.pool.height() > 300, "stuck at {}", h.pool.height());
    // at most 40 requesters are outstanding
    let (_, _, n) = h.pool.get_status();
    assert!(n <= DEFAULT_MAX_REQUESTERS);
}

#[test]
fn no_requests_before_peer_conn_wait() {
    let mut h = Harness::new(1);
    h.pool.set_peer_range("a", 1, 100, h.now);
    h.advance(Duration::from_secs(2));
    assert!(h.requests().is_empty());
    h.advance(Duration::from_secs(1));
    assert_eq!(h.requests().len(), 1);
}

#[test]
fn block_pool_timeout() {
    // TestBlockPoolTimeout: peers never answer -> every one of them times
    // out (peerTimeout) and is reported.
    let start = 42;
    let mut h = Harness::new(start);
    for i in 0..10 {
        h.pool
            .set_peer_range(&format!("p{i}"), start, 1000 + i, h.now);
    }
    let mut timed_out = HashSet::new();
    for _ in 0..200 {
        h.advance(Duration::from_secs(1));
        let _ = h.requests();
        for e in h.errors() {
            // Go's test also treats every error as a timeout: either no
            // block within peerTimeout or a receive rate below minimum.
            assert!(
                e.err == "peer did not send us anything"
                    || e.err == "peer is not sending us data fast enough",
                "{}",
                e.err
            );
            timed_out.insert(e.peer_id);
        }
        if timed_out.len() == 10 {
            break;
        }
    }
    assert_eq!(timed_out.len(), 10);
}

#[test]
fn retry_moves_request_to_another_peer() {
    // a requester whose peer does not answer within the retry timeout asks
    // a different peer (ignoring the previous one)
    let mut h = Harness::new(1);
    h.pool.set_peer_range("a", 1, 1, h.now);
    h.pool.set_peer_range("b", 1, 1, h.now);
    h.advance(PEER_CONN_WAIT);
    let first = h.requests();
    assert_eq!(first.len(), 1);
    h.advance(Duration::from_secs(29));
    assert!(h.requests().is_empty());
    h.advance(Duration::from_secs(1));
    let second = h.requests();
    assert_eq!(second.len(), 1);
    assert_ne!(second[0].peer_id, first[0].peer_id);
    // the block from the first peer is still accepted (it was asked)
    h.pool
        .add_block(&first[0].peer_id, block(1), None, 10, h.now)
        .unwrap();
}

#[test]
fn no_block_response_redoes_request() {
    let mut h = Harness::new(1);
    h.pool.set_peer_range("a", 1, 5, h.now);
    h.pool.set_peer_range("b", 1, 5, h.now);
    h.advance(PEER_CONN_WAIT);
    let r = h.requests().remove(0);
    assert_eq!(r.height, 1);
    h.pool.redo_request_from(1, &r.peer_id);
    h.advance(REQUEST_INTERVAL);
    let again: Vec<_> = h.requests().into_iter().filter(|x| x.height == 1).collect();
    assert_eq!(again.len(), 1);
    assert_ne!(again[0].peer_id, r.peer_id);
}

#[test]
fn unrequested_block_reports_peer() {
    // TestAddBlockDoesNotDeadlockOnSendError: a block from a peer that was
    // not asked is an error that disconnects the peer.
    let mut h = Harness::new(1);
    h.pool.set_peer_range("a", 1, 5, h.now);
    h.advance(PEER_CONN_WAIT);
    let _ = h.requests();
    let e = h
        .pool
        .add_block(
            "b",
            block(1),
            Some(ExtendedCommit {
                height: 1,
                ..Default::default()
            }),
            123,
            h.now,
        )
        .unwrap_err();
    assert!(e.contains("not b"), "{e}");
    assert_eq!(h.errors().len(), 1);
    assert_eq!(h.pool.height(), 1);
    // height mismatch between block and extended commit
    let e = h
        .pool
        .add_block(
            "a",
            block(1),
            Some(ExtendedCommit {
                height: 2,
                ..Default::default()
            }),
            1,
            h.now,
        )
        .unwrap_err();
    assert_eq!(e, "block height 1 != extCommit height 2");
    // a block far ahead of the pool
    let e = h
        .pool
        .add_block("a", block(99), None, 1, h.now)
        .unwrap_err();
    assert!(e.starts_with("peer sent us block #99 we didn't expect"));
}

#[test]
fn remove_peer() {
    // TestBlockPoolRemovePeer
    let h = Harness::new(1);
    for i in 1..=10 {
        h.pool.set_peer_range(&i.to_string(), 0, i, h.now);
    }
    assert_eq!(h.pool.max_peer_height(), 10);
    h.pool.remove_peer("Superman");
    h.pool.remove_peer("10");
    assert_eq!(h.pool.max_peer_height(), 9);
    for i in 1..=10 {
        h.pool.remove_peer(&i.to_string());
    }
    assert_eq!(h.pool.max_peer_height(), 0);
}

#[test]
fn malicious_node_lowering_height_is_banned() {
    // TestBlockPoolMaliciousNode (the reporting part): a peer that reports a
    // lower height than before is removed and banned.
    let h = Harness::new(1);
    h.pool.set_peer_range("m", 1, 100, h.now);
    h.pool.set_peer_range("m", 1, 50, h.now);
    assert!(h.pool.is_peer_banned("m", h.now));
    assert_eq!(h.pool.max_peer_height(), 0);
    // banned peers are ignored for 60 s
    h.pool.set_peer_range("m", 1, 200, h.now);
    assert_eq!(h.pool.num_peers(), 0);
    let later = h.now + BAN_DURATION;
    assert!(!h.pool.is_peer_banned("m", later));
    h.pool.set_peer_range("m", 1, 200, later);
    assert_eq!(h.pool.max_peer_height(), 200);
}

#[test]
fn bad_block_bans_sender_and_redoes() {
    // RemovePeerAndRedoAllPeerRequests
    let mut h = Harness::new(1);
    h.pool.set_peer_range("a", 1, 3, h.now);
    h.pool.set_peer_range("b", 1, 3, h.now);
    h.advance(PEER_CONN_WAIT);
    for _ in 0..5 {
        h.advance(REQUEST_INTERVAL);
    }
    let reqs = h.requests();
    let r1 = reqs.iter().find(|r| r.height == 1).unwrap().clone();
    h.pool
        .add_block(&r1.peer_id, block(1), None, 10, h.now)
        .unwrap();
    let bad = h.pool.remove_peer_and_redo_all_peer_requests(1, h.now);
    assert_eq!(bad, r1.peer_id);
    assert!(h.pool.is_peer_banned(&bad, h.now));
    h.advance(REQUEST_INTERVAL);
    let again: Vec<_> = h.requests().into_iter().filter(|r| r.height == 1).collect();
    assert_eq!(again.len(), 1);
    assert_ne!(again[0].peer_id, bad);
    // a late block from the banned peer is ignored silently
    assert!(h.pool.add_block(&bad, block(1), None, 1, h.now).is_ok());
}

#[test]
fn unreachable_base() {
    // TestBlockPoolMaliciousNodeUnreachableBase
    let honest = 20;
    let h = Harness::new(honest);
    h.pool.set_peer_range("honest1", 1, honest, h.now);
    h.pool.set_peer_range("honest2", 1, honest, h.now);
    h.pool
        .set_peer_range("poison", 1 << 60, (1 << 60) + 100, h.now);
    assert_eq!(h.pool.max_peer_height(), honest);
    assert!(h.pool.is_caught_up(h.now));
    h.pool.remove_peer("poison");
    assert_eq!(h.pool.max_peer_height(), honest);
    assert!(h.pool.is_caught_up(h.now));
}

#[test]
fn recalculate_params() {
    // TestRecalculateParams
    const MIB: usize = 1024 * 1024;
    let cases = [
        (1024, 5, 10),
        (MIB, 7, 10),
        (2 * MIB, 10, 10),
        (4 * MIB, 15, 9),
        (8 * MIB, 26, 7),
        (12 * MIB, 37, 6),
        (16 * MIB, 48, 4),
        (20 * MIB, 60, 2),
        (32 * MIB, 60, 2),
    ];
    for (size, secs, limit) in cases {
        let (rt, _r) = channel();
        let (et, _e) = channel();
        let pool = BlockPool::new(1, rt, et, Instant::now());
        {
            let mut g = pool.lock();
            g.last_received_blocks = BlockStats::new(1);
        }
        pool.record_block_size(size);
        assert_eq!(
            pool.params(),
            (limit, Duration::from_secs(secs)),
            "size {size}"
        );
    }
    let (rt, _r) = channel();
    let (et, _e) = channel();
    let pool = BlockPool::new(1, rt, et, Instant::now());
    assert_eq!(pool.params(), (5, Duration::from_secs(30)));
}

#[test]
fn bans_peer_with_base_greater_than_height() {
    let h = Harness::new(1);
    h.pool.set_peer_range("bad", 500, 100, h.now);
    assert!(h.pool.is_peer_banned("bad", h.now));
    assert_eq!(h.pool.max_peer_height(), 0);
}

#[test]
fn max_peer_height_refreshes_on_pop_request() {
    let h = Harness::new(10);
    h.pool.set_peer_range("A", 1, 20, h.now);
    h.pool.set_peer_range("B", 15, 100, h.now);
    assert_eq!(h.pool.max_peer_height(), 20);
    for height in 10..15 {
        h.pool.lock().requesters.insert(height, Requester::new());
        h.pool.pop_request();
    }
    assert_eq!(h.pool.max_peer_height(), 100);
}

#[test]
fn is_caught_up_all_peers_pruned_ahead() {
    let h = Harness::new(100);
    h.pool.set_peer_range("pruned1", 150, 300, h.now);
    h.pool.set_peer_range("pruned2", 110, 250, h.now);
    assert_eq!(h.pool.max_peer_height(), 0);
    assert!(!h.pool.is_caught_up(h.now + Duration::from_secs(10)));
}

#[test]
fn is_caught_up_fresh_network() {
    let h = Harness::new(1000);
    h.pool.set_peer_range("peer1", 0, 0, h.now);
    h.pool.set_peer_range("peer2", 0, 0, h.now);
    assert_eq!(h.pool.max_peer_height(), 0);
    assert!(h.pool.is_caught_up(h.now));
    // no peers: never caught up
    let h = Harness::new(1);
    assert!(!h.pool.is_caught_up(h.now + Duration::from_secs(10)));
}

#[test]
fn slow_peer_is_dropped() {
    // removeTimedoutPeers: with 40 requesters out, a peer delivering below
    // minRecvRate is reported and removed.
    let mut h = Harness::new(1);
    h.pool.set_peer_range("slow", 1, 10_000, h.now);
    h.advance(PEER_CONN_WAIT);
    for _ in 0..100 {
        h.advance(REQUEST_INTERVAL);
    }
    let reqs = h.requests();
    assert!(reqs.len() >= 5, "{}", reqs.len());
    // deliver tiny blocks slowly (still pending requests left)
    for r in reqs.iter().take(3) {
        h.advance(Duration::from_secs(2));
        h.pool
            .add_block("slow", block(r.height), None, 100, h.now)
            .unwrap();
    }
    let mut dropped = false;
    for _ in 0..12_000 {
        h.advance(Duration::from_millis(10));
        if h.errors()
            .iter()
            .any(|e| e.err == "peer is not sending us data fast enough")
        {
            dropped = true;
            break;
        }
    }
    assert!(dropped);
    assert_eq!(h.pool.num_peers(), 0);
}

#[test]
fn interpolate_clamps() {
    assert_eq!(interpolate(false, 0, 10, 20, 1, 5), 1);
    assert_eq!(interpolate(false, 100, 10, 20, 1, 5), 5);
    assert_eq!(interpolate(true, 0, 10, 20, 1, 5), 5);
}
