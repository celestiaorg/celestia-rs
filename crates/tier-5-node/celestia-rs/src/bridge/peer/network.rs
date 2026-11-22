//! Network-scoped provider discovery and publication of locally committed headers.
use anyhow::Result;
use libp2p::{StreamProtocol, gossipsub, identify, identity, kad, swarm::NetworkBehaviour};
use prost::Message;

use super::HeaderWire;

// libp2p's generated handler maps events from uninhabited handler variants.
#[allow(unreachable_code)]
mod composed {
    use super::{NetworkBehaviour, gossipsub, identify, kad};
    #[derive(NetworkBehaviour)]
    pub(crate) struct Behaviour {
        // Reject blocked connections before stateful behaviours observe establishment.
        pub(crate) gate:
            libp2p::allow_block_list::Behaviour<libp2p::allow_block_list::BlockedPeers>,
        pub(crate) autonat: libp2p_autonat::Behaviour,
        pub(crate) protected: super::super::protected::Protected,
        pub(crate) streams:
            libp2p::swarm::behaviour::toggle::Toggle<super::super::inbound::Inbound>,
        pub(crate) negotiated: libp2p_stream::Behaviour,
        pub(crate) identify: identify::Behaviour,
        pub(crate) dht: kad::Behaviour<kad::store::MemoryStore>,
        pub(crate) gossip: gossipsub::Behaviour,
    }
}

pub(super) use composed::{Behaviour, BehaviourEvent};

pub(super) fn header_topic(network: &str) -> gossipsub::IdentTopic {
    gossipsub::IdentTopic::new(format!("/{network}/header-sub/v0.0.1"))
}

// Go RoutingDiscovery uses the SHA2-256 multihash of the topic as the provider key.
pub(super) fn provider_key() -> kad::RecordKey {
    let mut key = vec![0x12, 32];
    key.extend_from_slice(&hash::sha256(b"/full/v0.1.0"));
    kad::RecordKey::new(&key)
}

impl Behaviour {
    pub(super) fn new(key: &identity::Keypair, network: &str, mode: kad::Mode) -> Result<Self> {
        let id = key.public().to_peer_id();
        let protocol = StreamProtocol::try_from_owned(format!("/celestia/{network}/kad/1.0.0"))?;
        let mut dht = kad::Behaviour::with_config(
            id,
            kad::store::MemoryStore::new(id),
            kad::Config::new(protocol),
        );
        dht.set_mode(Some(mode));
        let streams = if mode == kad::Mode::Server {
            dht.start_providing(provider_key())?;
            Some(super::inbound::Inbound::new(network)?)
        } else {
            None
        };
        let mut cfg = gossipsub::ConfigBuilder::default();
        // This bridge publishes its own verified commits. Hold received gossip for validation.
        cfg.validate_messages().message_id_fn(|message| {
            let id = HeaderWire::decode(message.data.as_slice())
                .ok()
                .and_then(|h| h.commit?.block_id)
                .and_then(|id| ctypes::BlockId::from_proto(&id).ok())
                .map_or_else(
                    || hash::sha256(&message.data).to_vec(),
                    |id| id.to_string().into_bytes(),
                );
            gossipsub::MessageId::from(id)
        });
        let mut gossip = gossipsub::Behaviour::new(
            gossipsub::MessageAuthenticity::Signed(key.clone()),
            cfg.build()?,
        )
        .map_err(anyhow::Error::msg)?;
        gossip.subscribe(&header_topic(network))?;
        Ok(Self {
            autonat: libp2p_autonat::Behaviour::new(id, libp2p_autonat::Config::default()),
            protected: super::protected::Protected::default(),
            gate: libp2p::allow_block_list::Behaviour::default(),
            streams: streams.into(),
            negotiated: libp2p_stream::Behaviour::new(),
            identify: identify::Behaviour::new(identify::Config::new(
                "ipfs/0.1.0".to_owned(),
                key.public(),
            )),
            dht,
            gossip,
        })
    }
}

