//! Authenticated, sequential DA sync and restartable sampling from a trusted checkpoint.
mod core;
mod data;
mod discovery;
mod exchange;
mod rpc;
mod stats;

use super::{HeaderResponse, HeaderWire, Origin, Proof, Request, Sample, identity};
use anyhow::{Context, Result, ensure};
use futures::{AsyncReadExt, AsyncWriteExt, StreamExt};
use kvdb::{KvBackend, RedbBackend, RedbOptions, WriteBatch};
use libp2p::{Multiaddr, PeerId, StreamProtocol, SwarmBuilder, noise, tcp, yamux};
use prost::Message;
use rand::seq::index;
use std::{collections::BTreeMap, io::Write as _, path::Path, time::Duration};

const TRUST_NS: i64 = 7 * 24 * 60 * 60 * 1_000_000_000;
const DRIFT_NS: i64 = 10_000_000_000;
const DEADLINE: Duration = Duration::from_secs(20);

/// Operator-supplied trust inputs for a standalone DA light node.
#[derive(clap::Args, Debug)]
pub struct LightArgs {
    /// Consensus chain ID and DA protocol namespace.
    #[arg(long, default_value = "celestia")]
    network: String,
    /// Trusted bridge multiaddress. Defaults to the selected release network bootstrappers.
    #[arg(long)]
    peer: Vec<Multiaddr>,
    /// Pin a trusted 32-byte header hash. A fresh unpinned store starts at the trusted peers’ head.
    #[arg(long)]
    trusted_hash: Option<String>,
    /// Listen for incoming DA peer connections at this TCP multiaddress.
    #[arg(long)]
    p2p_listen: Option<Multiaddr>,
    /// Consensus gRPC hostname or IP for proof-verified state reads.
    #[arg(long = "core.ip")]
    core_ip: Option<String>,
    /// Consensus gRPC port (default 9090).
    #[arg(long = "core.port", requires = "core_ip")]
    core_port: Option<u16>,
    /// Authenticate the core endpoint with TLS and the platform trust store.
    #[arg(long = "core.tls", requires = "core_ip")]
    core_tls: bool,
    /// Directory containing the Go core authentication token JSON for TLS.
    #[arg(long = "core.xtoken.path")]
    core_token_path: Option<std::path::PathBuf>,
    /// Serve authenticated sampled headers on this loopback HTTP socket.
    #[arg(long)]
    rpc_listen: Option<std::net::SocketAddr>,
    /// Exit after this height has been authenticated and sampled.
    #[arg(long)]
    until: Option<u64>,
}

#[derive(Clone)]
struct Remote {
    peer: PeerId,
    streams: Streams,
}

#[derive(Clone)]
struct Streams {
    control: libp2p_stream::Control,
    headers: StreamProtocol,
    samples: StreamProtocol,
    eds: StreamProtocol,
}

struct VerifiedSample {
    share: Vec<u8>,
    proof: celestia_proto::celestia::core::v1::proof::NmtProof,
    axis: i32,
}

impl Remote {
    async fn header(&mut self, origin: Origin) -> Result<Option<Vec<u8>>> {
        tokio::time::timeout(DEADLINE, async {
            let mut stream = self
                .streams
                .control
                .open_stream(self.peer, self.streams.headers.clone())
                .await?;
            let req = Request {
                data: Some(origin),
                amount: 1,
            };
            stream
                .write_all(&req.encode_length_delimited_to_vec())
                .await?;
            stream.close().await?;
            let response = HeaderResponse::decode(frame(&mut stream, 4 << 20).await?.as_slice())?;
            if response.status == 2 {
                return Ok(None);
            }
            ensure!(response.status == 1, "DA peer rejected header request");
            Ok(Some(response.body))
        })
        .await?
    }

