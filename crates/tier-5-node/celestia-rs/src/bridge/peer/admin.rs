//! Administrative reads are evaluated by the task that owns the running DA swarm.
use anyhow::{Context, Result, ensure};
use libp2p::swarm::{
    ConnectionId,
    dial_opts::{DialOpts, PeerCondition},
};
use libp2p::{Multiaddr, PeerId, Swarm, multiaddr::Protocol};
use serde_json::{Value, json};
use std::{collections::HashMap, time::Duration};
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
                | "p2p.Connect"
                | "p2p.ClosePeer"
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
        tokio::time::timeout(Duration::from_secs(30), result)
            .await
            .context("peer operation timed out")??
    }
}

type Reply = oneshot::Sender<Result<Value>>;

#[derive(Default)]
pub(super) struct Operations {
    dialing: HashMap<ConnectionId, Reply>,
    closing: HashMap<PeerId, Vec<Reply>>,
}

impl Operations {
    pub(super) fn dial_result(&mut self, id: ConnectionId, result: Result<()>) {
        if let Some(reply) = self.dialing.remove(&id) {
            let _ = reply.send(result.map(|()| Value::Null));
        }
    }

    pub(super) fn closed(&mut self, peer: PeerId) {
        for reply in self.closing.remove(&peer).unwrap_or_default() {
            let _ = reply.send(Ok(Value::Null));
        }
    }

    pub(super) fn handle(&mut self, request: Request, swarm: &mut Swarm<Behaviour>, network: &str) {
        let Request {
            method,
            args,
            reply,
        } = request;
        if reply.is_closed() {
            return;
        }
        if !matches!(method.as_str(), "p2p.Connect" | "p2p.ClosePeer") {
            let _ = reply.send(read(swarm, network, &method, &args));
            return;
        }
        self.dialing.retain(|_, r| !r.is_closed());
        self.closing.retain(|_, waiting| {
            waiting.retain(|r| !r.is_closed());
            !waiting.is_empty()
        });
        if self.dialing.len() + self.closing.values().map(Vec::len).sum::<usize>() >= 32 {
            let _ = reply.send(Err(anyhow::anyhow!("too many pending peer operations")));
            return;
        }
        match (method.as_str(), args.as_slice()) {
            ("p2p.Connect", [addr]) => match dial(swarm, addr) {
                Ok(Some(id)) => {
                    self.dialing.insert(id, reply);
                }
                result => {
                    let _ = reply.send(result.map(|_| Value::Null));
                }
            },
            ("p2p.ClosePeer", [id]) => match parse_peer(id) {
                Ok(id) if swarm.disconnect_peer_id(id).is_ok() => {
                    self.closing.entry(id).or_default().push(reply);
                }
                result => {
                    let _ = reply.send(result.map(|_| Value::Null));
                }
            },
            _ => {
                let _ = reply.send(Err(anyhow::anyhow!("invalid parameters")));
            }
        }
    }
}

fn parse_peer(value: &Value) -> Result<PeerId> {
    Ok(value.as_str().context("expected a peer ID")?.parse()?)
}

fn dial(swarm: &mut Swarm<Behaviour>, addr: &Value) -> Result<Option<ConnectionId>> {
    let id = parse_peer(&addr["ID"])?;
    if swarm.is_connected(&id) {
        return Ok(None);
    }
    let mut addresses = Vec::new();
    if !addr["Addrs"].is_null() {
        for value in addr["Addrs"]
            .as_array()
            .context("expected peer addresses")?
        {
            addresses.push(value.as_str().context("invalid peer address")?.parse()?);
        }
    }
    let options = DialOpts::peer_id(id)
        .condition(PeerCondition::Always)
        .addresses(addresses)
        .extend_addresses_through_behaviour()
        .build();
    let connection = options.connection_id();
    swarm.dial(options)?;
    Ok(Some(connection))
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
        ("p2p.Connectedness", [id]) => Ok(json!(u8::from(swarm.is_connected(&parse_peer(id)?)))),
        ("p2p.PeerInfo", [id]) => {
            let id = parse_peer(id)?;
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
