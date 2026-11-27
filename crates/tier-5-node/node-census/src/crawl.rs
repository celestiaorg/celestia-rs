//! Breadth-first P2P crawl: handshake for `NodeInfo`, then PEX for more
//! addresses.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use celestia_core_proto::tendermint::p2p as pb;
use p2p::pex::PEX_CHANNEL;
use p2p::transport::{Transport, TransportConfig};
use p2p::{ChannelDescriptor, MConnConfig, MConnection, NetAddress, NodeInfo, NodeKey};
use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;

use crate::classify::{Implementation, classify};
use crate::rpc::RpcPeer;

/// Channels the crawler advertises. celestia-core drops peers without the
/// CAT mempool (0x31, 0x32) and propagation (0x50) channels
/// (`celestia_peer_filter`), so they are advertised to stay connected long
/// enough to receive the PEX answer; nothing is ever sent on them.
const CHANNELS: [u8; 4] = [PEX_CHANNEL, 0x31, 0x32, 0x50];

/// How many addresses of one node ID are tried before giving up on it.
const MAX_ATTEMPTS_PER_ID: usize = 3;

/// Crawl parameters.
#[derive(Debug, Clone)]
pub struct CrawlConfig {
    /// Chain ID (`NodeInfo.network`); peers on other networks are rejected.
    pub chain_id: String,
    /// Addresses to start from (always dialed, even if not routable).
    pub seeds: Vec<NetAddress>,
    /// Peers reported by RPC nodes: counted as observed, and dialed.
    pub rpc_peers: Vec<RpcPeer>,
    /// Also dial non-routable addresses learned through PEX (tests only).
    pub allow_private: bool,
    /// TCP connect timeout.
    pub dial_timeout: Duration,
    /// SecretConnection and NodeInfo handshake timeout (each).
    pub handshake_timeout: Duration,
    /// How long to wait for a PEX answer after the handshake.
    pub pex_timeout: Duration,
    /// Concurrent probes.
    pub concurrency: usize,
    /// Upper bound on distinct node IDs tracked.
    pub max_nodes: usize,
}

/// A node that completed the handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observed {
    /// Authenticated node ID (hex).
    pub id: String,
    /// Self-reported `NodeInfo.version`.
    pub version: String,
    /// Implementation inferred from `version`.
    pub implementation: Implementation,
    /// Self-reported moniker.
    pub moniker: String,
    /// `NodeInfo.protocol_version.app`.
    pub app_version: u64,
    /// How the `NodeInfo` was obtained.
    pub source: Source,
    /// celestia-app release from the node's RPC `/abci_info`, when its RPC
    /// answered (`result.response.version`, for example `6.4.0`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_release: Option<String>,
    /// Where to ask for `/abci_info`; never published.
    #[serde(skip)]
    pub rpc_url: Option<String>,
}

/// Where an observation came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Our own handshake with the node.
    P2p,
    /// A public RPC node's `/net_info` (its handshake with the node).
    Rpc,
}

/// What a crawl found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CrawlResult {
    /// Observed nodes, one per ID: our handshake, or else an RPC report.
    pub reached: Vec<Observed>,
    /// Node IDs learned but never observed.
    pub unreachable: usize,
}

/// The result of one probe.
struct Probe {
    info: NodeInfo,
    rpc_url: Option<String>,
    id: String,
    addrs: Vec<NetAddress>,
}

/// Crawl bookkeeping.
#[derive(Default)]
struct Frontier {
    queue: VecDeque<NetAddress>,
    known: HashSet<String>,
    reached: HashSet<String>,
    dialed: HashSet<SocketAddr>,
    attempts: HashMap<String, usize>,
}

impl Frontier {
    fn push(&mut self, addr: NetAddress, cfg: &CrawlConfig, seed: bool) {
        let Some(id) = addr.id.as_ref().map(ToString::to_string) else {
            return;
        };
        if !seed && !cfg.allow_private && !addr.routable() {
            return;
        }
        if self.reached.contains(&id) || self.dialed.contains(&addr.socket_addr()) {
            return;
        }
        if !self.known.contains(&id) && self.known.len() >= cfg.max_nodes {
            return;
        }
        let attempts = self.attempts.entry(id.clone()).or_default();
        if *attempts >= MAX_ATTEMPTS_PER_ID {
            return;
        }
        *attempts += 1;
        self.dialed.insert(addr.socket_addr());
        self.known.insert(id);
        self.queue.push_back(addr);
    }
}

