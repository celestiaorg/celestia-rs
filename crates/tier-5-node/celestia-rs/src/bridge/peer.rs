//! Inbound go-header exchange and SHREX over negotiated libp2p streams.
mod inbound;
mod network;

use std::io::{ErrorKind, Write as _};
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use futures::{AsyncReadExt, AsyncWriteExt, StreamExt as _};
use libp2p::{
    Multiaddr, Stream, SwarmBuilder, identity, multiaddr::Protocol, noise, swarm::SwarmEvent, tcp,
    yamux,
};
use nodestore::{BlockStore, StateStore};
use prost::Message;
use tokio::sync::{Semaphore, oneshot};

use super::{Bridge, Committed, Server};

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

pub(crate) fn start(
    addr: SocketAddr,
    home: &Path,
    network: &str,
    blocks: BlockStore,
    states: StateStore,
) -> Result<Server> {
    ensure!(
        addr.ip().is_loopback(),
        "experimental bridge peer requires a loopback address"
    );
    let key = identity(&home.join("config/da_peer.key"))?;
    let listen = Multiaddr::from(match addr.ip() {
        IpAddr::V4(ip) => Protocol::Ip4(ip),
        IpAddr::V6(ip) => Protocol::Ip6(ip),
    })
    .with(Protocol::Tcp(addr.port()));
    let net = network.to_owned();
    let (stop, stopped) = oneshot::channel();
    let (ready, started) = mpsc::sync_channel(1);
    let bridge = Arc::new(Bridge {
        app: None,
        wallet: None,
        waiters: Arc::new(Semaphore::new(32)),
        blocks,
        states,
        worker: Arc::new(Semaphore::new(2)),
    });
    let thread = std::thread::Builder::new()
        .name("da-peer".into())
        .spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(anyhow::Error::from)
                .and_then(|rt| rt.block_on(run(key, listen, &net, bridge, stopped, &ready)));
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
    Ok(server)
}

