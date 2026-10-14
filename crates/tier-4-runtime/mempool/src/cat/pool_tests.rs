//! Ports of `mempool/cat/pool_test.go` and `priority_ordering_test.go`.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::type_complexity,
    clippy::needless_range_loop
)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use celestia_core_proto::abci::ExecTxResult;
use celestia_core_proto::tendermint::types::{Blob, BlobTx, IndexWrapper};
use prost::Message as _;

use super::pool::TxPool;
use super::test_util::{TestApp, new_default_tx, new_tx, setup, setup_with};
use crate::errors::MempoolError;
use crate::types::{CachedTx, MempoolConfig, TxInfo, TxKey, tx_key};

struct TestTx {
    tx: Vec<u8>,
    priority: i64,
}

fn ok_results(n: usize) -> Vec<ExecTxResult> {
    vec![ExecTxResult::default(); n]
}

fn cached(txs: &[TestTx]) -> Vec<CachedTx> {
    txs.iter().map(|t| CachedTx::new(t.tx.clone())).collect()
}

/// `checkTxs`: `n` txs with random 20-byte payloads and priorities in
/// [1000, 9999); each must be added.
fn check_txs(pool: &TxPool, n: usize, peer_id: u16) -> Vec<TestTx> {
    let current = pool.size();
    (0..n)
        .map(|i| {
            let prefix: [u8; 20] = rand::random();
            let priority = rand::random_range(1000..9999);
            let tx = new_tx(i, peer_id, &prefix, priority);
            let info = TxInfo {
                sender_id: peer_id,
                ..TxInfo::default()
            };
            pool.check_tx(&tx, info).unwrap();
            assert_eq!(pool.size(), current + i + 1);
            TestTx { tx, priority }
        })
        .collect()
}

fn must_check_tx(pool: &TxPool, spec: &str) {
    pool.check_tx(spec.as_bytes(), TxInfo::default()).unwrap();
}

fn exists(pool: &TxPool, spec: &str) -> bool {
    pool.has(&tx_key(spec.as_bytes()))
}

async fn fired(n: &tokio::sync::Notify) -> bool {
    tokio::time::timeout(Duration::from_millis(500), n.notified())
        .await
        .is_ok()
}

#[tokio::test]
async fn txs_available() {
    let (pool, _) = setup(0);
    pool.enable_txs_available();
    let n = pool.txs_available().unwrap();
    assert!(!fired(&n).await);

    let txs = check_txs(&pool, 100, 0);
    assert!(fired(&n).await);
    assert!(!fired(&n).await);

    let raw = cached(&txs);
    assert_eq!(pool.size(), 100);
    pool.lock()
        .update(1, &raw[..50], &ok_results(50), None, None)
        .unwrap();
    assert!(fired(&n).await);
    assert!(!fired(&n).await);

    // still at height 1
    let _ = check_txs(&pool, 100, 0);
    assert!(!fired(&n).await);
}

#[test]
fn size() {
    let (pool, _) = setup(0);
    let txs = check_txs(&pool, 100, 0);
    assert_eq!(pool.size(), 100);
    assert_eq!(pool.size_bytes(), 5800);
    let raw = cached(&txs);
    pool.lock()
        .update(1, &raw[..50], &ok_results(50), None, None)
        .unwrap();
    assert_eq!(pool.size(), 50);
    assert_eq!(pool.size_bytes(), 2900);
}

#[test]
fn recheck() {
    let (pool, app) = setup_with(10000, |c| c.recheck = true);
    let txs = check_txs(&pool, 10, 0);
    let raw = cached(&txs);
    let calls = app.check_tx_calls();
    pool.lock()
        .update(1, &raw[..5], &ok_results(5), None, None)
        .unwrap();
    assert_eq!(pool.size(), 5);
    // the 5 remaining txs were rechecked
    assert_eq!(app.check_tx_calls(), calls + 5);
    let _ = check_txs(&pool, 3, 0);
    assert_eq!(pool.size(), 8);
}

