//! Discover DA providers without adding them to the trusted checkpoint set.
use anyhow::Result;
use futures::StreamExt;
use libp2p::swarm::{NetworkBehaviour, SwarmEvent, dial_opts::DialOpts};
use libp2p::{Multiaddr, PeerId, StreamProtocol, Swarm, identify, identity, kad};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
use tokio::sync::watch;

// libp2p derives an event conversion for the stream handler's uninhabited event.
#[allow(unreachable_code)]
mod composed {
    use super::{NetworkBehaviour, identify, kad};

    #[derive(NetworkBehaviour)]
    pub(crate) struct Behaviour {
        pub(crate) streams: libp2p_stream::Behaviour,
        pub(crate) identify: identify::Behaviour,
        pub(crate) dht: kad::Behaviour<kad::store::MemoryStore>,
    }
}
pub(super) use composed::Behaviour;
use composed::BehaviourEvent;

impl Behaviour {
    pub(super) fn new(key: &identity::Keypair, network: &str) -> Result<Self> {
        let id = key.public().to_peer_id();
        let protocol = StreamProtocol::try_from_owned(format!("/celestia/{network}/kad/1.0.0"))?;
        let mut dht = kad::Behaviour::with_config(
            id,
            kad::store::MemoryStore::new(id),
            kad::Config::new(protocol),
        );
        dht.set_mode(Some(kad::Mode::Client));
        Ok(Self {
            streams: libp2p_stream::Behaviour::new(),
            identify: identify::Behaviour::new(identify::Config::new(
                "ipfs/0.1.0".into(),
                key.public(),
            )),
            dht,
        })
    }
}

pub(super) async fn maintain_connections(
    mut swarm: Swarm<Behaviour>,
    trusted: &BTreeMap<PeerId, Vec<Multiaddr>>,
    connected: watch::Sender<Vec<PeerId>>,
) {
    const MAX_PROVIDERS: usize = 16;
    let mut retry = tokio::time::interval(Duration::from_secs(3));
    retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut discovery = tokio::time::interval(Duration::from_secs(10));
    discovery.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut query = None;
    let mut providers = BTreeSet::new();
    for (peer, addresses) in trusted {
        for addr in addresses {
            swarm.behaviour_mut().dht.add_address(peer, addr.clone());
        }
    }
    loop {
        tokio::select! {
            _ = retry.tick() => {
                for (peer, addresses) in trusted {
                    if !swarm.is_connected(peer) {
                        let opts = DialOpts::peer_id(*peer).addresses(addresses.clone()).build();
                        let _ = swarm.dial(opts);
                    }
                }
            }
            _ = discovery.tick(), if query.is_none() => {
                query = Some(swarm.behaviour_mut().dht.get_providers(super::super::network::provider_key()));
            }
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::Behaviour(BehaviourEvent::Identify(identify::Event::Received { peer_id, info, .. })) => {
                        for addr in info.listen_addrs { swarm.behaviour_mut().dht.add_address(&peer_id, addr); }
                    }
                    SwarmEvent::Behaviour(BehaviourEvent::Dht(kad::Event::OutboundQueryProgressed { id, result, step, .. })) if Some(id) == query => {
                        if let kad::QueryResult::GetProviders(Ok(kad::GetProvidersOk::FoundProviders { providers: found, .. })) = result {
                            for peer in found {
                                if providers.len() >= MAX_PROVIDERS || peer == *swarm.local_peer_id() || trusted.contains_key(&peer) {continue;}
                                if swarm.is_connected(&peer) || swarm.dial(DialOpts::peer_id(peer).build()).is_ok() {
                                    providers.insert(peer);
                                }
                            }
                        }
                        if step.last { query = None; }
                    }
                    SwarmEvent::ConnectionClosed { peer_id, num_established: 0, .. } |
                    SwarmEvent::OutgoingConnectionError { peer_id: Some(peer_id), .. } => {
                        if !swarm.is_connected(&peer_id) { providers.remove(&peer_id); }
                    }
                    SwarmEvent::NewListenAddr { address, .. } => tracing::info!(%address, peer=%swarm.local_peer_id(), "DA light peer listening"),
                    SwarmEvent::ConnectionEstablished { .. } => {}
                    _ => continue,
                }
                let online: Vec<_> = trusted.keys().chain(providers.iter()).filter(|id| swarm.is_connected(id)).copied().collect();
                connected.send_if_modified(|old| { if *old == online { false } else { *old = online; true } });
            }
        }
    }
}
