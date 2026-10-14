//! Switch-level tests between Rust nodes over loopback TCP.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use p2p::pex::{AddrBook, PexConfig, PexReactor};
use p2p::{
    ChannelDescriptor, Envelope, MConnConfig, NodeInfo, NodeKey, ProtocolVersion, Reactor, Switch,
    SwitchConfig, WeakSwitch,
};

const ECHO: u8 = 0x70;

#[derive(Default)]
struct Echo {
    got: Mutex<Vec<Vec<u8>>>,
    peers: Mutex<Vec<String>>,
    sw: Mutex<WeakSwitch>,
}

impl Reactor for Echo {
    fn name(&self) -> String {
        "ECHO".into()
    }
    fn channels(&self) -> Vec<ChannelDescriptor> {
        vec![ChannelDescriptor {
            send_queue_capacity: 10,
            ..ChannelDescriptor::new(ECHO, 5)
        }]
    }
    fn set_switch(&self, sw: WeakSwitch) {
        *self.sw.lock().unwrap() = sw;
    }
    fn add_peer(&self, peer: &Arc<p2p::Peer>) {
        self.peers.lock().unwrap().push(peer.id().to_string());
    }
    fn receive(&self, env: Envelope) -> Result<(), String> {
        if env.message == b"bad" {
            return Err("bad message".into());
        }
        self.got.lock().unwrap().push(env.message);
        Ok(())
    }
}

fn node(seed: u8, persistent: Vec<String>) -> (Switch, Arc<Echo>, Arc<PexReactor>) {
    let key = NodeKey::from_seed(&[seed; 32]);
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
    let cfg = SwitchConfig {
        listen_address: Some("tcp://127.0.0.1:0".into()),
        allow_duplicate_ip: true,
        persistent_peers: persistent,
        mconn: MConnConfig {
            flush_throttle: Duration::from_millis(5),
            ping_interval: Duration::from_millis(500),
            pong_timeout: Duration::from_millis(300),
            ..MConnConfig::default()
        },
        ..SwitchConfig::default()
    };
    let sw = Switch::new(cfg, key, info).unwrap();
    let echo = Arc::new(Echo::default());
    sw.add_reactor(echo.clone()).unwrap();
    let book = Arc::new(AddrBook::new(None, false));
    let pex = PexReactor::new(
        book.clone(),
        PexConfig {
            ensure_peers_period: Duration::from_millis(300),
            ..PexConfig::default()
        },
    );
    sw.set_addr_book(book);
    sw.add_reactor(pex.clone()).unwrap();
    (sw, echo, pex)
}

async fn wait_for(mut f: impl FnMut() -> bool) {
    for _ in 0..200 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("condition not reached");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dial_exchange_pex_and_reconnect() {
    let (a, echo_a, _pex_a) = node(1, vec![]);
    a.start().await.unwrap();
    let a_addr = a.net_address().unwrap();
    let (b, echo_b, pex_b) = node(2, vec![a_addr.to_string()]);
    b.start().await.unwrap();

    wait_for(|| a.peers().len() == 1 && b.peers().len() == 1).await;
    let pa = b.get_peer(&a.node_id()).unwrap();
    assert!(pa.is_outbound() && pa.is_persistent());
    assert!(!a.get_peer(&b.node_id()).unwrap().is_outbound());
    assert_eq!(a.node_info().channels, vec![ECHO, 0x00]);

    // Messages both ways, and broadcast.
    let big = vec![9u8; 50_000];
    assert!(pa.send(ECHO, big.clone()).await);
    let mut rx = a.broadcast(ECHO, b"hello".to_vec());
    assert_eq!(rx.recv().await, Some(true));
    wait_for(|| echo_a.got.lock().unwrap().len() == 1 && echo_b.got.lock().unwrap().len() == 1)
        .await;
    assert_eq!(echo_a.got.lock().unwrap()[0], big);

    // PEX: B (outbound) asked A for addresses on connect.
    wait_for(|| pex_b.stats().addrs_received >= 1).await;
    assert!(pex_b.stats().requests_sent >= 1);

    // Pings flow (500ms interval) without disconnects.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(b.peers().len(), 1);

    // A reactor error disconnects; B redials its persistent peer A.
    assert!(pa.send(ECHO, b"bad".to_vec()).await);
    wait_for(|| a.peers().is_empty() || !pa.is_running()).await;
    wait_for(|| {
        b.get_peer(&a.node_id())
            .is_some_and(|p| !Arc::ptr_eq(&p, &pa))
            && a.peers().len() == 1
    })
    .await;
    assert!(echo_a.peers.lock().unwrap().len() >= 2);

    a.stop();
    b.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rejects_self_wrong_network_and_duplicates() {
    let (a, _, _) = node(3, vec![]);
    a.start().await.unwrap();
    let a_addr = a.net_address().unwrap();
    // Self-dial is rejected.
    assert!(a.dial_peer_with_address(&a_addr).await.is_err());
    assert!(a.peers().is_empty());

    // Different network.
    let key = NodeKey::from_seed(&[4; 32]);
    let info = NodeInfo {
        protocol_version: ProtocolVersion {
            p2p: 8,
            block: 11,
            app: 0,
        },
        network: "other-chain".into(),
        moniker: "x".into(),
        ..NodeInfo::default()
    };
    let c = Switch::new(
        SwitchConfig {
            listen_address: Some("tcp://127.0.0.1:0".into()),
            ..SwitchConfig::default()
        },
        key,
        info,
    )
    .unwrap();
    c.add_reactor(Arc::new(Echo::default())).unwrap();
    c.start().await.unwrap();
    let err = c.dial_peer_with_address(&a_addr).await.unwrap_err();
    assert!(err.to_string().contains("different network"), "{err}");

    // Wrong ID for the address.
    let mut wrong = a_addr.clone();
    wrong.id = Some(NodeKey::from_seed(&[5; 32]).id());
    let err = c.dial_peer_with_address(&wrong).await.unwrap_err();
    assert!(err.to_string().contains("mismatch"), "{err}");

    // Duplicate dial of a connected peer.
    let (d, _, _) = node(6, vec![]);
    d.start().await.unwrap();
    d.dial_peer_with_address(&a_addr).await.unwrap();
    assert!(matches!(
        d.dial_peer_with_address(&a_addr).await,
        Err(p2p::SwitchError::CurrentlyDialingOrExistingAddress(_))
    ));
    a.stop();
    c.stop();
    d.stop();
}
