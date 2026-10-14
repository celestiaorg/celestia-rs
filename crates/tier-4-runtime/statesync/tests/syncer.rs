//! Syncer behavior (ports of celestia-core `statesync/syncer_test.go`):
//! snapshot selection and rejection, offer / apply result codes, chunk
//! refetching, sender rejection, retries and app verification.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use celestia_core_proto::tendermint::abci;
use celestia_core_proto::tendermint::statesync as pbss;
use ctypes::Commit;
use statesync::chunks::ChunkQueue;
use statesync::messages::{CHUNK_CHANNEL, Msg};
use statesync::syncer::{Config, SyncError, Syncer};
use statesync::{AppConn, PeerHandle, PeerRef, Snapshot, State, StateProvider};

use abci::response_apply_snapshot_chunk::Result as ApplyR;
use abci::response_offer_snapshot::Result as OfferR;

#[derive(Default)]
struct Script {
    offers: VecDeque<i32>,
    applies: VecDeque<abci::ResponseApplySnapshotChunk>,
    info: Option<abci::ResponseInfo>,
    offered: Vec<(u64, u32, Vec<u8>, u64)>,
    applied: Vec<(u32, Vec<u8>, String)>,
}

#[derive(Default)]
struct ScriptConn(Mutex<Script>);

impl AppConn for ScriptConn {
    fn list_snapshots(&self) -> Result<abci::ResponseListSnapshots, String> {
        Ok(abci::ResponseListSnapshots::default())
    }
    fn load_snapshot_chunk(
        &self,
        _: abci::RequestLoadSnapshotChunk,
    ) -> Result<abci::ResponseLoadSnapshotChunk, String> {
        Ok(abci::ResponseLoadSnapshotChunk::default())
    }
    fn offer_snapshot(
        &self,
        req: abci::RequestOfferSnapshot,
    ) -> Result<abci::ResponseOfferSnapshot, String> {
        let mut s = self.0.lock().unwrap();
        let snap = req.snapshot.unwrap();
        s.offered
            .push((snap.height, snap.format, req.app_hash, req.app_version));
        let result = s.offers.pop_front().expect("unexpected offer");
        Ok(abci::ResponseOfferSnapshot { result })
    }
    fn apply_snapshot_chunk(
        &self,
        req: abci::RequestApplySnapshotChunk,
    ) -> Result<abci::ResponseApplySnapshotChunk, String> {
        let mut s = self.0.lock().unwrap();
        s.applied.push((req.index, req.chunk, req.sender));
        Ok(s.applies.pop_front().expect("unexpected apply"))
    }
    fn info(&self, _: abci::RequestInfo) -> Result<abci::ResponseInfo, String> {
        Ok(self.0.lock().unwrap().info.clone().unwrap())
    }
}

struct MockState {
    fail: Option<light::Error>,
}

fn app_hash_for(h: u64) -> Vec<u8> {
    vec![u8::try_from(h).unwrap(); 32]
}

impl StateProvider for MockState {
    fn app_hash(&self, height: u64) -> light::Result<Vec<u8>> {
        if let Some(e) = &self.fail {
            return Err(e.clone());
        }
        Ok(app_hash_for(height))
    }
    fn commit(&self, height: u64) -> light::Result<Commit> {
        Ok(Commit {
            height: i64::try_from(height).unwrap(),
            ..Commit::default()
        })
    }
    fn state(&self, height: u64) -> light::Result<State> {
        let mut s = statesync::empty_state("c", ctypes::Consensus { block: 11, app: 10 }, 1);
        s.last_block_height = i64::try_from(height).unwrap();
        s.app_hash = app_hash_for(height);
        s.consensus_params.version.app = 10;
        Ok(s)
    }
}

/// A peer that forwards chunk requests to a responder.
struct ReqPeer {
    id: String,
    tx: Mutex<mpsc::Sender<(String, pbss::ChunkRequest)>>,
}

impl PeerHandle for ReqPeer {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn send(&self, ch: u8, msg: Vec<u8>) -> bool {
        if ch == CHUNK_CHANNEL
            && let Ok(Msg::ChunkRequest(r)) = Msg::decode(&msg)
        {
            let _ = self.tx.lock().unwrap().send((self.id.clone(), r));
        }
        true
    }
}

