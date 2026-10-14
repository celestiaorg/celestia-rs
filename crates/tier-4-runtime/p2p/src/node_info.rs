//! `DefaultNodeInfo` exchanged in the P2P handshake (port of
//! `p2p/node_info.go`).

use celestia_core_proto::tendermint::p2p as pb;
use serde::{Deserialize, Serialize};

use crate::key::NodeId;
use crate::netaddress::{NetAddress, NetAddressError, remove_protocol};

/// Maximum encoded size of a NodeInfo (10 KB).
pub const MAX_NODE_INFO_SIZE: usize = 10240;
/// Maximum number of channels a node may advertise.
pub const MAX_NUM_CHANNELS: usize = 16;

/// celestia-core's P2P protocol version (`version.P2PProtocol`).
pub const P2P_PROTOCOL: u64 = 8;
/// celestia-core's block protocol version (`version.BlockProtocol`).
pub const BLOCK_PROTOCOL: u64 = 11;
/// celestia-core's software version string (`version.TMCoreSemVer`).
pub const TM_CORE_SEMVER: &str = "0.38.17";

/// Protocol versions of a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProtocolVersion {
    /// P2P protocol.
    pub p2p: u64,
    /// Block protocol.
    pub block: u64,
    /// App version.
    pub app: u64,
}

/// Application-specific node info.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NodeInfoOther {
    /// `"on"`, `"off"` or empty.
    pub tx_index: String,
    /// RPC listen address.
    pub rpc_address: String,
}

/// A node's self-description (`DefaultNodeInfo`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NodeInfo {
    /// Protocol versions.
    pub protocol_version: ProtocolVersion,
    /// Node ID as sent on the wire (hex).
    pub id: String,
    /// Self-reported listen address (`[tcp://]host:port`).
    pub listen_addr: String,
    /// Network = chain ID.
    pub network: String,
    /// Software version.
    pub version: String,
    /// Supported channel IDs.
    pub channels: Vec<u8>,
    /// Moniker.
    pub moniker: String,
    /// Other info.
    pub other: NodeInfoOther,
}

/// Go's `cmtstrings.IsASCIIText`: non-empty, all bytes in 32..=126.
fn is_ascii_text(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| (32..=126).contains(&b))
}

/// Go's `cmtstrings.ASCIITrim` (removes spaces) is empty.
fn ascii_trim_empty(s: &str) -> bool {
    s.bytes().all(|b| b == b' ')
}

impl NodeInfo {
    /// The node ID, if well-formed.
    pub fn node_id(&self) -> Result<NodeId, crate::key::InvalidId> {
        self.id.parse()
    }

    /// `HasChannel`.
    #[must_use]
    pub fn has_channel(&self, ch: u8) -> bool {
        self.channels.contains(&ch)
    }

    /// The self-reported address `ID@listen_addr` (`NetAddress`).
    pub fn net_address(&self) -> Result<NetAddress, NetAddressError> {
        NetAddress::parse(&format!(
            "{}@{}",
            self.id,
            remove_protocol(&self.listen_addr)
        ))
    }

    /// `Validate`: checks the self-reported info is safe.
    pub fn validate(&self) -> Result<(), String> {
        self.net_address().map_err(|e| e.to_string())?;
        if !self.version.is_empty()
            && (!is_ascii_text(&self.version) || ascii_trim_empty(&self.version))
        {
            return Err(format!(
                "info.Version must be valid ASCII text without tabs, but got {}",
                self.version
            ));
        }
        if self.channels.len() > MAX_NUM_CHANNELS {
            return Err(format!(
                "info.Channels is too long ({}). Max is {MAX_NUM_CHANNELS}",
                self.channels.len()
            ));
        }
        let mut seen = [false; 256];
        for &ch in &self.channels {
            if seen[usize::from(ch)] {
                return Err(format!("info.Channels contains duplicate channel id {ch}"));
            }
            seen[usize::from(ch)] = true;
        }
        if !is_ascii_text(&self.moniker) || ascii_trim_empty(&self.moniker) {
            return Err(format!(
                "info.Moniker must be valid non-empty ASCII text without tabs, but got {}",
                self.moniker
            ));
        }
        match self.other.tx_index.as_str() {
            "" | "on" | "off" => {}
            other => {
                return Err(format!(
                    "info.Other.TxIndex should be either 'on', 'off', or empty string, got '{other}'"
                ));
            }
        }
        let rpc = &self.other.rpc_address;
        if !rpc.is_empty() && (!is_ascii_text(rpc) || ascii_trim_empty(rpc)) {
            return Err(format!(
                "info.Other.RPCAddress={rpc} must be valid ASCII text without tabs"
            ));
        }
        Ok(())
    }

    /// `CompatibleWith`: same block version and network, and at least one
    /// common channel (unless we have none).
    pub fn compatible_with(&self, other: &Self) -> Result<(), String> {
        if self.protocol_version.block != other.protocol_version.block {
            return Err(format!(
                "peer is on a different Block version. Got {}, expected {}",
                other.protocol_version.block, self.protocol_version.block
            ));
        }
        if self.network != other.network {
            return Err(format!(
                "peer is on a different network. Got {}, expected {}",
                other.network, self.network
            ));
        }
        if self.channels.is_empty() {
            return Ok(());
        }
        if !self.channels.iter().any(|c| other.channels.contains(c)) {
            return Err(format!(
                "peer has no common channels. Our channels: {:?} ; Peer channels: {:?}",
                self.channels, other.channels
            ));
        }
        Ok(())
    }

    /// `ToProto` (gogoproto non-nullable fields are always present).
    #[must_use]
    pub fn to_proto(&self) -> pb::DefaultNodeInfo {
        pb::DefaultNodeInfo {
            protocol_version: Some(pb::ProtocolVersion {
                p2p: self.protocol_version.p2p,
                block: self.protocol_version.block,
                app: self.protocol_version.app,
            }),
            default_node_id: self.id.clone(),
            listen_addr: self.listen_addr.clone(),
            network: self.network.clone(),
            version: self.version.clone(),
            channels: self.channels.clone(),
            moniker: self.moniker.clone(),
            other: Some(pb::DefaultNodeInfoOther {
                tx_index: self.other.tx_index.clone(),
                rpc_address: self.other.rpc_address.clone(),
            }),
        }
    }

    /// `DefaultNodeInfoFromToProto`.
    #[must_use]
    pub fn from_proto(p: &pb::DefaultNodeInfo) -> Self {
        let pv = p.protocol_version.unwrap_or_default();
        let other = p.other.clone().unwrap_or_default();
        Self {
            protocol_version: ProtocolVersion {
                p2p: pv.p2p,
                block: pv.block,
                app: pv.app,
            },
            id: p.default_node_id.clone(),
            listen_addr: p.listen_addr.clone(),
            network: p.network.clone(),
            version: p.version.clone(),
            channels: p.channels.clone(),
            moniker: p.moniker.clone(),
            other: NodeInfoOther {
                tx_index: other.tx_index,
                rpc_address: other.rpc_address,
            },
        }
    }
}
