//! The reactor API (port of `p2p/base_reactor.go`).
//!
//! A reactor owns one or more channels. The switch routes every message
//! received on a reactor's channels to [`Reactor::receive`], one at a time
//! per reactor, from a bounded queue (Go's `BaseReactor` processor). An
//! `Err` from `receive` disconnects the peer (`StopPeerForError`), as a
//! decode failure does in Go.
//!
//! Methods are synchronous and must not block; reactors that need to wait
//! (e.g. `Peer::send`) spawn a task. Consensus, mempool, block sync and
//! propagation reactors implement this trait.

use std::sync::Arc;

use crate::mconnection::ChannelDescriptor;
use crate::peer::Peer;
use crate::switch::WeakSwitch;

/// A message received from a peer.
#[derive(Debug, Clone)]
pub struct Envelope {
    /// Sender.
    pub src: Arc<Peer>,
    /// Channel it arrived on.
    pub channel_id: u8,
    /// Raw message bytes (decode with the channel's protobuf type).
    pub message: Vec<u8>,
}

impl Envelope {
    /// Decode the message as `M`.
    pub fn decode<M: prost::Message + Default>(&self) -> Result<M, prost::DecodeError> {
        M::decode(self.message.as_slice())
    }
}

/// A protocol handler plugged into the [`crate::switch::Switch`].
pub trait Reactor: Send + Sync + 'static {
    /// Name, used in logs and as the peer-set key.
    fn name(&self) -> String;

    /// Channels handled by this reactor.
    fn channels(&self) -> Vec<ChannelDescriptor>;

    /// Called once when the reactor is added to a switch.
    fn set_switch(&self, _switch: WeakSwitch) {}

    /// Called when the switch starts.
    fn on_start(self: Arc<Self>) -> Result<(), String> {
        Ok(())
    }

    /// Called when the switch stops.
    fn on_stop(&self) {}

    /// Before the peer starts: an `Err` excludes it from this reactor.
    fn init_peer(&self, _peer: &Arc<Peer>) -> Result<(), String> {
        Ok(())
    }

    /// A peer connected.
    fn add_peer(&self, _peer: &Arc<Peer>) {}

    /// A peer disconnected (`reason` is empty for a graceful stop).
    fn remove_peer(&self, _peer: &Arc<Peer>, _reason: &str) {}

    /// Handle a received message. `Err` disconnects the peer.
    fn receive(&self, envelope: Envelope) -> Result<(), String>;

    /// Capacity of the incoming message queue (Go default 100).
    fn queue_size(&self) -> usize {
        100
    }
}
