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

/// `redial_persistent_peers` dials a disconnected persistent peer at once,
/// without waiting for the reconnect loop's next try, and leaves connected
/// peers alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn redial_persistent_peers_dials_now() {
    let (a, _echo_a, _pex_a) = node(9, vec![]);
    a.start().await.unwrap();
    let a_addr = a.net_address().unwrap();
    let (b, _echo_b, _pex_b) = node(10, vec![a_addr.to_string()]);
    b.start().await.unwrap();
    wait_for(|| b.peers().len() == 1).await;

    // Connected: nothing to redial.
    assert_eq!(b.redial_persistent_peers(), 0);

    // A graceful stop starts no reconnect loop, so B stays disconnected.
    let pa = b.get_peer(&a.node_id()).unwrap();
    b.stop_peer_gracefully(&pa, "ECHO");
    b.stop_peer_gracefully(&pa, "PEX");
    assert!(b.peers().is_empty());
    wait_for(|| a.peers().is_empty()).await;

    assert_eq!(b.redial_persistent_peers(), 1);
    wait_for(|| b.get_peer(&a.node_id()).is_some_and(|p| p.is_persistent())).await;

    a.stop();
    b.stop();
}

const SLOW: u8 = 0x71;

/// Records the order of `receive` and `remove_peer`; `receive` is slow, as
/// a busy reactor task is.
#[derive(Default)]
struct Slow {
    events: Mutex<Vec<String>>,
}

impl Reactor for Slow {
    fn name(&self) -> String {
        "SLOW".into()
    }
    fn channels(&self) -> Vec<ChannelDescriptor> {
        vec![ChannelDescriptor::new(SLOW, 5)]
    }
    fn receive(&self, env: Envelope) -> Result<(), String> {
        std::thread::sleep(Duration::from_millis(300));
        self.events
            .lock()
            .unwrap()
            .push(format!("receive {}", String::from_utf8_lossy(&env.message)));
        Ok(())
    }
    fn remove_peer(&self, _peer: &Arc<p2p::Peer>, _reason: &str) {
        self.events.lock().unwrap().push("remove_peer".into());
    }
}

/// As in Go, where the connection's receive routine calls `Receive` before
/// the switch stops the peer: a message that arrived before the remote
/// closed reaches the reactor before `remove_peer`. A seed sends its
/// PexAddrs and then closes, so the reply and the close arrive together.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn messages_before_a_close_reach_reactors_before_remove_peer() {
    let (a, _echo_a, _pex_a) = node(11, vec![]);
    let slow = Arc::new(Slow::default());
    a.add_reactor(slow.clone()).unwrap();
    a.start().await.unwrap();
    let a_addr = a.net_address().unwrap();
    let (b, _echo_b, _pex_b) = node(12, vec![]);
    b.add_reactor(Arc::new(Slow::default())).unwrap();
    b.start().await.unwrap();
    b.dial_peer_with_address(&a_addr).await.unwrap();
    wait_for(|| a.peers().len() == 1).await;

    let pa = b.get_peer(&a.node_id()).unwrap();
    // As a seed does after its PexAddrs: send, then flush and close.
    assert!(pa.send(SLOW, b"last".to_vec()).await);
    pa.flush_stop();

    wait_for(|| slow.events.lock().unwrap().len() == 2).await;
    assert_eq!(
        *slow.events.lock().unwrap(),
        vec!["receive last".to_owned(), "remove_peer".to_owned()]
    );
    a.stop();
}

/// A switch with a PEX reactor of `config` and its address book.
fn pex_node(seed: u8, config: PexConfig) -> (Switch, Arc<AddrBook>, Arc<PexReactor>) {
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
        ..SwitchConfig::default()
    };
    let sw = Switch::new(cfg, key, info).unwrap();
    let book = Arc::new(AddrBook::new(None, false));
    let pex = PexReactor::new(book.clone(), config);
    sw.set_addr_book(book.clone());
    sw.add_reactor(pex.clone()).unwrap();
    (sw, book, pex)
}

/// A fresh node whose only source is a seed fills its address book from
/// the seed, which answers the PexRequest and hangs up at once, as a Go
/// seed-mode node does.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fresh_node_gets_addresses_from_a_seed_that_hangs_up() {
    let (seed, seed_book, _) = pex_node(
        21,
        PexConfig {
            seed_mode: true,
            ..PexConfig::default()
        },
    );
    let src = p2p::NetAddress::parse(&format!(
        "{}@10.0.0.1:26656",
        NodeKey::from_seed(&[30; 32]).id()
    ))
    .unwrap();
    for i in 0..20u8 {
        let a = p2p::NetAddress::parse(&format!(
            "{}@10.1.{i}.1:26656",
            NodeKey::from_seed(&[40 + i; 32]).id()
        ))
        .unwrap();
        seed_book.add_address(&a, &src).unwrap();
    }
    seed.start().await.unwrap();

    let (client, book, pex) = pex_node(
        22,
        PexConfig {
            seeds: vec![seed.net_address().unwrap().to_string()],
            ensure_peers_period: Duration::from_millis(300),
            ..PexConfig::default()
        },
    );
    client.start().await.unwrap();
    wait_for(|| pex.stats().addrs_received >= 1 && book.size() >= 10).await;
    client.stop();
    seed.stop();
}

const BLOCK: u8 = 0x72;

/// A reactor whose `receive` blocks until the test releases it, as a stuck
/// reactor task does.
#[derive(Default)]
struct Blocked {
    released: Mutex<bool>,
    wake: std::sync::Condvar,
}

impl Blocked {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }
}

