//! Ports of `mempool/cat/reactor_test.go` and `reactor_sequence_test.go`.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::type_complexity,
    clippy::needless_range_loop
)]

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use celestia_core_proto::tendermint::mempool::{Message, SeenTx, Txs, WantTx, message::Sum};
use prost::Message as _;

use super::peers::MempoolPeer;
use super::reactor::{
    DEFAULT_GOSSIP_DELAY, MAX_SEEN_TX_BROADCAST, MAX_SIGNER_LENGTH, MEMPOOL_DATA_CHANNEL,
    MEMPOOL_WANTS_CHANNEL, Reactor, ReactorOptions,
};
use super::seen::{PendingTxInfo, SeenEntry};
use super::sticky::{select_sticky_peers, sticky_score64};
use super::test_util::{MockPeer, new_default_tx, setup_reactor, setup_reactor_with};
use super::{MAX_RECEIVED_BUFFER_SIZE, MAX_REQUESTS_PER_PEER};
use crate::types::{CachedTx, TxInfo, TxKey, tx_key};
use crate::{MAX_TXS_PER_MESSAGE, MEMPOOL_CHANNEL};

fn msg(sum: Sum) -> Vec<u8> {
    Message { sum: Some(sum) }.encode_to_vec()
}

fn seen(key: &TxKey, signer: &[u8], sequence: u64) -> Vec<u8> {
    msg(Sum::SeenTx(SeenTx {
        tx_key: key.to_vec(),
        sequence,
        signer: signer.to_vec(),
    }))
}

fn want(key: &TxKey) -> Vec<u8> {
    msg(Sum::WantTx(WantTx {
        tx_key: key.to_vec(),
    }))
}

fn txs(t: Vec<Vec<u8>>) -> Vec<u8> {
    msg(Sum::Txs(Txs { txs: t }))
}

fn init(r: &Reactor, p: &Arc<MockPeer>) -> u16 {
    r.init_peer(p.as_dyn());
    r.ids.get_id_for_peer(&p.id)
}

fn recv(r: &Reactor, p: &Arc<MockPeer>, ch: u8, m: &[u8]) -> Result<(), String> {
    r.receive(&p.as_dyn(), ch, m)
}

/// `requestTxFromPeer`: drives the real request path.
fn request_from(r: &Reactor, p: &Arc<MockPeer>, key: TxKey) {
    let id = r.ids.get_id_for_peer(&p.id);
    assert_ne!(id, 0);
    r.mempool.peer_has_tx(id, key);
    assert!(r.request_tx(key, &p.as_dyn()));
}

fn wants_sent(p: &MockPeer) -> Vec<Vec<u8>> {
    p.try_sends()
        .into_iter()
        .filter(|(ch, _)| *ch == MEMPOOL_WANTS_CHANNEL)
        .map(|(_, m)| m)
        .collect()
}

#[test]
fn send_want_tx_after_receiving_seen_tx() {
    let (r, _, app) = setup_reactor();
    let tx = new_default_tx("hello");
    let key = tx_key(&tx);
    app.set_sequence(b"test-signer", 1);
    let peer = MockPeer::random();
    init(&r, &peer);
    recv(
        &r,
        &peer,
        MEMPOOL_DATA_CHANNEL,
        &seen(&key, b"test-signer", 1),
    )
    .unwrap();
    assert_eq!(peer.try_sends(), vec![(MEMPOOL_WANTS_CHANNEL, want(&key))]);
}

#[test]
fn seen_tx_during_failed_send_is_not_lost() {
    let (r, pool, app) = setup_reactor();
    let key = tx_key(&new_default_tx("hello"));
    app.set_sequence(b"test-signer", 1);
    let peer_b = MockPeer::random();
    let peer_a = MockPeer::random();
    init(&r, &peer_b);
    let a_id = init(&r, &peer_a);
    let (r2, pa) = (r.clone(), peer_a.clone());
    let pool2 = pool.clone();
    *peer_b.try_send_hook.lock().unwrap() = Some(Box::new(move |_, _| {
        recv(
            &r2,
            &pa,
            MEMPOOL_DATA_CHANNEL,
            &seen(&key, b"test-signer", 1),
        )
        .unwrap();
        assert!(pa.calls().is_empty());
        assert!(pool2.seen_tracker.has(&key, a_id));
        Some(false)
    }));
    assert!(!r.request_tx(key, &peer_b.as_dyn()));
    assert_eq!(r.requests.for_tx(&key), a_id);
    assert!(r.requests.has(a_id, &key));
    assert_eq!(
        peer_a.try_sends(),
        vec![(MEMPOOL_WANTS_CHANNEL, want(&key))]
    );
}

#[test]
fn failed_send_falls_back_to_other_seen_peer() {
    let (r, pool, _) = setup_reactor();
    let key = tx_key(&new_default_tx("hello"));
    let peer_b = MockPeer::random();
    let peer_a = MockPeer::random();
    let b_id = init(&r, &peer_b);
    let a_id = init(&r, &peer_a);
    pool.peer_has_tx(b_id, key);
    pool.peer_has_tx(a_id, key);
    peer_b
        .try_send_ok
        .store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(!r.request_tx(key, &peer_b.as_dyn()));
    assert_eq!(r.requests.for_tx(&key), a_id);
    assert!(r.requests.has(a_id, &key));
    assert!(!pool.seen_tracker.has(&key, b_id));
    assert_eq!(wants_sent(&peer_a).len(), 1);
    assert_eq!(wants_sent(&peer_b).len(), 1);
}

#[test]
fn sends_tx_after_receiving_want_tx() {
    let (r, pool, _) = setup_reactor();
    let tx = new_default_tx("hello");
    let key = tx_key(&tx);
    let peer = MockPeer::random();
    pool.check_tx(&tx, TxInfo::default()).unwrap();
    let id = init(&r, &peer);
    recv(&r, &peer, MEMPOOL_WANTS_CHANNEL, &want(&key)).unwrap();
    assert_eq!(peer.sends(), vec![(MEMPOOL_DATA_CHANNEL, txs(vec![tx]))]);
    assert!(pool.seen_tracker.has(&key, id));
}

