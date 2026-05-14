//! Integration tests for the xtask CLI.

// Tests legitimately panic on assertion failures and unwrap result values.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command;

fn xtask_bin() -> &'static str {
    env!("CARGO_BIN_EXE_xtask")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
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
fn refresh_fixtures_dry_run_lists_scenario() {
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
        stdout.contains("bech32-addresses"),
        "dry-run should mention the bech32-addresses scenario; got: {stdout}"
    );
}

#[test]
fn refresh_fixtures_scenario_filter_dry_run() {
    let output = Command::new(xtask_bin())
        .args([
            "refresh-fixtures",
            "--scenario",
            "bech32-addresses",
            "--dry-run",
        ])
        .output()
        .expect("failed to invoke xtask binary");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("bech32-addresses"),
        "expected scenario listed; got: {stdout}"
    );
}

#[test]
fn refresh_fixtures_unknown_scenario_filter_errors() {
    let output = Command::new(xtask_bin())
        .args(["refresh-fixtures", "--scenario", "nope", "--dry-run"])
        .output()
        .expect("failed to invoke xtask binary");
    assert!(
        !output.status.success(),
        "unknown scenario filter should error; stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn workspace_root_is_a_git_repo() {
    let root = workspace_root();
    assert!(
        root.join(".git").exists() || root.join(".git").is_file(),
        "expected {} to be a git repo root",
        root.display()
    );
}
