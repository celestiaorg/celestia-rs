//! The node's P2P switch (`node/setup.go`: `createTransport`,
//! `createSwitch`): node key, NodeInfo, listener, persistent peers and
//! celestia-core's peer filter. Reactors are added by the caller before
//! [`start_switch`].

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, anyhow};
use p2p::{MConnConfig, NodeInfo, NodeInfoOther, NodeKey, ProtocolVersion, Switch, SwitchConfig};

/// The `[p2p]` options of `celestia-rs start`.
#[derive(Debug, Clone, Default)]
pub struct P2pOptions {
    /// Listen address (`host:port`).
    pub laddr: String,
    /// `ID@host:port` peers to keep connected.
    pub persistent_peers: Vec<String>,
    /// Address advertised to peers (default: the listen address).
    pub external_address: Option<String>,
    /// Node moniker.
    pub moniker: String,
    /// Allow several peers from one IP (local devnets).
    pub allow_duplicate_ip: bool,
}

/// Loads (or creates) `config/node_key.json`.
pub fn node_key(home: &Path) -> anyhow::Result<NodeKey> {
    let path = home.join("config").join("node_key.json");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    NodeKey::load_or_generate(&path).map_err(|e| anyhow!("node key {}: {e}", path.display()))
}

/// A switch for `chain_id` (`makeNodeInfo` + `createSwitch`), not started.
pub fn new_switch(home: &Path, chain_id: &str, opts: &P2pOptions) -> anyhow::Result<Switch> {
    let key = node_key(home)?;
    let info = NodeInfo {
        protocol_version: ProtocolVersion {
            p2p: p2p::node_info::P2P_PROTOCOL,
            block: p2p::node_info::BLOCK_PROTOCOL,
            app: 0,
        },
        network: chain_id.to_owned(),
        version: p2p::node_info::TM_CORE_SEMVER.into(),
        moniker: opts.moniker.clone(),
        listen_addr: format!(
            "tcp://{}",
            opts.external_address.as_deref().unwrap_or(&opts.laddr)
        ),
        other: NodeInfoOther {
            tx_index: "off".into(),
            rpc_address: String::new(),
        },
        ..NodeInfo::default()
    };
    let cfg = SwitchConfig {
        listen_address: Some(format!("tcp://{}", opts.laddr)),
        external_address: opts.external_address.as_ref().map(|a| format!("tcp://{a}")),
        allow_duplicate_ip: opts.allow_duplicate_ip,
        persistent_peers: opts.persistent_peers.clone(),
        mconn: MConnConfig {
            send_rate: 20_480_000,
            recv_rate: 20_480_000,
            flush_throttle: Duration::from_millis(10),
            ..MConnConfig::default()
        },
        ..SwitchConfig::default()
    };
    let sw = Switch::new(cfg, key, info).map_err(|e| anyhow!("switch: {e}"))?;
    sw.add_peer_filter(Arc::new(p2p::switch::celestia_peer_filter));
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