#[test]
fn receives_requested_tx() {
    let (r, pool, _) = setup_reactor();
    let peer = MockPeer::random();
    init(&r, &peer);
    let tx = new_default_tx("hello");
    let key = tx_key(&tx);
    request_from(&r, &peer, key);
    recv(&r, &peer, MEMPOOL_DATA_CHANNEL, &txs(vec![tx])).unwrap();
    assert!(pool.has(&key));
}

#[test]
fn broadcasts_seen_tx_after_receiving_tx() {
    let (r, _, _) = setup_reactor();
    let tx = new_default_tx("hello");
    let key = tx_key(&tx);
    let peers = [MockPeer::random(), MockPeer::random()];
    init(&r, &peers[0]);
    init(&r, &peers[1]);
    request_from(&r, &peers[0], key);
    recv(&r, &peers[0], MEMPOOL_CHANNEL, &txs(vec![tx])).unwrap();
    // only peer 1 gets SeenTx: peer 0 sent us the tx
    assert!(peers[0].sends().is_empty());
    assert_eq!(
        peers[1].sends(),
        vec![(MEMPOOL_DATA_CHANNEL, seen(&key, b"sender-000-0", 0))]
    );
}

#[test]
fn remove_peer_request_from_other_peer() {
    let (r, _, app) = setup_reactor();
    let key = tx_key(&new_default_tx("hello"));
    app.set_sequence(b"test-signer", 1);
    let peers = [MockPeer::random(), MockPeer::random()];
    init(&r, &peers[0]);
    init(&r, &peers[1]);
    let s = seen(&key, b"test-signer", 1);
    recv(&r, &peers[0], MEMPOOL_DATA_CHANNEL, &s).unwrap();
    recv(&r, &peers[1], MEMPOOL_DATA_CHANNEL, &s).unwrap();
    r.remove_peer(&peers[0].id);
    assert_eq!(wants_sent(&peers[0]).len(), 1);
    assert_eq!(wants_sent(&peers[1]).len(), 1);
    assert!(r.mempool.seen_tracker.has(&key, 2));
    assert_eq!(r.requests.for_tx(&key), 2);
    assert!(r.requests.has(2, &key));
    assert!(!r.mempool.seen_tracker.has(&key, 1));
}

#[test]
fn receive_empty_txs_message_disconnects_without_panicking() {
    let (r, _, _) = setup_reactor();
    let peer = MockPeer::random();
    init(&r, &peer);
    assert!(recv(&r, &peer, MEMPOOL_CHANNEL, &txs(Vec::new())).is_err());
    assert!(recv(&r, &peer, MEMPOOL_DATA_CHANNEL, &[]).is_err());
}

#[test]
fn receive_rejected_tx_still_requested() {
    let (r, pool, app) = setup_reactor();
    let key = tx_key(&new_default_tx("rejected tx"));
    app.set_sequence(b"test-signer", 1);
    let peer = MockPeer::random();
    pool.rejected_tx_cache().push(key, 1, "tx rejected");
    assert_eq!(
        pool.was_recently_rejected(&key),
        (true, 1, "tx rejected".into())
    );
    init(&r, &peer);
    recv(
        &r,
        &peer,
        MEMPOOL_DATA_CHANNEL,
        &seen(&key, b"test-signer", 1),
    )
    .unwrap();
    assert_eq!(wants_sent(&peer), vec![want(&key)]);
}

#[test]
fn try_request_queued_tx_requests_first_peer_only() {
    let (r, _, _) = setup_reactor();
    let key = tx_key(&new_default_tx("request-first-peer"));
    let peers = [MockPeer::random(), MockPeer::random()];
    let first = init(&r, &peers[0]);
    init(&r, &peers[1]);
    let entry = SeenEntry {
        tx_key: key,
        peers: HashSet::from([first]),
        added_at: std::time::SystemTime::now(),
        pending_tx_info: Some(PendingTxInfo {
            signer: b"signer".to_vec(),
            sequence: 1,
        }),
    };
    assert!(r.try_request_queued_tx(&entry));
    assert_eq!(wants_sent(&peers[0]), vec![want(&key)]);
    assert!(peers[1].calls().is_empty());
}

#[test]
fn options_verify_and_complete() {
    assert_eq!(DEFAULT_GOSSIP_DELAY, Duration::from_secs(60));
    let mut o = ReactorOptions::default();
    o.verify_and_complete().unwrap();
    assert_eq!(o.max_tx_size, 1024 * 1024);
    assert_eq!(o.max_gossip_delay, DEFAULT_GOSSIP_DELAY);
    assert_eq!(o.max_persistent_sticky_peers, 4);

    let mut o = ReactorOptions {
        max_gossip_delay: Duration::from_secs(30),
        ..ReactorOptions::default()
    };
    o.verify_and_complete().unwrap();
    assert_eq!(o.max_gossip_delay, Duration::from_secs(30));

    let mut o = ReactorOptions {
        max_tx_size: -1,
        ..ReactorOptions::default()
    };
    assert!(o.verify_and_complete().is_err());

    for (given, want) in [(-3, 4), (7, 7)] {
        let mut o = ReactorOptions {
            max_persistent_sticky_peers: given,
            ..ReactorOptions::default()
        };
        o.verify_and_complete().unwrap();
        assert_eq!(o.max_persistent_sticky_peers, want);
    }
}

#[test]
fn channels_match_go() {
    let (r, _, _) = setup_reactor();
    let ch = r.channels();
    let ids: Vec<(u8, i32, usize)> = ch
        .iter()
        .map(|d| (d.id, d.priority, d.send_queue_capacity))
        .collect();
    assert_eq!(ids, vec![(0x30, 1, 10), (0x31, 3, 1000), (0x32, 3, 1000)]);
    // Message{Txs{[1 MiB tx]}} and Message{SeenTx{32-byte key}}
    let big = txs(vec![vec![0; 1024 * 1024]]).len();
    assert_eq!(ch[0].recv_message_capacity, big);
    assert_eq!(ch[1].recv_message_capacity, big);
    assert_eq!(
        ch[2].recv_message_capacity,
        msg(Sum::SeenTx(SeenTx {
            tx_key: vec![0; 32],
            ..SeenTx::default()
        }))
        .len()
    );
}

