//! Administrative reads are evaluated by the task that owns the running DA swarm.
use anyhow::{Context, Result, ensure};
use libp2p::{Multiaddr, PeerId, Swarm, multiaddr::Protocol};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use super::network::Behaviour;

#[derive(Clone)]
pub(crate) struct Control(mpsc::Sender<Request>);

pub(super) struct Request {
    method: String,
    args: Vec<Value>,
    reply: oneshot::Sender<Result<Value>>,
}

impl Control {
    pub(super) fn channel() -> (Self, mpsc::Receiver<Request>) {
        let (sender, requests) = mpsc::channel(32);
        (Self(sender), requests)
    }
    pub(crate) fn handles(method: &str) -> bool {
        matches!(
            method,
            "p2p.Info"
                | "p2p.Network"
                | "p2p.Peers"
                | "p2p.PeerInfo"
                | "p2p.Connectedness"
                | "p2p.PubSubTopics"
                | "p2p.PubSubPeers"
        )
    }
    pub(crate) async fn call(&self, method: &str, args: &[Value]) -> Result<Value> {
        let (reply, result) = oneshot::channel();
        self.0
            .try_send(Request {
                method: method.into(),
                args: args.to_owned(),
                reply,
            })
            .map_err(|_| anyhow::anyhow!("DA swarm unavailable or busy"))?;
        result.await?
    }
}

impl Request {
    pub(super) fn respond(self, swarm: &mut Swarm<Behaviour>, network: &str) {
        if !self.reply.is_closed() {
            let result = read(swarm, network, &self.method, &self.args);
            let _ = self.reply.send(result);
        }
    }
}

fn info(id: PeerId, addresses: impl Iterator<Item = Multiaddr>) -> Value {
    let addresses = addresses
        .map(|mut a| {
            if a.iter().last() == Some(Protocol::P2p(id)) {
                a.pop();
            }
            a.to_string()
        })
        .collect::<Vec<_>>();
    json!({"ID":id.to_string(), "Addrs":addresses})
}

fn read(
    swarm: &mut Swarm<Behaviour>,
    network: &str,
    method: &str,
    args: &[Value],
) -> Result<Value> {
    let peer_id =
        |id: &Value| -> Result<PeerId> { Ok(id.as_str().context("expected a peer ID")?.parse()?) };
    match (method, args) {
        ("p2p.Network", []) => Ok(json!(network)),
        ("p2p.Info", []) => Ok(info(
            *swarm.local_peer_id(),
            swarm.external_addresses().cloned(),
        )),
        ("p2p.Peers", []) => Ok(json!(
            swarm
                .connected_peers()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        )),
        ("p2p.Connectedness", [id]) => Ok(json!(u8::from(swarm.is_connected(&peer_id(id)?)))),
        ("p2p.PeerInfo", [id]) => {
            let id = peer_id(id)?;
            if &id == swarm.local_peer_id() {
                return Ok(info(id, swarm.external_addresses().cloned()));
            }
            let mut addresses = Vec::new();
            if let Some(bucket) = swarm.behaviour_mut().dht.kbucket(id)
                && let Some(entry) = bucket.iter().find(|e| e.node.key.preimage() == &id)
            {
                addresses.extend(entry.node.value.iter().cloned());
            }
            Ok(info(id, addresses.into_iter()))
        }
        ("p2p.PubSubTopics", []) => Ok(json!(
            swarm
                .behaviour()
                .gossip
                .topics()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        )),
        ("p2p.PubSubPeers", [topic]) => {
            let topic = topic.as_str().context("expected a topic")?;
            let peers = swarm
                .behaviour()
                .gossip
                .all_peers()
                .filter(|(_, topics)| topics.iter().any(|t| t.as_str() == topic))
                .map(|(id, _)| id.to_string())
                .collect::<Vec<_>>();
            Ok(json!(peers))
        }
        _ => {
            ensure!(Control::handles(method), "unknown peer method");
            anyhow::bail!("invalid parameters")
        }
    }
}
