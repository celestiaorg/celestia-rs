//! TCP transport: dial/accept, SecretConnection upgrade, NodeInfo handshake
//! and connection filtering (port of `p2p/transport.go`'s
//! `MultiplexTransport`).

use std::collections::HashMap;
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Mutex, RwLock};
use std::time::Duration;

use celestia_core_proto::tendermint::p2p as pb;
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::TcpStream;

use crate::key::{NodeId, NodeKey};
use crate::netaddress::NetAddress;
use crate::node_info::{MAX_NODE_INFO_SIZE, NodeInfo};
use crate::secret_connection::{
    ByteWrite, SecretConnection, SecretReader, SecretWriter, encode_delimited, read_delimited,
};

/// Receiving half of a peer connection.
pub type PeerReader = SecretReader<ReadHalf<TcpStream>>;
/// Sending half of a peer connection.
pub type PeerWriter = SecretWriter<WriteHalf<TcpStream>>;

/// Why a connection was rejected (Go's `ErrRejected` flags).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectReason {
    /// Duplicate connection, IP or peer ID.
    Duplicate(String),
    /// Rejected by a connection filter.
    Filtered(String),
    /// SecretConnection failed or the authenticated ID did not match.
    AuthFailure(String),
    /// The peer's NodeInfo failed validation.
    NodeInfoInvalid(String),
    /// We connected to ourselves; the address is ours.
    SelfConnect(NetAddress),
    /// NodeInfo incompatible (`CompatibleWith`).
    Incompatible(String),
}

/// Transport errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// The connection was rejected.
    Rejected(RejectReason),
    /// Dial or socket error.
    Io(String),
    /// The transport is closed.
    Closed,
}

impl TransportError {
    /// Whether this is a self-connection rejection.
    #[must_use]
    pub fn is_self(&self) -> bool {
        matches!(self, Self::Rejected(RejectReason::SelfConnect(_)))
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(r) => match r {
                RejectReason::Duplicate(e) => write!(f, "duplicate: {e}"),
                RejectReason::Filtered(e) => write!(f, "filtered: {e}"),
                RejectReason::AuthFailure(e) => write!(f, "auth failure: {e}"),
                RejectReason::NodeInfoInvalid(e) => write!(f, "node info invalid: {e}"),
                RejectReason::SelfConnect(a) => write!(f, "self ID<{a}>"),
                RejectReason::Incompatible(e) => write!(f, "incompatible: {e}"),
            },
            Self::Io(e) => write!(f, "{e}"),
            Self::Closed => f.write_str("transport has been closed"),
        }
    }
}

impl std::error::Error for TransportError {}

/// Transport timeouts and filters.
#[derive(Debug, Clone)]
pub struct TransportConfig {
    /// TCP dial timeout (Go's effective default: 1s).
    pub dial_timeout: Duration,
    /// Timeout for each of the SecretConnection and NodeInfo handshakes
    /// (Go's effective default: 3s).
    pub handshake_timeout: Duration,
    /// Accept several connections from one IP.
    pub allow_duplicate_ip: bool,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            dial_timeout: Duration::from_secs(1),
            handshake_timeout: Duration::from_secs(3),
            allow_duplicate_ip: false,
        }
    }
}

/// An authenticated connection with the peer's NodeInfo.
pub struct UpgradedConn {
    /// Receiving half.
    pub reader: PeerReader,
    /// Sending half.
    pub writer: PeerWriter,
    /// The peer's NodeInfo (validated and compatible).
    pub node_info: NodeInfo,
    /// Authenticated peer ID.
    pub id: NodeId,
    /// Remote socket address.
    pub remote_addr: SocketAddr,
    /// The dialed address (outbound) or `id@remote_addr` (inbound).
    pub socket_addr: NetAddress,
    /// Whether we dialed.
    pub outbound: bool,
}

impl fmt::Debug for UpgradedConn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UpgradedConn")
            .field("id", &self.id)
            .field("remote_addr", &self.remote_addr)
            .field("outbound", &self.outbound)
            .finish_non_exhaustive()
    }
}