#[test]
fn drops_unrequested_tx() {
    let (r, pool, app) = setup_reactor();
    let peer = MockPeer::random();
    let id = init(&r, &peer);
    let tx = new_default_tx("unsolicited");
    let key = tx_key(&tx);
    recv(&r, &peer, MEMPOOL_DATA_CHANNEL, &txs(vec![tx])).unwrap();
    assert!(!pool.has(&key));
    assert_eq!(app.check_tx_calls(), 0);
    assert!(r.received_buffer.signer_keys().is_empty());
    assert!(pool.seen_tracker.has(&key, id));
}

#[test]
fn drops_stale_sequence_tx() {
    let (r, pool, app) = setup_reactor();
    app.set_sequence(b"sender-000-0", 1);
    let peer = MockPeer::random();
    let id = init(&r, &peer);
    let tx = new_default_tx("stale-tx");
    let key = tx_key(&tx);
    assert!(
        pool.seen_tracker
            .add_pending_tx(key, id, b"sender-000-0", 3)
    );
    request_from(&r, &peer, key);
    app.set_sequence(b"sender-000-0", 5);
    recv(&r, &peer, MEMPOOL_DATA_CHANNEL, &txs(vec![tx])).unwrap();
    assert!(!pool.has(&key));
    assert!(r.received_buffer.signer_keys().is_empty());
    assert_eq!(app.check_tx_calls(), 0);
    assert!(pool.seen_tracker.get(&key).is_none());
}

#[test]
fn future_tx_is_buffered_then_applied_in_order() {
    let (r, pool, app) = setup_reactor();
    let signer = b"sender-000-0";
    app.set_sequence(signer, 1);
    let peer = MockPeer::random();
    let id = init(&r, &peer);
    let tx = new_default_tx("future");
    let key = tx_key(&tx);
    assert!(pool.seen_tracker.add_pending_tx(key, id, signer, 2));
    request_from(&r, &peer, key);
    recv(&r, &peer, MEMPOOL_DATA_CHANNEL, &txs(vec![tx])).unwrap();
    assert!(!pool.has(&key));
    assert!(r.received_buffer.get(signer, 2).is_some());
    assert!(pool.seen_tracker.pending_tx_info(&key).is_none());
    // the gap closes
    app.set_sequence(signer, 2);
    r.process_received_buffer(signer);
    assert!(pool.has(&key));
    assert!(r.received_buffer.signer_keys().is_empty());
}

#[test]
fn sequence_validation() {
    let (r, pool, app) = setup_reactor();
    let signer = b"test-signer";

    pool.flush();
    let k1 = tx_key(&new_default_tx("test-tx-1"));
    app.set_sequence(signer, 5);
    let peer = MockPeer::random();
    init(&r, &peer);
    recv(&r, &peer, MEMPOOL_DATA_CHANNEL, &seen(&k1, signer, 5)).unwrap();
    assert_eq!(wants_sent(&peer), vec![want(&k1)]);

    pool.flush();
    let k2 = tx_key(&new_default_tx("test-tx-2"));
    app.set_sequence(signer, 10);
    let peer = MockPeer::random();
    init(&r, &peer);
    recv(&r, &peer, MEMPOOL_DATA_CHANNEL, &seen(&k2, signer, 7)).unwrap();
    assert!(peer.calls().is_empty());

    pool.flush();
    let k4 = tx_key(&new_default_tx("test-tx-4"));
    app.set_sequence(signer, 10);
    let peer = MockPeer::random();
    let id = init(&r, &peer);
    recv(&r, &peer, MEMPOOL_DATA_CHANNEL, &seen(&k4, signer, 11)).unwrap();
    assert!(peer.calls().is_empty());
    assert_eq!(r.requests.for_tx(&k4), 0);
    let e = pool.seen_tracker.pending_txs_for_signer(signer);
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].tx_key, k4);
    assert_eq!(
        e[0].pending_tx_info,
        Some(PendingTxInfo {
            signer: signer.to_vec(),
            sequence: 11
        })
    );
    assert_eq!(e[0].peer_ids(), vec![id]);
}

#[test]
fn seen_tx_upgrades_peer_entry_to_pending() {
    let (r, pool, app) = setup_reactor();
    let peer = MockPeer::random();
    let id = init(&r, &peer);
    let key = tx_key(b"invalid-tx-format");
    pool.peer_has_tx(id, key);
    let e = pool.seen_tracker.get(&key).unwrap();
    assert_eq!(e.peers, HashSet::from([id]));
    assert!(e.pending_tx_info.is_none());
    app.set_sequence(b"sender", 5);
    recv(&r, &peer, MEMPOOL_DATA_CHANNEL, &seen(&key, b"sender", 6)).unwrap();
    assert!(peer.calls().is_empty());
    assert_eq!(r.requests.for_tx(&key), 0);
    let e = pool.seen_tracker.pending_txs_for_signer(b"sender");
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].pending_tx_info.as_ref().unwrap().sequence, 6);
    assert_eq!(e[0].peer_ids(), vec![id]);
}

#[test]
fn requests_queued_tx_after_sequence_becomes_available() {
    let (r, pool, app) = setup_reactor();
    let signer = b"sender-000-0";
    app.set_sequence(signer, 1);
    let source = MockPeer::random();
    let source_id = init(&r, &source);
    let target = tx_key(&new_default_tx("future-tx"));
    recv(&r, &source, MEMPOOL_DATA_CHANNEL, &seen(&target, signer, 2)).unwrap();
    assert_eq!(pool.seen_tracker.pending_txs_for_signer(signer).len(), 1);

    let provider = MockPeer::random();
    init(&r, &provider);
    app.set_sequence(signer, 2);
    let prior = new_default_tx("prior-tx");
    request_from(&r, &provider, tx_key(&prior));
    recv(&r, &provider, MEMPOOL_DATA_CHANNEL, &txs(vec![prior])).unwrap();
    assert_eq!(r.requests.for_tx(&target), source_id);
    assert_eq!(wants_sent(&source), vec![want(&target)]);
    assert_eq!(pool.seen_tracker.pending_txs_for_signer(signer).len(), 1);
}

