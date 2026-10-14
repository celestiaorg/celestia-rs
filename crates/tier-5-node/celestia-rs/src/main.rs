//! `celestia-rs`: the celestia-rs node.
//!
//! For now the only mode is `abci`: serve the Rust state machine over the
//! ABCI socket protocol to a Go celestia-core process (`proxy_app`). With
//! `--home <dir>` state is persisted in `<dir>/data/application.redb`
//! (ADR 0002) and a restarted node resumes from its last committed height;
//! without it state is in memory and starts from genesis on every launch.

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "celestia-rs", version, about = "Rust Celestia consensus node")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Serve the state machine to celestia-core over ABCI (socket).
    Abci {
        /// Listen address.
        #[arg(long, default_value = "127.0.0.1:26658")]
        addr: String,
        /// Node home directory; state lives in `<home>/data`. In memory
        /// when omitted.
        #[arg(long)]
        home: Option<PathBuf>,
        /// Chain ID for a node bootstrapped by state sync (otherwise taken
        /// from InitChain and remembered).
        #[arg(long)]
        chain_id: Option<String>,
        /// Versions kept below the latest (pruning-keep-recent).
        #[arg(long, default_value_t = 100)]
        pruning_keep_recent: u64,
        /// Also keep every version divisible by this (0 = none).
        #[arg(long, default_value_t = 0)]
        pruning_keep_every: u64,
        /// Prune every N commits (0 = never prune, archive).
        #[arg(long, default_value_t = 10)]
        pruning_interval: u64,
        /// Decoded IAVL nodes kept in the LRU cache.
        #[arg(long, default_value_t = 500_000)]
        node_cache_size: usize,
        /// Take a state-sync snapshot every N heights (0 = disabled).
        #[arg(long, default_value_t = 0)]
        snapshot_interval: u64,
        /// Number of snapshots kept.
        #[arg(long, default_value_t = 2)]
        snapshot_keep_recent: u32,
        /// Node minimum gas prices for CheckTx (`app.toml`
        /// `minimum-gas-prices`, e.g. `0.002utia`). Empty (the v10 default)
        /// means `appconsts.DefaultMinGasPrice` (0.004utia).
        #[arg(long, default_value = "")]
        minimum_gas_prices: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    match Cli::parse().command {
        Cmd::Abci {
            addr,
            home,
            chain_id,
            pruning_keep_recent,
            pruning_keep_every,
            pruning_interval,
            node_cache_size,
            snapshot_interval,
            snapshot_keep_recent,
            minimum_gas_prices,
        } => {
            let mut app = match home {
                None => app::App::new(),
                Some(home) => {
                    let mut options = app::AppOptions {
                        chain_id,
                        ..app::AppOptions::default()
                    };
                    options.store.pruning = multistore::PruningOptions {
                        keep_recent: pruning_keep_recent,
                        keep_every: pruning_keep_every,
                        interval: pruning_interval,
                    };
                    options.store.node_cache_size = node_cache_size;
                    options.snapshots = snapshots::SnapshotOptions {
                        interval: snapshot_interval,
                        keep_recent: snapshot_keep_recent,
                    };
                    let app = app::App::open_with(&home, options)
                        .map_err(|e| anyhow::anyhow!("open {}: {}", home.display(), e.log))?;
                    let info = app.info();
                    tracing::info!(
                        home = %home.display(),
                        height = info.last_block_height,
                        app_hash = %hex(&info.last_block_app_hash),
                        "loaded persistent state"
                    );
                    app
                }
            };
            app.set_node_min_gas_price(utia_min_gas_price(&minimum_gas_prices)?);
            let listener = tokio::net::TcpListener::bind(&addr).await?;
            abci_server::serve(listener, Arc::new(app::AbciApp::new(app))).await
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02X}");
        s
    })
}

/// `ctx.MinGasPrices().AmountOf("utia")` for a `DecCoins` config string;
/// `None` when unset.
fn utia_min_gas_price(s: &str) -> anyhow::Result<Option<math::LegacyDec>> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    let mut amount = "0"
        .parse::<math::LegacyDec>()
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    for coin in s.split(',') {
        let coin = coin.trim();
        let split = coin
            .find(|c: char| c.is_ascii_alphabetic())
            .ok_or_else(|| anyhow::anyhow!("invalid decimal coin {coin}"))?;
        if &coin[split..] == "utia" {
            amount = coin[..split]
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid decimal coin {coin}: {e:?}"))?;
        }
    }
    Ok(Some(amount))
}
