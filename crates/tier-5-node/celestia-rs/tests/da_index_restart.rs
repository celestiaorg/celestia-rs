//! Reopen the index after abrupt shutdown and verify blockstore-backed DA reads.
#![cfg(feature = "bridge")]

use anyhow::{Result, ensure};
use std::{path::Path, process::Command};

#[test]
#[ignore = "starts a private validator on the default local ports"]
fn restart_preserves_indexed_paid_square() -> Result<()> {
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
    run("restart")
}
