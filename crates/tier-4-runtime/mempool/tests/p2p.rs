//! CAT gossip between Rust nodes over real P2P switches (loopback TCP):
//! `TestReactorBroadcastTxsMessage` and the malformed-message bans
//! (`TestTxsWithEmptyTxBansPeer`, `TestSeenTxWithOversizedSignerBansPeer`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use celestia_core_proto::abci::{RequestCheckTx, ResponseCheckTx};
use celestia_core_proto::tendermint::mempool::{Message, SeenTx, Txs, message::Sum};
use mempool::cat::{MEMPOOL_DATA_CHANNEL, P2pReactor, Reactor, ReactorOptions, TxPool};
use mempool::{AppConnMempool, MempoolConfig, TxInfo, tx_key};
use p2p::{MConnConfig, NodeInfo, NodeKey, ProtocolVersion, Switch, SwitchConfig};

/// `sender=key=priority` app (the Go tests' kvstore override).
#[derive(Default)]
struct App {
    seqs: Mutex<HashMap<Vec<u8>, u64>>,
}

impl AppConnMempool for App {
    fn check_tx(&self, req: RequestCheckTx) -> Result<ResponseCheckTx, String> {
        let parts: Vec<&[u8]> = req.tx.split(|&b| b == b'=').collect();
        let prio = parts
            .get(2)
            .and_then(|p| std::str::from_utf8(p).ok()?.parse().ok());
        Ok(match (parts.len(), prio) {
            (3, Some(priority)) => ResponseCheckTx {
                priority,
                address: parts[0].to_vec(),
                gas_wanted: 1,
                ..ResponseCheckTx::default()
            },
            _ => ResponseCheckTx {
                code: 101,
                gas_wanted: 1,
                ..ResponseCheckTx::default()
            },
        })
    }

    fn query_sequence(&self, signer: &[u8]) -> Result<u64, String> {
        Ok(self.seqs.lock().unwrap().get(signer).copied().unwrap_or(0))
    }
}

struct Node {
    sw: Switch,
    pool: Arc<TxPool>,
    reactor: Arc<Reactor>,
}

fn node(seed: u8, persistent: Vec<String>) -> Node {
    let info = NodeInfo {
        protocol_version: ProtocolVersion {
            p2p: 8,
            block: 11,
            app: 0,
        },
        network: "test-chain".into(),
        version: "0.38.17".into(),
        moniker: format!("node{seed}"),
        ..NodeInfo::default()
    };
    let sw = Switch::new(
        SwitchConfig {
            listen_address: Some("tcp://127.0.0.1:0".into()),
            allow_duplicate_ip: true,
            persistent_peers: persistent,
            mconn: MConnConfig {
                flush_throttle: Duration::from_millis(5),
                ..MConnConfig::default()
            },
            ..SwitchConfig::default()
        },
        NodeKey::from_seed(&[seed; 32]),
        info,
    )
    .unwrap();
    let pool = Arc::new(TxPool::new(
        MempoolConfig::test(),
        Arc::new(App::default()),
        1,
    ));
    let reactor = Reactor::new(
        pool.clone(),
        ReactorOptions {
            sticky_peer_salt: vec![seed],
            ..ReactorOptions::default()
        },
    )
    .unwrap();
    sw.add_reactor(P2pReactor::new(reactor.clone())).unwrap();
    Node { sw, pool, reactor }
}

async fn wait_for(mut f: impl FnMut() -> bool) {
    for _ in 0..300 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("condition not reached");
}

/// Three nodes in a line (0 - 1 - 2): txs checked into node 0 reach node 2
/// through node 1 (SeenTx -> WantTx -> Txs at every hop), and every pool
/// reaps them in priority order.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn txs_gossip_across_switches() {
    let n0 = node(1, vec![]);
    n0.sw.start().await.unwrap();
    let n1 = node(2, vec![n0.sw.net_address().unwrap().to_string()]);
    n1.sw.start().await.unwrap();
    let n2 = node(3, vec![n1.sw.net_address().unwrap().to_string()]);
    n2.sw.start().await.unwrap();
    wait_for(|| n1.sw.peers().len() == 2 && n2.sw.peers().len() == 1).await;
    assert_eq!(n0.sw.node_info().channels, vec![0x30, 0x31, 0x32]);

    let mut prios = Vec::new();
    for i in 0..10 {
        let prio = 1000 + (i * 7919) % 9000;
        let tx = format!(
            "sender-{i:03}-0={}={prio}",
            hex::encode_upper([i as u8; 20])
        );
        n0.pool.check_tx(tx.as_bytes(), TxInfo::default()).unwrap();
        prios.push(prio);
    }
    prios.sort_unstable_by(|a, b| b.cmp(a));
    for n in [&n1, &n2] {
        wait_for(|| n.pool.size() == 10).await;
        let got: Vec<i64> = n
            .pool
            .reap_max_txs(-1)
            .iter()
            .map(|t| n.pool.get_wrapped(&t.key()).unwrap().priority)
            .collect();
        assert_eq!(got, prios);
    }
    for n in [&n0, &n1, &n2] {
        n.sw.stop();
    }
}

/// An empty tx and an oversized SeenTx signer get the sender disconnected.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn malformed_messages_disconnect_peer() {
    let n0 = node(4, vec![]);
    n0.sw.start().await.unwrap();
    let n1 = node(5, vec![]);
    n1.sw.start().await.unwrap();
    n1.sw
        .dial_peer_with_address(&n0.sw.net_address().unwrap())
        .await
        .unwrap();
    wait_for(|| n0.sw.peers().len() == 1).await;
    let to_n0 = n1.sw.get_peer(&n0.sw.node_id()).unwrap();
    let empty = Message {
        sum: Some(Sum::Txs(Txs {
            txs: vec![Vec::new()],
        })),
    };
    assert!(to_n0.send_msg(MEMPOOL_DATA_CHANNEL, &empty).await);
    wait_for(|| n0.sw.peers().is_empty()).await;

    n1.sw
        .dial_peer_with_address(&n0.sw.net_address().unwrap())
        .await
        .unwrap();
    wait_for(|| n0.sw.peers().len() == 1).await;
    let to_n0 = n1.sw.get_peer(&n0.sw.node_id()).unwrap();
    let ok = Message {
        sum: Some(Sum::SeenTx(SeenTx {
            tx_key: tx_key(b"x").to_vec(),
            sequence: 1,
            signer: vec![7; 64],
        })),
    };
    assert!(to_n0.send_msg(MEMPOOL_DATA_CHANNEL, &ok).await);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(n0.sw.peers().len(), 1);
    let long = Message {
        sum: Some(Sum::SeenTx(SeenTx {
            tx_key: tx_key(b"x").to_vec(),
            sequence: 1,
            signer: vec![7; 65],
        })),
    };
    assert!(to_n0.send_msg(MEMPOOL_DATA_CHANNEL, &long).await);
    wait_for(|| n0.sw.peers().is_empty()).await;
    let _ = n0.reactor.options();
    n0.sw.stop();
    n1.sw.stop();
}