fn chunk_body(h: u64, f: u32, i: u32) -> Vec<u8> {
    vec![
        u8::try_from(h).unwrap(),
        u8::try_from(f).unwrap(),
        u8::try_from(i).unwrap(),
    ]
}

struct Harness {
    syncer: Arc<Syncer>,
    conn: Arc<ScriptConn>,
    peers: Vec<PeerRef>,
    _dir: tempfile::TempDir,
}

fn harness(peer_ids: &[&str], fail: Option<light::Error>) -> Harness {
    harness_answering(peer_ids, fail, u32::MAX)
}

/// A harness whose peers only answer chunk requests below `answer_below`.
fn harness_answering(peer_ids: &[&str], fail: Option<light::Error>, answer_below: u32) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let conn = Arc::new(ScriptConn::default());
    let cfg = Config {
        temp_dir: dir.path().to_path_buf(),
        chunk_request_timeout: Duration::from_millis(300),
        chunk_fetchers: 2,
        chunk_timeout: Duration::from_secs(5),
        ..Config::default()
    };
    let syncer = Arc::new(Syncer::new(
        cfg,
        Arc::clone(&conn) as Arc<dyn AppConn>,
        Arc::new(MockState { fail }),
    ));
    let (tx, rx) = mpsc::channel::<(String, pbss::ChunkRequest)>();
    let peers: Vec<PeerRef> = peer_ids
        .iter()
        .map(|id| {
            Arc::new(ReqPeer {
                id: (*id).to_owned(),
                tx: Mutex::new(tx.clone()),
            }) as PeerRef
        })
        .collect();
    let s2 = Arc::clone(&syncer);
    std::thread::spawn(move || {
        while let Ok((peer, r)) = rx.recv() {
            if r.index >= answer_below {
                continue;
            }
            let _ = s2.add_chunk(&statesync::chunks::Chunk {
                height: r.height,
                format: r.format,
                index: r.index,
                chunk: chunk_body(r.height, r.format, r.index),
                sender: peer,
            });
        }
    });
    Harness {
        syncer,
        conn,
        peers,
        _dir: dir,
    }
}

fn snap(height: u64, format: u32, chunks: u32) -> Snapshot {
    Snapshot {
        height,
        format,
        chunks,
        hash: vec![1, 2, 3],
        metadata: vec![],
        ..Snapshot::default()
    }
}

fn accept() -> abci::ResponseApplySnapshotChunk {
    abci::ResponseApplySnapshotChunk {
        result: ApplyR::Accept as i32,
        ..Default::default()
    }
}

fn info(h: u64) -> abci::ResponseInfo {
    abci::ResponseInfo {
        app_version: 10,
        last_block_height: i64::try_from(h).unwrap(),
        last_block_app_hash: app_hash_for(h),
        timeout_info: Some(app::timeout_info()),
        ..Default::default()
    }
}

/// `TestSyncer_SyncAny`: format and snapshot rejection, chunk retry with
/// refetch, retry-snapshot, then a successful restore.
#[test]
fn sync_any_full_flow() {
    let h = harness(&["aa", "bb"], None);
    // s(2,2) best, then s(2,1), then s(1,1)
    assert!(h.syncer.add_snapshot(&h.peers[0], snap(1, 1, 3)));
    assert!(!h.syncer.add_snapshot(&h.peers[1], snap(1, 1, 3)));
    assert!(h.syncer.add_snapshot(&h.peers[0], snap(2, 2, 3)));
    assert!(h.syncer.add_snapshot(&h.peers[1], snap(2, 1, 3)));
    {
        let mut s = h.conn.0.lock().unwrap();
        s.offers = [
            OfferR::RejectFormat as i32,
            OfferR::Reject as i32,
            OfferR::Accept as i32,
            OfferR::Accept as i32,
        ]
        .into();
        s.applies = [
            accept(),
            // chunk 1: retry and refetch it
            abci::ResponseApplySnapshotChunk {
                result: ApplyR::Retry as i32,
                refetch_chunks: vec![1],
                ..Default::default()
            },
            accept(),
            // chunk 2: retry the whole snapshot
            abci::ResponseApplySnapshotChunk {
                result: ApplyR::RetrySnapshot as i32,
                ..Default::default()
            },
            accept(),
            accept(),
            accept(),
        ]
        .into();
        s.info = Some(info(1));
    }
    let (state, commit) = h.syncer.sync_any(Duration::ZERO, &|| {}).unwrap();
    assert_eq!(state.last_block_height, 1);
    assert_eq!(commit.height, 1);
    assert_eq!(
        state.timeouts.commit,
        ctypes::time::duration_from_proto(app::timeout_info().timeout_commit.as_ref()).unwrap()
    );
    let s = h.conn.0.lock().unwrap();
    let offered: Vec<(u64, u32)> = s.offered.iter().map(|o| (o.0, o.1)).collect();
    // after RetrySnapshot the same snapshot is offered again
    assert_eq!(offered, vec![(2, 2), (2, 1), (1, 1), (1, 1)]);
    assert!(
        s.offered
            .iter()
            .all(|o| o.2 == app_hash_for(o.0) && o.3 == 10)
    );
    let applied: Vec<u32> = s.applied.iter().map(|a| a.0).collect();
    assert_eq!(applied, vec![0, 1, 1, 2, 0, 1, 2]);
    assert!(s.applied.iter().all(|a| a.1 == chunk_body(1, 1, a.0)));
    assert!(s.applied.iter().all(|a| a.2 == "aa" || a.2 == "bb"));
}