/// A recheck that fails removes the tx and caches the rejection.
#[test]
fn recheck_removes_invalid() {
    struct Flaky(TestApp);
    impl crate::AppConnMempool for Flaky {
        fn check_tx(
            &self,
            req: celestia_core_proto::abci::RequestCheckTx,
        ) -> Result<celestia_core_proto::abci::ResponseCheckTx, String> {
            let mut r = self.0.check_tx(req.clone())?;
            if req.r#type == 1 && req.tx.starts_with(b"bad") {
                r.code = 7;
                r.log = "recheck failed".into();
            }
            Ok(r)
        }
        fn query_sequence(&self, s: &[u8]) -> Result<u64, String> {
            self.0.query_sequence(s)
        }
    }
    let pool = TxPool::new(
        MempoolConfig::test(),
        Arc::new(Flaky(TestApp::default())),
        1,
    );
    must_check_tx(&pool, "bad=1=5");
    must_check_tx(&pool, "good=1=5");
    pool.lock().update(2, &[], &[], None, None).unwrap();
    assert!(!exists(&pool, "bad=1=5"));
    assert!(exists(&pool, "good=1=5"));
    assert_eq!(
        pool.was_recently_rejected(&tx_key(b"bad=1=5")),
        (true, 7, "recheck failed".into())
    );
}

#[test]
fn recheck_concurrency() {
    let (pool, _) = setup(0);
    check_txs(&pool, 50, 0);
    let p1 = pool.clone();
    let committer = std::thread::spawn(move || {
        let mut height = 1;
        for _ in 0..5 {
            std::thread::sleep(Duration::from_millis(10));
            let reaped = p1.reap_max_txs(5);
            if !reaped.is_empty() {
                p1.lock()
                    .update(height, &reaped, &ok_results(reaped.len()), None, None)
                    .unwrap();
                height += 1;
            }
        }
    });
    let p2 = pool.clone();
    let adder = std::thread::spawn(move || {
        for i in 0..20 {
            std::thread::sleep(Duration::from_millis(5));
            let _ = p2.check_tx(
                &new_default_tx(&format!("concurrent-tx-{i}")),
                TxInfo::default(),
            );
        }
    });
    committer.join().unwrap();
    adder.join().unwrap();
    pool.check_tx(&new_default_tx("final-test"), TxInfo::default())
        .unwrap();
}

#[test]
fn eviction() {
    let (pool, _) = setup_with(1000, |c| {
        c.size = 5;
        c.max_txs_bytes = 60;
    });
    let err = pool
        .check_tx(
            b"big=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef=1",
            TxInfo::default(),
        )
        .unwrap_err();
    assert!(err.to_string().contains("mempool is full"), "{err}");
    assert_eq!(pool.size(), 0);

    let big = "big=0123456789abcdef0123456789abcdef0123456789abcdef01234=2";
    must_check_tx(&pool, big);
    assert_eq!(pool.size(), 1);
    assert!(exists(&pool, big));
    assert_eq!(pool.size_bytes(), big.len() as i64);

    must_check_tx(&pool, "key1=0000=25");
    assert!(exists(&pool, "key1=0000=25"));
    assert!(!exists(&pool, big));
    assert!(pool.was_recently_evicted(&tx_key(big.as_bytes())));
    assert_eq!(pool.size_bytes(), "key1=0000=25".len() as i64);

    must_check_tx(&pool, "key2=0001=5");
    must_check_tx(&pool, "key3=0002=10");
    must_check_tx(&pool, "key4=0003=3");
    must_check_tx(&pool, "key5=0004=3");

    let err = pool
        .check_tx(b"key6=0005=1", TxInfo::default())
        .unwrap_err();
    assert!(err.to_string().contains("mempool is full"));
    assert!(!exists(&pool, "key6=0005=1"));
    assert!(pool.was_recently_evicted(&tx_key(b"key6=0005=1")));

    must_check_tx(&pool, "key7=0006=7");
    assert!(exists(&pool, "key7=0006=7"));
    assert!(!exists(&pool, "key5=0004=3"));
    assert!(pool.was_recently_evicted(&tx_key(b"key5=0004=3")));
    assert!(exists(&pool, "key4=0003=3"));

    must_check_tx(&pool, "key8=0007=20");
    assert!(exists(&pool, "key8=0007=20"));
    assert!(!exists(&pool, "key4=0003=3"));
    assert!(pool.was_recently_evicted(&tx_key(b"key4=0003=3")));

    must_check_tx(&pool, "key9=0008=9");
    assert!(exists(&pool, "key9=0008=9"));
    assert!(!exists(&pool, "key2=0001=5"));
    assert!(pool.was_recently_evicted(&tx_key(b"key2=0001=5")));

    must_check_tx(&pool, "key10=0123456789abcdef=11");
    assert!(exists(&pool, "key1=0000=25"));
    assert!(exists(&pool, "key8=0007=20"));
    assert!(exists(&pool, "key10=0123456789abcdef=11"));
    for gone in ["key3=0002=10", "key9=0008=9", "key7=0006=7"] {
        assert!(!exists(&pool, gone), "{gone}");
        assert!(
            pool.was_recently_evicted(&tx_key(gone.as_bytes())),
            "{gone}"
        );
    }

    pool.lock()
        .update(
            1,
            &[CachedTx::new(b"key10=0123456789abcdef=11".to_vec())],
            &ok_results(1),
            None,
            None,
        )
        .unwrap();
    assert!(!exists(&pool, "key10=0123456789abcdef=11"));
    must_check_tx(&pool, "key3=0002=10");
    assert!(exists(&pool, "key3=0002=10"));

    pool.remove_tx_by_key(&tx_key(b"key8=0007=20"));
    assert!(!exists(&pool, "key8=0007=20"));
    assert!(!pool.was_recently_evicted(&tx_key(b"key8=0007=20")));
}

