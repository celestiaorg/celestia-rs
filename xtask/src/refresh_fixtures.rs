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

pub const SCENARIOS: &[Scenario] = &[
    Scenario {
        name: "bech32-addresses",
        crate_name: "_bootstrap",
        output_path: "conformance/fixtures/_bootstrap/bech32-addresses.json",
    },
    Scenario {
        name: "hash-empty",
        crate_name: "hash",
        output_path: "conformance/fixtures/hash/empty.json",
    },
    Scenario {
        name: "hash-short-vectors",
        crate_name: "hash",
        output_path: "conformance/fixtures/hash/short-vectors.json",
    },
    Scenario {
        name: "hash-streaming",
        crate_name: "hash",
        output_path: "conformance/fixtures/hash/streaming.json",
    },
    Scenario {
        name: "hash-tmhash",
        crate_name: "hash",
        output_path: "conformance/fixtures/hash/tmhash.json",
    },
    Scenario {
        name: "encoding-protobuf",
        crate_name: "encoding",
        output_path: "conformance/fixtures/encoding/protobuf.json",
    },
    Scenario {
        name: "encoding-varint",
        crate_name: "encoding",
        output_path: "conformance/fixtures/encoding/varint.json",
    },
    Scenario {
        name: "encoding-hex-base64",
        crate_name: "encoding",
        output_path: "conformance/fixtures/encoding/hex-base64.json",
    },
    Scenario {
        name: "encoding-bech32-addresses",
        crate_name: "encoding",
        output_path: "conformance/fixtures/encoding/bech32-addresses.json",
    },
    Scenario {
        name: "crypto-ed25519",
        crate_name: "crypto",
        output_path: "conformance/fixtures/crypto/ed25519.json",
    },
    Scenario {
        name: "crypto-secp256k1",
        crate_name: "crypto",
        output_path: "conformance/fixtures/crypto/secp256k1.json",
    },
    Scenario {
        name: "crypto-bls12381",
        crate_name: "crypto",
        output_path: "conformance/fixtures/crypto/bls12381.json",
    },
    Scenario {
        name: "merkle-roots",
        crate_name: "merkle",
        output_path: "conformance/fixtures/merkle/roots.json",
    },
    Scenario {
        name: "merkle-proofs",
        crate_name: "merkle",
        output_path: "conformance/fixtures/merkle/proofs.json",
    },
    Scenario {
        name: "merkle-malformed-proofs",
        crate_name: "merkle",
        output_path: "conformance/fixtures/merkle/malformed-proofs.json",
    },
    Scenario {
        name: "tendermint-headers",
        crate_name: "tendermint-types",
        output_path: "conformance/fixtures/tendermint-types/headers.json",
    },
    Scenario {
        name: "tendermint-validators",
        crate_name: "tendermint-types",
        output_path: "conformance/fixtures/tendermint-types/validators.json",
    },
    Scenario {
        name: "tendermint-commits",
        crate_name: "tendermint-types",
        output_path: "conformance/fixtures/tendermint-types/commits.json",
    },
    Scenario {
        name: "tendermint-canonical-sign-bytes",
        crate_name: "tendermint-types",
        output_path: "conformance/fixtures/tendermint-types/canonical-sign-bytes.json",
    },
    Scenario {
        name: "tendermint-malformed",
        crate_name: "tendermint-types",
        output_path: "conformance/fixtures/tendermint-types/malformed.json",
    },
    Scenario {
        name: "cosmos-addresses",
        crate_name: "cosmos-types",
        output_path: "conformance/fixtures/cosmos-types/addresses.json",
    },
    Scenario {
        name: "cosmos-coins",
        crate_name: "cosmos-types",
        output_path: "conformance/fixtures/cosmos-types/coins.json",
    },
    Scenario {
        name: "cosmos-public-keys",
        crate_name: "cosmos-types",
        output_path: "conformance/fixtures/cosmos-types/public-keys.json",
    },
    Scenario {
        name: "cosmos-sign-docs",
        crate_name: "cosmos-types",
        output_path: "conformance/fixtures/cosmos-types/sign-docs.json",
    },
    Scenario {
        name: "cosmos-tx-raw",
        crate_name: "cosmos-types",
        output_path: "conformance/fixtures/cosmos-types/tx-raw.json",
    },
    Scenario {
        name: "cosmos-malformed",
        crate_name: "cosmos-types",
        output_path: "conformance/fixtures/cosmos-types/malformed.json",
    },
    Scenario {
        name: "abci-requests",
        crate_name: "abci-types",
        output_path: "conformance/fixtures/abci-types/requests.json",
    },
    Scenario {
        name: "abci-responses",
        crate_name: "abci-types",
        output_path: "conformance/fixtures/abci-types/responses.json",
    },
    Scenario {
        name: "abci-vote-extensions",
        crate_name: "abci-types",
        output_path: "conformance/fixtures/abci-types/vote-extensions.json",
    },
    Scenario {
        name: "abci-events",
        crate_name: "abci-types",
        output_path: "conformance/fixtures/abci-types/events.json",
    },
    Scenario {
        name: "abci-malformed",
        crate_name: "abci-types",
        output_path: "conformance/fixtures/abci-types/malformed.json",
    },
    Scenario {
        name: "nmt-hashes",
        crate_name: "nmt",
        output_path: "conformance/fixtures/nmt/hashes.json",
    },
    Scenario {
        name: "nmt-roots",
        crate_name: "nmt",
        output_path: "conformance/fixtures/nmt/roots.json",
    },
    Scenario {
        name: "nmt-proofs",
        crate_name: "nmt",
        output_path: "conformance/fixtures/nmt/proofs.json",
    },
    Scenario {
        name: "nmt-subroot-paths",
        crate_name: "nmt",
        output_path: "conformance/fixtures/nmt/subroot-paths.json",
    },
    Scenario {
        name: "nmt-malformed",
        crate_name: "nmt",
        output_path: "conformance/fixtures/nmt/malformed.json",
    },
    Scenario {
        name: "shares-namespaces",
        crate_name: "shares",
        output_path: "conformance/fixtures/shares/namespaces.json",
    },
    Scenario {
        name: "shares-info-reserved",
        crate_name: "shares",
        output_path: "conformance/fixtures/shares/info-reserved.json",
    },
    Scenario {
        name: "shares-builders",
        crate_name: "shares",
        output_path: "conformance/fixtures/shares/builders.json",
    },
    Scenario {
        name: "shares-compact",
        crate_name: "shares",
        output_path: "conformance/fixtures/shares/compact.json",
    },
    Scenario {
        name: "shares-sparse",
        crate_name: "shares",
        output_path: "conformance/fixtures/shares/sparse.json",
    },
    Scenario {
        name: "shares-padding",
        crate_name: "shares",
        output_path: "conformance/fixtures/shares/padding.json",
    },
    Scenario {
        name: "shares-malformed",
        crate_name: "shares",
        output_path: "conformance/fixtures/shares/malformed.json",
    },
    Scenario {
        name: "square-powers-of-two",
        crate_name: "square",
        output_path: "conformance/fixtures/square/powers-of-two.json",
    },
    Scenario {
        name: "square-inclusion-rules",
        crate_name: "square",
        output_path: "conformance/fixtures/square/inclusion-rules.json",
    },
    Scenario {
        name: "square-blob-tx",
        crate_name: "square",
        output_path: "conformance/fixtures/square/blob-tx.json",
    },
    Scenario {
        name: "square-commitments",
        crate_name: "square",
        output_path: "conformance/fixtures/square/commitments.json",
    },
    Scenario {
        name: "square-builders",
        crate_name: "square",
        output_path: "conformance/fixtures/square/builders.json",
    },
    Scenario {
        name: "square-squares",
        crate_name: "square",
        output_path: "conformance/fixtures/square/squares.json",
    },
    Scenario {
        name: "square-ranges",
        crate_name: "square",
        output_path: "conformance/fixtures/square/ranges.json",
    },
    Scenario {
        name: "square-deconstruct",
        crate_name: "square",
        output_path: "conformance/fixtures/square/deconstruct.json",
    },
    Scenario {
        name: "square-erasure",
        crate_name: "square",
        output_path: "conformance/fixtures/square/erasure.json",
    },
    Scenario {
        name: "square-dah",
        crate_name: "square",
        output_path: "conformance/fixtures/square/dah.json",
    },
    Scenario {
        name: "square-mainnet-blocks",
        crate_name: "square",
        output_path: "conformance/fixtures/square/mainnet-blocks.json",
    },
    Scenario {
        name: "square-malformed",
        crate_name: "square",
        output_path: "conformance/fixtures/square/malformed.json",
    },
    Scenario {
        name: "iavl-roots",
        crate_name: "iavl",
        output_path: "conformance/fixtures/iavl/roots.json",
    },
    Scenario {
        name: "iavl-iterators",
        crate_name: "iavl",
        output_path: "conformance/fixtures/iavl/iterators.json",
    },
    Scenario {
        name: "iavl-proofs",
        crate_name: "iavl",
        output_path: "conformance/fixtures/iavl/proofs.json",
    },
    Scenario {
        name: "iavl-malformed",
        crate_name: "iavl",
        output_path: "conformance/fixtures/iavl/malformed.json",
    },
    Scenario {
        name: "kvstore-basic",
        crate_name: "kvstore",
        output_path: "conformance/fixtures/kvstore/basic.json",
    },
    Scenario {
        name: "kvstore-cache",
        crate_name: "kvstore",
        output_path: "conformance/fixtures/kvstore/cache.json",
    },
    Scenario {
        name: "kvstore-prefix",
        crate_name: "kvstore",
        output_path: "conformance/fixtures/kvstore/prefix.json",
    },
    Scenario {
        name: "kvstore-gas",
        crate_name: "kvstore",
        output_path: "conformance/fixtures/kvstore/gas.json",
    },
    Scenario {
        name: "kvstore-malformed",
        crate_name: "kvstore",
        output_path: "conformance/fixtures/kvstore/malformed.json",
    },
    Scenario {
        name: "ics23-proofs-iavl",
        crate_name: "ics23-proofs",
        output_path: "conformance/fixtures/ics23-proofs/iavl.json",
    },
    Scenario {
        name: "ics23-proofs-simple",
        crate_name: "ics23-proofs",
        output_path: "conformance/fixtures/ics23-proofs/simple.json",
    },
    Scenario {
        name: "ics23-proofs-ops",
        crate_name: "ics23-proofs",
        output_path: "conformance/fixtures/ics23-proofs/ops.json",
    },
    Scenario {
        name: "ics23-proofs-malformed",
        crate_name: "ics23-proofs",
        output_path: "conformance/fixtures/ics23-proofs/malformed.json",
    },
    Scenario {
        name: "multistore-commit",
        crate_name: "multistore",
        output_path: "conformance/fixtures/multistore/commit.json",
    },
    Scenario {
        name: "multistore-cache",
        crate_name: "multistore",
        output_path: "conformance/fixtures/multistore/cache.json",
    },
    Scenario {
        name: "multistore-proofs",
        crate_name: "multistore",
        output_path: "conformance/fixtures/multistore/proofs.json",
    },
    Scenario {
        name: "multistore-malformed",
        crate_name: "multistore",
        output_path: "conformance/fixtures/multistore/malformed.json",
    },
    Scenario {
        name: "m2-integration-state-diffs",
        crate_name: "m2-integration",
        output_path: "conformance/fixtures/m2-integration/state-diffs.json",
    },
    Scenario {
        name: "ante-stateless",
        crate_name: "ante",
        output_path: "conformance/fixtures/ante/stateless.json",
    },
    Scenario {
        name: "ante-signatures",
        crate_name: "ante",
        output_path: "conformance/fixtures/ante/signatures.json",
    },
    Scenario {
        name: "ante-fees",
        crate_name: "ante",
        output_path: "conformance/fixtures/ante/fees.json",
    },
    Scenario {
        name: "ante-gas",
        crate_name: "ante",
        output_path: "conformance/fixtures/ante/gas.json",
    },
    Scenario {
        name: "ante-malformed",
        crate_name: "ante",
        output_path: "conformance/fixtures/ante/malformed.json",
    },
    Scenario {
        name: "auth-accounts",
        crate_name: "auth",
        output_path: "conformance/fixtures/auth/accounts.json",
    },
    Scenario {
        name: "bank-balances",
        crate_name: "bank",
        output_path: "conformance/fixtures/bank/balances.json",
    },
    Scenario {
        name: "bank-messages",
        crate_name: "bank",
        output_path: "conformance/fixtures/bank/messages.json",
    },
    Scenario {
        name: "staking-state",
        crate_name: "staking",
        output_path: "conformance/fixtures/staking/state.json",
    },
    Scenario {
        name: "staking-delegate",
        crate_name: "staking",
        output_path: "conformance/fixtures/staking/delegate.json",
    },
];

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
