//! Inbound go-header exchange and SHREX over negotiated libp2p streams.
mod admin;
mod bandwidth;
mod connections;
pub(crate) use admin::Control;
mod flood;
mod inbound;
#[cfg(feature = "da-light")]
pub(crate) mod light;
mod network;
mod ping;
mod protected;

use std::io::{ErrorKind, Write as _};
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use futures::{AsyncReadExt, AsyncWriteExt, StreamExt as _};
use libp2p::Transport as _;
use libp2p::{
    Multiaddr, Stream, SwarmBuilder, identity, multiaddr::Protocol, noise, swarm::SwarmEvent, tcp,
    yamux,
};
use prost::Message;
use tokio::sync::{Semaphore, oneshot};

use super::{Bridge, Committed, HeaderWire, Server, index};

fn identity(path: &Path) -> Result<identity::Keypair> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(identity::Keypair::from_protobuf_encoding(&bytes)?),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let key = identity::Keypair::generate_ed25519();
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                opts.mode(0o600);
            }
            let mut f = opts.open(path)?;
            f.write_all(&key.to_protobuf_encoding()?)?;
            f.sync_all()?;
            Ok(key)
        }
        Err(e) => Err(e.into()),
    }
}

pub(crate) struct Serving {
    bridge: Arc<Bridge>,
    metrics: Option<Arc<crate::metrics::Metrics>>,
}

impl Serving {
    pub(crate) fn new(
        index: Arc<index::Store>,
        metrics: Option<Arc<crate::metrics::Metrics>>,
    ) -> Self {
        Self {
            bridge: Arc::new(Bridge {
                #[cfg(feature = "da-fibre")]
                fibre: None,
                app: None,
                peer_control: None,
                wallet: None,
                waiters: Arc::new(Semaphore::new(32)),
                index,
                worker: Arc::new(Semaphore::new(2)),
            }),
            metrics,
        }
    }

    fn accept(
        self: &Arc<Self>,
        admissions: &Arc<Semaphore>,
        peer: libp2p::PeerId,
        inbound::Accepted { kind, mut stream }: inbound::Accepted,
    ) {
        let Ok(admission) = admissions.clone().try_acquire_owned() else {
            self.record(kind, "busy");
            return;
        };
        let serving = self.clone();
        tokio::spawn(async move {
            let _admission = admission;
            let Ok(Ok(_worker)) =
                tokio::time::timeout(TIMEOUT, serving.bridge.worker.acquire()).await
            else {
                serving.record(kind, "busy");
                return;
            };
            let outcome = match serve(&serving.bridge, kind, &mut stream).await {
                Ok(true) => "served",
                Ok(false) => "not_found",
                Err(error) => {
                    tracing::debug!(%peer, %error, "DA stream ended");
                    "error"
                }
            };
            serving.record(kind, outcome);
        });
    }

    fn record(&self, kind: Kind, outcome: &str) {
        if let Some(metrics) = &self.metrics {
            let protocol = match kind {
                Kind::Header => "header",
                Kind::Namespace => "namespace",
                Kind::Eds => "eds",
                Kind::Row => "row",
                Kind::Sample => "sample",
                Kind::Range => "range",
            };
            metrics.add(
                "da_stream_requests_total",
                &format!("protocol=\"{protocol}\",outcome=\"{outcome}\""),
                1.0,
            );
        }
    }
}

