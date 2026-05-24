//! Implementation of `xtask repo-health`.
//!
//! The command keeps the local "is the repo ready?" flow in one place instead
//! of relying on ad hoc command lists in milestone notes.

use crate::refresh_fixtures;
use anyhow::{Context, Result, bail};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy)]
enum CheckKind {
    Cargo,
    Go,
}

#[derive(Debug)]
struct Check {
    name: &'static str,
    program: &'static str,
    args: &'static [&'static str],
    cwd: PathBuf,
    kind: CheckKind,
}

impl Check {
    fn command_line(&self, offline: bool) -> String {
        let mut env_parts = Vec::new();
        match self.kind {
            CheckKind::Cargo if offline => env_parts.push("CARGO_NET_OFFLINE=true".to_string()),
            CheckKind::Go => {
                env_parts.push(format!("GOCACHE={}", go_cache_dir().display()));
                if offline {
                    env_parts.push("GOSUMDB=off".to_string());
                    env_parts.push("GOPROXY=off".to_string());
                }
            }
            CheckKind::Cargo => {}
        }
        let env_prefix = if env_parts.is_empty() {
            String::new()
        } else {
            format!("{} ", env_parts.join(" "))
        };
        format!("{env_prefix}{} {}", self.program, self.args.join(" "))
    }

    fn run(&self, offline: bool) -> Result<()> {
        println!("running {}", self.name);
        let mut command = Command::new(self.program);
        command.current_dir(&self.cwd).args(self.args);
        if offline {
            match self.kind {
                CheckKind::Cargo => {
                    command.env("CARGO_NET_OFFLINE", "true");
                }
                CheckKind::Go => {
                    command.env("GOSUMDB", "off").env("GOPROXY", "off");
                }
            }
        }
        if matches!(self.kind, CheckKind::Go) {
            command.env("GOCACHE", go_cache_dir());
        }
        let status = command
            .status()
            .with_context(|| format!("running `{}`", self.command_line(offline)))?;
        if !status.success() {
            bail!("{} failed with status {status}", self.name);
        }
        Ok(())
    }
}

/// Run the repository health checks.
pub fn run(dry_run: bool, offline: bool, skip_go: bool) -> Result<()> {
    let root = workspace_root();
    let fixturegen_dir = root.join("conformance/fixturegen");
    let checks = checks(&root, &fixturegen_dir, skip_go);

    for check in checks {
        if dry_run {
            println!("[dry-run] {}", check.command_line(offline));
            continue;
        }
        check.run(offline)?;
    }

    if dry_run {
        println!("[dry-run] fixture scenario inventory:");
    } else {
        println!("checking fixture scenario inventory");
    }
    refresh_fixtures::run(None, None, true)
}

fn checks(root: &Path, fixturegen_dir: &Path, skip_go: bool) -> Vec<Check> {
    let mut checks = vec![
        Check {
            name: "cargo fmt",
            program: "cargo",
            args: &["fmt", "--all", "--check"],
            cwd: root.to_path_buf(),
            kind: CheckKind::Cargo,
        },
        Check {
            name: "cargo clippy",
            program: "cargo",
            args: &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--all-features",
                "--locked",
                "--",
                "-D",
                "warnings",
            ],
            cwd: root.to_path_buf(),
            kind: CheckKind::Cargo,
        },
        Check {
            name: "cargo test",
            program: "cargo",
            args: &["test", "--workspace", "--all-features", "--locked"],
            cwd: root.to_path_buf(),
            kind: CheckKind::Cargo,
        },
    ];

    if !skip_go && fixturegen_dir.join("go.mod").exists() {
        checks.push(Check {
            name: "go fixturegen build",
            program: "go",
            args: &["build", "./..."],
            cwd: fixturegen_dir.to_path_buf(),
            kind: CheckKind::Go,
        });
        checks.push(Check {
            name: "go fixturegen tests",
            program: "go",
            args: &["test", "./..."],
            cwd: fixturegen_dir.to_path_buf(),
            kind: CheckKind::Go,
        });
    }

    checks
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

fn go_cache_dir() -> PathBuf {
    env::temp_dir().join("celestia-rs-go-build-cache")
}
