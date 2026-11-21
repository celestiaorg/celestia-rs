//! Deliver negotiated go-header and SHREX streams directly to the swarm event loop.
use super::Kind;
use anyhow::Result;
use libp2p::{
    Multiaddr, PeerId, Stream, StreamProtocol,
    core::{Endpoint, InboundUpgrade, UpgradeInfo, transport::PortUse, upgrade::DeniedUpgrade},
    swarm::{
        ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, OneShotHandler,
        SubstreamProtocol, THandler, THandlerInEvent, THandlerOutEvent, ToSwarm,
    },
};
use std::{
    collections::VecDeque,
    convert::Infallible,
    future::{Ready, ready},
    task::{Context, Poll},
};

#[derive(Debug, Clone)]
pub(crate) struct Protocol(Kind, StreamProtocol);

impl AsRef<str> for Protocol {
    fn as_ref(&self) -> &str {
        self.1.as_ref()
    }
}

#[derive(Clone)]
pub(crate) struct Protocols(Vec<Protocol>);
impl UpgradeInfo for Protocols {
    type Info = Protocol;
    type InfoIter = std::vec::IntoIter<Protocol>;
    fn protocol_info(&self) -> Self::InfoIter {
        self.0.clone().into_iter()
    }
}

#[derive(Debug)]
pub(crate) struct Accepted {
    pub(super) kind: Kind,
    pub(super) stream: Stream,
}
impl From<Infallible> for Accepted {
    fn from(never: Infallible) -> Self {
        match never {}
    }
}
impl InboundUpgrade<Stream> for Protocols {
    type Output = Accepted;
    type Error = Infallible;
    type Future = Ready<Result<Accepted, Infallible>>;
    fn upgrade_inbound(self, stream: Stream, protocol: Protocol) -> Self::Future {
        ready(Ok(Accepted {
            kind: protocol.0,
            stream,
        }))
    }
}

pub(crate) struct Inbound {
    protocols: Protocols,
    accepted: VecDeque<(PeerId, Accepted)>,
}

impl Inbound {
    pub(super) fn new(network: &str) -> Result<Self> {
        let mut protocols = Vec::new();
        for (kind, name) in [
            (Kind::Header, "header-ex/v0.0.3"),
            (Kind::Namespace, "shrex/v0.1.0/nd_v0"),
            (Kind::Range, "shrex/v0.1.0/rangeNamespaceData_v0"),
            (Kind::Row, "shrex/v0.1.0/row_v0"),
            (Kind::Sample, "shrex/v0.1.0/sample_v0"),
            (Kind::Eds, "shrex/v0.1.0/eds_v0"),
        ] {
            protocols.push(Protocol(
                kind,
                StreamProtocol::try_from_owned(format!("/{network}/{name}"))?,
            ));
        }
        Ok(Self {
            protocols: Protocols(protocols),
            accepted: VecDeque::new(),
        })
    }
    fn handler(&self) -> THandler<Self> {
        OneShotHandler::new(
            SubstreamProtocol::new(self.protocols.clone(), ()),
            libp2p::swarm::OneShotHandlerConfig::default(),
        )
    }
}

impl NetworkBehaviour for Inbound {
    type ConnectionHandler = OneShotHandler<Protocols, DeniedUpgrade, Accepted>;
    type ToSwarm = (PeerId, Accepted);

    fn handle_established_inbound_connection(
        &mut self,
        _: ConnectionId,
        _: PeerId,
        _: &Multiaddr,
        _: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(self.handler())
    }
    fn handle_established_outbound_connection(
        &mut self,
        _: ConnectionId,
        _: PeerId,
        _: &Multiaddr,
        _: Endpoint,
        _: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(self.handler())
    }
    fn on_swarm_event(&mut self, _: FromSwarm<'_>) {}
    fn on_connection_handler_event(
        &mut self,
        peer: PeerId,
        _: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        if let Ok(accepted) = event
            && self.accepted.len() < 64
        {
            self.accepted.push_back((peer, accepted));
        }
    }
    fn poll(&mut self, _: &mut Context<'_>) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        self.accepted.pop_front().map_or(Poll::Pending, |stream| {
            Poll::Ready(ToSwarm::GenerateEvent(stream))
        })
    }
}
