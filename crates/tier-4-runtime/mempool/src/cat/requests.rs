//! `mempool/cat/requests.go`: the outbound WantTx request scheduler.
//!
//! One request per tx at a time, at most [`MAX_REQUESTS_PER_PEER`]
//! outstanding per peer. After `response_time` without a response the
//! request is released (the callback may re-request from another peer) but
//! the peer's slot is kept for `global_timeout` so a late response is still
//! recognized as solicited. Go's `time.AfterFunc` timers become tokio tasks
//! (or threads outside a runtime).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::MAX_REQUESTS_PER_PEER;
use crate::types::TxKey;

/// `defaultGlobalRequestTimeout`.
pub const DEFAULT_GLOBAL_REQUEST_TIMEOUT: Duration = Duration::from_secs(3600);

/// Called with `(tx, peer)` when a request times out.
pub type OnTimeout = Arc<dyn Fn(TxKey, u16) + Send + Sync>;

/// A stoppable one-shot timer (`time.AfterFunc`).
#[derive(Debug)]
pub(crate) struct Timer {
    stopped: Arc<AtomicBool>,
    task: Option<tokio::task::AbortHandle>,
}

impl Timer {
    pub(crate) fn after(d: Duration, f: impl FnOnce() + Send + 'static) -> Self {
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = stopped.clone();
        let run = move || {
            if !flag.load(Ordering::SeqCst) {
                f();
            }
        };
        if let Ok(h) = tokio::runtime::Handle::try_current() {
            let task = h.spawn(async move {
                tokio::time::sleep(d).await;
                // The callback may call into the app (blocking).
                let _ = tokio::task::spawn_blocking(run).await;
            });
            return Self {
                stopped,
                task: Some(task.abort_handle()),
            };
        }
        std::thread::spawn(move || {
            std::thread::sleep(d);
            run();
        });
        Self {
            stopped,
            task: None,
        }
    }

    pub(crate) fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(t) = &self.task {
            t.abort();
        }
    }
}

#[derive(Debug, Default)]
struct Inner {
    by_peer: HashMap<u16, HashMap<TxKey, Timer>>,
    by_tx: HashMap<TxKey, u16>,
}

impl Inner {
    /// `deletePeerRequest`.
    fn delete_peer_request(&mut self, peer: u16, key: &TxKey) {
        if let Some(set) = self.by_peer.get_mut(&peer) {
            set.remove(key);
            if set.is_empty() {
                self.by_peer.remove(&peer);
            }
        }
    }
}

/// `requestScheduler`.
#[derive(Debug, Clone)]
pub struct RequestScheduler {
    inner: Arc<Mutex<Inner>>,
    response_time: Duration,
    global_timeout: Duration,
}

