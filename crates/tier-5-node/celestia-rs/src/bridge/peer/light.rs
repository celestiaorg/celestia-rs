//! Authenticated, sequential DA sync and restartable sampling from a trusted checkpoint.
use super::{HeaderResponse, HeaderWire, Origin, Proof, Request, Sample, identity};
use anyhow::{Context, Result, ensure};
use futures::{AsyncReadExt, AsyncWriteExt, StreamExt};
use kvdb::{KvBackend, RedbBackend, RedbOptions, WriteBatch};
use libp2p::{
    Multiaddr, PeerId, StreamProtocol, SwarmBuilder, multiaddr::Protocol, noise, swarm::SwarmEvent,
    tcp, yamux,
};
use prost::Message;
use rand::seq::index;
use std::{path::Path, time::Duration};

const TRUST_NS: i64 = 7 * 24 * 60 * 60 * 1_000_000_000;
const DRIFT_NS: i64 = 10_000_000_000;
const DEADLINE: Duration = Duration::from_secs(20);

/// Operator-supplied trust inputs for a standalone DA light node.
#[derive(clap::Args, Debug)]
pub struct LightArgs {
    /// Consensus chain ID and DA protocol namespace.
    #[arg(long)]
    network: String,
    /// Bridge multiaddress including its peer ID.
    #[arg(long)]
    peer: Multiaddr,
    /// Hex-encoded 32-byte header hash obtained through a trusted channel.
    #[arg(long)]
    trusted_hash: String,
    /// Exit after this height has been authenticated and sampled.
    #[arg(long)]
    until: Option<u64>,
}

struct Remote {
    control: libp2p_stream::Control,
    peer: PeerId,
    headers: StreamProtocol,
    samples: StreamProtocol,
}

impl Remote {
    async fn header(&mut self, origin: Origin) -> Result<Option<Vec<u8>>> {
        tokio::time::timeout(DEADLINE, async {
            let mut stream = self
                .control
                .open_stream(self.peer, self.headers.clone())
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
    ) -> Result<()> {
        let width = dah.row_roots.len();
        let (row, col) = (at / width, at % width);
        tokio::time::timeout(DEADLINE, async {
            let mut stream = self
                .control
                .open_stream(self.peer, self.samples.clone())
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
            app::inclusion_proof::verify_sample(
                dah,
                row,
                col,
                sample.axis == 1,
                &sample.share.context("missing share")?.data,
                &nodes,
            )
            .map_err(|e| anyhow::anyhow!("{}", e.log))
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

async fn follow(
    db: &RedbBackend,
    remote: &mut Remote,
    args: &LightArgs,
    anchor: Vec<u8>,
) -> Result<()> {
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
    let mut last = db
        .get(b"head")?
        .map(|raw| Verified::decode(&raw, &args.network, None))
        .transpose()?;
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
        let Some(raw) = remote.header(origin).await? else {
            ensure!(last.is_some(), "trusted checkpoint unavailable from bridge");
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };
        let block = Verified::decode(&raw, &args.network, last.as_ref())?;
        if last.is_none() {
            ensure!(block.hash == anchor, "untrusted checkpoint hash");
        }
        let indices = sample_indices(db, &block)?;
        for &at in &indices {
            remote.sample(block.height, &block.dah, at).await?;
        }
        let mut batch = WriteBatch::new();
        batch.delete(b"pending".to_vec());
        batch.put(b"head".to_vec(), raw);
        db.write(batch)?;
        println!(
            "sampled height {} ({} verified shares), hash {}",
            block.height,
            indices.len(),
            hex::encode_upper(&block.hash)
        );
        last = Some(block);
    }
}

/// Run DA-only sync in the operator binary with an isolated light store.
pub async fn run_light(home: &Path, args: LightArgs) -> Result<()> {
    let anchor = hex::decode(&args.trusted_hash)?;
    ensure!(anchor.len() == 32, "trusted hash must be 32 bytes");
    let Some(Protocol::P2p(peer)) = args.peer.iter().last() else {
        anyhow::bail!("bridge address must end in /p2p/PEER_ID")
    };
    let dir = home.join("da-light");
    std::fs::create_dir_all(&dir)?;
    let db = RedbBackend::open(dir.join("light.redb"), RedbOptions::default())?;
    let key = identity(&dir.join("peer.key"))?;
    let mut swarm = SwarmBuilder::with_existing_identity(key)
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_behaviour(|_| libp2p_stream::Behaviour::new())?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(120)))
        .build();
    let mut remote = Remote {
        control: swarm.behaviour().new_control(),
        peer,
        headers: StreamProtocol::try_from_owned(format!("/{}/header-ex/v0.0.3", args.network))?,
        samples: StreamProtocol::try_from_owned(format!(
            "/{}/shrex/v0.1.0/sample_v0",
            args.network
        ))?,
    };
    swarm.dial(args.peer.clone())?;
    tokio::time::timeout(DEADLINE, async {
        loop {
            match swarm.select_next_some().await {
                SwarmEvent::ConnectionEstablished { peer_id, .. } if peer_id == peer => {
                    return Ok(());
                }
                SwarmEvent::OutgoingConnectionError { error, .. } => {
                    return Err(anyhow::Error::from(error));
                }
                _ => {}
            }
        }
    })
    .await??;
    tokio::select! {
        result=follow(&db,&mut remote,&args,anchor) => result,
        result=tokio::signal::ctrl_c() => Ok(result?),
        result=async {loop {if let SwarmEvent::ConnectionClosed{num_established:0,..}=swarm.select_next_some().await {return Err(anyhow::anyhow!("bridge disconnected"))}}} => result,
    }
}