async fn run(
    key: identity::Keypair,
    listen: Multiaddr,
    network: &str,
    bridge: Arc<Bridge>,
    mut stopped: oneshot::Receiver<()>,
    ready: &mpsc::SyncSender<Result<(), String>>,
) -> Result<()> {
    let behaviour = network::Behaviour::new(&key, network)?;
    let mut swarm = SwarmBuilder::with_existing_identity(key)
        .with_tokio()
        .with_tcp(
            tcp::Config::default().nodelay(true),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_behaviour(move |_| behaviour)?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(120)))
        .build();
    swarm.listen_on(listen)?;
    let topic = network::header_topic(network);
    let topic_hash = topic.hash();
    let mut timer = tokio::time::interval(Duration::from_millis(500));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut announced = 0;
    let mut pending = tokio::task::JoinSet::new();
    // A Go DAS worker opens a separate stream for each sample in a batch.
    // Bound queued streams independently of concurrent square reconstruction.
    let admissions = Arc::new(Semaphore::new(64));
    loop {
        tokio::select! {
            _ = &mut stopped => return Ok(()),
            _ = timer.tick(), if pending.is_empty() && swarm.behaviour().gossip.mesh_peers(&topic_hash).next().is_some() => {
                let height = bridge.blocks.height();
                if height > announced {
                    let bridge = bridge.clone();
                    pending.spawn_blocking(move || (height, bridge.committed(height).and_then(|block| block.header_wire())));
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
                SwarmEvent::NewListenAddr { address, .. } => {
                    swarm.add_external_address(address.clone());
                    tracing::info!(%address, peer=%swarm.local_peer_id(), "DA peer listening");
                    let _ = ready.try_send(Ok(()));
                }
                SwarmEvent::Behaviour(network::BehaviourEvent::Identify(libp2p::identify::Event::Received { peer_id, info, .. })) => {
                    for addr in info.listen_addrs {
                        swarm.behaviour_mut().dht.add_address(&peer_id, addr);
                    }
                }
                SwarmEvent::ListenerClosed { reason, .. } => {
                    reason?;
                    bail!("DA peer listener closed");
                }
                SwarmEvent::Behaviour(network::BehaviourEvent::Streams((peer, inbound::Accepted {kind, mut stream}))) => {
                let Ok(admission) = admissions.clone().try_acquire_owned() else { continue };
                let bridge = bridge.clone();
                tokio::spawn(async move {
                    let _admission = admission;
                    let Ok(Ok(_worker)) = tokio::time::timeout(TIMEOUT, bridge.worker.acquire()).await else { return };
                    if let Err(error) = serve(&bridge, kind, &mut stream).await {
                        tracing::debug!(%peer, %error, "DA stream ended");
                    }
                });
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

#[derive(Clone, PartialEq, Message)]
struct HeaderWire {
    #[prost(message, optional, tag = "1")]
    header: Option<ctypes::proto::types::Header>,
    #[prost(message, optional, tag = "2")]
    commit: Option<ctypes::proto::types::Commit>,
    #[prost(message, optional, tag = "3")]
    validators: Option<ctypes::proto::types::ValidatorSet>,
    #[prost(bytes, tag = "4")]
    dah: Vec<u8>,
}

impl Committed {
    fn header_wire(&self) -> Result<Vec<u8>> {
        Ok(HeaderWire {
            header: Some(self.header.to_proto()),
            commit: Some(self.commit.to_proto()),
            validators: Some(self.validators.to_proto()?),
            dah: self.dah.to_proto_bytes()?,
        }
        .encode_to_vec())
    }
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

async fn serve(bridge: &Arc<Bridge>, kind: Kind, stream: &mut Stream) -> Result<()> {
    match kind {
        Kind::Header => {
            let data = tokio::time::timeout(TIMEOUT, read_frame(stream)).await??;
            let request = Request::decode(data.as_slice())?;
            let bridge = bridge.clone();
            let responses =
                tokio::task::spawn_blocking(move || bridge.header_responses(request)).await??;
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
    Ok(())
}

enum Payload {
    Range(Arc<Committed>, usize, usize),
    Frame(Vec<u8>),
    Rows(Vec<app::inclusion_proof::NamespaceRow>),
    Square(square::ExtendedDataSquare),
}

impl Bridge {
    fn shrex_response(&self, kind: Kind, height: i64, id: &[u8]) -> Result<Option<Payload>> {
        let head = self
            .states
            .load()?
            .context("missing state")?
            .last_block_height;
        if height > head || height < self.blocks.base() {
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
                Payload::Frame(HalfRow { shares: half }.encode_length_delimited_to_vec())
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
                        proof: Some(proof.into()),
                    }
                    .encode_length_delimited_to_vec(),
                )
            }
            _ => Payload::Square(block.eds),
        }))
    }

    fn header_responses(&self, request: Request) -> Result<Vec<HeaderResponse>> {
        let head = self
            .states
            .load()?
            .context("missing state")?
            .last_block_height;
        let (from, count) = match request.data.context("missing origin")? {
            Origin::Hash(h) => (
                self.blocks
                    .load_block_meta_by_hash(&h)?
                    .map_or(0, |m| m.header.height),
                1,
            ),
            Origin::Height(0) if request.amount > 0 => (head, 1),
            Origin::Height(h) => {
                ensure!((1..=64).contains(&request.amount), "invalid range size");
                (i64::try_from(h)?, i64::try_from(request.amount)?)
            }
        };
        if from <= 0 || from < self.blocks.base() || from > head {
            return Ok(vec![HeaderResponse {
                status: 2,
                body: vec![],
            }]);
        }
        (from..=head.min(from.saturating_add(count - 1)))
            .map(|height| {
                Ok(HeaderResponse {
                    status: 1,
                    body: self.committed(height)?.header_wire()?,
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

// SHWAP Row with half_side LEFT (the protobuf default).
#[derive(Clone, PartialEq, Message)]
struct HalfRow {
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