#[test]
fn sync_any_no_snapshots() {
    let h = harness(&["aa"], None);
    assert_eq!(
        h.syncer.sync_any(Duration::ZERO, &|| {}).err(),
        Some(SyncError::NoSnapshots)
    );
}

/// `TestSyncer_SyncAny_abort` / `_reject_sender` / `_abciError` and the
/// offer result table of `TestSyncer_offerSnapshot`.
#[test]
fn offer_results() {
    let cases = [
        (OfferR::Abort as i32, Err(SyncError::Abort)),
        (
            7,
            Err(SyncError::Msg(
                "snapshot restoration failed: unknown ResponseOfferSnapshot result 7".into(),
            )),
        ),
        (
            OfferR::Unknown as i32,
            Err(SyncError::Msg(
                "snapshot restoration failed: unknown ResponseOfferSnapshot result UNKNOWN".into(),
            )),
        ),
    ];
    for (result, want) in cases {
        let h = harness(&["aa"], None);
        h.syncer.add_snapshot(&h.peers[0], snap(1, 1, 1));
        h.conn.0.lock().unwrap().offers = [result].into();
        assert_eq!(
            h.syncer.sync_any(Duration::ZERO, &|| {}).map(|_| ()),
            want,
            "{result}"
        );
    }
    // reject sender: every peer of the snapshot is blacklisted, then none left
    let h = harness(&["aa", "bb", "cc"], None);
    h.syncer.add_snapshot(&h.peers[0], snap(2, 1, 1));
    h.syncer.add_snapshot(&h.peers[1], snap(2, 1, 1));
    h.syncer.add_snapshot(&h.peers[2], snap(1, 1, 1));
    h.conn.0.lock().unwrap().offers = [OfferR::RejectSender as i32, OfferR::Abort as i32].into();
    assert_eq!(
        h.syncer.sync_any(Duration::ZERO, &|| {}).err(),
        Some(SyncError::Abort)
    );
    let offered: Vec<u64> = h
        .conn
        .0
        .lock()
        .unwrap()
        .offered
        .iter()
        .map(|o| o.0)
        .collect();
    assert_eq!(offered, vec![2, 1]);
    assert!(!h.syncer.add_snapshot(&h.peers[0], snap(3, 1, 1)));
    assert!(h.syncer.add_snapshot(&h.peers[2], snap(3, 1, 1)));
}

/// `TestSyncer_applyChunks_Results` and `_RejectSenders`.
#[test]
fn apply_results() {
    let cases: [(i32, Option<SyncError>); 4] = [
        (ApplyR::Abort as i32, Some(SyncError::Abort)),
        (
            ApplyR::RejectSnapshot as i32,
            None, // rejected, then no more snapshots
        ),
        (
            ApplyR::Unknown as i32,
            Some(SyncError::Msg(
                "snapshot restoration failed: unknown ResponseApplySnapshotChunk result UNKNOWN"
                    .into(),
            )),
        ),
        (
            9,
            Some(SyncError::Msg(
                "snapshot restoration failed: unknown ResponseApplySnapshotChunk result 9".into(),
            )),
        ),
    ];
    for (result, want) in cases {
        let h = harness(&["aa"], None);
        h.syncer.add_snapshot(&h.peers[0], snap(1, 1, 2));
        {
            let mut s = h.conn.0.lock().unwrap();
            s.offers = [OfferR::Accept as i32].into();
            s.applies = [abci::ResponseApplySnapshotChunk {
                result,
                ..Default::default()
            }]
            .into();
        }
        let got = h.syncer.sync_any(Duration::ZERO, &|| {}).err();
        assert_eq!(
            got,
            Some(want.unwrap_or(SyncError::NoSnapshots)),
            "{result}"
        );
    }
}

