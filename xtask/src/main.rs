//! `cargo xtask` runner for celestia-rs.
//!
//! Subcommands:
//! - `refresh-fixtures` — regenerate conformance fixtures by shelling out to
//!   the Go program in `conformance/fixturegen/`.

// xtask is an internal CLI tool, not a library — stdout is its primary output.
#![allow(clippy::print_stdout)]

mod refresh_fixtures;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "xtask", about = "celestia-rs project task runner", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Regenerate conformance fixtures via the Go fixturegen tool.
    RefreshFixtures {
        /// Limit regeneration to a specific scenario (e.g. `bech32-addresses`).
        #[arg(long)]
        scenario: Option<String>,
        /// Limit regeneration to fixtures for a specific crate.
        #[arg(long = "crate-name")]
        crate_name: Option<String>,
        /// Print what would run without executing.
        #[arg(long)]
        dry_run: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Cmd::RefreshFixtures {
            scenario,
            crate_name,
            dry_run,
        } => refresh_fixtures::run(scenario.as_deref(), crate_name.as_deref(), dry_run),
    }
}
