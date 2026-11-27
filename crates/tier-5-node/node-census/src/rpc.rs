//! Peers reported by public RPC nodes (`/net_info`).
//!
//! An RPC node lists the `NodeInfo` each of its peers sent in the P2P
//! handshake, including peers behind NAT that never accept inbound
//! connections. These count as observed nodes, and their addresses seed
//! the crawl.
//!
//! The handshake carries only the celestia-core version and the app protocol
//! version. The celestia-app release (for example `6.4.0`) is read from a
//! node's own RPC `/abci_info`, when that RPC is open to the internet.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, ensure};
use p2p::NetAddress;
use serde_json::Value;

use crate::classify::classify;
use crate::crawl::{Observed, Source};

/// CometBFT's default RPC port.
const DEFAULT_RPC_PORT: u16 = 26657;

/// A peer listed by an RPC node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcPeer {
    /// What the peer reported in its handshake with the RPC node.
    pub observed: Observed,
    /// Where to dial it, if known.
    pub addr: Option<NetAddress>,
}

/// Extracts the peers of `chain_id` from a `/net_info` response body.
#[must_use]
pub fn parse_net_info(body: &Value, chain_id: &str) -> Vec<RpcPeer> {
    let Some(peers) = body["result"]["peers"].as_array() else {
        return Vec::new();
    };
    peers
        .iter()
        .filter_map(|p| {
            let ni = &p["node_info"];
            let id = ni["id"].as_str().filter(|s| !s.is_empty())?;
            if ni["network"].as_str() != Some(chain_id) {
                return None;
            }
            let version = ni["version"].as_str().unwrap_or_default();
            let app = &ni["protocol_version"]["app"];
            let app_version = app
                .as_u64()
                .or_else(|| app.as_str().and_then(|s| s.parse().ok()))
                .unwrap_or_default();
            let rpc_url = p["remote_ip"]
                .as_str()
                .and_then(|ip| ip.parse::<IpAddr>().ok())
                .and_then(|ip| {
                    rpc_url(ni["other"]["rpc_address"].as_str().unwrap_or_default(), ip)
                });
            let addr = dial_address(
                id,
                ni["listen_addr"].as_str().unwrap_or_default(),
                p["remote_ip"].as_str().unwrap_or_default(),
            );
            Some(RpcPeer {
                observed: Observed {
                    id: id.to_owned(),
                    version: version.to_owned(),
                    implementation: classify(version),
                    moniker: ni["moniker"].as_str().unwrap_or_default().to_owned(),
                    app_version,
                    source: Source::Rpc,
                    app_release: None,
                    rpc_url,
                },
                addr,
            })
        })
        .collect()
}

/// `id@ip:port` from the advertised listen address; the IP the RPC node saw
/// replaces an advertised IP that is unspecified, private or a host name.
fn dial_address(id: &str, listen_addr: &str, remote_ip: &str) -> Option<NetAddress> {
    let listen = listen_addr.trim_start_matches("tcp://");
    let (host, port) = listen.rsplit_once(':')?;
    let port: u16 = port.parse().ok()?;
    let advertised = host
        .trim_matches(['[', ']'])
        .parse::<IpAddr>()
        .ok()
        .map(|ip| SocketAddr::new(ip, port))
        .and_then(|sa| NetAddress::parse(&format!("{id}@{sa}")).ok())
        .filter(NetAddress::routable);
    advertised.or_else(|| {
        let ip: IpAddr = remote_ip.parse().ok()?;
        NetAddress::parse(&format!("{id}@{}", SocketAddr::new(ip, port))).ok()
    })
}

