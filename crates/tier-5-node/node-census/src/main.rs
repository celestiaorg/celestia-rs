//! `node-census`: crawl a Celestia consensus network and record how many
//! reachable nodes run celestia-rs and celestia-core (Go).

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, anyhow};
use clap::{Parser, Subcommand};
use node_census::crawl::{CrawlConfig, crawl};
use node_census::report::{Snapshot, record_to_dir};
use node_census::{networks, rpc};
use p2p::NetAddress;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Crawl one network and write a snapshot JSON file.
    Crawl {
        /// `mainnet` or `mocha`.
        #[arg(long)]
        network: String,
        /// Extra starting addresses (`id@host:port`), added to the built-in ones.
        #[arg(long = "seed")]
        seeds: Vec<String>,
        /// Extra RPC servers whose `/net_info` to read, added to the built-in ones.
        #[arg(long = "rpc")]
        rpcs: Vec<String>,
        /// Concurrent probes.
        #[arg(long, default_value_t = 128)]
        concurrency: usize,
        /// Upper bound on distinct node IDs.
        #[arg(long, default_value_t = 5000)]
        max_nodes: usize,
        /// Seconds to wait for a PEX answer after the handshake.
        #[arg(long, default_value_t = 10)]
        pex_timeout: u64,
        /// Output file.
        #[arg(long)]
        out: PathBuf,
    },
    /// Add snapshots to a data directory (`history.json`, `latest/<network>.json`).
    Record {
        /// Data directory (the `node-census-data` branch).
        #[arg(long)]
        data_dir: PathBuf,
        /// Snapshot files written by `crawl`.
        snapshots: Vec<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "node_census=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    match Cli::parse().command {
        Command::Crawl {
            network,
            seeds,
            rpcs,
            concurrency,
            max_nodes,
            pex_timeout,
            out,
        } => {
            let net = networks::by_name(&network)
                .ok_or_else(|| anyhow!("unknown network {network:?} (known: mainnet, mocha)"))?;
            let mut addrs = Vec::new();
            for s in net
                .seeds
                .iter()
                .copied()
                .chain(seeds.iter().map(String::as_str))
            {
                match NetAddress::parse(s) {
                    Ok(a) => addrs.push(a),
                    Err(e) => tracing::warn!(seed = s, error = %e, "skipping seed"),
                }
            }
            let mut rpc_peers = Vec::new();
            for url in net
                .rpcs
                .iter()
                .copied()
                .chain(rpcs.iter().map(String::as_str))
            {
                match rpc::fetch_net_info(url, Duration::from_secs(15)) {
                    Ok(body) => {
                        let peers = rpc::parse_net_info(&body, net.chain_id);
                        tracing::info!(url, peers = peers.len(), "net_info");
                        rpc_peers.extend(peers);
                    }
                    Err(e) => tracing::warn!(url, error = %e, "skipping RPC server"),
                }
            }
            anyhow::ensure!(
                !addrs.is_empty() || !rpc_peers.is_empty(),
                "no usable seed or RPC server for {network}"
            );
            let crawled_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            let started = Instant::now();
            let result = crawl(CrawlConfig {
                chain_id: net.chain_id.into(),
                seeds: addrs,
                rpc_peers,
                allow_private: false,
                dial_timeout: Duration::from_secs(3),
                handshake_timeout: Duration::from_secs(5),
                pex_timeout: Duration::from_secs(pex_timeout),
                concurrency,
                max_nodes,
            })
            .await;
            let snapshot = Snapshot::new(
                net.name,
                net.chain_id,
                crawled_at,
                started.elapsed().as_secs(),
                result,
            );
            tracing::info!(
                network = net.name,
                reachable = snapshot.summary.reachable,
                rust = snapshot.summary.rust,
                go = snapshot.summary.go,
                unknown = snapshot.summary.unknown,
                unreachable = snapshot.summary.unreachable,
                "crawl done"
            );
            // Exit non-zero without an output file, so the workflow keeps the
            // last good data.
            snapshot.ensure_publishable()?;
            let mut bz = serde_json::to_vec_pretty(&snapshot)?;
            bz.push(b'\n');
            std::fs::write(&out, bz).with_context(|| format!("write {}", out.display()))?;
        }
        Command::Record {
            data_dir,
            snapshots,
        } => record(&data_dir, &snapshots)?,
    }
    Ok(())
}

fn record(data_dir: &std::path::Path, snapshots: &[PathBuf]) -> anyhow::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    for path in snapshots {
        let bz = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let s: Snapshot =
            serde_json::from_slice(&bz).with_context(|| format!("parse {}", path.display()))?;
        record_to_dir(data_dir, &s)?;
    }
    Ok(())
}