/// Crawls from the seeds until no new addresses remain.
pub async fn crawl(cfg: CrawlConfig) -> CrawlResult {
    let transport = Arc::new(crawler_transport(&cfg));
    let mut frontier = Frontier::default();
    for s in &cfg.seeds {
        frontier.push(s.clone(), &cfg, true);
    }
    let mut observed: HashMap<String, Observed> = HashMap::new();
    for p in &cfg.rpc_peers {
        observed
            .entry(p.observed.id.clone())
            .or_insert_with(|| p.observed.clone());
        if let Some(a) = &p.addr {
            frontier.push(a.clone(), &cfg, false);
        }
    }
    let mut tasks = JoinSet::new();
    loop {
        while tasks.len() < cfg.concurrency.max(1) {
            let Some(addr) = frontier.queue.pop_front() else {
                break;
            };
            let transport = transport.clone();
            let pex_timeout = cfg.pex_timeout;
            tasks.spawn(async move {
                let r = probe(&transport, &addr, pex_timeout).await;
                if let Err(e) = &r {
                    tracing::debug!(%addr, error = %e, "probe failed");
                }
                r
            });
        }
        let Some(joined) = tasks.join_next().await else {
            break;
        };
        let Ok(Ok(p)) = joined else {
            continue;
        };
        if frontier.reached.insert(p.id.clone()) {
            tracing::info!(id = %p.id, version = %p.info.version, peers = p.addrs.len(), "reached");
            observed.insert(
                p.id.clone(),
                Observed {
                    id: p.id,
                    implementation: classify(&p.info.version),
                    version: p.info.version,
                    moniker: p.info.moniker,
                    app_version: p.info.protocol_version.app,
                    source: Source::P2p,
                    app_release: None,
                    rpc_url: p.rpc_url,
                },
            );
        }
        for a in p.addrs {
            frontier.push(a, &cfg, false);
        }
    }
    CrawlResult {
        unreachable: frontier
            .known
            .iter()
            .filter(|id| !observed.contains_key(*id))
            .count(),
        reached: observed.into_values().collect(),
    }
}

fn crawler_transport(cfg: &CrawlConfig) -> Transport {
    let key = NodeKey::generate();
    let info = NodeInfo {
        id: key.id().to_string(),
        protocol_version: p2p::ProtocolVersion {
            p2p: p2p::node_info::P2P_PROTOCOL,
            block: p2p::node_info::BLOCK_PROTOCOL,
            app: 0,
        },
        // Never listened on: the crawler only dials out.
        listen_addr: "tcp://0.0.0.0:26656".into(),
        network: cfg.chain_id.clone(),
        version: concat!("node-census/", env!("CARGO_PKG_VERSION")).into(),
        moniker: "node-census".into(),
        channels: CHANNELS.to_vec(),
        ..NodeInfo::default()
    };
    Transport::new(
        key,
        info,
        TransportConfig {
            dial_timeout: cfg.dial_timeout,
            handshake_timeout: cfg.handshake_timeout,
            allow_duplicate_ip: true,
        },
    )
}

async fn probe(
    transport: &Transport,
    addr: &NetAddress,
    pex_timeout: Duration,
) -> Result<Probe, String> {
    let conn = transport.dial(addr).await.map_err(|e| e.to_string())?;
    let remote = conn.remote_addr;
    let info = conn.node_info.clone();
    let id = conn.id.to_string();
    let descs: Vec<_> = CHANNELS
        .iter()
        .map(|&c| ChannelDescriptor::new(c, 1))
        .collect();
    let addrs = match MConnection::start(conn.reader, conn.writer, &descs, MConnConfig::default()) {
        Ok((mc, rx)) => {
            let req = pb::Message {
                sum: Some(pb::message::Sum::PexRequest(pb::PexRequest {})),
            };
            let addrs = if mc
                .send(PEX_CHANNEL, prost::Message::encode_to_vec(&req))
                .await
            {
                tokio::time::timeout(pex_timeout, pex_answer(rx))
                    .await
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            mc.stop();
            addrs
        }
        Err(e) => {
            tracing::debug!(%addr, error = %e, "mconnection");
            Vec::new()
        }
    };
    transport.cleanup(remote);
    let rpc_url = crate::rpc::rpc_url(&info.other.rpc_address, remote.ip());
    Ok(Probe {
        info,
        rpc_url,
        id,
        addrs,
    })
}

/// Waits for the first `PexAddrs` message; other channels are drained.
async fn pex_answer(mut rx: tokio::sync::mpsc::Receiver<(u8, Vec<u8>)>) -> Vec<NetAddress> {
    while let Some((ch, bz)) = rx.recv().await {
        if ch != PEX_CHANNEL {
            continue;
        }
        let Ok(msg) = celestia_core_proto::gogo_decode::<pb::Message>(&bz) else {
            return Vec::new();
        };
        if let Some(pb::message::Sum::PexAddrs(a)) = msg.sum {
            return a
                .addrs
                .iter()
                .filter_map(|p| NetAddress::from_proto(p).ok())
                .collect();
        }
    }
    Vec::new()
}
