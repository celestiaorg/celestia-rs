//! The node's P2P switch (`node/setup.go`: `createTransport`,
//! `createSwitch`, `createPEXReactorAndAddToSwitch`): node key, NodeInfo,
//! listener, persistent peers, seeds + PEX with the address book, and
//! celestia-core's peer filter. Other reactors are added by the caller
//! before [`start_switch`].

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, anyhow};
use p2p::{MConnConfig, NodeInfo, NodeInfoOther, NodeKey, ProtocolVersion, Switch, SwitchConfig};

/// celestia-app's minimum P2P send / receive rate (`overrideP2PConfig`:
/// 200 MiB/s; lower configured values are raised).
pub const MIN_RATE: i64 = 200 * 1_048_576;

/// The `[p2p]` options of `celestia-rs start`.
#[derive(Debug, Clone, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct P2pOptions {
    /// Listen address (`host:port`).
    pub laddr: String,
    /// `ID@host:port` peers to keep connected.
    pub persistent_peers: Vec<String>,
    /// `ID@host:port` seed nodes (dialed by PEX for addresses).
    pub seeds: Vec<String>,
    /// Address advertised to peers (default: the listen address).
    pub external_address: Option<String>,
    /// Node moniker.
    pub moniker: String,
    /// Allow several peers from one IP (local devnets).
    pub allow_duplicate_ip: bool,
    /// Peer exchange (Go default `true`).
    pub pex: bool,
    /// Seed (crawler) mode.
    pub seed_mode: bool,
    /// Only routable addresses in the address book.
    pub addr_book_strict: bool,
    /// Inbound peer limit (0: Go default 40).
    pub max_inbound: usize,
    /// Outbound peer limit (0: Go default 10).
    pub max_outbound: usize,
    /// Peer IDs exempt from the limits.
    pub unconditional_peer_ids: Vec<String>,
    /// Peer IDs never gossiped.
    pub private_peer_ids: Vec<String>,
    /// Send / receive rates (bytes/s; raised to [`MIN_RATE`]).
    pub send_rate: i64,
    /// Receive rate.
    pub recv_rate: i64,
}

/// Loads (or creates) `config/node_key.json`.
pub fn node_key(home: &Path) -> anyhow::Result<NodeKey> {
    let path = home.join("config").join("node_key.json");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    NodeKey::load_or_generate(&path).map_err(|e| anyhow!("node key {}: {e}", path.display()))
}

/// A switch for `chain_id` (`makeNodeInfo` + `createSwitch`), not started;
/// with PEX (and the address book `config/addrbook.json`) when enabled.
pub fn new_switch(
    home: &Path,
    chain_id: &str,
    opts: &P2pOptions,
    app_version: u64,
) -> anyhow::Result<Switch> {
    let key = node_key(home)?;
    let rpc = crate::rpc_service::RpcSettings::load(home)?;
    let info = NodeInfo {
        protocol_version: ProtocolVersion {
            p2p: p2p::node_info::P2P_PROTOCOL,
            block: p2p::node_info::BLOCK_PROTOCOL,
            app: app_version,
        },
        network: chain_id.to_owned(),
        version: crate::version::node_version(),
        moniker: opts.moniker.clone(),
        listen_addr: format!(
            "tcp://{}",
            opts.external_address.as_deref().unwrap_or(&opts.laddr)
        ),
        other: NodeInfoOther {
            tx_index: if rpc.tx_index { "on" } else { "off" }.into(),
            rpc_address: rpc.rpc.laddr,
        },
        ..NodeInfo::default()
    };
    let rate = |r: i64| r.max(MIN_RATE);
    let defaults = SwitchConfig::default();
    let cfg = SwitchConfig {
        listen_address: Some(format!("tcp://{}", opts.laddr)),
        external_address: opts.external_address.as_ref().map(|a| format!("tcp://{a}")),
        allow_duplicate_ip: opts.allow_duplicate_ip,
        persistent_peers: opts.persistent_peers.clone(),
        max_num_inbound_peers: if opts.max_inbound == 0 {
            defaults.max_num_inbound_peers
        } else {
            opts.max_inbound
        },
        max_num_outbound_peers: if opts.max_outbound == 0 {
            defaults.max_num_outbound_peers
        } else {
            opts.max_outbound
        },
        unconditional_peer_ids: opts.unconditional_peer_ids.clone(),
        private_peer_ids: opts.private_peer_ids.clone(),
        mconn: MConnConfig {
            send_rate: rate(opts.send_rate),
            recv_rate: rate(opts.recv_rate),
            flush_throttle: Duration::from_millis(10),
            ..MConnConfig::default()
        },
        ..defaults
    };
    let sw = Switch::new(cfg, key, info).map_err(|e| anyhow!("switch: {e}"))?;
    sw.add_peer_filter(Arc::new(p2p::switch::celestia_peer_filter));
    if opts.pex {
        let book = Arc::new(p2p::pex::addrbook::AddrBook::new(
            Some(home.join("config").join("addrbook.json")),
            opts.addr_book_strict,
        ));
        let ids: Vec<p2p::NodeId> = opts
            .private_peer_ids
            .iter()
            .filter_map(|s| s.parse().ok())
            .collect();
        book.add_private_ids(&ids);
        let pex = p2p::pex::reactor::PexReactor::new(
            book.clone(),
            p2p::pex::reactor::PexConfig {
                seed_mode: opts.seed_mode,
                seeds: opts.seeds.clone(),
                ..p2p::pex::reactor::PexConfig::default()
            },
        );
        sw.set_addr_book(book);
        sw.add_reactor(pex).map_err(|e| anyhow!("pex: {e}"))?;
    } else if !opts.seeds.is_empty() {
        tracing::warn!("p2p.seeds are only used with pex = true; ignoring them");
    }
    Ok(sw)
}

/// Starts the switch (listener + persistent peers) from a blocking thread.
pub fn start_switch(rt: &tokio::runtime::Handle, sw: &Switch) -> anyhow::Result<()> {
    rt.block_on(sw.start())
        .map_err(|e| anyhow!("{e}"))
        .context("start p2p switch")?;
    tracing::info!(
        node_id = %sw.node_id(),
        addr = ?sw.net_address(),
        channels = %hex::encode(&sw.node_info().channels),
        "p2p switch started"
    );
    Ok(())
}