// celestia-node v0.36.0 and v0.36.0-mocha, nodebuilder/p2p/bootstrap.go.
fn release_peers(chain: &str) -> &'static [&'static str] {
    match chain {
        "celestia" => &[
            "/dnsaddr/da-bootstrapper-1.celestia-bootstrap.net/p2p/12D3KooWSqZaLcn5Guypo2mrHr297YPJnV8KMEMXNjs3qAS8msw8",
            "/dnsaddr/da-bootstrapper-2.celestia-bootstrap.net/p2p/12D3KooWQpuTFELgsUypqp9N4a1rKBccmrmQVY8Em9yhqppTJcXf",
            "/dnsaddr/boot.celestia.pops.one/p2p/12D3KooWBBzzGy5hAHUQVh2vBvL25CKwJ7wbmooPcz4amQhzJHJq",
            "/dnsaddr/celestia.qubelabs.io/p2p/12D3KooWAzucnC7yawvLbmVxv53ihhjbHFSVZCsPuuSzTg6A7wgx",
            "/dnsaddr/celestia-mainnet-boot.01node.com/p2p/12D3KooWJ77n9mC9xpKHeoEc49UkP4M5EixcPoKf5Xb1tsHo99P7",
        ],
        "mocha-5" => &[
            "/dns4/da-bootstrapper-1.celestia-mocha.com/tcp/2121/p2p/12D3KooWPHgLgPTtPjuKtZjXbZRiUSVMvFpqek8T1we93auG3uoK",
            "/dns4/da-bootstrapper-2.celestia-mocha.com/tcp/2121/p2p/12D3KooWRmUch5JsfLgkDrhEGu8rSCeTP2nJvGPygkqRUWfqiAJ4",
            "/dnsaddr/mocha-boot.pops.one/p2p/12D3KooWAj2P9akCYyzKUwmicvi1g6jxVE9Ld3tdmLqTque5NuZP",
            "/dnsaddr/celestia-mocha.qubelabs.io/p2p/12D3KooWQVmHy7JpfxpKZfLjvn12GjvMgKrWdsHkFbV2kKqQFBCG",
            "/dns4/celestia-mocha-boot.01node.com/tcp/2121/p2p/12D3KooWMCHtyp3Ld7tGJQavrbwmzbbevBxNqLxY28dYjHAwiwGV",
        ],
        _ => &[],
    }
}

pub(super) struct Peering {
    pub(super) bootstrap: std::collections::BTreeMap<libp2p::PeerId, Vec<libp2p::Multiaddr>>,
    pub(super) announce: Vec<libp2p::Multiaddr>,
}

impl Peering {
    pub(super) fn new(
        chain: &str,
        override_peers: &[libp2p::Multiaddr],
        announce: Vec<libp2p::Multiaddr>,
    ) -> Result<Self> {
        let seeds = if override_peers.is_empty() {
            release_peers(chain)
                .iter()
                .map(|s| s.parse())
                .collect::<Result<Vec<_>, _>>()?
        } else {
            override_peers.to_vec()
        };
        let mut bootstrap = std::collections::BTreeMap::<_, Vec<_>>::new();
        for mut addr in seeds {
            let Some(libp2p::multiaddr::Protocol::P2p(id)) = addr.pop() else {
                anyhow::bail!("bootstrap address must end in /p2p/PEER_ID");
            };
            anyhow::ensure!(!addr.is_empty(), "bootstrap address needs a transport");
            bootstrap.entry(id).or_default().push(addr);
        }
        for addr in &announce {
            use libp2p::multiaddr::Protocol;
            let mut endpoint = addr.clone();
            let valid = match endpoint.pop() {
                Some(Protocol::Tcp(port)) => port != 0,
                Some(Protocol::Ws(_) | Protocol::Wss(_)) => {
                    let suffix = endpoint.pop();
                    let port = if suffix == Some(Protocol::Tls) {
                        endpoint.pop()
                    } else {
                        suffix
                    };
                    matches!(port, Some(Protocol::Tcp(port)) if port != 0)
                }
                Some(Protocol::QuicV1) => {
                    matches!(endpoint.pop(), Some(Protocol::Udp(port)) if port != 0)
                }
                _ => false,
            };
            anyhow::ensure!(
                valid,
                "announced DA address needs a TCP, WebSocket or QUIC v1 endpoint"
            );
        }
        Ok(Self {
            bootstrap,
            announce,
        })
    }

    pub(super) fn connect(&self, swarm: &mut libp2p::Swarm<Behaviour>) {
        for (peer, addrs) in &self.bootstrap {
            if !swarm.is_connected(peer) {
                for addr in addrs {
                    swarm.behaviour_mut().dht.add_address(peer, addr.clone());
                }
                let opts = libp2p::swarm::dial_opts::DialOpts::peer_id(*peer)
                    .addresses(addrs.clone())
                    .build();
                if let Err(error) = swarm.dial(opts) {
                    tracing::debug!(%peer, %error, "DA bootstrap dial deferred");
                }
            }
        }
    }
}
