//! Reopen the index after abrupt shutdown and verify that retained data survives consensus pruning.
#![cfg(feature = "bridge")]

use anyhow::{Context, Result, ensure};
use std::{path::Path, process::Command};

#[test]
#[ignore = "starts a private validator on the default local ports"]
fn consensus_pruning_preserves_indexed_paid_square() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let work = temp.path();
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../conformance/da/index-restart.sh");
    let run = |phase: &str| -> Result<()> {
        let status = Command::new("bash")
            .arg(&script)
            .arg(env!("CARGO_BIN_EXE_celestia-rs"))
            .arg(work)
            .arg(phase)
            .status()?;
        ensure!(status.success(), "DA restart phase {phase} failed");
        Ok(())
    };
    run("seed")?;
    let receipt: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(work.join("receipt"))?)?;
    let height: i64 = receipt["height"]
        .as_str()
        .context("missing height")?
        .parse()?;
    {
        let (blocks, states) = celestia_rs::node::open_stores(&work.join("node"))?;
        let state = states.load()?.context("missing state")?;
        blocks.prune_blocks(height + 1, &state)?;
        ensure!(
            blocks.load_block(height)?.is_none(),
            "consensus retained the paid block"
        );
    }
    run("restart")
}
