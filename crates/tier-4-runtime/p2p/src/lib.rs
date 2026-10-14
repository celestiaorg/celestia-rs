//! celestia-core v0.42.3 compatible P2P stack (ADR 0003, M10).
//!
//! Layers, bottom up:
//!
//! - [`key`] / [`netaddress`]: node keys (`node_key.json`, node ID =
//!   `hex(sha256(pubkey)[:20])`) and `ID@IP:port` addresses.
//! - [`secret_connection`]: the STS handshake (X25519 + Merlin transcript +
//!   HKDF-SHA256, ed25519 challenge signatures) and ChaCha20-Poly1305
//!   framing, byte-exact with Go.
//! - [`mconnection`]: channel multiplexing with priorities, packet
//!   fragmentation, flow control and ping/pong.
//! - [`node_info`] / [`transport`]: the `DefaultNodeInfo` handshake and
//!   compatibility checks; dial/accept with timeouts and duplicate/self
//!   filtering.
//! - [`switch`] / [`peer`] / [`reactor`]: the reactor registry, peer
//!   lifecycle, broadcast and persistent peers with backoff.
//! - [`pex`]: the address book (`addrbook.json`) and PEX reactor.

pub mod flowrate;
pub mod key;
pub mod mconnection;
pub mod netaddress;
pub mod node_info;
pub mod peer;
pub mod pex;
pub mod reactor;
pub mod secret_connection;
pub mod switch;
pub mod transport;

pub use key::{NodeId, NodeKey};
pub use mconnection::{ChannelDescriptor, MConnConfig, MConnection};
pub use netaddress::NetAddress;
pub use node_info::{NodeInfo, NodeInfoOther, ProtocolVersion};
pub use peer::Peer;
pub use reactor::{Envelope, Reactor};
pub use switch::{Switch, SwitchConfig, SwitchError, WeakSwitch};
pub use transport::{Transport, TransportConfig, TransportError};
