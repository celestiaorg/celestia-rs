//! Discover DA providers without adding them to the trusted checkpoint set.
use futures::StreamExt;
use libp2p::swarm::{SwarmEvent, dial_opts::DialOpts};
use libp2p::{PeerId, Swarm, identify, kad};
use std::{collections::BTreeSet, time::Duration};
use tokio::sync::watch;

use super::super::{
    admin,
    network::{Behaviour, BehaviourEvent, Peering},
};

pub(super) async fn maintain_connections(
    mut swarm: Swarm<Behaviour>,
    peers: &Peering,
    connected: watch::Sender<Vec<PeerId>>,
    network: &str,
    mut requests: tokio::sync::mpsc::Receiver<admin::Request>,
    mut operations: admin::Operations,
    blocklist: admin::Blocklist,
) {
    const MAX_PROVIDERS: usize = 16;
    let trusted = &peers.bootstrap;
    let mut retry = tokio::time::interval(Duration::from_secs(3));
    retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut discovery = tokio::time::interval(Duration::from_secs(10));
    discovery.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut pmp = super::super::nat_pmp::Mapper::default();
    let mut query = None;
    let mut providers = BTreeSet::new();
    for (peer, addresses) in trusted {
        for addr in addresses {
            swarm.behaviour_mut().dht.add_address(peer, addr.clone());
        }
    }
    loop {
        tokio::select! {
            addresses = pmp.changed() => pmp.apply(addresses, peers, &mut swarm),
            Some(request) = requests.recv() => operations.handle(request, &mut swarm, network, &blocklist),
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
                    SwarmEvent::ConnectionClosed { peer_id, num_established: 0, .. } => {
                        operations.closed(peer_id);
                        providers.remove(&peer_id);
                    }
                    SwarmEvent::OutgoingConnectionError { connection_id, error, peer_id, .. } => {
                        operations.dial_result(connection_id, Err(error.into()));
                        let Some(peer_id) = peer_id else { continue };
                        if !swarm.is_connected(&peer_id) { providers.remove(&peer_id); }
                    }
                    SwarmEvent::NewListenAddr { address, .. } => { pmp.refresh(&swarm); peers.listen_address(&mut swarm, &address); tracing::info!(%address, peer=%swarm.local_peer_id(), "DA light peer listening"); }
                    SwarmEvent::ExpiredListenAddr { address, .. } => {pmp.refresh(&swarm);peers.expire_address(&mut swarm, &address);}
                    SwarmEvent::Behaviour(BehaviourEvent::PortMapping(libp2p::upnp::Event::GatewayNotFound)) => pmp.fallback(&swarm),
                    SwarmEvent::ExternalAddrConfirmed { address } if !peers.advertised(&address) => swarm.remove_external_address(&address),
                    SwarmEvent::ExternalAddrExpired { address } if peers.announce.contains(&address) => swarm.add_external_address(address),
                    SwarmEvent::ConnectionEstablished { connection_id, .. } => operations.dial_result(connection_id, Ok(())),
                    _ => continue,
                }
                let online: Vec<_> = trusted.keys().chain(providers.iter()).filter(|id| swarm.is_connected(id)).copied().collect();
                connected.send_if_modified(|old| { if *old == online { false } else { *old = online; true } });
            }
        }
    }
}