#[test]
fn pending_seen_cleared_when_tx_arrives() {
    let (r, pool, app) = setup_reactor();
    let signer = b"sender-000-0";
    app.set_sequence(signer, 1);
    let source = MockPeer::random();
    init(&r, &source);
    let target_tx = new_default_tx("future-tx");
    let target = tx_key(&target_tx);
    recv(&r, &source, MEMPOOL_DATA_CHANNEL, &seen(&target, signer, 2)).unwrap();
    assert_eq!(pool.seen_tracker.pending_txs_for_signer(signer).len(), 1);
    let provider = MockPeer::random();
    init(&r, &provider);
    request_from(&r, &provider, target);
    recv(&r, &provider, MEMPOOL_DATA_CHANNEL, &txs(vec![target_tx])).unwrap();
    assert!(pool.seen_tracker.pending_txs_for_signer(signer).is_empty());
    assert!(source.try_sends().is_empty());
}

#[test]
fn pending_seen_cleared_on_peer_removal() {
    let (r, pool, app) = setup_reactor();
    let signer = b"sender-000-0";
    app.set_sequence(signer, 1);
    let source = MockPeer::random();
    init(&r, &source);
    let target = tx_key(&new_default_tx("future-tx"));
    recv(&r, &source, MEMPOOL_DATA_CHANNEL, &seen(&target, signer, 2)).unwrap();
    assert_eq!(pool.seen_tracker.pending_txs_for_signer(signer).len(), 1);
    r.remove_peer(&source.id);
    assert!(pool.seen_tracker.pending_txs_for_signer(signer).is_empty());
    assert!(pool.seen_tracker.get(&target).is_none());
}

#[test]
fn pending_seen_dropped_when_sequence_advances() {
    let (r, pool, app) = setup_reactor();
    let signer = b"sender-000-0";
    app.set_sequence(signer, 1);
    let source = MockPeer::random();
    init(&r, &source);
    let target = tx_key(&new_default_tx("future-tx"));
    recv(&r, &source, MEMPOOL_DATA_CHANNEL, &seen(&target, signer, 2)).unwrap();
    app.set_sequence(signer, 4);
    r.process_pending_seen_for_signer(signer);
    assert!(pool.seen_tracker.pending_txs_for_signer(signer).is_empty());
    assert!(source.try_sends().is_empty());
}

fn pending_setup(
    expected: u64,
    seqs: &[u64],
) -> (
    Arc<Reactor>,
    Arc<MockPeer>,
    Vec<TxKey>,
    Arc<super::test_util::TestApp>,
) {
    let (r, pool, app) = setup_reactor();
    app.set_sequence(b"sender-000-0", expected);
    let peer = MockPeer::random();
    let id = init(&r, &peer);
    let keys: Vec<TxKey> = seqs
        .iter()
        .map(|s| {
            let k = tx_key(&new_default_tx(&format!("tx-{s}")));
            pool.seen_tracker.add_pending_tx(k, id, b"sender-000-0", *s);
            k
        })
        .collect();
    (r, peer, keys, app)
}

#[test]
fn process_pending_requests_consecutive_sequences() {
    let (r, peer, keys, _) = pending_setup(5, &[5, 6, 7]);
    r.process_pending_seen_for_signer(b"sender-000-0");
    let sent: HashSet<Vec<u8>> = wants_sent(&peer).into_iter().collect();
    assert_eq!(sent, keys.iter().map(want).collect());
    let entries = r
        .mempool
        .seen_tracker
        .pending_txs_for_signer(b"sender-000-0");
    assert_eq!(entries.len(), 3);
    for e in entries {
        assert_ne!(r.requests.for_tx(&e.tx_key), 0);
    }
}

#[test]
fn process_pending_stops_at_gap() {
    let (r, peer, keys, _) = pending_setup(5, &[5, 6, 8]);
    r.process_pending_seen_for_signer(b"sender-000-0");
    assert_eq!(wants_sent(&peer), vec![want(&keys[0]), want(&keys[1])]);
    assert_eq!(r.requests.for_tx(&keys[2]), 0);
}

#[test]
fn process_pending_skips_already_in_mempool() {
    let (r, pool, app) = setup_reactor();
    app.set_sequence(b"sender-000-0", 5);
    let peer = MockPeer::random();
    let id = init(&r, &peer);
    let tx5 = new_default_tx("tx-5");
    pool.check_tx(&tx5, TxInfo::default()).unwrap();
    let k5 = tx_key(&tx5);
    let k6 = tx_key(&new_default_tx("tx-6"));
    pool.seen_tracker.add_pending_tx(k5, id, b"sender-000-0", 5);
    pool.seen_tracker.add_pending_tx(k6, id, b"sender-000-0", 6);
    r.process_pending_seen_for_signer(b"sender-000-0");
    assert_eq!(wants_sent(&peer), vec![want(&k6)]);
    for e in pool.seen_tracker.pending_txs_for_signer(b"sender-000-0") {
        assert_ne!(e.pending_tx_info.unwrap().sequence, 5);
    }
}

#[test]
fn process_pending_respects_max_buffer() {
    let seqs: Vec<u64> = (1..=(MAX_RECEIVED_BUFFER_SIZE as u64 + 10)).collect();
    let (r, _, _, _) = pending_setup(1, &seqs);
    r.process_pending_seen_for_signer(b"sender-000-0");
    let requested = r
        .mempool
        .seen_tracker
        .pending_txs_for_signer(b"sender-000-0")
        .iter()
        .filter(|e| r.requests.for_tx(&e.tx_key) != 0)
        .count();
    assert!(requested <= MAX_RECEIVED_BUFFER_SIZE);
}