impl RequestScheduler {
    /// `newRequestScheduler`.
    #[must_use]
    pub fn new(response_time: Duration, global_timeout: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            response_time,
            global_timeout,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `Add`: reserves the request. False if the tx is already requested,
    /// the peer is at capacity, or the peer still holds a slot for this tx.
    pub fn add(&self, key: TxKey, peer: u16, on_timeout: Option<OnTimeout>) -> bool {
        if peer == 0 {
            return false;
        }
        let mut s = self.lock();
        if s.by_tx.contains_key(&key) {
            return false;
        }
        if s.by_peer.get(&peer).map_or(0, HashMap::len) >= MAX_REQUESTS_PER_PEER {
            return false;
        }
        if s.by_peer
            .get(&peer)
            .is_some_and(|set| set.contains_key(&key))
        {
            return false;
        }
        let inner = self.inner.clone();
        let global = self.global_timeout;
        let timer = Timer::after(self.response_time, move || {
            {
                let mut s = inner
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                // Fulfilled by another peer while waiting for the lock.
                if s.by_tx.get(&key) != Some(&peer) {
                    return;
                }
                s.by_tx.remove(&key);
            }
            if let Some(cb) = on_timeout {
                cb(key, peer);
            }
            // Keep the slot for a late response; garbage-collect it later.
            let gc = Timer::after(global, move || {
                inner
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .delete_peer_request(peer, &key);
            });
            drop(gc);
        });
        s.by_peer.entry(peer).or_default().insert(key, timer);
        s.by_tx.insert(key, peer);
        true
    }

    /// `ForTx`: the peer the tx is requested from (0: none).
    #[must_use]
    pub fn for_tx(&self, key: &TxKey) -> u16 {
        self.lock().by_tx.get(key).copied().unwrap_or(0)
    }

    /// `Has`: whether `peer` holds a request slot for the tx.
    #[must_use]
    pub fn has(&self, peer: u16, key: &TxKey) -> bool {
        self.lock()
            .by_peer
            .get(&peer)
            .is_some_and(|s| s.contains_key(key))
    }

    /// `CountForPeer`.
    #[must_use]
    pub fn count_for_peer(&self, peer: u16) -> usize {
        self.lock().by_peer.get(&peer).map_or(0, HashMap::len)
    }

    /// `ClearAllRequestsFrom`: drops the peer's requests, returning their
    /// keys.
    #[allow(clippy::must_use_candidate)]
    pub fn clear_all_requests_from(&self, peer: u16) -> Vec<TxKey> {
        let mut s = self.lock();
        let Some(requests) = s.by_peer.remove(&peer) else {
            return Vec::new();
        };
        let mut keys = Vec::with_capacity(requests.len());
        for (key, timer) in requests {
            timer.stop();
            s.by_tx.remove(&key);
            keys.push(key);
        }
        keys
    }

    /// `Remove`: drops the request if it still belongs to `peer`.
    #[allow(clippy::must_use_candidate)]
    pub fn remove(&self, key: &TxKey, peer: u16) -> bool {
        let mut s = self.lock();
        if s.by_tx.get(key) != Some(&peer) {
            return false;
        }
        if let Some(t) = s.by_peer.get(&peer).and_then(|set| set.get(key)) {
            t.stop();
            s.delete_peer_request(peer, key);
        }
        s.by_tx.remove(key);
        true
    }

    /// `MarkReceived`: true if `peer` had a request slot for the tx.
    #[allow(clippy::must_use_candidate)]
    pub fn mark_received(&self, peer: u16, key: &TxKey) -> bool {
        let mut s = self.lock();
        let Some(set) = s.by_peer.get_mut(&peer) else {
            return false;
        };
        let Some(timer) = set.remove(key) else {
            return false;
        };
        timer.stop();
        // Go leaves an empty per-peer map behind here.
        // A late response clears the tx's request only if it is still ours.
        if s.by_tx.get(key) == Some(&peer) {
            s.by_tx.remove(key);
        }
        true
    }

    /// `Close`: stops every timer.
    pub fn close(&self) {
        let s = self.lock();
        for set in s.by_peer.values() {
            for t in set.values() {
                t.stop();
            }
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::type_complexity,
    clippy::needless_range_loop
)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use crate::types::tx_key;

    const MS: Duration = Duration::from_millis(1);
    const MINUTE: Duration = Duration::from_secs(60);

    #[tokio::test(flavor = "multi_thread")]
    async fn rerequest() {
        let r = RequestScheduler::new(10 * MS, MINUTE);
        let key = tx_key(b"tx");
        assert_eq!(r.for_tx(&key), 0);
        assert!(!r.has(1, &key));
        assert!(!r.mark_received(1, &key));

        let (tx, rx) = mpsc::channel();
        let r2 = r.clone();
        assert!(r.add(
            key,
            1,
            Some(Arc::new(move |k, p| {
                assert_eq!((k, p), (key, 1));
                assert!(r2.add(k, 2, Some(Arc::new(|_, _| panic!("unexpected timeout")))));
                tx.send(()).unwrap();
            }))
        ));
        assert_eq!(r.for_tx(&key), 1);
        assert!(r.has(1, &key));
        assert!(!r.add(key, 1, None));

        tokio::task::spawn_blocking(move || rx.recv().unwrap())
            .await
            .unwrap();
        assert!(r.has(1, &key));
        assert!(r.has(2, &key));
        assert_eq!(r.for_tx(&key), 2);
        assert!(r.mark_received(2, &key));
        assert!(r.mark_received(1, &key));
        r.close();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn late_response_does_not_clobber_rerequest() {
        let r = RequestScheduler::new(10 * MS, MINUTE);
        let key = tx_key(b"tx");
        let (tx, rx) = mpsc::channel();
        let r2 = r.clone();
        assert!(r.add(
            key,
            1,
            Some(Arc::new(move |k, _| {
                assert!(r2.add(k, 2, None));
                tx.send(()).unwrap();
            }))
        ));
        tokio::task::spawn_blocking(move || rx.recv().unwrap())
            .await
            .unwrap();
        assert_eq!(r.for_tx(&key), 2);
        assert!(r.mark_received(1, &key));
        assert_eq!(r.for_tx(&key), 2);
        assert!(r.mark_received(2, &key));
        assert_eq!(r.for_tx(&key), 0);
        r.close();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn timed_out_peer_cannot_be_readded() {
        let r = RequestScheduler::new(10 * MS, MINUTE);
        let key = tx_key(b"tx");
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Mutex::new(release_rx);
        assert!(r.add(
            key,
            1,
            Some(Arc::new(move |_, _| {
                entered_tx.send(()).unwrap();
                let _ = release_rx.lock().unwrap().recv();
            }))
        ));
        tokio::task::spawn_blocking(move || entered_rx.recv().unwrap())
            .await
            .unwrap();
        assert_eq!(r.for_tx(&key), 0);
        assert!(r.has(1, &key));
        assert!(!r.add(key, 1, None));
        assert!(r.add(key, 2, None));
        drop(release_tx);
        r.close();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn non_responsive_peer() {
        let r = RequestScheduler::new(10 * MS, MS);
        let key = tx_key(b"tx");
        assert!(r.add(key, 1, None));
        for _ in 0..20 {
            if r.for_tx(&key) == 0 {
                return;
            }
            tokio::time::sleep(5 * MS).await;
        }
        panic!("request did not time out");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_adds_and_reads() {
        let r = RequestScheduler::new(10 * MS, MS);
        let n = 5usize;
        let keys: Vec<TxKey> = (0..n)
            .map(|i| tx_key(format!("tx{i}").as_bytes()))
            .collect();
        let done = Arc::new(AtomicBool::new(false));
        let adds: Vec<_> = (1..n * n)
            .map(|i| {
                let (r, keys) = (r.clone(), keys.clone());
                std::thread::spawn(move || {
                    r.add(keys[i % n], i as u16, None);
                })
            })
            .collect();
        let receivers: Vec<_> = (1..n * n)
            .map(|i| {
                let (r, keys, done) = (r.clone(), keys.clone(), done.clone());
                std::thread::spawn(move || {
                    let mark = || {
                        for k in &keys {
                            if r.has(i as u16, k) {
                                r.mark_received(i as u16, k);
                            }
                        }
                    };
                    while !done.load(Ordering::SeqCst) {
                        mark();
                    }
                    mark();
                })
            })
            .collect();
        for h in adds {
            h.join().unwrap();
        }
        done.store(true, Ordering::SeqCst);
        for h in receivers {
            h.join().unwrap();
        }
        for k in &keys {
            assert_eq!(r.for_tx(k), 0);
        }
        r.close();
    }

    #[test]
    fn remove() {
        let r = RequestScheduler::new(MINUTE, MINUTE);
        let key = tx_key(b"tx");
        r.remove(&key, 1);
        assert!(r.add(key, 1, None));
        r.remove(&key, 2);
        assert_eq!(r.for_tx(&key), 1);
        assert!(r.has(1, &key));
        r.remove(&key, 1);
        assert_eq!(r.for_tx(&key), 0);
        assert!(!r.has(1, &key));
        assert_eq!(r.count_for_peer(1), 0);
        assert!(r.add(key, 1, None));
        r.close();
    }

    #[test]
    fn remove_does_not_remove_another_peers_request() {
        let r = RequestScheduler::new(MINUTE, MINUTE);
        let key = tx_key(b"tx");
        assert!(r.add(key, 1, None));
        r.clear_all_requests_from(1);
        assert!(r.add(key, 2, None));
        r.remove(&key, 1);
        assert_eq!(r.for_tx(&key), 2);
        assert!(r.has(2, &key));
        r.close();
    }

    #[test]
    fn per_peer_limit() {
        let r = RequestScheduler::new(MINUTE, MINUTE);
        let keys: Vec<TxKey> = (0..MAX_REQUESTS_PER_PEER)
            .map(|i| tx_key(format!("tx-{i}").as_bytes()))
            .collect();
        for k in &keys {
            assert!(r.add(*k, 1, None));
        }
        let overflow = tx_key(b"overflow");
        assert!(!r.add(overflow, 1, None));
        assert!(r.add(overflow, 2, None));
        assert!(r.mark_received(1, &keys[0]));
        assert!(r.add(tx_key(b"extra"), 1, None));
        r.close();
    }

    #[test]
    fn count_for_peer() {
        let r = RequestScheduler::new(MINUTE, MINUTE);
        assert_eq!(r.count_for_peer(1), 0);
        for i in 0..5 {
            assert!(r.add(tx_key(format!("tx-a-{i}").as_bytes()), 1, None));
        }
        for i in 0..3 {
            assert!(r.add(tx_key(format!("tx-b-{i}").as_bytes()), 2, None));
        }
        assert_eq!((r.count_for_peer(1), r.count_for_peer(2)), (5, 3));
        r.mark_received(1, &tx_key(b"tx-a-0"));
        assert_eq!(r.count_for_peer(1), 4);
        r.clear_all_requests_from(2);
        assert_eq!((r.count_for_peer(1), r.count_for_peer(2)), (4, 0));
        r.close();
    }
}
