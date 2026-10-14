//! Test doubles: the Go tests' kvstore-based `application` /
//! `sequenceTrackingApp` and a recording mock peer.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::type_complexity,
    clippy::needless_range_loop
)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use celestia_core_proto::abci::{RequestCheckTx, ResponseCheckTx};

use super::peers::{MempoolPeer, PeerState};
use super::pool::TxPool;
use super::reactor::{Reactor, ReactorOptions};
use crate::proxy::AppConnMempool;
use crate::types::MempoolConfig;

/// CheckTx parses `sender=key=priority`: priority from the last part,
/// sender (address) from the first, gas 1; QuerySequence reads a map
/// (default 0).
#[derive(Default)]
pub struct TestApp {
    sequences: Mutex<HashMap<Vec<u8>, u64>>,
    check_tx_calls: AtomicUsize,
    /// Advance the sender's sequence on every accepted New CheckTx (like
    /// the check state does).
    pub auto_sequence: AtomicBool,
}

impl TestApp {
    pub fn set_sequence(&self, signer: &[u8], seq: u64) {
        self.sequences.lock().unwrap().insert(signer.to_vec(), seq);
    }

    pub fn check_tx_calls(&self) -> usize {
        self.check_tx_calls.load(Ordering::SeqCst)
    }
}

impl AppConnMempool for TestApp {
    fn check_tx(&self, req: RequestCheckTx) -> Result<ResponseCheckTx, String> {
        self.check_tx_calls.fetch_add(1, Ordering::SeqCst);
        let parts: Vec<&[u8]> = req.tx.split(|&b| b == b'=').collect();
        if parts.len() != 3 {
            return Ok(ResponseCheckTx {
                code: 101,
                gas_wanted: 1,
                log: "invalid-tx-format".into(),
                ..ResponseCheckTx::default()
            });
        }
        let Some(priority) = std::str::from_utf8(parts[2])
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
        else {
            return Ok(ResponseCheckTx {
                code: 100,
                gas_wanted: 1,
                ..ResponseCheckTx::default()
            });
        };
        // sequenceTrackingTestApp: the last field is also the sequence,
        // and the signer's expected sequence moves past it.
        let mut sequence = 0;
        if self.auto_sequence.load(Ordering::SeqCst) {
            sequence = u64::try_from(priority).unwrap_or(0);
            self.sequences
                .lock()
                .unwrap()
                .insert(parts[0].to_vec(), sequence + 1);
        }
        Ok(ResponseCheckTx {
            sequence,
            priority,
            address: parts[0].to_vec(),
            code: 0,
            gas_wanted: 1,
            ..ResponseCheckTx::default()
        })
    }

    fn query_sequence(&self, signer: &[u8]) -> Result<u64, String> {
        Ok(self
            .sequences
            .lock()
            .unwrap()
            .get(signer)
            .copied()
            .unwrap_or(0))
    }
}

/// `newTx(i, peerID, msg, priority)`.
pub fn new_tx(i: usize, peer_id: u16, msg: &[u8], priority: i64) -> Vec<u8> {
    format!(
        "sender-{i:03}-{peer_id}={}={priority}",
        hex::encode_upper(msg)
    )
    .into_bytes()
}

/// `newDefaultTx(msg)`.
pub fn new_default_tx(msg: &str) -> Vec<u8> {
    new_tx(0, 0, msg.as_bytes(), 1)
}

pub fn setup_with(
    cache_size: usize,
    f: impl FnOnce(&mut MempoolConfig),
) -> (Arc<TxPool>, Arc<TestApp>) {
    let app = Arc::new(TestApp::default());
    let mut cfg = MempoolConfig::test();
    cfg.cache_size = cache_size;
    f(&mut cfg);
    (Arc::new(TxPool::new(cfg, app.clone(), 1)), app)
}

pub fn setup(cache_size: usize) -> (Arc<TxPool>, Arc<TestApp>) {
    setup_with(cache_size, |_| {})
}

/// `setupReactor` / `newMempoolWithApp` + `NewReactor`.
pub fn setup_reactor_with(opts: ReactorOptions) -> (Arc<Reactor>, Arc<TxPool>, Arc<TestApp>) {
    let app = Arc::new(TestApp::default());
    let pool = Arc::new(TxPool::new(MempoolConfig::test(), app.clone(), 1));
    let r = Reactor::new(pool.clone(), opts).unwrap();
    (r, pool, app)
}

pub fn setup_reactor() -> (Arc<Reactor>, Arc<TxPool>, Arc<TestApp>) {
    setup_reactor_with(ReactorOptions::default())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    Send(u8, Vec<u8>),
    TrySend(u8, Vec<u8>),
}

pub struct FixedHeight(pub i64);

impl PeerState for FixedHeight {
    fn get_height(&self) -> i64 {
        self.0
    }
}

type Hook = Box<dyn FnMut(u8, &[u8]) -> Option<bool> + Send>;

/// A recording peer (`mocks.Peer`).
pub struct MockPeer {
    pub id: String,
    pub persistent: bool,
    pub height: Option<i64>,
    pub calls: Mutex<Vec<Call>>,
    pub send_ok: AtomicBool,
    pub try_send_ok: AtomicBool,
    /// Runs on TrySend; `Some(r)` overrides the result.
    pub try_send_hook: Mutex<Option<Hook>>,
}

impl MockPeer {
    pub fn named(id: &str) -> Arc<Self> {
        Arc::new(Self {
            id: id.into(),
            persistent: false,
            height: None,
            calls: Mutex::new(Vec::new()),
            send_ok: AtomicBool::new(true),
            try_send_ok: AtomicBool::new(true),
            try_send_hook: Mutex::new(None),
        })
    }

    /// `genPeer`: a random node ID.
    pub fn random() -> Arc<Self> {
        let id: [u8; 20] = rand::random();
        Self::named(&hex::encode(id))
    }

    pub fn with(id: &str, persistent: bool) -> Arc<Self> {
        let mut p = Arc::try_unwrap(Self::named(id)).ok().unwrap();
        p.persistent = persistent;
        Arc::new(p)
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    pub fn try_sends(&self) -> Vec<(u8, Vec<u8>)> {
        self.calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::TrySend(ch, m) => Some((ch, m)),
                Call::Send(..) => None,
            })
            .collect()
    }

    pub fn sends(&self) -> Vec<(u8, Vec<u8>)> {
        self.calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::Send(ch, m) => Some((ch, m)),
                Call::TrySend(..) => None,
            })
            .collect()
    }

    pub fn as_dyn(self: &Arc<Self>) -> Arc<dyn MempoolPeer> {
        self.clone()
    }
}

impl MempoolPeer for MockPeer {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn is_persistent(&self) -> bool {
        self.persistent
    }

    fn peer_state(&self) -> Option<Arc<dyn PeerState>> {
        self.height
            .map(|h| Arc::new(FixedHeight(h)) as Arc<dyn PeerState>)
    }

    fn send(&self, ch: u8, msg: Vec<u8>) -> bool {
        self.calls.lock().unwrap().push(Call::Send(ch, msg));
        self.send_ok.load(Ordering::SeqCst)
    }

    fn try_send(&self, ch: u8, msg: Vec<u8>) -> bool {
        self.calls
            .lock()
            .unwrap()
            .push(Call::TrySend(ch, msg.clone()));
        let hook = self.try_send_hook.lock().unwrap().take();
        if let Some(mut h) = hook {
            let r = h(ch, &msg);
            if let Some(r) = r {
                return r;
            }
        }
        self.try_send_ok.load(Ordering::SeqCst)
    }
}
