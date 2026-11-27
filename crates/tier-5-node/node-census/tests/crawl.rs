//! End-to-end crawl over loopback against in-process P2P nodes that speak
//! the real SecretConnection, NodeInfo handshake and PEX protocol.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use node_census::classify::Implementation;
use node_census::crawl::{CrawlConfig, Observed, Source, crawl};
use node_census::rpc::RpcPeer;
use p2p::pex::reactor::cap_addresses;
use p2p::transport::{Transport, TransportConfig};
use p2p::{ChannelDescriptor, MConnConfig, MConnection, NetAddress, NodeInfo, NodeKey};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

const CHAIN: &str = "census-test";

/// A listening fake node; `peers` is what it answers to a PEX request.
struct FakeNode {
    addr: NetAddress,
    peers: Arc<Mutex<Vec<NetAddress>>>,
}

async fn fake_node(version: &str) -> FakeNode {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local = listener.local_addr().unwrap();
    let key = NodeKey::generate();
    let id = key.id().to_string();
    let info = NodeInfo {
        id: id.clone(),
        protocol_version: p2p::ProtocolVersion {
            p2p: 8,
            block: 11,
            app: 6,
        },
        listen_addr: format!("tcp://{local}"),
        network: CHAIN.into(),
        version: version.into(),
        moniker: format!("fake-{version}"),
        channels: vec![0x00, 0x31, 0x32, 0x50],
        ..NodeInfo::default()
    };
    let transport = Arc::new(Transport::new(
        key,
        info,
        TransportConfig {
            allow_duplicate_ip: true,
            ..TransportConfig::default()
        },
    ));
    let peers = Arc::new(Mutex::new(Vec::new()));
    let served = peers.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let transport = transport.clone();
            let served = served.clone();
            tokio::spawn(async move {
                let Ok(conn) = transport.accept(stream).await else {
                    return;
                };
                let descs: Vec<_> = [0x00, 0x31, 0x32, 0x50]
                    .into_iter()
                    .map(|id| ChannelDescriptor::new(id, 1))
                    .collect();
                let (mc, mut rx) =
                    MConnection::start(conn.reader, conn.writer, &descs, MConnConfig::default())
                        .unwrap();
                while let Some((ch, _)) = rx.recv().await {
                    if ch == 0x00 {
                        let msg = cap_addresses(&served.lock().await);
                        mc.send(0x00, prost::Message::encode_to_vec(&msg)).await;
                    }
                }
                transport.cleanup(conn.remote_addr);
            });
        }
    });
    FakeNode {
        addr: NetAddress::parse(&format!("{id}@{local}")).unwrap(),
        peers,
    }
}

fn config(seeds: Vec<NetAddress>) -> CrawlConfig {
    CrawlConfig {
        chain_id: CHAIN.into(),
        seeds,
        rpc_peers: Vec::new(),
        allow_private: true,
        dial_timeout: Duration::from_secs(1),
        handshake_timeout: Duration::from_secs(3),
        pex_timeout: Duration::from_secs(3),
        concurrency: 8,
        max_nodes: 100,
    }
}

#[tokio::test]
async fn discovers_peers_through_pex_and_classifies_them() {
    let rust = fake_node("0.38.17-celestia-rs.0.4.2").await;
    let go = fake_node("0.38.17").await;
    // A node ID nobody listens for: discovered but unreachable.
    let dead_port = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let dead = NetAddress::parse(&format!(
        "{}@127.0.0.1:{dead_port}",
        NodeKey::generate().id()
    ))
    .unwrap();
    *rust.peers.lock().await = vec![go.addr.clone(), dead.clone()];
    *go.peers.lock().await = vec![rust.addr.clone()];

    let result = crawl(config(vec![rust.addr.clone()])).await;

    let mut reached: Vec<_> = result
        .reached
        .iter()
        .map(|n| (n.version.as_str(), n.implementation))
        .collect();
    reached.sort_unstable();
    assert_eq!(
        reached,
        [
            ("0.38.17", Implementation::Go),
            ("0.38.17-celestia-rs.0.4.2", Implementation::Rust),
        ]
    );
    assert_eq!(result.unreachable, 1);
    assert!(result.reached.iter().all(|n| n.app_version == 6));
}

#[tokio::test]
async fn skips_private_addresses_unless_allowed() {
    let rust = fake_node("0.38.17-celestia-rs.0.4.2").await;
    let go = fake_node("0.38.17").await;
    *rust.peers.lock().await = vec![go.addr.clone()];

    let result = crawl(CrawlConfig {
        allow_private: false,
        ..config(vec![rust.addr.clone()])
    })
    .await;

    // Seeds are always dialed; loopback addresses learned through PEX are not.
    assert_eq!(result.reached.len(), 1);
    assert_eq!(result.unreachable, 0);
}

#[tokio::test]
async fn counts_rpc_reported_peers_and_prefers_a_direct_handshake() {
    let go = fake_node("0.38.17").await;
    // Behind NAT: known only from another node's /net_info.
    let hidden = Observed {
        id: NodeKey::generate().id().to_string(),
        version: "0.38.17-celestia-rs.0.4.2".into(),
        implementation: Implementation::Rust,
        moniker: "nat".into(),
        app_version: 6,
        source: Source::Rpc,
        app_release: None,
        rpc_url: None,
    };
    let reported_go = Observed {
        id: go.addr.id.unwrap().to_string(),
        version: "stale".into(),
        implementation: Implementation::Unknown,
        moniker: "stale".into(),
        app_version: 0,
        source: Source::Rpc,
        app_release: None,
        rpc_url: None,
    };
    let result = crawl(CrawlConfig {
        rpc_peers: vec![
            RpcPeer {
                observed: hidden.clone(),
                addr: None,
            },
            RpcPeer {
                observed: reported_go,
                addr: Some(go.addr.clone()),
            },
        ],
        ..config(Vec::new())
    })
    .await;

    let mut got: Vec<_> = result
        .reached
        .iter()
        .map(|n| (n.implementation, n.source))
        .collect();
    got.sort_unstable();
    assert_eq!(
        got,
        [
            (Implementation::Rust, Source::Rpc),
            (Implementation::Go, Source::P2p)
        ]
    );
    assert_eq!(result.unreachable, 0);
}