    async fn sample(
        &mut self,
        height: u64,
        dah: &square::DataAvailabilityHeader,
        at: usize,
    ) -> Result<VerifiedSample> {
        let width = dah.row_roots.len();
        let (row, col) = (at / width, at % width);
        tokio::time::timeout(DEADLINE, async {
            let mut stream = self
                .streams
                .control
                .open_stream(self.peer, self.streams.samples.clone())
                .await?;
            let mut id = height.to_be_bytes().to_vec();
            id.extend_from_slice(&u16::try_from(row)?.to_be_bytes());
            id.extend_from_slice(&u16::try_from(col)?.to_be_bytes());
            stream.write_all(&id).await?;
            stream.close().await?;
            let status = Status::decode(frame(&mut stream, 64).await?.as_slice())?;
            ensure!(
                status.code == 1,
                "sample unavailable at {height} ({row}, {col})"
            );
            let sample = Sample::decode(frame(&mut stream, 8192).await?.as_slice())?;
            let Proof {
                start,
                end,
                nodes,
                leaf_hash,
                ignore_max,
            } = sample.proof.context("missing sample proof")?;
            ensure!(sample.axis == 0 || sample.axis == 1, "invalid sample axis");
            let idx = i64::try_from(if sample.axis == 1 { row } else { col })?;
            ensure!(
                start == idx && end == idx + 1 && leaf_hash.is_empty() && ignore_max,
                "wrong sample proof range"
            );
            let share = sample.share.context("missing share")?.data;
            app::inclusion_proof::verify_sample(dah, row, col, sample.axis == 1, &share, &nodes)
                .map_err(|e| anyhow::anyhow!("{}", e.log))?;
            Ok(VerifiedSample {
                share,
                proof: celestia_proto::celestia::core::v1::proof::NmtProof {
                    start: start.try_into()?,
                    end: end.try_into()?,
                    nodes,
                    leaf_hash,
                },
                axis: sample.axis,
            })
        })
        .await?
    }
}

#[derive(Clone, PartialEq, Message)]
struct Status {
    #[prost(int32, tag = "1")]
    code: i32,
}

async fn frame(stream: &mut libp2p::Stream, limit: usize) -> Result<Vec<u8>> {
    let mut len = 0;
    for shift in [0, 7, 14, 21] {
        let mut byte = [0];
        stream.read_exact(&mut byte).await?;
        len |= usize::from(byte[0] & 127) << shift;
        ensure!(len <= limit, "oversized DA response");
        if byte[0] < 128 {
            let mut data = vec![0; len];
            stream.read_exact(&mut data).await?;
            return Ok(data);
        }
    }
    anyhow::bail!("invalid DA frame")
}

struct Verified {
    validators: ctypes::ValidatorSet,
    signed: ctypes::SignedHeader,
    dah: square::DataAvailabilityHeader,
    hash: Vec<u8>,
    height: u64,
}

impl Verified {
    fn decode(bytes: &[u8], chain: &str, last: Option<&Self>) -> Result<Self> {
        let wire = HeaderWire::decode(bytes)?;
        let h = ctypes::Header::from_proto(&wire.header.context("missing header")?)?;
        let commit = ctypes::Commit::from_proto(&wire.commit.context("missing commit")?)?;
        let set =
            ctypes::ValidatorSet::from_proto(&wire.validators.context("missing validators")?)?;
        let dah = square::DataAvailabilityHeader::from_proto_bytes(&wire.dah)?;
        dah.validate_basic()?;
        ensure!(
            dah.row_roots.len().is_power_of_two() && (2..=1024).contains(&dah.row_roots.len()),
            "invalid EDS width"
        );
        let hash = h.hash().context("empty header")?;
        let height = u64::try_from(h.height)?;
        let now = light::verifier::now();
        ensure!(
            !light::verifier::header_expired(&h, TRUST_NS, now)
                && h.time.unix_nanos() <= now.unix_nanos() + i128::from(DRIFT_NS),
            "header outside trust period"
        );
        ensure!(
            set.hash() == h.validators_hash
                && h.data_hash == dah.hash()
                && commit.block_id.hash == hash,
            "extended header commitment mismatch"
        );
        ctypes::verify_commit_light(chain, &set, &commit.block_id, h.height, &commit)?;
        if let Some(last) = last {
            ensure!(
                h.last_block_id.hash == last.hash,
                "header chain discontinuity"
            );
        }
        let signed = ctypes::SignedHeader {
            header: Some(h),
            commit: Some(commit),
        };
        signed.validate_basic(chain)?;
        if let Some(last) = last {
            light::verifier::verify_adjacent(&last.signed, &signed, &set, TRUST_NS, now, DRIFT_NS)?;
        }
        Ok(Self {
            validators: set,
            signed,
            dah,
            hash,
            height,
        })
    }
}