#[test]
fn mempool_full_error_message_matches_go() {
    let (pool, _) = setup_with(10, |c| c.max_txs_bytes = 3);
    let err = pool.check_tx(b"a=b=1", TxInfo::default()).unwrap_err();
    let key = tx_key(b"a=b=1");
    let want = format!(
        "rejected valid incoming transaction; mempool is full ({}). Size: (0:0)",
        hex::encode_upper(format!("TxKey{{{}}}", hex::encode_upper(key)))
    );
    assert_eq!(err.to_string(), want);
}

#[test]
fn flush() {
    let (pool, _) = setup(0);
    let txs = check_txs(&pool, 100, 0);
    assert_eq!(pool.size_bytes(), 5800);
    let raw = cached(&txs);
    pool.lock()
        .update(1, &raw[..50], &ok_results(50), None, None)
        .unwrap();
    pool.flush();
    assert_eq!(pool.size(), 0);
    assert_eq!(pool.size_bytes(), 0);
}

#[test]
fn reap_max_bytes_max_gas() {
    let (pool, _) = setup(0);
    let txs = check_txs(&pool, 100, 0);
    assert_eq!(pool.size_bytes(), 5800);
    let by_key: HashMap<TxKey, i64> = txs.iter().map(|t| (tx_key(&t.tx), t.priority)).collect();
    let mut priorities: Vec<i64> = txs.iter().map(|t| t.priority).collect();
    priorities.sort_unstable_by(|a, b| b.cmp(a));
    let ensure = |reaped: &[CachedTx]| {
        let got: Vec<i64> = reaped.iter().map(|t| by_key[&t.key()]).collect();
        assert_eq!(got, priorities[..got.len()]);
    };

    let reaped = pool.reap_max_bytes_max_gas(-1, 50);
    ensure(&reaped);
    assert_eq!(reaped.len(), 50);
    assert_eq!((pool.size(), pool.size_bytes()), (100, 5800));

    let reaped = pool.reap_max_bytes_max_gas(1200, -1);
    ensure(&reaped);
    assert_eq!(reaped.len(), 20);

    let reaped = pool.reap_max_bytes_max_gas(2000, 25);
    ensure(&reaped);
    assert_eq!(reaped.len(), 25);
    assert_eq!((pool.size(), pool.size_bytes()), (100, 5800));
}

#[test]
fn tx_larger_than_max_bytes() {
    let (pool, _) = setup(0);
    let big = format!(
        "sender-1-1={}=2",
        hex::encode_upper(rand::random::<[u8; 32]>()).repeat(3)
    );
    let small = format!(
        "sender-2-1={}=1",
        hex::encode_upper(rand::random::<[u8; 20]>())
    );
    let info = TxInfo {
        sender_id: 1,
        ..TxInfo::default()
    };
    pool.check_tx(big.as_bytes(), info.clone()).unwrap();
    pool.check_tx(small.as_bytes(), info).unwrap();
    let reaped = pool.reap_max_bytes_max_gas(100, -1);
    assert_eq!(reaped.len(), 1);
    assert_eq!(reaped[0].tx(), small.as_bytes());
}

