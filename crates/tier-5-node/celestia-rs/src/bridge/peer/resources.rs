//! Live resource scopes for DA TCP connections and Yamux substreams.
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

pub(super) struct Stream {
    pub outbound: bool,
    pub protocol: OnceLock<Arc<str>>,
    // Bytes reserved by the DA protocol observer during stream negotiation.
    pub memory: AtomicUsize,
}

impl Stream {
    pub fn new(outbound: bool) -> Self {
        Self {
            outbound,
            protocol: OnceLock::new(),
            memory: AtomicUsize::default(),
        }
    }
}

#[derive(Default)]
pub(super) struct Scope {
    streams: [usize; 2],
    conns: [usize; 2],
    memory: usize,
}

impl Scope {
    pub fn connection(&mut self, outbound: bool) {
        self.conns[usize::from(outbound)] += 1;
    }
    fn stream(&mut self, stream: &Stream, memory: usize) {
        self.streams[usize::from(stream.outbound)] += 1;
        self.memory += memory;
    }
    fn json(&self) -> Value {
        json!({"NumStreamsInbound":self.streams[0],"NumStreamsOutbound":self.streams[1],
            "NumConnsInbound":self.conns[0],"NumConnsOutbound":self.conns[1],
            "NumFD":self.conns.iter().sum::<usize>(), "Memory":self.memory})
    }
}

#[derive(Default)]
pub(super) struct Snapshot {
    system: Scope,
    transient: Scope,
    peers: BTreeMap<libp2p::PeerId, Scope>,
    protocols: BTreeMap<Arc<str>, Scope>,
    services: BTreeMap<&'static str, Scope>,
}

impl Snapshot {
    pub fn connection(&mut self, peer: libp2p::PeerId, outbound: bool) {
        self.system.connection(outbound);
        self.peers.entry(peer).or_default().connection(outbound);
    }
    pub fn stream(&mut self, peer: libp2p::PeerId, stream: &Stream) {
        let memory = stream.memory.load(Ordering::Relaxed);
        self.system.stream(stream, memory);
        self.peers.entry(peer).or_default().stream(stream, memory);
        let Some(protocol) = stream.protocol.get() else {
            self.transient.stream(stream, memory);
            return;
        };
        self.protocols
            .entry(protocol.clone())
            .or_default()
            .stream(stream, memory);
        let service = match protocol.as_ref() {
            "/ipfs/ping/1.0.0" => "libp2p.ping",
            "/ipfs/id/1.0.0" | "/ipfs/id/push/1.0.0" => "libp2p.identify",
            p if !stream.outbound && p.split('/').nth(2) == Some("shrex") => "shrex",
            _ => return,
        };
        self.services
            .entry(service)
            .or_default()
            .stream(stream, memory);
    }
    pub fn json(self) -> Value {
        fn scopes<K: std::fmt::Display>(map: BTreeMap<K, Scope>) -> Value {
            Value::Object(
                map.into_iter()
                    .map(|(k, v)| (k.to_string(), v.json()))
                    .collect(),
            )
        }
        json!({"System":self.system.json(),"Transient":self.transient.json(),"Peers":scopes(self.peers),
            "Protocols":scopes(self.protocols),"Services":scopes(self.services)})
    }
}