// Persist the randomly selected cells before issuing requests so restarts cannot
// skip an unavailable cell by drawing new coordinates for the same header.
fn sample_indices(db: &RedbBackend, block: &Verified) -> Result<Vec<usize>> {
    if let Some(saved) = db.get(b"pending")? {
        let (hash, indices): (Vec<u8>, Vec<usize>) = serde_json::from_slice(&saved)?;
        ensure!(hash == block.hash, "pending sampling header changed");
        return Ok(indices);
    }
    let cells = block.dah.row_roots.len().pow(2);
    let indices = index::sample(&mut rand::thread_rng(), cells, cells.min(16)).into_vec();
    let mut batch = WriteBatch::new();
    batch.put(
        b"pending".to_vec(),
        serde_json::to_vec(&(&block.hash, &indices))?,
    );
    db.write(batch)?;
    Ok(indices)
}

#[derive(Clone)]
struct Peers {
    trusted: Vec<PeerId>,
    streams: Streams,
    connected: tokio::sync::watch::Receiver<Vec<PeerId>>,
}

impl Peers {
    fn remote(&self, peer: PeerId) -> Remote {
        Remote {
            peer,
            streams: self.streams.clone(),
        }
    }

    fn online(&self) -> Vec<Remote> {
        self.connected
            .borrow()
            .iter()
            .map(|&peer| self.remote(peer))
            .collect()
    }

    // go-header v0.8.10 returns early on agreement, otherwise the highest
    // valid response from the configured trusted peers once all requests finish.
    async fn trusted_head(&mut self, chain: &str) -> Option<Vec<u8>> {
        let count = self.trusted.len();
        let quorum = (2 * count).div_ceil(3);
        let mut responses = self
            .trusted
            .iter()
            .map(|&id| self.remote(id))
            .map(|mut peer| async move {
                let bytes = peer.header(Origin::Height(0)).await.ok()??;
                Verified::decode(&bytes, chain, None).ok()
            })
            .collect::<futures::stream::FuturesUnordered<_>>();
        let mut votes = BTreeMap::new();
        let mut best = None::<Verified>;
        while let Some(candidate) = responses.next().await {
            let Some(header) = candidate else { continue };
            let n = votes.entry(header.hash.clone()).or_insert(0usize);
            *n += 1;
            if *n >= quorum {
                return Some(header.hash);
            }
            if best.as_ref().is_none_or(|prev| header.height > prev.height) {
                best = Some(header);
            }
        }
        best.map(|header| header.hash)
    }

    async fn verified_header(
        &mut self,
        origin: Origin,
        chain: &str,
        last: Option<&Verified>,
    ) -> Option<(Vec<u8>, Verified)> {
        for mut remote in self.online() {
            let result = async {
                let Some(raw) = remote.header(origin.clone()).await? else {
                    return Ok(None);
                };
                let verified = Verified::decode(&raw, chain, last)?;
                if let Origin::Hash(anchor) = &origin {
                    ensure!(&verified.hash == anchor, "untrusted checkpoint hash");
                }
                Ok::<_, anyhow::Error>(Some((raw, verified)))
            }
            .await;
            match result {
                Ok(Some(header)) => return Some(header),
                Ok(None) => {}
                Err(error) => tracing::warn!(peer=%remote.peer, %error, "DA header unavailable"),
            }
        }
        None
    }

    // A discovered peer can advertise a tip. Its signatures must link to the
    // sampled validator set before its height can affect RPC or sampling status.
    async fn network_tip(&self, head: &Verified, chain: &str) -> (Option<Verified>, bool) {
        let mut pending = self
            .online()
            .into_iter()
            .map(|mut remote| async move {
                let raw = remote.header(Origin::Height(0)).await.ok()??;
                let tip = Verified::decode(&raw, chain, None).ok()?;
                if tip.hash != head.hash {
                    if tip.height <= head.height {
                        return None;
                    }
                    if tip.height == head.height + 1
                        && tip.signed.header.as_ref()?.last_block_id.hash != head.hash
                    {
                        return None;
                    }
                    light::verify(
                        &head.signed,
                        &head.validators,
                        &tip.signed,
                        &tip.validators,
                        TRUST_NS,
                        light::verifier::now(),
                        DRIFT_NS,
                        light::DEFAULT_TRUST_LEVEL,
                    )
                    .ok()?;
                }
                Some(tip)
            })
            .collect::<futures::stream::FuturesUnordered<_>>();
        let mut best = None::<Verified>;
        let mut matching = false;
        let gather = async {
            while let Some(result) = pending.next().await {
                if let Some(tip) = result {
                    matching |= tip.hash == head.hash;
                    if tip.height > head.height
                        && best.as_ref().is_none_or(|prev| tip.height > prev.height)
                    {
                        best = Some(tip);
                    }
                }
            }
        };
        let _ = tokio::time::timeout(Duration::from_secs(2), gather).await;
        (best, matching)
    }