/// Rejected senders: their unreturned chunks are discarded and refetched
/// from other peers, the peer is blacklisted and the callback fires.
#[test]
fn reject_senders() {
    let h = harness_answering(&["aa"], None, 1);
    let rejected = Arc::new(Mutex::new(Vec::new()));
    let r2 = Arc::clone(&rejected);
    h.syncer.set_on_peer_rejected(Arc::new(move |p: &str| {
        r2.lock().unwrap().push(p.to_owned());
    }));
    h.syncer.add_snapshot(&h.peers[0], snap(1, 1, 2));
    {
        let mut s = h.conn.0.lock().unwrap();
        s.offers = [OfferR::Accept as i32].into();
        s.applies = [
            abci::ResponseApplySnapshotChunk {
                result: ApplyR::Accept as i32,
                reject_senders: vec![String::new(), "aa".into()],
                ..Default::default()
            },
            // no peer left for chunk 1: the chunk never arrives
        ]
        .into();
    }
    let dir = tempfile::tempdir().unwrap();
    let q = Arc::new(
        ChunkQueue::new(&snap(1, 1, 2), dir.path())
            .unwrap()
            .with_chunk_timeout(Duration::from_secs(1)),
    );
    let mut s = snap(1, 1, 2);
    let res = h.syncer.sync(&mut s, &q);
    assert_eq!(res.err(), Some(SyncError::Timeout));
    assert_eq!(*rejected.lock().unwrap(), vec!["aa".to_owned()]);
    assert!(!h.syncer.add_snapshot(&h.peers[0], snap(5, 1, 1)));
}

/// `TestSyncer_verifyApp`.
#[test]
fn verify_app() {
    let cases = [
        (info(1), None),
        (
            abci::ResponseInfo {
                app_version: 9,
                ..info(1)
            },
            Some(SyncError::Msg(
                "snapshot restoration failed: app version mismatch. Expected: 10, got: 9".into(),
            )),
        ),
        (
            abci::ResponseInfo {
                last_block_app_hash: vec![9; 32],
                ..info(1)
            },
            Some(SyncError::Msg(
                "snapshot restoration failed: verification failed".into(),
            )),
        ),
        (
            abci::ResponseInfo {
                last_block_height: 2,
                ..info(1)
            },
            Some(SyncError::Msg(
                "snapshot restoration failed: verification failed".into(),
            )),
        ),
    ];
    for (i, want) in cases {
        let h = harness(&["aa"], None);
        h.syncer.add_snapshot(&h.peers[0], snap(1, 1, 1));
        {
            let mut s = h.conn.0.lock().unwrap();
            s.offers = [OfferR::Accept as i32].into();
            s.applies = [accept()].into();
            s.info = Some(i);
        }
        assert_eq!(h.syncer.sync_any(Duration::ZERO, &|| {}).err(), want);
    }
}

/// State provider failures: no witnesses aborts, anything else rejects the
/// snapshot.
#[test]
fn state_provider_errors() {
    let h = harness(&["aa"], Some(light::Error::NoWitnesses));
    h.syncer.add_snapshot(&h.peers[0], snap(1, 1, 1));
    assert_eq!(
        h.syncer.sync_any(Duration::ZERO, &|| {}).err(),
        Some(SyncError::Msg(
            "snapshot restoration failed: no witnesses connected. please reset light client".into()
        ))
    );
    let h = harness(&["aa"], Some(light::Error::msg("boom")));
    h.syncer.add_snapshot(&h.peers[0], snap(1, 1, 1));
    assert_eq!(
        h.syncer.sync_any(Duration::ZERO, &|| {}).err(),
        Some(SyncError::NoSnapshots)
    );
    assert!(h.conn.0.lock().unwrap().offered.is_empty());
}