#[test]
fn process_pending_skips_already_requested() {
    let (r, peer, keys, _) = pending_setup(5, &[5, 6]);
    let id = r.ids.get_id_for_peer(&peer.id);
    assert!(r.requests.add(keys[0], id, None));
    r.process_pending_seen_for_signer(b"sender-000-0");
    assert_eq!(wants_sent(&peer), vec![want(&keys[1])]);
}

#[test]
fn request_tx_reserves_before_sending() {
    let (r, _, _) = setup_reactor();
    let (a, b) = (MockPeer::random(), MockPeer::random());
    let a_id = init(&r, &a);
    init(&r, &b);
    let key = tx_key(&new_default_tx("tx"));
    assert!(r.request_tx(key, &a.as_dyn()));
    assert!(!r.request_tx(key, &b.as_dyn()));
    assert!(!r.request_tx(key, &a.as_dyn()));
    assert_eq!(r.requests.for_tx(&key), a_id);
    assert_eq!(wants_sent(&a).len(), 1);
    assert!(b.calls().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn request_timeout_drops_unresponsive_peer_and_asks_another() {
    let (r, pool, _) = setup_reactor_with(ReactorOptions {
        max_gossip_delay: Duration::from_millis(100),
        ..ReactorOptions::default()
    });
    let (a, b) = (MockPeer::random(), MockPeer::random());
    let a_id = init(&r, &a);
    let b_id = init(&r, &b);
    let key = tx_key(&new_default_tx("tx"));
    pool.peer_has_tx(a_id, key);
    pool.peer_has_tx(b_id, key);
    assert!(r.request_tx(key, &a.as_dyn()));
    let mut moved = false;
    for _ in 0..200 {
        if r.requests.for_tx(&key) == b_id {
            moved = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(moved);
    assert!(!pool.seen_tracker.has(&key, a_id));
    assert!(pool.seen_tracker.has(&key, b_id));
    assert_eq!(wants_sent(&a).len(), 1);
    assert_eq!(wants_sent(&b).len(), 1);
    r.stop();
}

#[test]
fn request_tx_rolls_back_reservation_on_failed_send() {
    let (r, _, _) = setup_reactor();
    let (a, b) = (MockPeer::random(), MockPeer::random());
    init(&r, &a);
    let b_id = init(&r, &b);
    let key = tx_key(&new_default_tx("tx"));
    a.try_send_ok
        .store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(!r.request_tx(key, &a.as_dyn()));
    assert_eq!(r.requests.for_tx(&key), 0);
    assert!(r.request_tx(key, &b.as_dyn()));
    assert_eq!(r.requests.for_tx(&key), b_id);
}

fn fill(r: &Reactor, peer: u16, tag: &str) -> Vec<TxKey> {
    (0..MAX_REQUESTS_PER_PEER)
        .map(|i| {
            let k = tx_key(format!("fill-tx-{tag}-{i}").as_bytes());
            assert!(r.requests.add(k, peer, None));
            k
        })
        .collect()
}

#[test]
fn try_request_queued_tx_respects_per_peer_limit() {
    let (r, pool, app) = setup_reactor();
    app.set_sequence(b"sender-000-0", 1);
    let (p1, p2) = (MockPeer::random(), MockPeer::random());
    let id1 = init(&r, &p1);
    let id2 = init(&r, &p2);
    fill(&r, id1, "1");
    assert_eq!(r.requests.count_for_peer(id1), MAX_REQUESTS_PER_PEER);
    let target = tx_key(&new_default_tx("target-tx"));
    pool.seen_tracker
        .add_pending_tx(target, id1, b"sender-000-0", 1);
    pool.peer_has_tx(id2, target);
    let entry = pool.seen_tracker.get(&target).unwrap();
    assert!(r.try_request_queued_tx(&entry));
    assert!(r.requests.has(id2, &target));
    assert!(!r.requests.has(id1, &target));
    assert_eq!(wants_sent(&p2), vec![want(&target)]);
}

#[test]
fn try_request_queued_tx_falls_back_to_alternative_peer() {
    let (r, pool, app) = setup_reactor();
    app.set_sequence(b"sender-000-0", 1);
    let ps = [MockPeer::random(), MockPeer::random(), MockPeer::random()];
    let ids: Vec<u16> = ps.iter().map(|p| init(&r, p)).collect();
    fill(&r, ids[0], "1");
    fill(&r, ids[1], "2");
    let target = tx_key(&new_default_tx("target-tx"));
    pool.seen_tracker
        .add_pending_tx(target, ids[0], b"sender-000-0", 1);
    pool.peer_has_tx(ids[1], target);
    pool.peer_has_tx(ids[2], target);
    let entry = pool.seen_tracker.get(&target).unwrap();
    assert!(r.try_request_queued_tx(&entry));
    assert!(r.requests.has(ids[2], &target));
    assert!(!r.requests.has(ids[0], &target));
    assert!(!r.requests.has(ids[1], &target));
    assert_eq!(wants_sent(&ps[2]), vec![want(&target)]);
}

#[test]
fn seen_tx_at_peer_capacity_is_queued() {
    let (r, pool, app) = setup_reactor();
    app.set_sequence(b"test-signer", 1);
    let peer = MockPeer::random();
    let id = init(&r, &peer);
    let fills = fill(&r, id, "x");
    let key = tx_key(&new_default_tx("hello"));
    recv(
        &r,
        &peer,
        MEMPOOL_DATA_CHANNEL,
        &seen(&key, b"test-signer", 1),
    )
    .unwrap();
    assert!(peer.calls().is_empty());
    assert_eq!(r.requests.for_tx(&key), 0);
    let pending = pool.seen_tracker.pending_txs_for_signer(b"test-signer");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].tx_key, key);
    assert!(r.requests.mark_received(id, &fills[0]));
    r.process_pending_seen_for_signer(b"test-signer");
    assert_eq!(r.requests.for_tx(&key), id);
    assert_eq!(wants_sent(&peer), vec![want(&key)]);
}

#[test]
fn find_new_peer_skips_full_peer() {
    let (r, pool, _) = setup_reactor();
    let (full, free) = (MockPeer::random(), MockPeer::random());
    let full_id = init(&r, &full);
    let free_id = init(&r, &free);
    fill(&r, full_id, "f");
    let key = tx_key(&new_default_tx("hello"));
    pool.peer_has_tx(full_id, key);
    pool.peer_has_tx(free_id, key);
    r.find_new_peer_to_request_tx(key);
    assert_eq!(r.requests.for_tx(&key), free_id);
    assert!(full.calls().is_empty());
    assert_eq!(wants_sent(&free), vec![want(&key)]);
}

#[test]
fn malformed_messages_disconnect() {
    let (r, _, _) = setup_reactor();
    let peer = MockPeer::random();
    init(&r, &peer);
    let too_many: Vec<Vec<u8>> = (0..=MAX_TXS_PER_MESSAGE)
        .map(|i| format!("tx-{i}").into_bytes())
        .collect();
    assert!(recv(&r, &peer, MEMPOOL_CHANNEL, &txs(too_many)).is_err());
    assert!(recv(&r, &peer, MEMPOOL_CHANNEL, &txs(vec![vec![1], vec![2]])).is_err());
    assert!(recv(&r, &peer, MEMPOOL_CHANNEL, &txs(vec![Vec::new()])).is_err());
    // the wants channel has no precheck, the reactor still rejects batches
    assert!(
        recv(
            &r,
            &peer,
            MEMPOOL_WANTS_CHANNEL,
            &txs(vec![vec![1], vec![2]])
        )
        .is_err()
    );
    let key = tx_key(&new_default_tx("test-tx"));
    let long: Vec<u8> = (0..=MAX_SIGNER_LENGTH).map(|i| i as u8).collect();
    let e = recv(&r, &peer, MEMPOOL_DATA_CHANNEL, &seen(&key, &long, 1)).unwrap_err();
    assert_eq!(e, "signer field exceeds maximum length");
    let ok: Vec<u8> = (0..MAX_SIGNER_LENGTH).map(|i| i as u8).collect();
    assert!(recv(&r, &peer, MEMPOOL_DATA_CHANNEL, &seen(&key, &ok, 1)).is_ok());
    let e = recv(&r, &peer, MEMPOOL_DATA_CHANNEL, &seen(&key, &[], 0)).unwrap_err();
    assert_eq!(e, "SeenTx missing required signer field");
    let bad_key = msg(Sum::SeenTx(SeenTx {
        tx_key: vec![1; 31],
        sequence: 0,
        signer: b"s".to_vec(),
    }));
    assert_eq!(
        recv(&r, &peer, MEMPOOL_DATA_CHANNEL, &bad_key).unwrap_err(),
        "incorrect tx key size. Expected 32 bytes, got 31"
    );
    let bad_want = msg(Sum::WantTx(WantTx { tx_key: vec![1; 3] }));
    assert!(recv(&r, &peer, MEMPOOL_WANTS_CHANNEL, &bad_want).is_err());
    assert!(recv(&r, &peer, MEMPOOL_DATA_CHANNEL, &[0x0a]).is_err());
}

#[test]
fn seen_tx_direct_path_enforces_per_peer_request_limit() {
    let (r, _, _) = setup_reactor();
    let peer = MockPeer::random();
    let id = init(&r, &peer);
    for i in 0..MAX_REQUESTS_PER_PEER * 4 {
        let key = tx_key(format!("fake-seen-tx-{i}").as_bytes());
        recv(
            &r,
            &peer,
            MEMPOOL_DATA_CHANNEL,
            &seen(&key, b"test-signer", 0),
        )
        .unwrap();
    }
    assert!(r.requests.count_for_peer(id) <= MAX_REQUESTS_PER_PEER);
}

#[test]
fn seen_tx_skips_peers_far_behind() {
    let (r, _, _) = setup_reactor();
    let behind = {
        let mut p = Arc::try_unwrap(MockPeer::random()).ok().unwrap();
        p.height = Some(1);
        Arc::new(p)
    };
    let current = {
        let mut p = Arc::try_unwrap(MockPeer::random()).ok().unwrap();
        p.height = Some(95);
        Arc::new(p)
    };
    init(&r, &behind);
    init(&r, &current);
    r.broadcast_seen_tx_with_height([1; 32], 100, b"s", 1);
    assert!(behind.sends().is_empty());
    assert_eq!(current.sends().len(), 1);
}

// --- sticky broadcast ---

fn plan_peers(
    signer: &[u8],
    salt: &[u8],
    total: usize,
    bottom: usize,
    top: usize,
) -> Vec<(String, bool)> {
    let mut plans: Vec<(String, bool)> = (0..total)
        .map(|i| {
            (
                format!("test-peer-{i:04}-deterministic-id-padding-fillerz"),
                false,
            )
        })
        .collect();
    let mut order: Vec<usize> = (0..total).collect();
    order.sort_by(|&a, &b| {
        let (sa, sb) = (
            sticky_score64(signer, &plans[a].0, salt),
            sticky_score64(signer, &plans[b].0, salt),
        );
        sb.cmp(&sa).then_with(|| plans[a].0.cmp(&plans[b].0))
    });
    for &i in order.iter().take(top) {
        plans[i].1 = true;
    }
    for &i in order.iter().skip(total - bottom) {
        plans[i].1 = true;
    }
    plans
}

fn sticky_reactor(
    opts: ReactorOptions,
    plans: &[(String, bool)],
) -> (Arc<Reactor>, Vec<Arc<MockPeer>>) {
    let (r, _, _) = setup_reactor_with(opts);
    let peers: Vec<Arc<MockPeer>> = plans
        .iter()
        .map(|(id, p)| {
            let m = MockPeer::with(id, *p);
            r.init_peer(m.as_dyn());
            m
        })
        .collect();
    (r, peers)
}

fn recipients(peers: &[Arc<MockPeer>]) -> HashSet<String> {
    peers
        .iter()
        .filter(|p| !p.sends().is_empty())
        .map(|p| p.id.clone())
        .collect()
}

#[test]
fn broadcast_seen_tx_includes_persistent_beyond_cap() {
    let (signer, salt) = (b"signer-A", b"salt-A");
    let plans = plan_peers(signer, salt, 30, 4, 0);
    let (r, peers) = sticky_reactor(
        ReactorOptions {
            sticky_peer_salt: salt.to_vec(),
            max_persistent_sticky_peers: 4,
            ..ReactorOptions::default()
        },
        &plans,
    );
    let persistent: HashSet<String> = plans.iter().filter(|p| p.1).map(|p| p.0.clone()).collect();
    assert_eq!(persistent.len(), 4);
    let ordered = select_sticky_peers(signer, &r.ids.get_all(), 30, salt);
    r.broadcast_seen_tx_with_height([0xab; 32], 1, signer, 1);
    let rec = recipients(&peers);
    for p in ordered.iter().take(MAX_SEEN_TX_BROADCAST) {
        assert!(rec.contains(&p.peer.id()));
    }
    assert!(persistent.is_subset(&rec));
    assert_eq!(rec.len(), MAX_SEEN_TX_BROADCAST + 4);
}

#[test]
fn broadcast_seen_tx_no_persistent_no_change() {
    let (signer, salt) = (b"signer-B", b"salt-B");
    let plans = plan_peers(signer, salt, 30, 0, 0);
    let (r, peers) = sticky_reactor(
        ReactorOptions {
            sticky_peer_salt: salt.to_vec(),
            ..ReactorOptions::default()
        },
        &plans,
    );
    let ordered = select_sticky_peers(signer, &r.ids.get_all(), 30, salt);
    r.broadcast_seen_tx_with_height([0xcd; 32], 1, signer, 1);
    let rec = recipients(&peers);
    assert_eq!(rec.len(), MAX_SEEN_TX_BROADCAST);
    for p in ordered.iter().take(MAX_SEEN_TX_BROADCAST) {
        assert!(rec.contains(&p.peer.id()));
    }
}

#[test]
fn broadcast_seen_tx_persistent_in_top_cap_no_extras() {
    let (signer, salt) = (b"signer-C", b"salt-C");
    let plans = plan_peers(signer, salt, 30, 0, 2);
    let (r, peers) = sticky_reactor(
        ReactorOptions {
            sticky_peer_salt: salt.to_vec(),
            ..ReactorOptions::default()
        },
        &plans,
    );
    r.broadcast_seen_tx_with_height([0xef; 32], 1, signer, 1);
    assert_eq!(recipients(&peers).len(), MAX_SEEN_TX_BROADCAST);
}

#[test]
fn broadcast_seen_tx_respects_max_persistent() {
    let (signer, salt) = (b"signer-D", b"salt-D");
    let plans = plan_peers(signer, salt, 30, 10, 0);
    let (r, peers) = sticky_reactor(
        ReactorOptions {
            sticky_peer_salt: salt.to_vec(),
            max_persistent_sticky_peers: 4,
            ..ReactorOptions::default()
        },
        &plans,
    );
    r.broadcast_seen_tx_with_height([0x12; 32], 1, signer, 1);
    assert_eq!(recipients(&peers).len(), MAX_SEEN_TX_BROADCAST + 4);
}

#[test]
fn broadcast_seen_tx_deterministic_persistent_selection() {
    let (signer, salt) = (b"signer-E", b"salt-E");
    let plans = plan_peers(signer, salt, 30, 10, 0);
    let opts = ReactorOptions {
        sticky_peer_salt: salt.to_vec(),
        max_persistent_sticky_peers: 3,
        ..ReactorOptions::default()
    };
    let (r1, p1) = sticky_reactor(opts.clone(), &plans);
    let (r2, p2) = sticky_reactor(opts, &plans);
    r1.broadcast_seen_tx_with_height([0x77; 32], 1, signer, 1);
    r2.broadcast_seen_tx_with_height([0x77; 32], 1, signer, 1);
    let (a, b) = (recipients(&p1), recipients(&p2));
    assert_eq!(a, b);
    assert_eq!(a.len(), MAX_SEEN_TX_BROADCAST + 3);
}

// --- reactor_sequence_test.go ---

#[test]
fn pending_queue_sorting() {
    let (r, pool, app) = setup_reactor();
    let signer = b"sort-test-signer";
    app.set_sequence(signer, 1);
    let peers = [MockPeer::random(), MockPeer::random(), MockPeer::random()];
    for p in &peers {
        init(&r, p);
    }
    for (i, s) in [10u64, 5, 15, 2, 8].into_iter().enumerate() {
        let tx = format!("sort-test-signer=out-of-order-{s}={s}");
        let key = tx_key(tx.as_bytes());
        recv(
            &r,
            &peers[i % 3],
            MEMPOOL_DATA_CHANNEL,
            &seen(&key, signer, s),
        )
        .unwrap();
    }
    let got: Vec<u64> = pool
        .seen_tracker
        .pending_txs_for_signer(signer)
        .iter()
        .map(|e| e.pending_tx_info.as_ref().unwrap().sequence)
        .collect();
    assert_eq!(got, vec![2, 5, 8, 10, 15]);
}

#[test]
fn single_outstanding_request_per_signer() {
    let (r, pool, app) = setup_reactor();
    let signer = b"single-request-signer";
    app.set_sequence(signer, 1);
    let peer = MockPeer::random();
    init(&r, &peer);
    let txs: Vec<Vec<u8>> = (1..=10)
        .map(|i| format!("single-request-signer=seq-{i:03}={i}").into_bytes())
        .collect();
    for (i, t) in txs.iter().enumerate() {
        recv(
            &r,
            &peer,
            MEMPOOL_DATA_CHANNEL,
            &seen(&tx_key(t), signer, i as u64 + 1),
        )
        .unwrap();
    }
    assert_ne!(r.requests.for_tx(&tx_key(&txs[0])), 0);
    let entries = pool.seen_tracker.pending_txs_for_signer(signer);
    assert!(!entries.is_empty());
    let active = txs
        .iter()
        .filter(|t| r.requests.for_tx(&tx_key(t)) != 0)
        .count();
    assert!(active <= 1);
}

// --- multi-reactor gossip over an in-memory network ---

/// A peer that delivers into another reactor's inbox (as the real switch
/// would, asynchronously).
struct Link {
    remote_id: String,
    local_id: String,
    inbox: tokio::sync::mpsc::UnboundedSender<(String, u8, Vec<u8>)>,
}

impl MempoolPeer for Link {
    fn id(&self) -> String {
        self.remote_id.clone()
    }
    fn is_persistent(&self) -> bool {
        false
    }
    fn peer_state(&self) -> Option<Arc<dyn super::peers::PeerState>> {
        Some(Arc::new(super::test_util::FixedHeight(1)))
    }
    fn send(&self, ch: u8, m: Vec<u8>) -> bool {
        self.inbox.send((self.local_id.clone(), ch, m)).is_ok()
    }
    fn try_send(&self, ch: u8, m: Vec<u8>) -> bool {
        self.send(ch, m)
    }
}

/// `makeAndConnectReactors` + `waitForTxsOnReactors`: txs checked into
/// reactor 0 reach every reactor (full mesh), in priority order.
#[tokio::test(flavor = "multi_thread")]
async fn broadcast_txs_across_reactors() {
    const N: usize = 6;
    let mut reactors = Vec::new();
    let mut inboxes = Vec::new();
    for i in 0..N {
        let (r, _, _) = setup_reactor();
        r.set_sticky_salt(&[i as u8 + 1]);
        r.start();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        reactors.push(r);
        inboxes.push((tx, Some(rx)));
    }
    let name = |i: usize| format!("node-{i}");
    for i in 0..N {
        for j in 0..N {
            if i != j {
                reactors[i].init_peer(Arc::new(Link {
                    remote_id: name(j),
                    local_id: name(i),
                    inbox: inboxes[j].0.clone(),
                }));
            }
        }
    }
    for (i, r) in reactors.iter().enumerate() {
        let mut rx = inboxes[i].1.take().unwrap();
        let r = r.clone();
        tokio::spawn(async move {
            while let Some((from, ch, m)) = rx.recv().await {
                let r = r.clone();
                tokio::task::spawn_blocking(move || {
                    let src = r.ids.get_peer(r.ids.get_id_for_peer(&from)).unwrap();
                    r.receive(&src, ch, &m).unwrap();
                })
                .await
                .unwrap();
            }
        });
    }
    let mut sent = Vec::new();
    for i in 0..10 {
        let prio = rand::random_range(1000..9999);
        let tx = super::test_util::new_tx(i, 0, &rand::random::<[u8; 20]>(), prio);
        reactors[0]
            .mempool
            .check_tx(&tx, TxInfo::default())
            .unwrap();
        sent.push(CachedTx::new(tx));
    }
    let want: HashSet<CachedTx> = sent.iter().cloned().collect();
    for (i, r) in reactors.iter().enumerate() {
        let mut ok = false;
        for _ in 0..300 {
            if r.mempool.size() >= sent.len() {
                ok = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(ok, "reactor {i} has {} txs", r.mempool.size());
        let reaped: HashSet<CachedTx> = r
            .mempool
            .reap_max_txs(sent.len() as i64)
            .into_iter()
            .collect();
        assert_eq!(reaped, want, "reactor {i}");
    }
    for r in &reactors {
        r.stop();
    }
}

/// Sequential txs from one signer, checked into node 0 in order, reach a
/// second node that tracks the signer's sequence like the app would.
#[tokio::test(flavor = "multi_thread")]
async fn sequential_txs_gossip_in_order() {
    let (r0, _, app0) = setup_reactor();
    let (r1, pool1, app1) = setup_reactor();
    r0.start();
    r1.start();
    let (tx0, mut rx0) = tokio::sync::mpsc::unbounded_channel();
    let (tx1, mut rx1) = tokio::sync::mpsc::unbounded_channel();
    r0.init_peer(Arc::new(Link {
        remote_id: "n1".into(),
        local_id: "n0".into(),
        inbox: tx1,
    }));
    r1.init_peer(Arc::new(Link {
        remote_id: "n0".into(),
        local_id: "n1".into(),
        inbox: tx0,
    }));
    let signer = b"sequential-signer";
    app0.set_sequence(signer, 1);
    app1.set_sequence(signer, 1);
    // node 1's check state advances as txs are accepted
    app0.auto_sequence
        .store(true, std::sync::atomic::Ordering::SeqCst);
    app1.auto_sequence
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let r1c = r1.clone();
    tokio::spawn(async move {
        while let Some((from, ch, m)) = rx1.recv().await {
            let r = r1c.clone();
            tokio::task::spawn_blocking(move || {
                let src = r.ids.get_peer(r.ids.get_id_for_peer(&from)).unwrap();
                r.receive(&src, ch, &m).unwrap();
            })
            .await
            .unwrap();
        }
    });
    let r0c = r0.clone();
    tokio::spawn(async move {
        while let Some((from, ch, m)) = rx0.recv().await {
            let r = r0c.clone();
            tokio::task::spawn_blocking(move || {
                let src = r.ids.get_peer(r.ids.get_id_for_peer(&from)).unwrap();
                r.receive(&src, ch, &m).unwrap();
            })
            .await
            .unwrap();
        }
    });
    let n = 20u64;
    for i in 1..=n {
        let tx = format!("sequential-signer=tx-seq-{i:03}={i}");
        let rsp = r0
            .mempool
            .check_tx(tx.as_bytes(), TxInfo::default())
            .unwrap();
        assert_eq!((rsp.code, rsp.sequence), (0, i));
    }
    for _ in 0..300 {
        if pool1.size() as u64 == n {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(pool1.size() as u64, n);
    r0.stop();
    r1.stop();
}

#[test]
fn want_tx_for_unknown_tx_is_ignored() {
    let (r, _, _) = setup_reactor();
    let peer = MockPeer::random();
    init(&r, &peer);
    let _: HashMap<u8, u8> = HashMap::new();
    recv(&r, &peer, MEMPOOL_WANTS_CHANNEL, &want(&[9; 32])).unwrap();
    assert!(peer.calls().is_empty());
}
