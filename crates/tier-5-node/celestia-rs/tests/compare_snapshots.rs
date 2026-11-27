//! `celestia-rs compare-snapshots`: two state-sync snapshot directories
//! (Go's layout) compared item by item, on the Go oracle's format 2
//! fixtures.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn go_snapshots() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../conformance/fixtures/synthetic/v1-v3-snapshots")
}

fn compare(a: &Path, b: &Path, height: &str) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_celestia-rs"))
        .args(["compare-snapshots", "--height", height, "--format", "2"])
        .arg(a)
        .arg(b)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

fn copy_height(from: &Path, height: &str, to: &Path, as_height: &str) {
    let src = from.join(height).join("2");
    let dst = to.join(as_height).join("2");
    std::fs::create_dir_all(&dst).unwrap();
    for e in std::fs::read_dir(&src).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), dst.join(e.file_name())).unwrap();
    }
}

#[test]
fn equal_items_pass_and_the_first_difference_is_named() {
    let go = go_snapshots();
    let (ok, text) = compare(&go, &go, "12");
    assert!(ok, "{text}");
    assert!(text.contains("items equal"), "{text}");
    assert!(text.contains("chunk bytes identical: true"), "{text}");

    // Another height's stream under height 12 differs.
    let other = tempfile::tempdir().unwrap();
    copy_height(&go, "7", other.path(), "12");
    let (ok, text) = compare(&go, other.path(), "12");
    assert!(!ok, "{text}");
    assert!(text.contains("first difference: item "), "{text}");

    let (ok, text) = compare(&go, other.path(), "99");
    assert!(!ok, "{text}");
    assert!(text.contains("no chunks"), "{text}");
}
