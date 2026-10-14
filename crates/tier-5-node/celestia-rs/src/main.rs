//! `celestia-rs`: the celestia-rs node.
//!
//! For now the only mode is `abci`: serve the Rust state machine over the
//! ABCI socket protocol to a Go celestia-core process (`proxy_app`). State
//! is in memory and starts from genesis on every launch.

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
        Cmd::Abci { addr } => {
            let listener = tokio::net::TcpListener::bind(&addr).await?;
            abci_server::serve(listener, Arc::new(app::AbciApp::new(app::App::new()))).await
        }
    }
}