/// Releases the reactor when dropped, so a failed assertion does not leave
/// a worker thread blocked and the runtime unable to shut down.
struct ReleaseOnDrop(Arc<Blocked>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

impl Reactor for Blocked {
    fn name(&self) -> String {
        "BLOCK".into()
    }
    fn channels(&self) -> Vec<ChannelDescriptor> {
        vec![ChannelDescriptor::new(BLOCK, 5)]
    }
    fn receive(&self, _env: Envelope) -> Result<(), String> {
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.wake.wait(released).unwrap();
        }
        Ok(())
    }
}

/// The wait for a closing peer's messages is bounded: when a reactor never
/// finishes, the switch still removes the peer after the drain limit (5 s),
/// and other peers keep exchanging messages during the wait.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stuck_reactor_delays_peer_removal_by_at_most_the_drain_limit() {
    let (a, echo_a, _) = node(13, vec![]);
    let blocked = Arc::new(Blocked::default());
    a.add_reactor(blocked.clone()).unwrap();
    let _release = ReleaseOnDrop(blocked);
    a.start().await.unwrap();
    let a_addr = a.net_address().unwrap();
    let (b, _, _) = node(14, vec![]);
    b.add_reactor(Arc::new(Blocked::default())).unwrap();
    b.start().await.unwrap();
    let (c, _, _) = node(15, vec![]);
    c.start().await.unwrap();
    b.dial_peer_with_address(&a_addr).await.unwrap();
    c.dial_peer_with_address(&a_addr).await.unwrap();
    wait_for(|| a.peers().len() == 2).await;

    // B's last message sticks in A's reactor; then B closes.
    let pb = b.get_peer(&a.node_id()).unwrap();
    assert!(pb.send(BLOCK, b"stuck".to_vec()).await);
    pb.flush_stop();
    let closed = tokio::time::Instant::now();

    // C is served while A waits for B's messages.
    let pc = c.get_peer(&a.node_id()).unwrap();
    assert!(pc.send(ECHO, b"from c".to_vec()).await);
    wait_for(|| echo_a.got.lock().unwrap().contains(&b"from c".to_vec())).await;
    assert!(
        a.get_peer(&b.node_id()).is_some(),
        "B removed before the limit"
    );

    // B is removed after the limit even though the reactor never returns.
    wait_for(|| a.get_peer(&b.node_id()).is_none()).await;
    let waited = closed.elapsed();
    assert!(
        waited >= Duration::from_secs(4) && waited < Duration::from_secs(8),
        "{waited:?}"
    );
    assert!(a.get_peer(&c.node_id()).is_some());

    a.stop();
    b.stop();
    c.stop();
}

const BULK: u8 = 0x73;

/// Counts the bytes of every message on [`BULK`].
#[derive(Default)]
struct Bulk {
    bytes: std::sync::atomic::AtomicUsize,
}

impl Reactor for Bulk {
    fn name(&self) -> String {
        "BULK".into()
    }
    fn channels(&self) -> Vec<ChannelDescriptor> {
        vec![ChannelDescriptor {
            send_queue_capacity: 1000,
            recv_buffer_capacity: 50 * 4096,
            recv_message_capacity: 4 << 20,
            ..ChannelDescriptor::new(BULK, 30)
        }]
    }
    fn receive(&self, env: Envelope) -> Result<(), String> {
        self.bytes
            .fetch_add(env.message.len(), std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

/// A switch with the node's connection settings: rates of 200 MiB/s,
/// 1024-byte packet payloads, a 10 ms flush throttle.
fn bulk_node(seed: u8) -> (Switch, Arc<Bulk>) {
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
        mconn: MConnConfig {
            send_rate: 200 * 1_048_576,
            recv_rate: 200 * 1_048_576,
            flush_throttle: Duration::from_millis(10),
            ..MConnConfig::default()
        },
        ..SwitchConfig::default()
    };
    let sw = Switch::new(cfg, key, info).unwrap();
    let bulk = Arc::new(Bulk::default());
    sw.add_reactor(bulk.clone()).unwrap();
    (sw, bulk)
}

/// With the node's configured rates, a connection carries block-sized
/// messages far faster than the old 512,000 B/s default, in both
/// directions (the dialer's outbound and the listener's inbound side).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_connection_carries_block_sized_messages_at_the_configured_rate() {
    const MSG: usize = 80 * 1024;
    const COUNT: usize = 100;
    let (a, bulk_a) = bulk_node(31);
    a.start().await.unwrap();
    let (b, bulk_b) = bulk_node(32);
    b.start().await.unwrap();
    b.dial_peer_with_address(&a.net_address().unwrap())
        .await
        .unwrap();
    wait_for(|| a.peers().len() == 1).await;
    let to_a = b.get_peer(&a.node_id()).unwrap();
    let to_b = a.get_peer(&b.node_id()).unwrap();

    for (peer, bulk, side) in [(&to_a, &bulk_a, "outbound"), (&to_b, &bulk_b, "inbound")] {
        let start = std::time::Instant::now();
        for _ in 0..COUNT {
            assert!(peer.send(BULK, vec![7u8; MSG]).await);
        }
        let want = MSG * COUNT;
        while bulk.bytes.load(std::sync::atomic::Ordering::SeqCst) < want {
            assert!(start.elapsed() < Duration::from_secs(60), "{side}: stalled");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        #[allow(clippy::cast_precision_loss)]
        let rate = want as f64 / start.elapsed().as_secs_f64();
        // Debug builds reach about 3 MB/s here and release builds over
        // 100 MB/s; a connection held to 512,000 B/s stays below the bound.
        assert!(rate > 2.0 * 512_000.0, "{side}: {rate:.0} B/s");
    }
    a.stop();
    b.stop();
}