    async fn sample(&mut self, block: &Verified, index: usize) -> bool {
        for mut remote in self.online() {
            match remote.sample(block.height, &block.dah, index).await {
                Ok(_) => return true,
                Err(error) => {
                    tracing::warn!(peer=%remote.peer, height=block.height, index, %error, "DA sample unavailable");
                }
            }
        }
        false
    }
}

fn load_checkpoint(db: &RedbBackend, args: &LightArgs, anchor: &[u8]) -> Result<Option<Verified>> {
    let trust = serde_json::to_vec(&(&args.network, &anchor))?;
    if let Some(stored) = db.get(b"trust")? {
        ensure!(
            stored == trust,
            "light store belongs to a different checkpoint"
        );
    } else {
        let mut batch = WriteBatch::new();
        batch.put(b"trust".to_vec(), trust);
        db.write(batch)?;
    }
    let stored_head = db.get(b"head")?;
    let last = stored_head
        .as_deref()
        .map(|raw| Verified::decode(raw, &args.network, None))
        .transpose()?;
    if let (Some(raw), Some(block)) = (stored_head, &last)
        && db.get(b"tail")?.is_none()
    {
        let mut upgrade = WriteBatch::new();
        rpc::store_header(db, &mut upgrade, block, &raw)?;
        db.write(upgrade)?;
    }
    Ok(last)
}

