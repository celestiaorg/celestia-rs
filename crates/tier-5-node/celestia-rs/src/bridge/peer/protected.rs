//! Connection-manager protection tags are process-local, like Go's BasicConnMgr.
use std::{
    collections::{HashMap, HashSet, VecDeque},
    convert::Infallible,
    task::{Context, Poll},
};

use libp2p::{
    Multiaddr, PeerId,
    core::{Endpoint, transport::PortUse, upgrade::DeniedUpgrade},
    swarm::{
        ConnectionDenied, ConnectionHandler, ConnectionHandlerEvent, ConnectionId, FromSwarm,
        NetworkBehaviour, NotifyHandler, SubstreamProtocol, ToSwarm, handler::ConnectionEvent,
    },
};

#[derive(Default)]
pub(crate) struct Protected {
    tags: HashMap<PeerId, HashSet<String>>,
    connections: HashMap<PeerId, HashSet<ConnectionId>>,
    updates: VecDeque<(PeerId, ConnectionId, bool)>,
}

impl Protected {
    pub(super) fn contains(&self, peer: PeerId, tag: &str) -> bool {
        self.tags
            .get(&peer)
            .is_some_and(|tags| tag.is_empty() || tags.contains(tag))
    }

    pub(super) fn protect(&mut self, peer: PeerId, tag: &str) {
        let tags = self.tags.entry(peer).or_default();
        let previously_empty = tags.is_empty();
        tags.insert(tag.to_owned());
        if previously_empty {
            self.notify(peer, true);
        }
    }

    pub(super) fn unprotect(&mut self, peer: PeerId, tag: &str) -> bool {
        let Some(tags) = self.tags.get_mut(&peer) else {
            return false;
        };
        tags.remove(tag);
        if !tags.is_empty() {
            return true;
        }
        self.tags.remove(&peer);
        self.notify(peer, false);
        false
    }

    fn notify(&mut self, peer: PeerId, protect: bool) {
        if let Some(connections) = self.connections.get(&peer) {
            self.updates
                .extend(connections.iter().map(|id| (peer, *id, protect)));
        }
    }
}

pub(crate) struct KeepAlive(bool);

impl ConnectionHandler for KeepAlive {
    type FromBehaviour = bool;
    type ToBehaviour = Infallible;
    type InboundProtocol = DeniedUpgrade;
    type OutboundProtocol = DeniedUpgrade;
    type InboundOpenInfo = ();
    type OutboundOpenInfo = ();

    fn listen_protocol(&self) -> SubstreamProtocol<DeniedUpgrade> {
        SubstreamProtocol::new(DeniedUpgrade, ())
    }
    fn connection_keep_alive(&self) -> bool {
        self.0
    }
    fn on_behaviour_event(&mut self, protect: bool) {
        self.0 = protect;
    }
    fn on_connection_event(&mut self, _: ConnectionEvent<'_, DeniedUpgrade, DeniedUpgrade>) {}
    fn poll(
        &mut self,
        _: &mut Context<'_>,
    ) -> Poll<ConnectionHandlerEvent<DeniedUpgrade, (), Infallible>> {
        Poll::Pending
    }
}

impl NetworkBehaviour for Protected {
    type ConnectionHandler = KeepAlive;
    type ToSwarm = Infallible;
    fn handle_established_inbound_connection(
        &mut self,
        _: ConnectionId,
        peer: PeerId,
        _: &Multiaddr,
        _: &Multiaddr,
    ) -> Result<KeepAlive, ConnectionDenied> {
        Ok(KeepAlive(self.tags.contains_key(&peer)))
    }
    fn handle_established_outbound_connection(
        &mut self,
        _: ConnectionId,
        peer: PeerId,
        _: &Multiaddr,
        _: Endpoint,
        _: PortUse,
    ) -> Result<KeepAlive, ConnectionDenied> {
        Ok(KeepAlive(self.tags.contains_key(&peer)))
    }
    fn on_swarm_event(&mut self, event: FromSwarm<'_>) {
        match event {
            FromSwarm::ConnectionEstablished(e) => {
                self.connections
                    .entry(e.peer_id)
                    .or_default()
                    .insert(e.connection_id);
            }
            FromSwarm::ConnectionClosed(e) => {
                if let Some(ids) = self.connections.get_mut(&e.peer_id) {
                    ids.remove(&e.connection_id);
                    if ids.is_empty() {
                        self.connections.remove(&e.peer_id);
                    }
                }
            }
            _ => {}
        }
    }
    fn on_connection_handler_event(&mut self, _: PeerId, _: ConnectionId, never: Infallible) {
        match never {}
    }
    fn poll(&mut self, _: &mut Context<'_>) -> Poll<ToSwarm<Infallible, bool>> {
        self.updates
            .pop_front()
            .map_or(Poll::Pending, |(peer_id, id, event)| {
                Poll::Ready(ToSwarm::NotifyHandler {
                    peer_id,
                    handler: NotifyHandler::One(id),
                    event,
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protection_tags_drive_idle_connection_lifetime() {
        let peer = PeerId::random();
        let id = ConnectionId::new_unchecked(1);
        let mut policy = Protected::default();
        policy.protect(peer, "protected-bootstrap");
        policy.connections.entry(peer).or_default().insert(id);
        let mut handler = policy
            .handle_established_inbound_connection(
                id,
                peer,
                &Multiaddr::empty(),
                &Multiaddr::empty(),
            )
            .expect("local policy admits the connection");
        assert!(handler.connection_keep_alive());
        policy.protect(peer, "operator");
        assert!(policy.unprotect(peer, "protected-bootstrap"));
        assert!(policy.updates.is_empty());
        assert!(!policy.unprotect(peer, "operator"));
        let (remote, connection, keep) = policy.updates.pop_front().expect("notify idle handler");
        assert_eq!((remote, connection), (peer, id));
        handler.on_behaviour_event(keep);
        assert!(!handler.connection_keep_alive());
    }
}