/// The RPC URL of a node from its advertised `NodeInfo.other.rpc_address`
/// and the IP it was seen at. An advertised host that is not a routable IP
/// (`0.0.0.0`, `127.0.0.1`, private, a host name) is replaced with `ip`; a
/// missing port is the default 26657. `None` for a Unix socket or when the
/// result would not be routable.
#[must_use]
pub fn rpc_url(rpc_address: &str, ip: IpAddr) -> Option<String> {
    if rpc_address.contains("://") && !rpc_address.starts_with("tcp://") {
        return None;
    }
    let addr = rpc_address.trim_start_matches("tcp://");
    let (host, port) = match addr.rsplit_once(':') {
        Some((h, p)) if !h.ends_with(':') => (h, p.parse().ok()?),
        _ => (addr, DEFAULT_RPC_PORT),
    };
    let advertised = host
        .trim_matches(['[', ']'])
        .parse::<IpAddr>()
        .ok()
        .filter(|a| routable(*a));
    let ip = advertised.or_else(|| Some(ip).filter(|a| routable(*a)))?;
    Some(format!("http://{}", SocketAddr::new(ip, port)))
}

fn routable(ip: IpAddr) -> bool {
    NetAddress::parse(&format!(
        "{}@{}",
        "0".repeat(40),
        SocketAddr::new(ip, DEFAULT_RPC_PORT)
    ))
    .is_ok_and(|a| a.routable())
}

/// The celestia-app release in an `/abci_info` body
/// (`result.response.version`), without a leading `v`. `None` when it is
/// missing or does not look like a version string.
#[must_use]
pub fn parse_abci_info(body: &Value) -> Option<String> {
    let v = body["result"]["response"]["version"].as_str()?.trim();
    let v = v.strip_prefix('v').unwrap_or(v);
    let ok = !v.is_empty()
        && v.len() <= 64
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+' | b'_'));
    ok.then(|| v.to_owned())
}

/// Asks each node with an RPC URL for `/abci_info` through `fetch`, at most
/// `concurrency` at a time, and records the release it reports. A node that
/// does not answer keeps `app_release: None`. Returns how many answered.
pub async fn probe_app_releases<F>(nodes: &mut [Observed], concurrency: usize, fetch: F) -> usize
where
    F: Fn(String) -> anyhow::Result<Value> + Send + Sync + 'static,
{
    let fetch = Arc::new(fetch);
    let limit = Arc::new(tokio::sync::Semaphore::new(concurrency.max(1)));
    let mut tasks = tokio::task::JoinSet::new();
    for (i, n) in nodes.iter().enumerate() {
        let Some(url) = n.rpc_url.clone() else {
            continue;
        };
        let (fetch, limit) = (fetch.clone(), limit.clone());
        tasks.spawn(async move {
            let _permit = limit.acquire_owned().await.ok()?;
            let r = tokio::task::spawn_blocking(move || fetch(url)).await.ok()?;
            Some((i, parse_abci_info(&r.ok()?)?))
        });
    }
    let mut answered = 0;
    while let Some(joined) = tasks.join_next().await {
        if let Ok(Some((i, release))) = joined {
            nodes[i].app_release = Some(release);
            answered += 1;
        }
    }
    answered
}

/// Fetches `<base>/abci_info` with `curl`.
pub fn fetch_abci_info(base: &str, timeout: Duration) -> anyhow::Result<Value> {
    fetch_json(base, "abci_info", timeout)
}

/// Fetches `<base>/net_info` with `curl`.
pub fn fetch_net_info(base: &str, timeout: Duration) -> anyhow::Result<Value> {
    fetch_json(base, "net_info", timeout)
}

fn fetch_json(base: &str, path: &str, timeout: Duration) -> anyhow::Result<Value> {
    let url = format!("{}/{path}", base.trim_end_matches('/'));
    let out = std::process::Command::new("curl")
        .args(["-fsSL", "--proto", "=https,http", "--max-time"])
        .arg(timeout.as_secs().max(1).to_string())
        .arg(&url)
        .output()
        .context("run curl")?;
    ensure!(
        out.status.success(),
        "curl {url}: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    serde_json::from_slice(&out.stdout).with_context(|| format!("parse {url}"))
}