async fn resolve_anchor(
    db: &RedbBackend,
    peers: &mut Peers,
    args: &LightArgs,
    anchor: Option<Vec<u8>>,
) -> Result<Vec<u8>> {
    Ok(match (anchor, db.get(b"trust")?) {
        (Some(hash), _) => hash,
        (None, Some(stored)) => {
            let (network, hash): (String, Vec<u8>) = serde_json::from_slice(&stored)?;
            ensure!(
                network == args.network && hash.len() == 32,
                "light store belongs to a different network or has an invalid checkpoint"
            );
            hash
        }
        (None, None) => loop {
            if let Some(hash) = peers.trusted_head(&args.network).await {
                break hash;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        },
    })
}

async fn follow(
    db: &RedbBackend,
    peers: &mut Peers,
    args: &LightArgs,
    anchor: Option<Vec<u8>>,
    status: &tokio::sync::watch::Sender<stats::Sampling>,
) -> Result<()> {
    let anchor = resolve_anchor(db, peers, args, anchor).await?;
    let mut last = load_checkpoint(db, args, &anchor)?;
    if let Some(head) = &last {
        status.send_modify(|s| s.complete(head.height));
    }
    loop {
        if last
            .as_ref()
            .is_some_and(|h| args.until.is_some_and(|end| h.height >= end))
        {
            return Ok(());
        }
        let origin = if let Some(h) = &last {
            Origin::Height(h.height + 1)
        } else {
            Origin::Hash(anchor.clone())
        };
        if let Some(last) = &last {
            ensure!(
                !light::verifier::header_expired(
                    last.signed
                        .header
                        .as_ref()
                        .context("missing trusted header")?,
                    TRUST_NS,
                    light::verifier::now()
                ),
                "trusted header expired"
            );
        }
        let Some((raw, block)) = peers
            .verified_header(origin, &args.network, last.as_ref())
            .await
        else {
            if let Some(head) = &last {
                let (tip, matching) = peers.network_tip(head, &args.network).await;
                let done = tip.is_none() && matching;
                let network = tip.map_or(head.height, |tip| tip.height);
                status.send_modify(|s| {
                    s.network = network;
                    s.done = done;
                });
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };
        status.send_modify(|s| s.begin_header(last.as_ref(), &block));
        let indices = sample_indices(db, &block)?;
        let completed = db
            .get(b"completed-samples")?
            .map(|raw| serde_json::from_slice::<usize>(&raw))
            .transpose()?
            .unwrap_or(0);
        ensure!(completed <= indices.len(), "invalid persisted sample count");
        for (n, &at) in indices.iter().enumerate().skip(completed) {
            while !peers.sample(&block, at).await {
                ensure!(
                    !light::verifier::header_expired(
                        block.signed.header.as_ref().context("missing header")?,
                        TRUST_NS,
                        light::verifier::now()
                    ),
                    "sampling header expired"
                );
                status.send_modify(|s| s.retries = s.retries.saturating_add(1));
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            let mut progress = WriteBatch::new();
            progress.put(b"completed-samples".to_vec(), serde_json::to_vec(&(n + 1))?);
            db.write(progress)?;
        }
        let mut batch = WriteBatch::new();
        batch.delete(b"pending".to_vec());
        batch.delete(b"completed-samples".to_vec());
        rpc::store_header(db, &mut batch, &block, &raw)?;
        batch.put(b"head".to_vec(), raw);
        db.write(batch)?;
        writeln!(
            std::io::stdout().lock(),
            "sampled height {} ({} verified shares), hash {}",
            block.height,
            indices.len(),
            hex::encode_upper(&block.hash)
        )?;
        status.send_modify(|s| s.complete(block.height));
        last = Some(block);
    }
}

/// Run DA-only sync in the operator binary with an isolated light store.
pub async fn run_light(home: &Path, args: LightArgs) -> Result<()> {
    let anchor = args.trusted_hash.as_deref().map(hex::decode).transpose()?;
    ensure!(
        anchor.as_ref().is_none_or(|h| h.len() == 32),
        "trusted hash must be 32 bytes"
    );
    let addresses = super::network::Peering::new(&args.network, &args.peer, vec![])?.bootstrap;
    ensure!(
        !addresses.is_empty(),
        "provide a trusted peer for this network"
    );
    let dir = home.join("da-light");
    std::fs::create_dir_all(&dir)?;
    let db = std::sync::Arc::new(RedbBackend::open(
        dir.join("light.redb"),
        RedbOptions::default(),
    )?);
    // Bind synchronously with startup so a busy port is reported to the operator.
    let (status, observer) = tokio::sync::watch::channel(stats::Sampling::default());
    let key = identity(&dir.join("peer.key"))?;
    let behaviour = discovery::Behaviour::new(&key, &args.network)?;
    let mut swarm = SwarmBuilder::with_existing_identity(key)
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_dns()?
        .with_behaviour(|_| behaviour)?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(120)))
        .build();
    if let Some(addr) = &args.p2p_listen {
        swarm.listen_on(addr.clone())?;
    }
    for (peer, addrs) in &addresses {
        for addr in addrs {
            swarm.add_peer_address(*peer, addr.clone());
        }
    }
    let headers = StreamProtocol::try_from_owned(format!("/{}/header-ex/v0.0.3", args.network))?;
    exchange::start(
        swarm.behaviour().streams.new_control(),
        headers.clone(),
        db.clone(),
    )?;
    let samples =
        StreamProtocol::try_from_owned(format!("/{}/shrex/v0.1.0/sample_v0", args.network))?;
    let eds = StreamProtocol::try_from_owned(format!("/{}/shrex/v0.1.0/eds_v0", args.network))?;
    let (connected, online) = tokio::sync::watch::channel(Vec::new());
    let mut peers = Peers {
        trusted: addresses.keys().copied().collect(),
        streams: Streams {
            control: swarm.behaviour().streams.new_control(),
            headers,
            samples,
            eds,
        },
        connected: online,
    };
    let listener = rpc::listener(&args, home, db.clone(), observer, peers.clone()).await?;
    tokio::select! {
        result = listener => result,
        result = follow(&db, &mut peers, &args, anchor, &status) => result,
        result = tokio::signal::ctrl_c() => Ok(result?),
        () = discovery::maintain_connections(swarm, &addresses, connected) => anyhow::bail!("DA network stopped"),
    }
}
