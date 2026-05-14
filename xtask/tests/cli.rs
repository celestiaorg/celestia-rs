//! Integration tests for the xtask CLI.

// Tests legitimately panic on assertion failures and unwrap result values.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::process::Command;

fn xtask_bin() -> &'static str {
    env!("CARGO_BIN_EXE_xtask")
}

#[test]
fn help_prints_usage() {
    let output = Command::new(xtask_bin())
        .arg("--help")
        .output()
        .expect("failed to invoke xtask binary");
    assert!(output.status.success(), "xtask --help should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("refresh-fixtures"),
        "help should list the refresh-fixtures subcommand; got: {stdout}"
    );
}

#[test]
fn refresh_fixtures_dry_run_succeeds() {
    let output = Command::new(xtask_bin())
        .args(["refresh-fixtures", "--dry-run"])
        .output()
        .expect("failed to invoke xtask binary");
    assert!(
        output.status.success(),
        "refresh-fixtures --dry-run should succeed; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("[dry-run]"),
        "dry-run should print a [dry-run] prefix; got: {stdout}"
    );
}

#[test]
fn refresh_fixtures_without_dry_run_fails() {
    let output = Command::new(xtask_bin())
        .args(["refresh-fixtures"])
        .output()
        .expect("failed to invoke xtask binary");
    assert!(
        !output.status.success(),
        "refresh-fixtures without --dry-run should fail until Task 9 wires it up"
    );
}
