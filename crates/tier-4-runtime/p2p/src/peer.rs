//! A connected peer (port of `p2p/peer.go`).

use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::key::NodeId;
use crate::mconnection::{
    ChannelDescriptor, ConnState, ConnectionStatus, MConnConfig, MConnection, Received,
};
use crate::netaddress::NetAddress;
use crate::node_info::NodeInfo;
use crate::transport::UpgradedConn;

/// A connected, authenticated peer. Shared as `Arc<Peer>`.
pub struct Peer {
    id: NodeId,
    node_info: NodeInfo,
    outbound: bool,
    persistent: bool,
    socket_addr: NetAddress,
    remote_addr: SocketAddr,
    mconn: MConnection,
    data: Mutex<HashMap<String, Arc<dyn Any + Send + Sync>>>,
}

impl fmt::Debug for Peer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let dir = if self.outbound { "out" } else { "in" };
        write!(f, "Peer{{MConn{{{}}} {} {dir}}}", self.remote_addr, self.id)
    }
}

impl fmt::Display for Peer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl Peer {
    /// Start the peer's MConnection over an upgraded connection. Returns the
    /// peer and the stream of received messages.
    pub fn start(
        conn: UpgradedConn,
        persistent: bool,
        descs: &[ChannelDescriptor],
        config: MConnConfig,
    ) -> Result<(Arc<Self>, mpsc::Receiver<Received>), String> {
        let (mconn, rx) = MConnection::start(conn.reader, conn.writer, descs, config)?;
        let peer = Arc::new(Self {
            id: conn.id,
            node_info: conn.node_info,
            outbound: conn.outbound,
            persistent,
            socket_addr: conn.socket_addr,
            remote_addr: conn.remote_addr,
            mconn,
            data: Mutex::new(HashMap::new()),
        });
        Ok((peer, rx))
    }

    /// The peer's ID.
    #[must_use]
    pub fn id(&self) -> NodeId {
        self.id
    }

    /// The peer's NodeInfo.
    #[must_use]
    pub fn node_info(&self) -> &NodeInfo {
        &self.node_info
    }

    /// Whether we dialed the peer.
    #[must_use]
    pub fn is_outbound(&self) -> bool {
        self.outbound
    }

    /// Whether we redial the peer after a disconnect.
    #[must_use]
    pub fn is_persistent(&self) -> bool {
        self.persistent
    }

    /// The dialed address (outbound) or `id@remote` (inbound).
    #[must_use]
    pub fn socket_addr(&self) -> &NetAddress {
        &self.socket_addr
    }

    /// Remote socket address.
    #[must_use]
    pub fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }

    /// Remote IP.
    #[must_use]
    pub fn remote_ip(&self) -> IpAddr {
        self.remote_addr.ip()
    }

    /// Whether the connection is running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.mconn.is_running()
    }

    /// Queue `msg` on channel `ch`, waiting up to 10s (`Send`). False if the
    /// peer does not advertise the channel.
    pub async fn send(&self, ch: u8, msg: Vec<u8>) -> bool {
        if !self.is_running() || !self.node_info.has_channel(ch) {
            return false;
        }
        self.mconn.send(ch, msg).await
    }

    /// Queue `msg` without waiting (`TrySend`).
    #[must_use]
    pub fn try_send(&self, ch: u8, msg: Vec<u8>) -> bool {
        if !self.is_running() || !self.node_info.has_channel(ch) {
            return false;
        }
        self.mconn.try_send(ch, msg)
    }

    /// Encode and send a protobuf message.
    pub async fn send_msg<M: prost::Message>(&self, ch: u8, msg: &M) -> bool {
        self.send(ch, msg.encode_to_vec()).await
    }

    /// Encode and try to send a protobuf message.
    #[must_use]
    pub fn try_send_msg<M: prost::Message>(&self, ch: u8, msg: &M) -> bool {
        self.try_send(ch, msg.encode_to_vec())
    }

    /// Heuristic: can a send on `ch` proceed without blocking.
    #[must_use]
    pub fn can_send(&self, ch: u8) -> bool {
        self.mconn.can_send(ch)
    }

    /// Connection status.
    #[must_use]
    pub fn status(&self) -> ConnectionStatus {
        self.mconn.status()
    }

    /// Stop and close the connection.
    pub fn stop(&self) {
        self.mconn.stop();
    }

    /// Flush queued messages, then close (`FlushStop`).
    pub fn flush_stop(&self) {
        self.mconn.flush_stop();
    }

    /// Wait until the connection stops.
    pub async fn closed(&self) -> ConnState {
        self.mconn.closed().await
    }

    /// Store reactor-specific data on the peer (`Set`).
    pub fn set(&self, key: &str, value: Arc<dyn Any + Send + Sync>) {
        self.data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key.to_string(), value);
    }

    /// Fetch reactor-specific data (`Get`).
    #[must_use]
    pub fn get(&self, key: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        self.data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .cloned()
    }
}