#[test]
fn reap_max_txs() {
    let (pool, _) = setup(0);
    let txs = check_txs(&pool, 100, 0);
    let by_key: HashMap<TxKey, i64> = txs.iter().map(|t| (tx_key(&t.tx), t.priority)).collect();
    let ensure = |r: &[CachedTx]| {
        for w in r.windows(2) {
            assert!(by_key[&w[0].key()] >= by_key[&w[1].key()]);
        }
    };
    let r = pool.reap_max_txs(-1);
    ensure(&r);
    assert_eq!(r.len(), 100);
    let r = pool.reap_max_txs(1);
    ensure(&r);
    assert_eq!(r.len(), 1);
    let r = pool.reap_max_txs(50);
    ensure(&r);
    assert_eq!(r.len(), 50);
    assert_eq!((pool.size(), pool.size_bytes()), (100, 5800));
}

#[test]
fn check_tx_exceeds_max_size() {
    let (pool, _) = setup(0);
    let max = pool.config().max_tx_bytes;
    let tx = vec![7u8; max + 1];
    assert_eq!(
        pool.check_tx(&tx, TxInfo::default()).unwrap_err(),
        MempoolError::TxTooLarge {
            max,
            actual: max + 1
        }
    );
    let tx = vec![7u8; max - 1];
    assert!(!matches!(
        pool.check_tx(&tx, TxInfo::default()),
        Err(MempoolError::TxTooLarge { .. })
    ));
}

#[test]
fn check_tx_same_peer() {
    let (pool, _) = setup(100);
    let tx = format!(
        "sender-0={}=50",
        hex::encode_upper(rand::random::<[u8; 20]>())
    );
    let info = TxInfo {
        sender_id: 1,
        ..TxInfo::default()
    };
    pool.check_tx(tx.as_bytes(), info.clone()).unwrap();
    assert_eq!(
        pool.check_tx(tx.as_bytes(), info).unwrap_err(),
        MempoolError::TxInMempool
    );
}

#[test]
fn concurrent_txs() {
    let (pool, _) = setup(10);
    let p = pool.clone();
    let adder = std::thread::spawn(move || {
        for _ in 0..10 {
            check_txs(&p, 10, 0);
        }
    });
    adder.join().unwrap();
    let mut height = 1;
    loop {
        let reaped = pool.reap_max_txs(50);
        if reaped.is_empty() {
            break;
        }
        let results: Vec<ExecTxResult> = (0..reaped.len())
            .map(|i| ExecTxResult {
                code: if i % 2 == 0 { 0 } else { 100 },
                ..ExecTxResult::default()
            })
            .collect();
        pool.lock()
            .update(height, &reaped, &results, None, None)
            .unwrap();
        height += 1;
    }
    assert_eq!(pool.size(), 0);
    assert_eq!(pool.size_bytes(), 0);
}

