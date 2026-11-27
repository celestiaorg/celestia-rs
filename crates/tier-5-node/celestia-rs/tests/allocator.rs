//! The binary reports which global allocator it was built with.

use std::process::Command;

/// The allocator the build's features select: jemalloc wins over the
/// default mimalloc when both are enabled (as with `--all-features`).
fn expected() -> &'static str {
    if cfg!(feature = "jemalloc") {
        "jemalloc"
    } else if cfg!(feature = "mimalloc") {
        "mimalloc"
    } else {
        "system"
    }
}

#[test]
fn version_long_names_the_allocator() {
    let out = Command::new(env!("CARGO_BIN_EXE_celestia-rs"))
        .args(["version", "--long"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let out = String::from_utf8(out.stdout).unwrap();
    let line = format!("allocator: {}", expected());
    assert!(out.lines().any(|l| l == line), "no `{line}` in:\n{out}");
}