/// The multiplex transport.
#[derive(Debug)]
pub struct Transport {
    node_key: NodeKey,
    node_info: RwLock<NodeInfo>,
    config: TransportConfig,
    /// Open connections: remote socket address -> IP (`ConnSet`).
    conns: Mutex<HashMap<SocketAddr, IpAddr>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Transport {
    /// A transport for our key and NodeInfo.
    #[must_use]
    pub fn new(node_key: NodeKey, node_info: NodeInfo, config: TransportConfig) -> Self {
        Self {
            node_key,
            node_info: RwLock::new(node_info),
            config,
            conns: Mutex::new(HashMap::new()),
        }
    }

    /// Our NodeInfo.
    #[must_use]
    pub fn node_info(&self) -> NodeInfo {
        self.node_info
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Replace our NodeInfo (e.g. after binding the listener).
    pub fn set_node_info(&self, ni: NodeInfo) {
        *self
            .node_info
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = ni;
    }

    /// Register a channel in our NodeInfo (`AddChannel`).
    pub fn add_channel(&self, ch: u8) {
        let mut ni = self
            .node_info
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !ni.has_channel(ch) {
            ni.channels.push(ch);
        }
    }

    /// Dial and upgrade an outbound connection (`Dial`).
    pub async fn dial(&self, addr: &NetAddress) -> Result<UpgradedConn, TransportError> {
        let stream = tokio::time::timeout(
            self.config.dial_timeout,
            TcpStream::connect(addr.socket_addr()),
        )
        .await
        .map_err(|_| TransportError::Io(format!("dial tcp {}: i/o timeout", addr.dial_string())))?
        .map_err(|e| TransportError::Io(format!("dial tcp {}: {e}", addr.dial_string())))?;
        self.upgrade(stream, Some(addr)).await
    }

    /// Upgrade an accepted (inbound) connection.
    pub async fn accept(&self, stream: TcpStream) -> Result<UpgradedConn, TransportError> {
        self.upgrade(stream, None).await
    }

    fn filter_conn(&self, remote: SocketAddr) -> Result<(), TransportError> {
        let mut conns = lock(&self.conns);
        if conns.contains_key(&remote) {
            return Err(TransportError::Rejected(RejectReason::Duplicate(format!(
                "conn {remote}"
            ))));
        }
        if !self.config.allow_duplicate_ip && conns.values().any(|ip| *ip == remote.ip()) {
            return Err(TransportError::Rejected(RejectReason::Filtered(format!(
                "ip<{}> already connected",
                remote.ip()
            ))));
        }
        conns.insert(remote, remote.ip());
        Ok(())
    }

    /// Forget a connection (`Cleanup`).
    pub fn cleanup(&self, remote: SocketAddr) {
        lock(&self.conns).remove(&remote);
    }

    async fn upgrade(
        &self,
        stream: TcpStream,
        dialed: Option<&NetAddress>,
    ) -> Result<UpgradedConn, TransportError> {
        let remote = stream
            .peer_addr()
            .map_err(|e| TransportError::Io(e.to_string()))?;
        let _ = stream.set_nodelay(true);
        self.filter_conn(remote)?;
        let res = self.upgrade_inner(stream, remote, dialed).await;
        if res.is_err() {
            self.cleanup(remote);
        }
        res
    }

    async fn upgrade_inner(
        &self,
        stream: TcpStream,
        remote: SocketAddr,
        dialed: Option<&NetAddress>,
    ) -> Result<UpgradedConn, TransportError> {
        let timeout = self.config.handshake_timeout;
        let auth = |e: String| TransportError::Rejected(RejectReason::AuthFailure(e));
        let sc = tokio::time::timeout(timeout, SecretConnection::handshake(stream, &self.node_key))
            .await
            .map_err(|_| auth("secret conn failed: i/o timeout".into()))?
            .map_err(|e| auth(format!("secret conn failed: {e}")))?;
        let conn_id = sc.remote_id();
        if let Some(d) = dialed
            && d.id != Some(conn_id)
        {
            return Err(auth(format!(
                "conn.ID ({conn_id}) dialed ID ({}) mismatch",
                d.id.map(|i| i.to_string()).unwrap_or_default()
            )));
        }
        let (mut reader, mut writer) = sc.into_split();
        let ours = self.node_info();
        let out = encode_delimited(&ours.to_proto());
        let exchange = async {
            let (w, r) = tokio::join!(
                writer.write_all_bytes(&out),
                read_delimited(&mut reader, MAX_NODE_INFO_SIZE)
            );
            w?;
            r
        };
        let (body, _) = tokio::time::timeout(timeout, exchange)
            .await
            .map_err(|_| auth("handshake failed: i/o timeout".into()))?
            .map_err(|e| auth(format!("handshake failed: {e}")))?;
        let theirs = celestia_core_proto::gogo_decode::<pb::DefaultNodeInfo>(body.as_slice())
            .map_err(|e| auth(format!("handshake failed: {e}")))?;
        let theirs = NodeInfo::from_proto(&theirs);
        theirs
            .validate()
            .map_err(|e| TransportError::Rejected(RejectReason::NodeInfoInvalid(e)))?;
        if theirs.node_id().ok() != Some(conn_id) {
            return Err(auth(format!(
                "conn.ID ({conn_id}) NodeInfo.ID ({}) mismatch",
                theirs.id
            )));
        }
        if ours.id == theirs.id {
            let _ = writer.shutdown_bytes().await;
            return Err(TransportError::Rejected(RejectReason::SelfConnect(
                NetAddress::from_socket(conn_id, remote),
            )));
        }
        ours.compatible_with(&theirs)
            .map_err(|e| TransportError::Rejected(RejectReason::Incompatible(e)))?;
        let socket_addr = dialed
            .cloned()
            .unwrap_or_else(|| NetAddress::from_socket(conn_id, remote));
        Ok(UpgradedConn {
            reader,
            writer,
            node_info: theirs,
            id: conn_id,
            remote_addr: remote,
            socket_addr,
            outbound: dialed.is_some(),
        })
    }
}