#[test]
fn expired_txs_timestamp() {
    let (pool, _) = setup_with(5000, |c| c.ttl_duration = Duration::from_millis(100));
    let added1 = check_txs(&pool, 10, 0);
    std::thread::sleep(Duration::from_millis(50));
    let added2 = check_txs(&pool, 10, 1);
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let mut g = pool.lock();
        let h = g.height() + 1;
        g.update(h, &[], &[], None, None).unwrap();
        drop(g);
        let all_purged = added1.iter().all(|t| {
            let k = tx_key(&t.tx);
            pool.was_recently_evicted(&k) && !pool.has(&k) && !pool.rejected_tx_cache().has_key(&k)
        });
        if all_purged {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "first batch not purged"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    for t in &added2 {
        assert!(pool.has(&tx_key(&t.tx)));
    }
}

#[test]
fn expired_txs_num_blocks() {
    let app = Arc::new(TestApp::default());
    let cfg = MempoolConfig {
        cache_size: 500,
        ttl_num_blocks: 10,
        ..MempoolConfig::test()
    };
    let pool = TxPool::new(cfg, app, 100);
    let _ = check_txs(&pool, 100, 0);
    let reaped = pool.reap_max_txs(5);
    pool.lock()
        .update(101, &reaped, &ok_results(reaped.len()), None, None)
        .unwrap();
    assert_eq!(pool.size(), 95);
    let _ = check_txs(&pool, 50, 1);
    assert_eq!(pool.size(), 145);
    let reaped = pool.reap_max_txs(5);
    pool.lock()
        .update(111, &reaped, &ok_results(reaped.len()), None, None)
        .unwrap();
    assert!(pool.size() >= 45);
}

#[test]
fn check_tx_post_check_error() {
    for err in [Some("test error"), None] {
        let app = Arc::new(TestApp::default());
        let pool =
            TxPool::new(MempoolConfig::test(), app, 1).with_post_check(Arc::new(move |_, _| {
                match err {
                    Some(e) => Err(e.to_string()),
                    None => Ok(()),
                }
            }));
        let res = pool.check_tx(b"sender=0000=1", TxInfo::default());
        match err {
            Some(e) => assert_eq!(res.unwrap_err(), MempoolError::PostCheck(e.into())),
            None => assert!(res.is_ok()),
        }
    }
}

/// A `BlobTx` in the pool is removed by the block's `IndexWrapper` (both
/// key as the inner tx).
#[test]
fn remove_blob_tx() {
    /// kvstore's CheckTx: accepts everything.
    struct AcceptAll;
    impl crate::AppConnMempool for AcceptAll {
        fn check_tx(
            &self,
            _: celestia_core_proto::abci::RequestCheckTx,
        ) -> Result<celestia_core_proto::abci::ResponseCheckTx, String> {
            Ok(celestia_core_proto::abci::ResponseCheckTx {
                gas_wanted: 1,
                ..Default::default()
            })
        }
        fn query_sequence(&self, _: &[u8]) -> Result<u64, String> {
            Ok(0)
        }
    }
    let cfg = MempoolConfig {
        cache_size: 100,
        ..MempoolConfig::test()
    };
    let pool = TxPool::new(cfg, Arc::new(AcceptAll), 1);
    let original = b"sender=00001".to_vec();
    let iw = IndexWrapper {
        tx: original.clone(),
        share_indexes: vec![100],
        type_id: "INDX".into(),
    }
    .encode_to_vec();
    let btx = BlobTx {
        tx: original,
        blobs: vec![Blob {
            namespace_id: vec![1; 28],
            data: vec![1, 2, 3, 4, 5, 6, 7, 8, 1, 2, 3, 4, 5, 6, 7, 8, 9],
            share_version: 0,
            namespace_version: 0,
        }],
        type_id: "BLOB".into(),
    }
    .encode_to_vec();
    assert_eq!(tx_key(&btx), tx_key(&iw));
    assert_eq!(tx_key(&btx), tx_key(b"sender=00001"));
    pool.check_tx(&btx, TxInfo::default()).unwrap();
    assert_eq!(pool.size(), 1);
    pool.lock()
        .update(1, &[CachedTx::new(iw)], &ok_results(1), None, None)
        .unwrap();
    assert_eq!(pool.size(), 0);
    assert_eq!(pool.size_bytes(), 0);
}

#[test]
fn concurrently_adding_tx() {
    let (pool, _) = setup(500);
    let tx = b"sender=0000=1".to_vec();
    let hs: Vec<_> = (1..=10u16)
        .map(|sender| {
            let (pool, tx) = (pool.clone(), tx.clone());
            std::thread::spawn(move || {
                let info = TxInfo {
                    sender_id: sender,
                    ..TxInfo::default()
                };
                pool.try_add_new_tx(CachedTx::new(tx.clone()), tx_key(&tx), &info)
                    .err()
            })
        })
        .collect();
    let errs: Vec<_> = hs.into_iter().filter_map(|h| h.join().unwrap()).collect();
    for e in &errs {
        assert_eq!(*e, MempoolError::TxInMempool);
    }
    assert!(errs.len() < 10);
}

#[tokio::test]
async fn broadcast_queue() {
    let (pool, _) = setup(1);
    for i in 0..10 {
        pool.check_tx(&new_default_tx(&i.to_string()), TxInfo::default())
            .unwrap();
    }
    for i in 0..10 {
        let w = tokio::time::timeout(Duration::from_secs(5), pool.next_to_broadcast())
            .await
            .unwrap();
        assert_eq!(w.tx.tx(), new_default_tx(&i.to_string()).as_slice());
    }
}

#[test]
fn rejection_indexing() {
    // CheckTx rejection
    let (pool, _) = setup(100);
    let key = tx_key(b"invalid-tx-format");
    assert_eq!(pool.was_recently_rejected(&key), (false, 0, String::new()));
    pool.check_tx(b"invalid-tx-format", TxInfo::default())
        .unwrap();
    assert_eq!(
        pool.was_recently_rejected(&key),
        (true, 101, "invalid-tx-format".into())
    );

    // PreCheck rejection
    let pool = TxPool::new(MempoolConfig::test(), Arc::new(TestApp::default()), 1).with_pre_check(
        Arc::new(|tx: &CachedTx| {
            if tx.tx().windows(6).any(|w| w == b"reject") {
                return Err("rejected by precheck".into());
            }
            Ok(())
        }),
    );
    let tx = b"reject-me=test=1".to_vec();
    let key = tx_key(&tx);
    assert!(
        pool.try_add_new_tx(CachedTx::new(tx), key, &TxInfo::default())
            .is_err()
    );
    assert_eq!(
        pool.was_recently_rejected(&key),
        (true, 0, "rejected by precheck".into())
    );

    // PostCheck rejection
    let pool = TxPool::new(MempoolConfig::test(), Arc::new(TestApp::default()), 1).with_post_check(
        Arc::new(|_, res| {
            if res.priority < 10 {
                return Err("priority too low".into());
            }
            Ok(())
        }),
    );
    let tx = b"sender-1=test=5".to_vec();
    let key = tx_key(&tx);
    assert!(
        pool.try_add_new_tx(CachedTx::new(tx), key, &TxInfo::default())
            .is_err()
    );
    assert_eq!(
        pool.was_recently_rejected(&key),
        (true, 0, "priority too low".into())
    );

    // valid
    let (pool, _) = setup(100);
    let tx = b"sender-1=test=100".to_vec();
    let key = tx_key(&tx);
    pool.try_add_new_tx(CachedTx::new(tx), key, &TxInfo::default())
        .unwrap();
    assert_eq!(pool.was_recently_rejected(&key), (false, 0, String::new()));
}

#[test]
fn pre_and_post_check_helpers() {
    let pre = crate::pre_check_max_bytes(10);
    // proto size = 1 + 1 + len
    assert!(pre(&CachedTx::new(vec![0; 8])).is_ok());
    assert_eq!(
        pre(&CachedTx::new(vec![0; 9])).unwrap_err(),
        "tx size is too big: 11, max: 10"
    );
    let post = crate::post_check_max_gas(10);
    let rsp = |g| celestia_core_proto::abci::ResponseCheckTx {
        gas_wanted: g,
        ..Default::default()
    };
    let t = CachedTx::new(vec![1]);
    assert!(post(&t, &rsp(10)).is_ok());
    assert_eq!(
        post(&t, &rsp(11)).unwrap_err(),
        "gas wanted 11 is greater than max gas 10"
    );
    assert_eq!(post(&t, &rsp(-1)).unwrap_err(), "gas wanted -1 is negative");
    assert!(crate::post_check_max_gas(-1)(&t, &rsp(1 << 40)).is_ok());
}

// --- priority_ordering_test.go ---

fn priorities_of(pool: &TxPool, reaped: &[CachedTx]) -> Vec<i64> {
    reaped
        .iter()
        .map(|t| pool.get_wrapped(&t.key()).unwrap().priority)
        .collect()
}

#[test]
fn reap_order_by_priority() {
    let cases: &[(&[(usize, i64)], &[i64])] = &[
        (
            &[(0, 1), (1, 5), (2, 3), (3, 10), (4, 7)],
            &[10, 7, 5, 3, 1],
        ),
        (&[(0, 42)], &[42]),
        (&[(0, 5), (1, 5), (2, 5)], &[5, 5, 5]),
        (&[(0, 1), (1, 100), (2, 1), (3, 100)], &[100, 100, 1, 1]),
        (&[(0, 100), (1, 50), (2, 25), (3, 10)], &[100, 50, 25, 10]),
        (&[(0, 10), (1, 25), (2, 50), (3, 100)], &[100, 50, 25, 10]),
        (&[(0, 1), (1, 1_000_000), (2, 500)], &[1_000_000, 500, 1]),
    ];
    for (txs, want) in cases {
        let (pool, _) = setup(0);
        for &(sender, prio) in *txs {
            let tx = new_tx(sender, 0, format!("msg-{sender}").as_bytes(), prio);
            pool.check_tx(&tx, TxInfo::default()).unwrap();
        }
        assert_eq!(priorities_of(&pool, &pool.reap_max_txs(-1)), *want);
        assert_eq!(
            priorities_of(&pool, &pool.reap_max_bytes_max_gas(-1, -1)),
            *want
        );
    }
}

#[test]
fn ties_reap_fifo() {
    let (pool, _) = setup(0);
    let txs: Vec<Vec<u8>> = (0..3)
        .map(|s| new_tx(s, 0, format!("msg-{s}").as_bytes(), 5))
        .collect();
    for t in &txs {
        pool.check_tx(t, TxInfo::default()).unwrap();
    }
    let reaped: Vec<Vec<u8>> = pool
        .reap_max_txs(-1)
        .iter()
        .map(|t| t.tx().to_vec())
        .collect();
    assert_eq!(reaped, txs);
}

#[test]
fn same_sender_sequence_ordering() {
    let (pool, _) = setup(0);
    for (seq, prio) in [(0u8, 100), (1, 100)] {
        let tx = format!("sender-A-{seq}-0={}={prio}", hex::encode_upper([seq]));
        pool.check_tx(tx.as_bytes(), TxInfo::default()).unwrap();
    }
    for (seq, prio) in [(0u8, 10), (1, 10)] {
        let tx = format!("sender-B-{seq}-0={}={prio}", hex::encode_upper([seq + 10]));
        pool.check_tx(tx.as_bytes(), TxInfo::default()).unwrap();
    }
    assert_eq!(
        priorities_of(&pool, &pool.reap_max_txs(-1)),
        vec![100, 100, 10, 10]
    );
}

#[test]
fn stress_non_increasing() {
    let (pool, _) = setup(0);
    for i in 0..200 {
        let prio = rand::random_range(1..=10000);
        let tx = new_tx(i, 0, format!("stress-{i}").as_bytes(), prio);
        pool.check_tx(&tx, TxInfo::default()).unwrap();
    }
    let check = |r: Vec<CachedTx>, n: usize| {
        assert_eq!(r.len(), n);
        let p = priorities_of(&pool, &r);
        assert!(p.windows(2).all(|w| w[0] >= w[1]));
    };
    check(pool.reap_max_txs(-1), 200);
    check(pool.reap_max_bytes_max_gas(-1, -1), 200);
    check(pool.reap_max_txs(50), 50);
    check(pool.reap_max_bytes_max_gas(-1, 50), 50);
}

#[test]
fn aggregation_edge_cases() {
    let cases: &[(&[(&str, &str, i64)], &[i64], bool)] = &[
        (
            &[("alice", "01", 100), ("alice", "02", 1), ("bob", "03", 60)],
            &[60, 100, 1],
            true,
        ),
        (
            &[
                ("alice", "01", 1000),
                ("alice", "02", 1),
                ("alice", "03", 1),
                ("bob", "04", 500),
            ],
            &[500, 1000, 1, 1],
            true,
        ),
        (
            &[
                ("alice", "01", 100),
                ("alice", "02", 100),
                ("bob", "03", 50),
                ("bob", "04", 50),
            ],
            &[100, 100, 50, 50],
            false,
        ),
    ];
    for (txs, want, violates) in cases {
        let (pool, _) = setup(0);
        for (sender, key, prio) in *txs {
            pool.check_tx(
                format!("{sender}={key}={prio}").as_bytes(),
                TxInfo::default(),
            )
            .unwrap();
        }
        let got = priorities_of(&pool, &pool.reap_max_txs(-1));
        assert_eq!(got, *want);
        let strictly = got.windows(2).all(|w| w[0] >= w[1]);
        assert_eq!(strictly, !violates);
    }
}

/// `determineLeftoverTxs` picks the starting offset with the highest
/// aggregated priority once a set no longer fits.
#[test]
fn reap_leftover_picks_best_offset() {
    let (pool, _) = setup(0);
    // a: one 30-byte tx at priority 50; b, c: small txs at 40 and 30.
    let a = format!("a={}=50", "x".repeat(26));
    pool.check_tx(a.as_bytes(), TxInfo::default()).unwrap();
    pool.check_tx(b"b=1=40", TxInfo::default()).unwrap();
    pool.check_tx(b"c=1=30", TxInfo::default()).unwrap();
    // 20 bytes: a does not fit; starting at b gives b and c (agg 35).
    let got = priorities_of(&pool, &pool.reap_max_bytes_max_gas(20, -1));
    assert_eq!(got, vec![40, 30]);
}
