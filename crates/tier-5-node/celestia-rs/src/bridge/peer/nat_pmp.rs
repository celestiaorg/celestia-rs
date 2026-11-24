//! Fall back to NAT-PMP on the local IPv4 router when IGD discovery fails.
mod gateway;
mod protocol;

use super::network::{Behaviour, Peering};
use libp2p::{Multiaddr, Swarm, multiaddr::Protocol};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::Ipv4Addr,
    time::Duration,
};
use tokio::sync::watch;

type Addresses = BTreeSet<Multiaddr>;
type Ports = BTreeMap<(u8, u16), u16>;

#[derive(Default)]
pub(super) struct Mapper {
    listeners: Option<watch::Sender<Addresses>>,
    updates: Option<watch::Receiver<Addresses>>,
    advertised: Addresses,
}

impl Mapper {
    pub(super) fn fallback(&mut self, swarm: &Swarm<Behaviour>) {
        if self.listeners.is_some() {
            return;
        }
        let (listeners, desired) = watch::channel(swarm.listeners().cloned().collect());
        let (output, updates) = watch::channel(Addresses::new());
        tokio::spawn(maintain(desired, output));
        self.listeners = Some(listeners);
        self.updates = Some(updates);
    }
    pub(super) fn refresh(&self, swarm: &Swarm<Behaviour>) {
        if let Some(tx) = &self.listeners {
            let desired = swarm.listeners().cloned().collect();
            tx.send_if_modified(|current| {
                if *current == desired {
                    return false;
                }
                *current = desired;
                true
            });
        }
    }
    pub(super) async fn changed(&mut self) -> Addresses {
        if let Some(updates) = &mut self.updates
            && updates.changed().await.is_ok()
        {
            return updates.borrow_and_update().clone();
        }
        std::future::pending().await
    }
    pub(super) fn apply(&mut self, next: Addresses, peers: &Peering, swarm: &mut Swarm<Behaviour>) {
        for old in self.advertised.difference(&next) {
            peers.expire_address(swarm, old);
        }
        for address in next.difference(&self.advertised) {
            if peers.advertised(address) {
                swarm.add_external_address(address.clone());
            }
        }
        self.advertised = next;
    }
}

fn endpoint(addr: &Multiaddr, local: Ipv4Addr) -> Option<(u8, u16)> {
    let mut parts = addr.iter();
    if parts.next() != Some(Protocol::Ip4(local)) {
        return None;
    }
    match parts.next()? {
        Protocol::Tcp(port) if port != 0 => Some((2, port)),
        Protocol::Udp(port) if port != 0 => Some((1, port)),
        _ => None,
    }
}

fn public(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    a != 0
        && a < 224
        && !ip.is_private()
        && !ip.is_loopback()
        && !ip.is_link_local()
        && !ip.is_documentation()
        && !(a == 100 && (64..128).contains(&b))
        && !(a == 198 && matches!(b, 18 | 19))
        && !(a == 192 && b == 0 && c == 0)
}

async fn maintain(mut desired: watch::Receiver<Addresses>, output: watch::Sender<Addresses>) {
    let mut client = None;
    let mut ports = Ports::new();
    loop {
        let addresses = desired.borrow_and_update().clone();
        let mut delay = 20;
        let mut mapped = Addresses::new();
        let next_client = if !addresses.is_empty()
            && let Ok(Ok(router)) = tokio::task::spawn_blocking(gateway::default_gateway).await
        {
            protocol::Client::new(router).await.ok()
        } else {
            None
        };
        if next_client != client {
            if let Some(previous) = client {
                release(&previous, &mut ports).await;
            }
            client = next_client;
        }
        if let Some(conn) = &client {
            let wanted: BTreeSet<_> = addresses
                .iter()
                .filter_map(|addr| endpoint(addr, conn.local))
                .collect();
            for (key, outside) in std::mem::take(&mut ports) {
                if wanted.contains(&key) {
                    ports.insert(key, outside);
                } else {
                    let _ = conn.map(key.0, key.1, outside, 0).await;
                }
            }
            if !wanted.is_empty()
                && let Ok(ip) = conn.external().await
                && public(ip)
            {
                for (opcode, inside) in wanted {
                    let key = (opcode, inside);
                    let previous = *ports.get(&key).unwrap_or(&inside);
                    if let Ok((outside, seconds)) = conn.map(opcode, inside, previous, 60).await {
                        ports.insert(key, outside);
                        delay = delay.min((seconds / 3).max(1));
                        for addr in addresses
                            .iter()
                            .filter(|addr| endpoint(addr, conn.local) == Some(key))
                        {
                            let host = addr.replace(0, |_| Some(Protocol::Ip4(ip)));
                            let transport = if opcode == 1 {
                                Protocol::Udp(outside)
                            } else {
                                Protocol::Tcp(outside)
                            };
                            if let Some(addr) =
                                host.and_then(|addr| addr.replace(1, |_| Some(transport)))
                            {
                                mapped.insert(addr);
                            }
                        }
                    }
                }
            }
        }
        output.send_if_modified(|current| {
            if *current == mapped {
                return false;
            }
            *current = mapped;
            true
        });
        tokio::select! {
            result=desired.changed()=>if result.is_err(){break;},
            ()=tokio::time::sleep(Duration::from_secs(u64::from(delay)))=>{}
        }
    }
    if let Some(client) = client {
        release(&client, &mut ports).await;
    }
}

async fn release(client: &protocol::Client, ports: &mut Ports) {
    for ((opcode, inside), outside) in std::mem::take(ports) {
        let _ = client.map(opcode, inside, outside, 0).await;
    }
}
