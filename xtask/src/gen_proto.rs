//! `cargo xtask gen-proto` — regenerate committed prost bindings from the
//! vendored `.proto` sources under `proto/`.
//!
//! Uses `protox` (a pure-Rust protobuf compiler) so no `protoc` binary is
//! required. Generated files are committed; regenerate only after bumping
//! the vendored proto sources.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// One generation target: a set of proto roots compiled into one crate.
struct Target {
    /// Directories searched for imports (relative to repo root).
    includes: &'static [&'static str],
    /// Directories whose `.proto` files are compiled (relative to repo root).
    sources: &'static [&'static str],
    /// Output directory for generated Rust (relative to repo root).
    out: &'static str,
    /// `(proto path, rust path)` pairs resolved to other crates.
    extern_paths: &'static [(&'static str, &'static str)],
}

const TARGETS: &[Target] = &[
    Target {
        includes: &["proto/celestia-core", "proto/third_party"],
        sources: &["proto/celestia-core"],
        out: "crates/tier-1-protocol-types/celestia-core-proto/src/gen",
        extern_paths: &[],
    },
    Target {
        includes: &[
            "proto/cosmos-sdk",
            "proto/ibc-go",
            "proto/celestia-app",
            "proto/hyperlane-cosmos",
            "proto/packet-forward-middleware",
            "proto/celestia-core",
            "proto/third_party",
        ],
        sources: &[
            "proto/cosmos-sdk",
            "proto/ibc-go",
            "proto/celestia-app",
            "proto/hyperlane-cosmos",
            "proto/packet-forward-middleware",
        ],
        out: "crates/tier-1-protocol-types/celestia-proto/src/gen",
        extern_paths: &[
            (".tendermint", "::celestia_core_proto::tendermint"),
            (".cosmos.ics23.v1", "::ics23"),
        ],
    },
];

pub fn run(dry_run: bool) -> Result<()> {
    let root = repo_root()?;
    for target in TARGETS {
        let mut files = Vec::new();
        for src in target.sources {
            collect_protos(&root.join(src), &mut files)?;
        }
        files.sort();
        let out = root.join(target.out);
        println!("gen-proto: {} files -> {}", files.len(), out.display());
        if dry_run {
            continue;
        }
        let includes: Vec<PathBuf> = target.includes.iter().map(|i| root.join(i)).collect();
        let fds =
            protox::compile(&files, &includes).map_err(|e| anyhow::anyhow!("protox: {e:?}"))?;
        if out.exists() {
            std::fs::remove_dir_all(&out)?;
        }
        std::fs::create_dir_all(&out)?;
        let mut config = prost_build::Config::new();
        config.out_dir(&out);
        config.btree_map(["."]);
        config.include_file("mod.rs");
        for (proto, rust) in target.extern_paths {
            config.extern_path(*proto, *rust);
        }
        config
            .compile_fds(fds)
            .context("prost-build code generation")?;
    }
    Ok(())
}

fn collect_protos(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            collect_protos(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "proto") {
            out.push(path);
        }
    }
    Ok(())
}

fn repo_root() -> Result<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Ok(manifest
        .parent()
        .context("xtask has a parent dir")?
        .to_path_buf())
}
