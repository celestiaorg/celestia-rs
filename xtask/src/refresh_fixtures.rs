//! Implementation of `xtask refresh-fixtures`.
//!
//! The Rust side is a thin orchestrator that calls into the Go program at
//! `conformance/fixturegen/`. Every scenario known to celestia-rs is declared
//! in [`SCENARIOS`]; subcommand filters apply against this list.

// Internal CLI tool — stdout is the user-facing output channel; expects on
// `CARGO_MANIFEST_DIR`-derived paths are guaranteed by the build system.
#![allow(clippy::print_stdout, clippy::expect_used)]

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A scenario known to the Rust runner.
#[derive(Debug, Clone, Copy)]
pub struct Scenario {
    /// Stable scenario name, must match the Go subcommand name.
    pub name: &'static str,
    /// Crate-name tag (for `--crate-name` filtering). Use `"_bootstrap"` for
    /// fixtures not owned by a specific crate.
    pub crate_name: &'static str,
    /// Output path relative to repo root.
    pub output_path: &'static str,
}

pub const SCENARIOS: &[Scenario] = &[Scenario {
    name: "bech32-addresses",
    crate_name: "_bootstrap",
    output_path: "conformance/fixtures/_bootstrap/bech32-addresses.json",
}];

/// Resolve the workspace root from `CARGO_MANIFEST_DIR` (set by cargo at build time).
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask must live inside a workspace")
        .to_path_buf()
}

/// Run `refresh-fixtures` with the given filters.
pub fn run(scenario: Option<&str>, crate_name: Option<&str>, dry_run: bool) -> Result<()> {
    let selected: Vec<&Scenario> = SCENARIOS
        .iter()
        .filter(|s| scenario.is_none_or(|n| n == s.name))
        .filter(|s| crate_name.is_none_or(|c| c == s.crate_name))
        .collect();

    if selected.is_empty() {
        anyhow::bail!("no scenarios matched (scenario={scenario:?}, crate_name={crate_name:?})");
    }

    let root = workspace_root();
    let fixturegen_dir = root.join("conformance/fixturegen");

    for sc in selected {
        let out_abs = root.join(sc.output_path);
        if dry_run {
            println!("[dry-run] {} → {}", sc.name, out_abs.display());
            continue;
        }
        let status = Command::new("go")
            .current_dir(&fixturegen_dir)
            .args(["run", ".", sc.name, "--out"])
            .arg(&out_abs)
            .status()
            .with_context(|| format!("running `go run . {}`", sc.name))?;
        if !status.success() {
            anyhow::bail!("fixturegen {} failed: {status}", sc.name);
        }
        println!("✓ regenerated {}", sc.output_path);
    }
    Ok(())
}
