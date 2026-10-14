//! `cargo xtask` runner for celestia-rs.
//!
//! Subcommands:
//! - `refresh-fixtures` — regenerate conformance fixtures by shelling out to
//!   the Go program in `conformance/fixturegen/`.
//! - `repo-health` — run the local repository health gates.
//! - `gen-proto` — regenerate committed prost bindings from `proto/`.

// xtask is an internal CLI tool, not a library — stdout is its primary output.
#![allow(clippy::print_stdout)]

mod gen_proto;
mod refresh_fixtures;
mod repo_health;

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
    /// Run the local repository health gates.
    RepoHealth {
        /// Print the checks without executing them.
        #[arg(long)]
        dry_run: bool,
        /// Run cargo and Go commands without accessing the network.
        #[arg(long)]
        offline: bool,
        /// Skip Go fixturegen checks.
        #[arg(long)]
        skip_go: bool,
    },
    /// Regenerate committed prost bindings from the vendored `.proto` files.
    GenProto {
        /// Print what would be generated without writing files.
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
        Cmd::RepoHealth {
            dry_run,
            offline,
            skip_go,
        } => repo_health::run(dry_run, offline, skip_go),
        Cmd::GenProto { dry_run } => gen_proto::run(dry_run),
    }
}
