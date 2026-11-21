//! Inbound go-header exchange and SHREX over negotiated libp2p streams.
use std::io::{ErrorKind, Write as _};
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use futures::{AsyncReadExt, AsyncWriteExt, StreamExt as _};
use libp2p::{
    Multiaddr, Stream, StreamProtocol, SwarmBuilder, identity, multiaddr::Protocol, noise,
    swarm::SwarmEvent, tcp, yamux,
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
        blocks,
        states,
        worker: Arc::new(Semaphore::new(1)),
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
    let mut swarm = SwarmBuilder::with_existing_identity(key)
        .with_tokio()
        .with_tcp(
            tcp::Config::default().nodelay(true),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_behaviour(|_| libp2p_stream::Behaviour::new())?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(120)))
        .build();
    let mut control = swarm.behaviour().new_control();
    let mut streams = futures::stream::SelectAll::new();
    for (kind, protocol) in [
        (Kind::Header, "header-ex/v0.0.3"),
        (Kind::Namespace, "shrex/v0.1.0/nd_v0"),
        (Kind::Eds, "shrex/v0.1.0/eds_v0"),
    ] {
        let id = StreamProtocol::try_from_owned(format!("/{network}/{protocol}"))?;
        streams.push(
            control
                .accept(id)?
                .map(move |(peer, stream)| (kind, peer, stream)),
        );
    }
    swarm.listen_on(listen)?;
    loop {
        tokio::select! {
            _ = &mut stopped => return Ok(()),
            event = swarm.select_next_some() => match event {
                SwarmEvent::NewListenAddr { address, .. } => {
                    tracing::info!(%address, peer=%swarm.local_peer_id(), "DA peer listening");
                    let _ = ready.try_send(Ok(()));
                }
                SwarmEvent::ListenerClosed { reason, .. } => {
                    reason?;
                    bail!("DA peer listener closed");
                }
                _ => {}
            },
            (kind, peer, mut stream) = streams.select_next_some() => {
                let Ok(permit) = bridge.worker.clone().try_acquire_owned() else { continue };
                let bridge = bridge.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Err(error) = serve(&bridge, kind, &mut stream).await {
                        tracing::debug!(%peer, %error, "DA stream ended");
                    }
                });
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Header,
    Namespace,
    Eds,
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
        Kind::Eds | Kind::Namespace => {
            let mut id = vec![
                0;
                if matches!(kind, Kind::Namespace) {
                    8 + shares::NAMESPACE_SIZE
                } else {
                    8
                }
            ];
            tokio::time::timeout(TIMEOUT, stream.read_exact(&mut id)).await??;
            let height = i64::try_from(u64::from_be_bytes(id[..8].try_into()?))?;
            ensure!(height > 0, "invalid block height");
            let bridge = bridge.clone();
            let data = tokio::task::spawn_blocking(move || -> Result<_> {
                let head = bridge
                    .states
                    .load()?
                    .context("missing state")?
                    .last_block_height;
                if height > head || height < bridge.blocks.base() {
                    return Ok(None);
                }
                let block = bridge.committed(height)?;
                Ok(Some(match kind {
                    Kind::Namespace => {
                        let ns = shares::Namespace::from_slice(&id[8..])?;
                        let rows = app::inclusion_proof::namespace_rows(&block.eds, &block.dah, ns)
                            .map_err(|e| anyhow::anyhow!("{}", e.log))?;
                        Payload::Rows(rows)
                    }
                    _ => Payload::Square(block.eds),
                }))
            })
            .await?;
            let status = match &data {
                Ok(Some(_)) => 1,
                Ok(None) => 2,
                Err(_) => 3,
            };
            write(stream, &[2, 8, status]).await?;
            match data? {
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
    Rows(Vec<app::inclusion_proof::NamespaceRow>),
    Square(square::ExtendedDataSquare),
}

impl Bridge {
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
        proof: Some(Proof {
            start: i64::from(p.start),
            end: i64::from(p.end),
            nodes: p.nodes,
            leaf_hash: p.leaf_hash,
            ignore_max: true,
        }),
    }
    .encode_length_delimited_to_vec()
}