pub(crate) fn start(
    addr: SocketAddr,
    home: &Path,
    network: &str,
    serving: Serving,
    bootstrap: &[Multiaddr],
    announce: Vec<Multiaddr>,
) -> Result<(Server, Control)> {
    let peers = network::Peering::new(network, bootstrap, announce)?;
    let key = identity(&home.join("config/da_peer.key"))?;
    let listen = Multiaddr::from(match addr.ip() {
        IpAddr::V4(ip) => Protocol::Ip4(ip),
        IpAddr::V6(ip) => Protocol::Ip6(ip),
    })
    .with(Protocol::Tcp(addr.port()));
    let net = network.to_owned();
    let (stop, stopped) = oneshot::channel();
    let (ready, started) = mpsc::sync_channel(1);
    let (control, requests) = admin::Control::channel();
    let blocklist = admin::Blocklist::open(home)?;
    let thread = std::thread::Builder::new()
        .name("da-peer".into())
        .spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(anyhow::Error::from)
                .and_then(|rt| {
                    rt.block_on(run(
                        key,
                        listen,
                        &net,
                        serving,
                        stopped,
                        &ready,
                        (peers, requests, blocklist),
                    ))
                });
            if let Err(e) = result {
                let _ = ready.try_send(Err(e.to_string()));
                tracing::error!(error=%e,"DA peer stopped");
            }
        })?;
    let server = Server {
        stop: Some(stop),
        thread: Some(thread),
    };
    started
        .recv_timeout(Duration::from_secs(10))?
        .map_err(anyhow::Error::msg)?;
    Ok((server, control))
}

fn observed_swarm(
    key: identity::Keypair,
    behaviour: network::Behaviour,
    connections: &connections::Connections,
) -> Result<libp2p::Swarm<network::Behaviour>> {
    let observed = connections.clone();
    let security = noise::Config::new(&key)?;
    let swarm = SwarmBuilder::with_existing_identity(key)
        .with_tokio()
        .with_other_transport(move |_| {
            tcp::tokio::Transport::new(tcp::Config::default().nodelay(true))
                .upgrade(libp2p::core::upgrade::Version::V1Lazy)
                .authenticate(security)
                .multiplex(yamux::Config::default())
                .map(move |(peer, mux), point| (peer, observed.observe(peer, &point, mux)))
        })?
        .with_dns()?
        .with_behaviour(move |_| behaviour)?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(120)))
        .build();
    Ok(swarm)
}

