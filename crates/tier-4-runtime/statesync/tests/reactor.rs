//! Two Rust nodes over loopback p2p: one serves a multi-chunk snapshot
//! through its statesync reactor, the other discovers it, fetches every
//! chunk (one of them is first served corrupted and refetched) and
//! restores it (cf. celestia-core `statesync/reactor_test.go`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use celestia_core_proto::tendermint::abci;
use ctypes::Commit;
use p2p::{MConnConfig, NodeInfo, NodeKey, ProtocolVersion, Switch, SwitchConfig};
use statesync::syncer::Config;
use statesync::{AppConn, Reactor, State, StateProvider};

const CHUNKS: u32 = 6;
const HEIGHT: u64 = 7;

fn body(i: u32) -> Vec<u8> {
    vec![u8::try_from(i).unwrap(); 1000 + 10 * usize::try_from(i).unwrap()]
}

/// Serves one snapshot.
struct Server;

impl AppConn for Server {
    fn list_snapshots(&self) -> Result<abci::ResponseListSnapshots, String> {
        Ok(abci::ResponseListSnapshots {
            snapshots: vec![
                abci::Snapshot {
                    height: HEIGHT,
                    format: 1,
                    chunks: CHUNKS,
                    hash: vec![9; 32],
                    metadata: vec![1],
                },
                abci::Snapshot {
                    height: 3,
                    format: 1,
                    chunks: 1,
                    hash: vec![8; 32],
                    metadata: vec![],
                },
            ],
        })
    }
    fn load_snapshot_chunk(
        &self,
        req: abci::RequestLoadSnapshotChunk,
    ) -> Result<abci::ResponseLoadSnapshotChunk, String> {
        assert_eq!((req.height, req.format), (HEIGHT, 1));
        Ok(abci::ResponseLoadSnapshotChunk {
            chunk: if req.chunk < CHUNKS {
                body(req.chunk)
            } else {
                vec![]
            },
        })
    }
    fn offer_snapshot(
        &self,
        _: abci::RequestOfferSnapshot,
    ) -> Result<abci::ResponseOfferSnapshot, String> {
        Err("server only".into())
    }
    fn apply_snapshot_chunk(
        &self,
        _: abci::RequestApplySnapshotChunk,
    ) -> Result<abci::ResponseApplySnapshotChunk, String> {
        Err("server only".into())
    }
    fn info(&self, _: abci::RequestInfo) -> Result<abci::ResponseInfo, String> {
        Err("server only".into())
    }
}

/// Restores: checks every chunk, asks for chunk 2 to be refetched once.
#[derive(Default)]
struct Client {
    applied: Mutex<Vec<u32>>,
    refetched: Mutex<bool>,
}

impl AppConn for Client {
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
        let s = req.snapshot.unwrap();
        assert_eq!((s.height, s.chunks), (HEIGHT, CHUNKS));
        assert_eq!(req.app_hash, vec![7u8; 32]);
        Ok(abci::ResponseOfferSnapshot {
            result: abci::response_offer_snapshot::Result::Accept as i32,
        })
    }
    fn apply_snapshot_chunk(
        &self,
        req: abci::RequestApplySnapshotChunk,
    ) -> Result<abci::ResponseApplySnapshotChunk, String> {
        use abci::response_apply_snapshot_chunk::Result as R;
        assert_eq!(req.chunk, body(req.index));
        assert!(!req.sender.is_empty());
        self.applied.lock().unwrap().push(req.index);
        let mut refetched = self.refetched.lock().unwrap();
        if req.index == 2 && !*refetched {
            *refetched = true;
            return Ok(abci::ResponseApplySnapshotChunk {
                result: R::Retry as i32,
                refetch_chunks: vec![2],
                reject_senders: vec![],
            });
        }
        Ok(abci::ResponseApplySnapshotChunk {
            result: R::Accept as i32,
            ..Default::default()
        })
    }
    fn info(&self, _: abci::RequestInfo) -> Result<abci::ResponseInfo, String> {
        Ok(abci::ResponseInfo {
            app_version: 10,
            last_block_height: i64::try_from(HEIGHT).unwrap(),
            last_block_app_hash: vec![7u8; 32],
            ..Default::default()
        })
    }
}

struct Trusted;

impl StateProvider for Trusted {
    fn app_hash(&self, h: u64) -> light::Result<Vec<u8>> {
        Ok(vec![u8::try_from(h).unwrap(); 32])
    }
    fn commit(&self, h: u64) -> light::Result<Commit> {
        Ok(Commit {
            height: i64::try_from(h).unwrap(),
            ..Commit::default()
        })
    }
    fn state(&self, h: u64) -> light::Result<State> {
        let mut s = statesync::empty_state("c", ctypes::Consensus { block: 11, app: 10 }, 1);
        s.last_block_height = i64::try_from(h).unwrap();
        s.consensus_params.version.app = 10;
        Ok(s)
    }
}

fn node(
    seed: u8,
    persistent: Vec<String>,
    conn: Arc<dyn AppConn>,
    tmp: &std::path::Path,
) -> (Switch, Arc<Reactor>) {
    let info = NodeInfo {
        protocol_version: ProtocolVersion {
            p2p: 8,
            block: 11,
            app: 0,
        },
        network: "ss-test".into(),
        version: "0.38.17".into(),
        moniker: format!("node{seed}"),
        ..NodeInfo::default()
    };
    let cfg = SwitchConfig {
        listen_address: Some("tcp://127.0.0.1:0".into()),
        allow_duplicate_ip: true,
        persistent_peers: persistent,
        mconn: MConnConfig {
            flush_throttle: Duration::from_millis(5),
            ..MConnConfig::default()
        },
        ..SwitchConfig::default()
    };
    let sw = Switch::new(cfg, NodeKey::from_seed(&[seed; 32]), info).unwrap();
    let r = Arc::new(Reactor::new(
        Config {
            temp_dir: tmp.to_path_buf(),
            chunk_request_timeout: Duration::from_secs(2),
            ..Config::default()
        },
        conn,
    ));
    sw.add_reactor(r.clone()).unwrap();
    (sw, r)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sync_between_rust_nodes() {
    let tmp = tempfile::tempdir().unwrap();
    let (server, server_r) = node(1, vec![], Arc::new(Server), tmp.path());
    server.start().await.unwrap();
    let addr = format!("{}", server.net_address().unwrap());
    let client_conn = Arc::new(Client::default());
    let (client, client_r) = node(2, vec![addr], client_conn.clone(), tmp.path());
    client.start().await.unwrap();

    // the server answers snapshot listings from its app (newest first)
    let recent = server_r.recent_snapshots(10).unwrap();
    assert_eq!(
        recent.iter().map(|s| s.height).collect::<Vec<_>>(),
        vec![HEIGHT, 3]
    );

    let (state, commit) = tokio::task::spawn_blocking(move || {
        client_r.sync(Arc::new(Trusted), Duration::from_secs(5))
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(state.last_block_height, 7);
    assert_eq!(commit.height, 7);
    let applied = client_conn.applied.lock().unwrap().clone();
    assert_eq!(applied, vec![0, 1, 2, 2, 3, 4, 5]);
    // the temp dir holds no leftover chunk files
    assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    client.stop();
    server.stop();
}
