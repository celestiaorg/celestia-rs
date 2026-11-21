//! Network-scoped provider discovery and publication of locally committed headers.
use anyhow::Result;
use libp2p::{StreamProtocol, gossipsub, identify, identity, kad, swarm::NetworkBehaviour};
use prost::Message;

use super::HeaderWire;

#[derive(NetworkBehaviour)]
pub(super) struct Behaviour {
    pub(super) streams: libp2p_stream::Behaviour,
    pub(super) identify: identify::Behaviour,
    pub(super) dht: kad::Behaviour<kad::store::MemoryStore>,
    pub(super) gossip: gossipsub::Behaviour,
}

pub(super) fn header_topic(network: &str) -> gossipsub::IdentTopic {
    gossipsub::IdentTopic::new(format!("/{network}/header-sub/v0.0.1"))
}

impl Behaviour {
    pub(super) fn new(key: &identity::Keypair, network: &str) -> Result<Self> {
        let id = key.public().to_peer_id();
        let protocol = StreamProtocol::try_from_owned(format!("/celestia/{network}/kad/1.0.0"))?;
        let mut dht = kad::Behaviour::with_config(
            id,
            kad::store::MemoryStore::new(id),
            kad::Config::new(protocol),
        );
        dht.set_mode(Some(kad::Mode::Server));
        // Go RoutingDiscovery uses the SHA2-256 multihash of the topic as the provider key.
        let mut provider_key = vec![0x12, 32];
        provider_key.extend_from_slice(&hash::sha256(b"/full/v0.1.0"));
        dht.start_providing(kad::RecordKey::new(&provider_key))?;
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
            streams: libp2p_stream::Behaviour::new(),
            identify: identify::Behaviour::new(identify::Config::new(
                "ipfs/0.1.0".to_owned(),
                key.public(),
            )),
            dht,
            gossip,
        })
    }
}