async fn run(
    key: identity::Keypair,
    listen: Multiaddr,
    network: &str,
    serving: Serving,
    mut stopped: oneshot::Receiver<()>,
    ready: &mpsc::SyncSender<Result<(), String>>,
    (peers, mut requests, blocklist): (
        network::Peering,
        tokio::sync::mpsc::Receiver<admin::Request>,
        admin::Blocklist,
    ),
) -> Result<()> {
    let mut behaviour = network::Behaviour::new(&key, network, libp2p::kad::Mode::Server)?;
    blocklist.restore(&mut behaviour)?;
    for peer in peers.bootstrap.keys() {
        behaviour.protected.protect(*peer, "protected-bootstrap");
    }
    let serving = Arc::new(serving);
    let bridge = &serving.bridge;
    let ping = ping::Ping::start(behaviour.negotiated.new_control())?;
    flood::start(
        behaviour.negotiated.new_control(),
        key.clone(),
        network,
        bridge.clone(),
    )?;
    let connections = connections::Connections::default();
    let mut swarm = observed_swarm(key, behaviour, &connections)?;
    swarm.listen_on(listen)?;
    for address in &peers.announce {
        swarm.add_external_address(address.clone());
    }
    let mut reconnect = tokio::time::interval(Duration::from_secs(30));
    reconnect.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let topic = network::header_topic(network);
    let topic_hash = topic.hash();
    let mut timer = tokio::time::interval(Duration::from_millis(500));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut operations = admin::Operations::new(ping, connections);
    let mut announced = 0;
    let mut pending = tokio::task::JoinSet::new();
    // A Go DAS worker opens a separate stream for each sample in a batch.
    // Bound queued streams independently of concurrent square reconstruction.
    let admissions = Arc::new(Semaphore::new(64));
    loop {
        tokio::select! {
            Some(request) = requests.recv() => operations.handle(request, &mut swarm, network, &blocklist),
            _ = &mut stopped => return Ok(()),
            _ = reconnect.tick(), if !peers.bootstrap.is_empty() => peers.connect(&mut swarm),
            _ = timer.tick(), if pending.is_empty() && swarm.behaviour().gossip.mesh_peers(&topic_hash).next().is_some() => {
                let height = bridge.index.head()?;
                if height > announced {
                    let bridge = bridge.clone();
                    pending.spawn_blocking(move || (height, bridge.index.header_wire(height)));
                }
            }
            Some(result) = pending.join_next(), if !pending.is_empty() => {
                match result? {
                    (height, Ok(data)) => {
                        if swarm.behaviour_mut().gossip.publish(topic.clone(), data).is_ok() {
                            announced = height;
                        }
                    }
                    (height, Err(error)) => tracing::debug!(height, %error, "header publication deferred"),
                }
            }
            event = swarm.select_next_some() => match event {
                SwarmEvent::ConnectionEstablished {connection_id, ..} => operations.dial_result(connection_id, Ok(())),
                SwarmEvent::OutgoingConnectionError {connection_id, error, ..} => operations.dial_result(connection_id, Err(error.into())),
                SwarmEvent::ConnectionClosed {peer_id, num_established: 0, ..} => operations.closed(peer_id),
                SwarmEvent::NewListenAddr { address, .. } => {
                    if peers.announce.is_empty() { swarm.add_external_address(address.clone()); }
                    tracing::info!(%address, peer=%swarm.local_peer_id(), "DA peer listening");
                    let _ = ready.try_send(Ok(()));
                }
                SwarmEvent::Behaviour(network::BehaviourEvent::Autonat(event)) => tracing::debug!(?event, "DA AutoNAT probe"),
                SwarmEvent::Behaviour(network::BehaviourEvent::Identify(libp2p::identify::Event::Received { peer_id, info, .. })) => {
                    for addr in info.listen_addrs {
                        swarm.behaviour_mut().dht.add_address(&peer_id, addr);
                    }
                }
                SwarmEvent::ListenerClosed { reason, .. } => {
                    reason?;
                    bail!("DA peer listener closed");
                }
                SwarmEvent::Behaviour(network::BehaviourEvent::Streams((peer, request))) => {
                    serving.accept(&admissions, peer, request);
                }
                _ => {}
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    Header,
    Namespace,
    Eds,
    Row,
    Sample,
    Range,
}

#[derive(Clone, PartialEq, Message)]
struct Request {
    #[prost(oneof = "Origin", tags = "1,2")]
    data: Option<Origin>,
    #[prost(uint64, tag = "3")]
    amount: u64,
}
#[derive(Clone, PartialEq, prost::Oneof)]
enum Origin {
    #[prost(uint64, tag = "1")]
    Height(u64),
    #[prost(bytes, tag = "2")]
    Hash(Vec<u8>),
}

#[derive(Clone, PartialEq, Message)]
struct HeaderResponse {
    #[prost(bytes, tag = "1")]
    body: Vec<u8>,
    #[prost(int32, tag = "2")]
    status: i32,
}

const TIMEOUT: Duration = Duration::from_secs(15);

async fn read_frame(stream: &mut Stream) -> Result<Vec<u8>> {
    // Requests contain only a height/hash and amount. Limit the frame before allocating.
    let mut len = 0usize;
    for shift in [0, 7] {
        let mut b = [0];
        stream.read_exact(&mut b).await?;
        len |= usize::from(b[0] & 0x7f) << shift;
        if b[0] < 128 {
            ensure!(len <= 1024, "header request too large");
            let mut data = vec![0; len];
            stream.read_exact(&mut data).await?;
            return Ok(data);
        }
    }
    bail!("invalid request length")
}

async fn write(stream: &mut Stream, bytes: &[u8]) -> Result<()> {
    tokio::time::timeout(TIMEOUT, stream.write_all(bytes)).await??;
    Ok(())
}

async fn serve(bridge: &Arc<Bridge>, kind: Kind, stream: &mut Stream) -> Result<bool> {
    let found;
    match kind {
        Kind::Header => {
            let data = tokio::time::timeout(TIMEOUT, read_frame(stream)).await??;
            let request = Request::decode(data.as_slice())?;
            let bridge = bridge.clone();
            let responses =
                tokio::task::spawn_blocking(move || bridge.header_responses(request)).await??;
            found = responses.iter().any(|r| r.status == 1);
            for response in responses {
                write(stream, &response.encode_length_delimited_to_vec()).await?;
            }
        }
        Kind::Eds | Kind::Namespace | Kind::Row | Kind::Sample | Kind::Range => {
            let extra = match kind {
                Kind::Namespace => shares::NAMESPACE_SIZE,
                Kind::Range => 8,
                Kind::Row => 2,
                Kind::Sample => 4,
                _ => 0,
            };
            let mut id = vec![0; 8 + extra];
            tokio::time::timeout(TIMEOUT, stream.read_exact(&mut id)).await??;
            let height = i64::try_from(u64::from_be_bytes(id[..8].try_into()?))?;
            ensure!(height > 0, "invalid block height");
            let bridge = bridge.clone();
            let data =
                tokio::task::spawn_blocking(move || bridge.shrex_response(kind, height, &id))
                    .await?;
            let status = match &data {
                Ok(Some(_)) => 1,
                Ok(None) => 2,
                Err(_) => 3,
            };
            write(stream, &[2, 8, status]).await?;
            found = status == 1;
            match data? {
                Some(Payload::Range(block, from, end)) => {
                    let width = block.eds.original_width();
                    for row in from / width..=(end - 1) / width {
                        let start = from.saturating_sub(row * width);
                        let stop = (end - row * width).min(width);
                        let block = block.clone();
                        let frame = tokio::task::spawn_blocking(move || -> Result<_> {
                            let shares = (start..stop)
                                .map(|col| {
                                    Ok(WireShare {
                                        data: block.eds.share(row, col)?.to_vec(),
                                    })
                                })
                                .collect::<Result<Vec<_>>>()?;
                            let proof = if start == 0 && stop == width {
                                None
                            } else {
                                Some(
                                    app::inclusion_proof::row_range_proof(
                                        &block.eds, &block.dah, row, start, stop,
                                    )
                                    .map_err(|e| anyhow::anyhow!("{}", e.log))?
                                    .into(),
                                )
                            };
                            Ok(Row { shares, proof }.encode_length_delimited_to_vec())
                        })
                        .await??;
                        write(stream, &frame).await?;
                    }
                }
                Some(Payload::Frame(frame)) => write(stream, &frame).await?,
                Some(Payload::Rows(rows)) => {
                    for row in rows {
                        write(stream, &row_wire(row)).await?;
                    }
                }
                Some(Payload::Square(eds)) => {
                    for row in 0..eds.original_width() {
                        for col in 0..eds.original_width() {
                            write(stream, eds.share(row, col)?).await?;
                        }
                    }
                }
                None => {}
            }
        }
    }
    tokio::time::timeout(TIMEOUT, stream.close()).await??;
    Ok(found)
}

enum Payload {
    Range(Arc<Committed>, usize, usize),
    Frame(Vec<u8>),
    Rows(Vec<app::inclusion_proof::NamespaceRow>),
    Square(square::ExtendedDataSquare),
}

impl Bridge {
    fn shrex_response(&self, kind: Kind, height: i64, id: &[u8]) -> Result<Option<Payload>> {
        let head = self.index.head()?;
        if height > head || height < self.index.tail()? {
            return Ok(None);
        }
        let block = self.committed(height)?;
        Ok(Some(match kind {
            Kind::Namespace => {
                let ns = shares::Namespace::from_slice(&id[8..])?;
                let rows = app::inclusion_proof::namespace_rows(&block.eds, &block.dah, ns)
                    .map_err(|e| anyhow::anyhow!("{}", e.log))?;
                Payload::Rows(rows)
            }
            Kind::Range => {
                let from = usize::try_from(u32::from_be_bytes(id[8..12].try_into()?))?;
                let end = usize::try_from(u32::from_be_bytes(id[12..16].try_into()?))?;
                let width = block.eds.original_width();
                ensure!(from < end && end <= width * width, "invalid ODS range");
                let first = block.eds.share(from / width, from % width)?;
                let last = block.eds.share((end - 1) / width, (end - 1) % width)?;
                // Q0 namespaces are sorted in row-major order.
                ensure!(
                    first[..shares::NAMESPACE_SIZE] == last[..shares::NAMESPACE_SIZE],
                    "range spans namespaces"
                );
                Payload::Range(Arc::new(block), from, end)
            }
            Kind::Row => {
                let row = usize::from(u16::from_be_bytes(id[8..10].try_into()?));
                let half = block.eds.row(row)?[..block.eds.original_width()]
                    .iter()
                    .map(|data| WireShare { data: data.clone() })
                    .collect();
                Payload::Frame(
                    HalfRow {
                        shares: half,
                        side: 0,
                    }
                    .encode_length_delimited_to_vec(),
                )
            }
            Kind::Sample => {
                let row = usize::from(u16::from_be_bytes(id[8..10].try_into()?));
                let col = usize::from(u16::from_be_bytes(id[10..12].try_into()?));
                let proof = app::inclusion_proof::row_range_proof(
                    &block.eds,
                    &block.dah,
                    row,
                    col,
                    col + 1,
                )
                .map_err(|e| anyhow::anyhow!("{}", e.log))?;
                let share = WireShare {
                    data: block.eds.share(row, col)?.to_vec(),
                };
                Payload::Frame(
                    Sample {
                        share: Some(share),
                        axis: 0,
                        proof: Some(proof.into()),
                    }
                    .encode_length_delimited_to_vec(),
                )
            }
            _ => Payload::Square(block.eds),
        }))
    }

    fn header_responses(&self, request: Request) -> Result<Vec<HeaderResponse>> {
        let head = self.index.head()?;
        let (from, count) = match request.data.context("missing origin")? {
            Origin::Hash(h) => (self.index.by_hash(&h)?.unwrap_or(0), 1),
            Origin::Height(0) if request.amount > 0 => (head, 1),
            Origin::Height(h) => {
                ensure!((1..=64).contains(&request.amount), "invalid range size");
                (i64::try_from(h)?, i64::try_from(request.amount)?)
            }
        };
        if from <= 0 || from < self.index.tail()? || from > head {
            return Ok(vec![HeaderResponse {
                status: 2,
                body: vec![],
            }]);
        }
        (from..=head.min(from.saturating_add(count - 1)))
            .map(|height| {
                Ok(HeaderResponse {
                    status: 1,
                    body: self.index.header_wire(height)?,
                })
            })
            .collect()
    }
}

#[derive(Clone, PartialEq, Message)]
struct WireShare {
    #[prost(bytes, tag = "1")]
    data: Vec<u8>,
}
#[derive(Clone, PartialEq, Message)]
struct Proof {
    #[prost(int64, tag = "1")]
    start: i64,
    #[prost(int64, tag = "2")]
    end: i64,
    #[prost(bytes, repeated, tag = "3")]
    nodes: Vec<Vec<u8>>,
    #[prost(bytes, tag = "4")]
    leaf_hash: Vec<u8>,
    #[prost(bool, tag = "5")]
    ignore_max: bool,
}
#[derive(Clone, PartialEq, Message)]
struct Row {
    #[prost(message, repeated, tag = "1")]
    shares: Vec<WireShare>,
    #[prost(message, optional, tag = "2")]
    proof: Option<Proof>,
}
fn row_wire(row: app::inclusion_proof::NamespaceRow) -> Vec<u8> {
    let p = row.proof;
    Row {
        shares: row
            .shares
            .into_iter()
            .map(|data| WireShare { data })
            .collect(),
        proof: Some(p.into()),
    }
    .encode_length_delimited_to_vec()
}

// SHWAP Row with an original or parity half.
#[derive(Clone, PartialEq, Message)]
struct HalfRow {
    #[prost(int32, tag = "2")]
    side: i32,
    #[prost(message, repeated, tag = "1")]
    shares: Vec<WireShare>,
}

// SHWAP Sample with proof_type ROW (the protobuf default).
#[derive(Clone, PartialEq, Message)]
struct Sample {
    #[prost(message, optional, tag = "1")]
    share: Option<WireShare>,
    #[prost(message, optional, tag = "2")]
    proof: Option<Proof>,
    #[prost(int32, tag = "3")]
    axis: i32,
}

impl From<celestia_proto::celestia::core::v1::proof::NmtProof> for Proof {
    fn from(p: celestia_proto::celestia::core::v1::proof::NmtProof) -> Self {
        Self {
            start: i64::from(p.start),
            end: i64::from(p.end),
            nodes: p.nodes,
            leaf_hash: p.leaf_hash,
            ignore_max: true,
        }
    }
}
