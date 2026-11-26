//! Checks that `.github/dependabot.yml` covers every Go module.
//!
//! A Go module that no `gomod` entry lists still gets security updates
//! from the repository-level defaults. Those pull requests skip the
//! grouping and the `ignore` rules that keep pinned reference modules in
//! place (docs/security.md, "Dependabot configuration").

// Tests legitimately panic on assertion failures and unwrap result values.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Every directory under `dir` that holds a `go.mod`, relative to `root`.
fn go_module_dirs(root: &Path, dir: &Path, out: &mut BTreeSet<String>) {
    if dir.join("go.mod").is_file() {
        let rel = dir.strip_prefix(root).unwrap();
        out.insert(format!("/{}", rel.to_str().unwrap()));
    }
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            go_module_dirs(root, &path, out);
        }
    }
}

/// Every `- /path` list item in the dependabot configuration.
fn listed_dirs(config: &str) -> BTreeSet<String> {
    config
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- /"))
        .map(|dir| format!("/{}", dir.trim()))
        .collect()
}

#[test]
fn dependabot_lists_every_go_module() {
    let root = workspace_root();
    let mut modules = BTreeSet::new();
    go_module_dirs(&root, &root.join("conformance"), &mut modules);
    assert!(
        !modules.is_empty(),
        "no Go modules found under conformance/"
    );

    let config = fs::read_to_string(root.join(".github/dependabot.yml")).unwrap();
    let listed = listed_dirs(&config);

    let missing: Vec<_> = modules.difference(&listed).collect();
    assert!(
        missing.is_empty(),
        "Go modules missing from .github/dependabot.yml: {missing:#?}"
    );
}

/// Every `interval:` value in the dependabot configuration.
fn intervals(config: &str) -> Vec<String> {
    config
        .lines()
        .filter_map(|line| line.trim().strip_prefix("interval:"))
        .map(|value| value.trim().to_string())
        .collect()
}

#[test]
fn dependabot_runs_monthly() {
    let root = workspace_root();
    let config = fs::read_to_string(root.join(".github/dependabot.yml")).unwrap();
    let intervals = intervals(&config);
    assert!(!intervals.is_empty(), "no schedule intervals found");
    assert!(
        intervals.iter().all(|i| i == "monthly"),
        "every dependabot entry must run monthly, found: {intervals:?}"
    );
}

#[test]
fn dependabot_groups_rust_majors() {
    let root = workspace_root();
    let config = fs::read_to_string(root.join(".github/dependabot.yml")).unwrap();
    assert!(
        config.contains("rust-major:") && config.contains("update-types: [major]"),
        "cargo major version updates must share one group"
    );
}

/// The `- package-ecosystem:` entries of the configuration, one string each.
fn entries(config: &str) -> Vec<String> {
    config
        .split("\n  - package-ecosystem:")
        .skip(1)
        .map(|entry| format!("package-ecosystem:{entry}"))
        .collect()
}

#[test]
fn dependabot_cools_down_every_entry() {
    let root = workspace_root();
    let config = fs::read_to_string(root.join(".github/dependabot.yml")).unwrap();
    let entries = entries(&config);
    assert!(!entries.is_empty(), "no dependabot entries found");
    for entry in &entries {
        assert!(
            entry.contains("cooldown:\n      default-days: 7"),
            "every dependabot entry needs a seven-day cooldown:\n{entry}"
        );
    }
}

#[test]
fn dependabot_opens_no_go_pull_requests() {
    let root = workspace_root();
    let config = fs::read_to_string(root.join(".github/dependabot.yml")).unwrap();
    let go: Vec<_> = entries(&config)
        .into_iter()
        .filter(|entry| entry.starts_with("package-ecosystem: gomod"))
        .collect();
    assert!(!go.is_empty(), "no gomod entries found");
    for entry in &go {
        assert!(
            entry.contains("open-pull-requests-limit: 0")
                && entry.contains("- dependency-name: \"*\""),
            "gomod entries must ignore every dependency (alerts only):\n{entry}"
        );
    }
}

#[test]
fn dependabot_defers_consensus_majors() {
    let root = workspace_root();
    let config = fs::read_to_string(root.join(".github/dependabot.yml")).unwrap();
    for name in ["secp256k1", "\"tendermint*\""] {
        assert!(
            config.contains(&format!(
                "- dependency-name: {name}\n        update-types: [\"version-update:semver-major\"]"
            )),
            "major versions of {name} must be migrated by hand"
        );
    }
}
