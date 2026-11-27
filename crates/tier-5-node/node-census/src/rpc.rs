//! Peers reported by public RPC nodes (`/net_info`).
//!
//! An RPC node lists the `NodeInfo` each of its peers sent in the P2P
//! handshake, including peers behind NAT that never accept inbound
//! connections. These count as observed nodes, and their addresses seed
//! the crawl.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use anyhow::{Context as _, ensure};
use p2p::NetAddress;
use serde_json::Value;

use crate::classify::classify;
use crate::crawl::{Observed, Source};

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

/// Fetches `<base>/net_info` with `curl`.
pub fn fetch_net_info(base: &str, timeout: Duration) -> anyhow::Result<Value> {
    let url = format!("{}/net_info", base.trim_end_matches('/'));
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
