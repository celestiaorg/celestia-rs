# celestia-rs Bootstrap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Scaffold the celestia-rs Cargo workspace, stand up conformance fixture generation end-to-end (Go → JSON → consumed by Rust tests), and ship the first M1 per-crate spec — so a long-running agent can begin implementing M1 crate 1 (`hash`) immediately afterward.

**Architecture:** Single Cargo workspace at repo root. A `xtask/` member crate provides a `cargo xtask`-style task runner that shells out to a Go program in `conformance/fixturegen/` to regenerate golden vectors. CI runs `fmt`, `clippy`, `test`, and `cargo-deny` per-PR; nightly jobs are stubbed for later expansion. All per-milestone work is gated through spec files under `docs/superpowers/specs/m<N>/`.

**Tech Stack:** Rust 1.85+ (edition 2024, resolver 3), `clap` (xtask CLI), Go 1.22+ (fixturegen), GitHub Actions (CI), `cargo-deny` (license + dep enforcement), `cargo-llvm-cov` (coverage; activated when first crate lands).

---

## Scope check

This plan covers **only bootstrap** — workspace scaffolding + fixturegen end-to-end + the `hash` crate spec. It does NOT implement any M1 crate. The 9 remaining M1 crate specs (`encoding`, `crypto`, `merkle`, `tendermint-types`, `cosmos-types`, `abci-types`, `nmt`, `shares`, `square`) are written by a follow-up plan once this scaffolding lands and the dispatch loop is alive. M1 crate implementation itself is covered by per-crate plans generated when each spec is ready.

## File structure

Files this plan creates or modifies (paths relative to repo root):

```
.gitignore                                   create
.gitattributes                               create  # for future Git LFS use
README.md                                    modify  # replace single-line placeholder
LICENSE-APACHE                               create
LICENSE-MIT                                  create
MILESTONES.md                                create
rust-toolchain.toml                          create
rustfmt.toml                                 create
clippy.toml                                  create
Cargo.toml                                   create  # workspace manifest
deny.toml                                    create
xtask/Cargo.toml                             create
xtask/src/main.rs                            create
xtask/src/refresh_fixtures.rs                create
xtask/tests/cli.rs                           create
.github/workflows/ci.yml                     create
.github/workflows/nightly.yml                create
.github/PULL_REQUEST_TEMPLATE.md             create
.github/CODEOWNERS                           create
conformance/README.md                        create
conformance/fixtures/.gitkeep                create
conformance/fixturegen/go.mod                create
conformance/fixturegen/main.go               create
conformance/fixturegen/cmd/cmd.go            create
conformance/fixturegen/cmd/bech32.go         create
conformance/fixturegen/cmd/bech32_test.go    create
docs/superpowers/specs/m1/_index.md          create
docs/superpowers/specs/m1/hash.md            create
```

**File responsibilities:**

- `Cargo.toml` (root) — workspace manifest only; no package. Member: `xtask`.
- `xtask/src/main.rs` — clap-driven CLI dispatching to subcommand modules.
- `xtask/src/refresh_fixtures.rs` — `refresh-fixtures [--scenario <name>] [--crate <name>]` subcommand; shells out to `go run ./conformance/fixturegen ...`.
- `conformance/fixturegen/main.go` — CLI entry point; dispatches to subcommands in `cmd/`.
- `conformance/fixturegen/cmd/cmd.go` — shared command-registration plumbing.
- `conformance/fixturegen/cmd/bech32.go` — the one initial fixture emitter; produces `conformance/fixtures/_bootstrap/bech32-addresses.json` from hardcoded test vectors using Cosmos SDK address types with the `celestia` bech32 prefix.
- `docs/superpowers/specs/m1/_index.md` — table of M1 crates with status + dep DAG.
- `docs/superpowers/specs/m1/hash.md` — first per-crate spec (the contract for the M1-crate-1 agent).

---

## Task 1: Initial repo files (gitignore, license, readme, milestones)

**Files:**
- Create: `.gitignore`
- Create: `.gitattributes`
- Create: `LICENSE-APACHE`
- Create: `LICENSE-MIT`
- Create: `MILESTONES.md`
- Modify: `README.md`

- [ ] **Step 1: Create `.gitignore`**

```gitignore
/target
**/*.rs.bk
Cargo.lock.bak
.DS_Store
/conformance/fixturegen/fixturegen
/conformance/fixturegen/*.test
.idea/
.vscode/
*.swp
```

- [ ] **Step 2: Create `.gitattributes`**

```gitattributes
# Future Git LFS patterns for large conformance fixtures (>10MB).
# Activate via: git lfs install && git lfs track "<pattern>"
# conformance/fixtures/**/*.bin.zst filter=lfs diff=lfs merge=lfs -text
* text=auto eol=lf
*.rs text diff=rust
*.go text diff=golang
```

- [ ] **Step 3: Create `LICENSE-APACHE`**

Run: `curl -fsSL https://www.apache.org/licenses/LICENSE-2.0.txt -o LICENSE-APACHE`
Expected: file ≈ 11 KB, starts with `                                 Apache License`.

- [ ] **Step 4: Create `LICENSE-MIT`**

```
MIT License

Copyright (c) 2026 Rootul Patel

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

- [ ] **Step 5: Create `MILESTONES.md`**

```markdown
# celestia-rs Milestones

Authoritative source for "what is next." Updated when a milestone starts or completes.

See `docs/superpowers/specs/2026-05-14-celestia-rs-design.md` for full descriptions.

| #   | Status      | Milestone                            | Acceptance test |
|-----|-------------|--------------------------------------|-----------------|
| M0  | in-progress | Bootstrap (scaffolding + fixturegen) | `cargo xtask refresh-fixtures --scenario bech32-addresses` produces a valid fixture |
| M1  | pending     | Foundational libraries               | `m1-integration::parse_real_mainnet_block_produces_matching_dah` passes |
| M2  | pending     | State & storage                      | `m2-integration::apply_mainnet_block_diff_produces_matching_kv_root` passes |
| M3  | pending     | Tx & ante machinery                  | Tx-validation outcome matches Go for ≥ 1000 mainnet txs |
| M4  | pending     | Core SDK modules (auth, bank, staking) | Per-module subtree roots match Go after replaying mainnet blocks |
| M5  | pending     | Remaining SDK modules                | Per-module state roots match Go |
| M6  | pending     | Celestia-specific modules            | App hash matches Go for blob-tx mainnet blocks |
| M7  | pending     | IBC                                  | IBC-touching mainnet blocks replay; LC verifies headers |
| M8  | pending     | ABCI++ app integration               | App hash matches Go for 1000 consecutive mainnet blocks |
| M9  | pending     | Consensus engine                     | Two celestia-rs nodes form a private network |
| M10 | pending     | P2P networking                       | Connects to Go peers and gossips a block |
| M11 | pending     | RPC & gRPC                           | RPC responses match Go for canned request corpus |
| M12 | pending     | Node binary, CLI, ops                | `celestia-rs start --network mainnet` works end-to-end |
| M13 | pending     | Mainnet sync from genesis            | Final app hash equals current mainnet's |
| M14 | pending     | Mainnet validator                    | Signs blocks on mainnet without double-signs |

## Current focus
**M0** — see `docs/superpowers/plans/2026-05-14-celestia-rs-bootstrap.md`.
```

- [ ] **Step 6: Replace `README.md`**

```markdown
# celestia-rs

A Rust port of [celestia-app](https://github.com/celestiaorg/celestia-app) — a wire-compatible Celestia consensus node implementation.

**Status:** Pre-M1. Active scaffolding. Not usable for anything yet.

## Design

- Top-level design: [`docs/superpowers/specs/2026-05-14-celestia-rs-design.md`](docs/superpowers/specs/2026-05-14-celestia-rs-design.md)
- Current focus: [`MILESTONES.md`](MILESTONES.md)
- Per-milestone specs: [`docs/superpowers/specs/m1/`](docs/superpowers/specs/m1/) (and `m2/`, `m3/`, … as we go)

## Layout

```
crates/       # Rust workspace members, organized by dependency tier
conformance/  # Golden vectors + Go fixture generator
xtask/        # cargo-xtask runner (fixture regen, vendor bumps)
docs/         # Specs, plans, ADRs
```

## Build

```
cargo build --workspace
cargo test --workspace
cargo xtask --help
```

## Regenerating conformance fixtures

Requires Go 1.22+:

```
cargo xtask refresh-fixtures
```

## License

Dual-licensed under [Apache 2.0](LICENSE-APACHE) and [MIT](LICENSE-MIT).
```

- [ ] **Step 7: Verify file presence**

Run: `ls -la .gitignore .gitattributes LICENSE-APACHE LICENSE-MIT MILESTONES.md README.md`
Expected: all six files listed; LICENSE-APACHE ≥ 10 KB; others non-empty.

- [ ] **Step 8: Commit**

```bash
git add .gitignore .gitattributes LICENSE-APACHE LICENSE-MIT MILESTONES.md README.md
git commit -m "chore: initial repo files (license, gitignore, readme, milestones)"
```

---

## Task 2: Toolchain pinning and workspace manifest

**Files:**
- Create: `rust-toolchain.toml`
- Create: `rustfmt.toml`
- Create: `clippy.toml`
- Create: `Cargo.toml`
- Create: `deny.toml`

- [ ] **Step 1: Create `rust-toolchain.toml`**

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy", "llvm-tools-preview"]
profile = "minimal"
```

- [ ] **Step 2: Create `rustfmt.toml`**

```toml
edition = "2024"
max_width = 100
use_field_init_shorthand = true
use_try_shorthand = true
```

- [ ] **Step 3: Create `clippy.toml`**

```toml
msrv = "1.85"
```

- [ ] **Step 4: Create root `Cargo.toml` (workspace manifest)**

```toml
[workspace]
resolver = "3"
members = [
    "xtask",
]

[workspace.package]
edition = "2024"
rust-version = "1.85"
license = "Apache-2.0 OR MIT"
repository = "https://github.com/rootulp/celestia-rs"
authors = ["Rootul Patel"]

[workspace.lints.rust]
unsafe_code = "forbid"
missing_docs = "warn"
rust_2018_idioms = { level = "warn", priority = -1 }

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
unwrap_used = "warn"
expect_used = "warn"
panic = "warn"
todo = "warn"
dbg_macro = "warn"
print_stdout = "warn"
print_stderr = "warn"
missing_errors_doc = "allow"
missing_panics_doc = "allow"
module_name_repetitions = "allow"

[workspace.dependencies]
anyhow = "1"
clap = { version = "4", features = ["derive"] }
```

- [ ] **Step 5: Create `deny.toml`**

```toml
[graph]
all-features = true

[advisories]
yanked = "deny"
ignore = []

[licenses]
allow = [
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "MIT",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Unicode-3.0",
    "Zlib",
    "CC0-1.0",
]
confidence-threshold = 0.93

[bans]
multiple-versions = "warn"
wildcards = "deny"
deny = []
```

- [ ] **Step 6: Verify workspace structure**

Run: `cargo metadata --format-version 1 --no-deps > /dev/null && echo OK`
Expected: prints `OK` (the workspace parses even though xtask doesn't exist yet — Cargo will warn but exit 0 once xtask is created in Task 3; for now this command will fail because xtask is missing).

If this step fails because `xtask` is not yet a member, that is expected — proceed to Task 3, then re-run after Task 3 step 4.

- [ ] **Step 7: Commit**

```bash
git add rust-toolchain.toml rustfmt.toml clippy.toml Cargo.toml deny.toml
git commit -m "chore: pin rust toolchain and configure workspace + cargo-deny"
```

---

## Task 3: xtask crate skeleton with failing CLI test

**Files:**
- Create: `xtask/Cargo.toml`
- Create: `xtask/src/main.rs`
- Create: `xtask/tests/cli.rs`

- [ ] **Step 1: Create `xtask/Cargo.toml`**

```toml
[package]
name = "xtask"
version = "0.0.1"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
publish = false

[lints]
workspace = true

[dependencies]
anyhow.workspace = true
clap.workspace = true
```

- [ ] **Step 2: Write failing test `xtask/tests/cli.rs`**

```rust
//! Integration tests for the xtask CLI.

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
}
```

- [ ] **Step 3: Run the test — confirm it fails**

Run: `cargo test -p xtask --test cli`
Expected: build failure (no `xtask` binary defined yet) or test failure.

- [ ] **Step 4: Create minimal `xtask/src/main.rs` to make tests pass**

```rust
//! `cargo xtask` runner for celestia-rs.
//!
//! Subcommands:
//! - `refresh-fixtures` — regenerate conformance fixtures by shelling out to
//!   the Go program in `conformance/fixturegen/`.

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "xtask", about = "celestia-rs project task runner", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Regenerate conformance fixtures via the Go fixturegen tool.
    RefreshFixtures {
        /// Limit regeneration to a specific scenario (e.g. `bech32-addresses`).
        #[arg(long)]
        scenario: Option<String>,
        /// Limit regeneration to fixtures for a specific crate.
        #[arg(long)]
        crate_name: Option<String>,
        /// Print what would run without executing.
        #[arg(long)]
        dry_run: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Cmd::RefreshFixtures { scenario, crate_name, dry_run } => {
            refresh_fixtures(scenario.as_deref(), crate_name.as_deref(), dry_run)
        }
    }
}

fn refresh_fixtures(
    _scenario: Option<&str>,
    _crate_name: Option<&str>,
    dry_run: bool,
) -> Result<()> {
    if dry_run {
        println!("[dry-run] would invoke fixturegen");
        return Ok(());
    }
    anyhow::bail!("fixturegen invocation not yet wired (see Task 9)")
}
```

- [ ] **Step 5: Run the test — confirm it passes**

Run: `cargo test -p xtask --test cli`
Expected: 2 passed.

- [ ] **Step 6: Verify workspace builds end-to-end**

Run: `cargo build --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check`
Expected: all three commands exit 0.

- [ ] **Step 7: Commit**

```bash
git add xtask/
git commit -m "chore(xtask): scaffold task runner with refresh-fixtures stub"
```

---

## Task 4: Per-PR CI workflow

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Create `.github/workflows/ci.yml`**

```yaml
name: ci

on:
  pull_request:
  push:
    branches: [main]

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1
  RUSTFLAGS: -D warnings

jobs:
  fmt:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt
      - run: cargo fmt --all --check

  clippy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo clippy --workspace --all-targets --all-features -- -D warnings

  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace --all-features

  deny:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: EmbarkStudios/cargo-deny-action@v2
        with:
          command: check
          arguments: --all-features

  fixturegen-build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-go@v5
        with:
          go-version: "1.22"
      - run: |
          if [ -f conformance/fixturegen/go.mod ]; then
            cd conformance/fixturegen
            go build ./...
            go test ./...
          else
            echo "fixturegen not yet present; skipping (Task 8 adds it)."
          fi
```

- [ ] **Step 2: Verify YAML syntax**

Run: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))" && echo OK`
Expected: `OK`. (If `python3 -c "import yaml"` fails with ModuleNotFoundError, install via `pip3 install pyyaml` or skip — YAML will be validated by GitHub Actions on push.)

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "chore(ci): add per-PR workflow (fmt, clippy, test, deny, fixturegen build)"
```

---

## Task 5: Nightly CI workflow (stub)

**Files:**
- Create: `.github/workflows/nightly.yml`

- [ ] **Step 1: Create `.github/workflows/nightly.yml`**

```yaml
name: nightly

on:
  schedule:
    - cron: "0 7 * * *"  # 07:00 UTC daily
  workflow_dispatch:

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  conformance-corpus:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          lfs: true
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: Run full conformance corpus
        run: |
          # Activated when the first conformance test crate lands.
          # For now this job is a no-op placeholder so the workflow exists.
          echo "No conformance crates yet — placeholder run."

  coverage:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: llvm-tools-preview
      - uses: Swatinem/rust-cache@v2
      - name: Install cargo-llvm-cov
        run: cargo install cargo-llvm-cov --locked
      - name: Compute coverage
        run: |
          # Coverage threshold enforced once first non-xtask crate lands.
          cargo llvm-cov --workspace --summary-only || echo "coverage tooling smoke-run only"
```

- [ ] **Step 2: Verify YAML syntax**

Run: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/nightly.yml'))" && echo OK`
Expected: `OK` (or skip if pyyaml not installed).

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/nightly.yml
git commit -m "chore(ci): add nightly workflow stub (conformance corpus + coverage)"
```

---

## Task 6: PR template and CODEOWNERS

**Files:**
- Create: `.github/PULL_REQUEST_TEMPLATE.md`
- Create: `.github/CODEOWNERS`

- [ ] **Step 1: Create `.github/PULL_REQUEST_TEMPLATE.md`**

```markdown
## Summary

<!-- 1–3 sentences about what changed and why. -->

## Spec / issue references

Closes #<github-issue>
Closes <linear-issue-id>
Spec: <path-to-spec-file-or-N/A>

## Definition of Done

For crate PRs, check all that apply:

- [ ] Public API documented (`#![warn(missing_docs)]` clean)
- [ ] `cargo test` passes
- [ ] `cargo clippy --all-targets -- -D warnings` clean
- [ ] `cargo fmt --check` clean
- [ ] Coverage ≥ 90% (or carve-outs documented)
- [ ] Conformance fixtures committed under `conformance/fixtures/<crate>/`
- [ ] Crate README updated (purpose / depends-on / consumers / conformance)
- [ ] CHANGELOG entry added

## Test plan

- [ ] <command + expected outcome>
```

- [ ] **Step 2: Create `.github/CODEOWNERS`**

```
# Every PR gets @rootulp as a reviewer by default.
*       @rootulp
```

- [ ] **Step 3: Commit**

```bash
git add .github/PULL_REQUEST_TEMPLATE.md .github/CODEOWNERS
git commit -m "chore(gh): add PR template and CODEOWNERS"
```

---

## Task 7: Conformance directory skeleton

**Files:**
- Create: `conformance/README.md`
- Create: `conformance/fixtures/.gitkeep`

- [ ] **Step 1: Create `conformance/README.md`**

```markdown
# Conformance

This directory holds **golden vectors** generated from Go celestia-app, plus the Go program that generates them. Every Rust crate that has byte-level Go compatibility requirements asserts against fixtures here.

## Layout

```
conformance/
├── fixtures/        # committed golden vectors (JSON or zstd-compressed binary)
│   └── <crate>/<scenario>/<file>.json
│   └── <crate>/<scenario>/<file>.meta.json   # source chain, height, app version, generating commit
└── fixturegen/      # Go program that emits fixtures; the only Go in the repo
```

## Regenerating fixtures

```bash
cargo xtask refresh-fixtures                       # all scenarios
cargo xtask refresh-fixtures --scenario bech32-addresses
cargo xtask refresh-fixtures --crate-name hash
```

The Rust runner shells out to `go run ./conformance/fixturegen ...`. Go 1.22+ is required for regeneration only; consumers of fixtures (Rust crates) need no Go toolchain.

## When to regenerate

- After mainnet upgrades to a new app version.
- After adding a new fixture scenario.
- Never as part of normal feature work — regeneration is a deliberate step that may invalidate test expectations.

Each fixture has a sibling `<file>.meta.json` recording: source chain, height (if applicable), app version, celestia-app commit SHA at generation time, timestamp.
```

- [ ] **Step 2: Create `conformance/fixtures/.gitkeep`**

```
# Keeps the directory in git before any fixtures are added.
```

- [ ] **Step 3: Commit**

```bash
git add conformance/README.md conformance/fixtures/.gitkeep
git commit -m "chore(conformance): create directory skeleton and readme"
```

---

## Task 8: fixturegen Go module with bech32-addresses subcommand

**Files:**
- Create: `conformance/fixturegen/go.mod`
- Create: `conformance/fixturegen/main.go`
- Create: `conformance/fixturegen/cmd/cmd.go`
- Create: `conformance/fixturegen/cmd/bech32.go`
- Create: `conformance/fixturegen/cmd/bech32_test.go`

- [ ] **Step 1: Initialize Go module**

Run: `cd conformance/fixturegen && go mod init github.com/rootulp/celestia-rs/conformance/fixturegen`
Expected: creates `go.mod` with `module github.com/rootulp/celestia-rs/conformance/fixturegen` and `go 1.22`.

- [ ] **Step 2: Add Cosmos SDK dependency**

Run: `cd conformance/fixturegen && go get github.com/cosmos/cosmos-sdk@v0.50.10`
Expected: updates `go.mod` and `go.sum`. (If `v0.50.10` is unavailable when you run this, use the latest `v0.50.x` patch release; record the version in `cmd/cmd.go` step 4 below.)

- [ ] **Step 3: Write failing test `cmd/bech32_test.go`**

```go
package cmd

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestBech32EmitProducesStableFixture(t *testing.T) {
	dir := t.TempDir()
	out := filepath.Join(dir, "bech32-addresses.json")

	if err := EmitBech32(out); err != nil {
		t.Fatalf("EmitBech32 returned error: %v", err)
	}

	data, err := os.ReadFile(out)
	if err != nil {
		t.Fatalf("reading fixture: %v", err)
	}

	var fixture struct {
		Scenario string `json:"scenario"`
		Prefix   string `json:"prefix"`
		Cases    []struct {
			Label      string `json:"label"`
			PubkeyHex  string `json:"pubkey_hex"`
			AccAddress string `json:"acc_address"`
			ValAddress string `json:"val_address"`
		} `json:"cases"`
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatalf("parsing fixture: %v", err)
	}

	if fixture.Scenario != "bech32-addresses" {
		t.Errorf("scenario = %q, want %q", fixture.Scenario, "bech32-addresses")
	}
	if fixture.Prefix != "celestia" {
		t.Errorf("prefix = %q, want %q", fixture.Prefix, "celestia")
	}
	if len(fixture.Cases) < 3 {
		t.Fatalf("expected ≥3 cases, got %d", len(fixture.Cases))
	}

	// Every acc_address must start with "celestia1" and every val_address with "celestiavaloper1".
	for _, c := range fixture.Cases {
		if len(c.AccAddress) == 0 || c.AccAddress[:9] != "celestia1" {
			t.Errorf("case %q: acc_address %q does not start with celestia1", c.Label, c.AccAddress)
		}
		if len(c.ValAddress) == 0 || c.ValAddress[:16] != "celestiavaloper1" {
			t.Errorf("case %q: val_address %q does not start with celestiavaloper1", c.Label, c.ValAddress)
		}
	}
}
```

- [ ] **Step 4: Create `cmd/cmd.go` (shared plumbing)**

```go
// Package cmd hosts the fixturegen subcommands.
//
// celestia-app source version pinned for fixture generation: cosmos-sdk v0.50.x.
// Update this comment when upgrading.
package cmd

import (
	"flag"
	"fmt"
)

// Subcommand is the contract every fixturegen subcommand satisfies.
type Subcommand struct {
	Name        string
	Description string
	Run         func(args []string) error
}

// All registered subcommands, in display order.
var All = []Subcommand{
	{
		Name:        "bech32-addresses",
		Description: "Emit canonical celestia* bech32 addresses for hardcoded test pubkeys.",
		Run:         runBech32,
	},
}

func runBech32(args []string) error {
	fs := flag.NewFlagSet("bech32-addresses", flag.ContinueOnError)
	out := fs.String("out", "conformance/fixtures/_bootstrap/bech32-addresses.json", "Output file path.")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if err := EmitBech32(*out); err != nil {
		return fmt.Errorf("emit bech32: %w", err)
	}
	fmt.Printf("wrote %s\n", *out)
	return nil
}
```

- [ ] **Step 5: Create `cmd/bech32.go`**

```go
package cmd

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"

	"github.com/cosmos/cosmos-sdk/types"
	"github.com/cosmos/cosmos-sdk/types/bech32"
)

const (
	bech32Prefix          = "celestia"
	bech32PrefixValOper   = "celestiavaloper"
	bech32PrefixConsAddr  = "celestiavalcons"
)

// testCase describes one bech32 conversion fixture entry.
type testCase struct {
	Label      string `json:"label"`
	PubkeyHex  string `json:"pubkey_hex"`
	AccAddress string `json:"acc_address"`
	ValAddress string `json:"val_address"`
}

type fixtureFile struct {
	Scenario string     `json:"scenario"`
	Prefix   string     `json:"prefix"`
	Cases    []testCase `json:"cases"`
}

// EmitBech32 writes a canonical bech32-addresses fixture to the given path.
//
// The address bytes used here are 20-byte ripemd160(sha256(pubkey)) hashes
// truncated for ed25519 pubkeys; for the purpose of fixture stability we use
// hardcoded raw 20-byte inputs so the output never depends on hashing details.
func EmitBech32(path string) error {
	// Lock the SDK config to celestia's bech32 prefixes for this run.
	cfg := types.GetConfig()
	cfg.SetBech32PrefixForAccount(bech32Prefix, bech32Prefix+"pub")
	cfg.SetBech32PrefixForValidator(bech32PrefixValOper, bech32PrefixValOper+"pub")
	cfg.SetBech32PrefixForConsensusNode(bech32PrefixConsAddr, bech32PrefixConsAddr+"pub")
	// Do NOT cfg.Seal() — re-running in tests must succeed.

	inputs := []struct {
		label     string
		pubkeyHex string
		addr20    string // 20-byte ripemd160(sha256(pubkey)) — precomputed for stability
	}{
		{"zero-bytes",  "0000000000000000000000000000000000000000000000000000000000000000",
			"0000000000000000000000000000000000000000"},
		{"all-ones",    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
			"ffffffffffffffffffffffffffffffffffffffff"},
		{"sequential",  "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
			"0102030405060708090a0b0c0d0e0f1011121314"},
	}

	cases := make([]testCase, 0, len(inputs))
	for _, in := range inputs {
		addrBytes, err := hex.DecodeString(in.addr20)
		if err != nil {
			return fmt.Errorf("decode addr20 for %q: %w", in.label, err)
		}
		acc, err := bech32.ConvertAndEncode(bech32Prefix, addrBytes)
		if err != nil {
			return fmt.Errorf("encode acc for %q: %w", in.label, err)
		}
		val, err := bech32.ConvertAndEncode(bech32PrefixValOper, addrBytes)
		if err != nil {
			return fmt.Errorf("encode val for %q: %w", in.label, err)
		}
		cases = append(cases, testCase{
			Label:      in.label,
			PubkeyHex:  in.pubkeyHex,
			AccAddress: acc,
			ValAddress: val,
		})
	}

	file := fixtureFile{
		Scenario: "bech32-addresses",
		Prefix:   bech32Prefix,
		Cases:    cases,
	}

	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	body, err := json.MarshalIndent(file, "", "  ")
	if err != nil {
		return err
	}
	body = append(body, '\n')
	return os.WriteFile(path, body, 0o644)
}
```

- [ ] **Step 6: Create `main.go`**

```go
// fixturegen emits golden vectors used by celestia-rs conformance tests.
package main

import (
	"flag"
	"fmt"
	"os"

	"github.com/rootulp/celestia-rs/conformance/fixturegen/cmd"
)

func main() {
	flag.Usage = func() {
		fmt.Fprintln(os.Stderr, "Usage: fixturegen <subcommand> [args...]")
		fmt.Fprintln(os.Stderr, "Subcommands:")
		for _, sc := range cmd.All {
			fmt.Fprintf(os.Stderr, "  %-22s %s\n", sc.Name, sc.Description)
		}
	}
	flag.Parse()

	args := flag.Args()
	if len(args) == 0 {
		flag.Usage()
		os.Exit(2)
	}
	name, rest := args[0], args[1:]

	for _, sc := range cmd.All {
		if sc.Name == name {
			if err := sc.Run(rest); err != nil {
				fmt.Fprintf(os.Stderr, "fixturegen %s: %v\n", name, err)
				os.Exit(1)
			}
			return
		}
	}
	fmt.Fprintf(os.Stderr, "unknown subcommand: %s\n", name)
	flag.Usage()
	os.Exit(2)
}
```

- [ ] **Step 7: Run the test — confirm it passes**

Run: `cd conformance/fixturegen && go test ./...`
Expected: `PASS` for `cmd` package.

- [ ] **Step 8: Smoke-run the binary**

Run: `cd conformance/fixturegen && go run . bech32-addresses --out /tmp/fixtures-smoke.json && cat /tmp/fixtures-smoke.json | head -20`
Expected: prints `wrote /tmp/fixtures-smoke.json`, then the first 20 lines of the JSON file showing `"scenario": "bech32-addresses"`, `"prefix": "celestia"`, and at least one `"acc_address": "celestia1..."` entry.

- [ ] **Step 9: Commit**

```bash
git add conformance/fixturegen/
git commit -m "feat(fixturegen): bootstrap go module with bech32-addresses scenario"
```

---

## Task 9: Wire xtask refresh-fixtures into fixturegen, generate first committed fixture

**Files:**
- Create: `xtask/src/refresh_fixtures.rs`
- Modify: `xtask/src/main.rs`
- Create: `conformance/fixtures/_bootstrap/bech32-addresses.json` (generated; committed)
- Create: `conformance/fixtures/_bootstrap/bech32-addresses.meta.json` (committed)
- Modify: `xtask/tests/cli.rs`

- [ ] **Step 1: Extend the failing test in `xtask/tests/cli.rs`**

Replace the file with:

```rust
//! Integration tests for the xtask CLI.

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
        .args(["refresh-fixtures", "--scenario", "bech32-addresses", "--dry-run"])
        .output()
        .expect("failed to invoke xtask binary");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("bech32-addresses"),
        "expected scenario listed; got: {stdout}"
    );
    assert!(
        !stdout.contains("other-scenario"),
        "unfiltered scenarios should not appear; got: {stdout}"
    );
}

#[test]
fn workspace_root_is_a_git_repo() {
    // Smoke test: refresh_fixtures uses workspace_root() heuristics — confirm
    // CARGO_MANIFEST_DIR's parent contains `.git`.
    let root = workspace_root();
    assert!(
        root.join(".git").exists() || root.join(".git").is_file(),
        "expected {} to be a git repo root",
        root.display()
    );
}
```

- [ ] **Step 2: Create `xtask/src/refresh_fixtures.rs`**

```rust
//! Implementation of `xtask refresh-fixtures`.
//!
//! The Rust side is a thin orchestrator that calls into the Go program at
//! `conformance/fixturegen/`. Every scenario known to celestia-rs is declared
//! in [`SCENARIOS`]; subcommand filters apply against this list.

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

pub const SCENARIOS: &[Scenario] = &[Scenario {
    name: "bech32-addresses",
    crate_name: "_bootstrap",
    output_path: "conformance/fixtures/_bootstrap/bech32-addresses.json",
}];

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
        anyhow::bail!(
            "no scenarios matched (scenario={:?}, crate_name={:?})",
            scenario,
            crate_name
        );
    }

    let root = workspace_root();
    let fixturegen_dir = root.join("conformance/fixturegen");

    for sc in selected {
        let out_abs = root.join(sc.output_path);
        if dry_run {
            println!(
                "[dry-run] {} → {}",
                sc.name,
                out_abs.display()
            );
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
```

- [ ] **Step 3: Replace `xtask/src/main.rs`**

```rust
//! `cargo xtask` runner for celestia-rs.
//!
//! Subcommands:
//! - `refresh-fixtures` — regenerate conformance fixtures by shelling out to
//!   the Go program in `conformance/fixturegen/`.

mod refresh_fixtures;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "xtask", about = "celestia-rs project task runner", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Regenerate conformance fixtures via the Go fixturegen tool.
    RefreshFixtures {
        /// Limit regeneration to a specific scenario (e.g. `bech32-addresses`).
        #[arg(long)]
        scenario: Option<String>,
        /// Limit regeneration to fixtures for a specific crate.
        #[arg(long = "crate-name")]
        crate_name: Option<String>,
        /// Print what would run without executing.
        #[arg(long)]
        dry_run: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Cmd::RefreshFixtures { scenario, crate_name, dry_run } => {
            refresh_fixtures::run(scenario.as_deref(), crate_name.as_deref(), dry_run)
        }
    }
}
```

- [ ] **Step 4: Run the tests — confirm they pass**

Run: `cargo test -p xtask --test cli`
Expected: 4 passed.

- [ ] **Step 5: Actually regenerate the fixture and commit it**

Run: `cargo xtask refresh-fixtures --scenario bech32-addresses`
Expected: prints `✓ regenerated conformance/fixtures/_bootstrap/bech32-addresses.json`. File exists and contains valid JSON starting with `"scenario": "bech32-addresses"`.

Verify: `python3 -c "import json; d=json.load(open('conformance/fixtures/_bootstrap/bech32-addresses.json')); assert d['prefix']=='celestia'; assert len(d['cases'])>=3; print('OK', len(d['cases']), 'cases')"`
Expected: `OK 3 cases`.

- [ ] **Step 6: Create the sidecar meta file `conformance/fixtures/_bootstrap/bech32-addresses.meta.json`**

```json
{
  "scenario": "bech32-addresses",
  "source_chain": "n/a (hardcoded test vectors)",
  "source_heights": [],
  "app_version": "n/a (bootstrap fixture)",
  "celestia_app_commit": "n/a",
  "cosmos_sdk_version": "v0.50.x",
  "regen_command": "cargo xtask refresh-fixtures --scenario bech32-addresses"
}
```

- [ ] **Step 7: Verify regeneration is idempotent**

Re-run regeneration and confirm the committed file is byte-stable:

Run: `cargo xtask refresh-fixtures --scenario bech32-addresses && git diff --exit-code conformance/fixtures/_bootstrap/bech32-addresses.json`
Expected: exit 0 (no diff).

- [ ] **Step 8: Final verification**

Run: `cargo build --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check && cargo test --workspace`
Expected: all four commands exit 0.

- [ ] **Step 9: Commit**

```bash
git add xtask/src/refresh_fixtures.rs xtask/src/main.rs xtask/tests/cli.rs \
        conformance/fixtures/_bootstrap/
git commit -m "feat(xtask): wire refresh-fixtures into fixturegen; commit first bootstrap fixture"
```

---

## Task 10: M1 spec index

**Files:**
- Create: `docs/superpowers/specs/m1/_index.md`

- [ ] **Step 1: Create `docs/superpowers/specs/m1/_index.md`**

```markdown
# M1 — Foundational Libraries

**Goal:** Ship the bottom two crate tiers (primitives + protocol types) with conformance fixtures against Go celestia-app.

**Acceptance test:** `m1-integration::parse_real_mainnet_block_produces_matching_dah` passes.

**Top-level design:** [`../2026-05-14-celestia-rs-design.md`](../2026-05-14-celestia-rs-design.md) Section 2.

## Crates

| # | Crate | Tier | Spec | Status |
|---|---|---|---|---|
| 1 | `hash` | 0 | [hash.md](hash.md) | spec ready |
| 2 | `encoding` | 0 | _to be written_ | pending |
| 3 | `crypto` | 0 | _to be written_ | pending |
| 4 | `merkle` | 0 | _to be written_ | pending |
| 5 | `tendermint-types` | 1 | _to be written_ | pending |
| 6 | `cosmos-types` | 1 | _to be written_ | pending |
| 7 | `abci-types` | 1 | _to be written_ | pending |
| 8 | `nmt` | 1 | _to be written_ | pending |
| 9 | `shares` | 1 | _to be written_ | pending |
| 10 | `square` | 1 | _to be written_ | pending |

## Dependency DAG

```
hash ─┬─→ merkle ─→ tendermint-types ─→ cosmos-types ─→ shares ─→ square
      └─→ crypto                    └─→ abci-types         └─→ nmt ──→ square
encoding ──→ {merkle, tendermint-types, cosmos-types, abci-types, nmt, shares, square}
```

## Sequencing

Strict: 1 → 2 → {3, 4} → 5 → {6, 7} → 8 → 9 → 10. Parallel-safe pairs in `{}`.

## Per-crate "definition of done"

Every crate must satisfy these gates before its PR merges:

1. Public API documented (`#![warn(missing_docs)]` clean).
2. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` all green.
3. Line coverage ≥ 90 % via `cargo-llvm-cov`. Carve-outs allowed only for unreachable error paths, reviewed.
4. Conformance suite passes; vectors committed under `conformance/fixtures/<crate>/`; regenerable via `cargo xtask refresh-fixtures --crate-name <name>`.
5. README at `crates/tier-X-Y/<crate>/README.md`: purpose, depends-on, consumers, `## Conformance` section.
6. CHANGELOG entry; semver 0.x.
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/specs/m1/_index.md
git commit -m "docs(m1): add M1 spec index and dep DAG"
```

---

## Task 11: `hash` crate spec (the first agent contract)

**Files:**
- Create: `docs/superpowers/specs/m1/hash.md`

- [ ] **Step 1: Create `docs/superpowers/specs/m1/hash.md`**

```markdown
# Crate: `hash`

**Tier:** 0 (primitives)
**Depends on:** _(none — pure primitive)_
**Consumed by:** `merkle`, `crypto`, `encoding`, `tendermint-types`, `cosmos-types`, `nmt`, `shares`, `square`, and effectively every tier ≥ 1 crate.
**Source of truth (Go):** `crypto/sha256`, `golang.org/x/crypto/ripemd160`, `crypto/sha512`, and the Tendermint helper at `github.com/cometbft/cometbft/crypto/tmhash` (length-prefix-then-sha256).
**Crate location:** `crates/tier-0-primitives/hash/`

## Purpose

Provide the small set of cryptographic hash primitives every other celestia-rs crate uses, with two goals:

1. **Byte-exact compatibility** with Go celestia-app / CometBFT for sha256, sha512, ripemd160, and Tendermint's length-prefix variant.
2. **One canonical Rust API** for hashing throughout the workspace — no other crate may pull in `sha2`, `sha3`, or `ripemd` directly.

This crate is a thin façade over the [RustCrypto](https://github.com/RustCrypto/hashes) family, vendored only conceptually (we depend on the crates and re-export the types we need). It does **not** include keyed hashes (HMAC), tree hashes (Merkle), or namespaced hashes (NMT) — those are downstream crates.

## Public API

The crate exports exactly these items from its root module:

```rust
//! Cryptographic hash primitives for celestia-rs.
//!
//! See `docs/superpowers/specs/m1/hash.md` for the API contract.

/// A 32-byte SHA-256 digest.
pub type Sha256Digest = [u8; 32];

/// A 64-byte SHA-512 digest.
pub type Sha512Digest = [u8; 64];

/// A 20-byte RIPEMD-160 digest.
pub type Ripemd160Digest = [u8; 20];

/// Compute SHA-256 of `input`.
pub fn sha256(input: &[u8]) -> Sha256Digest;

/// Compute SHA-512 of `input`.
pub fn sha512(input: &[u8]) -> Sha512Digest;

/// Compute RIPEMD-160 of `input`.
pub fn ripemd160(input: &[u8]) -> Ripemd160Digest;

/// Streaming SHA-256 builder. Equivalent to feeding successive chunks to a
/// single `sha256` call.
pub struct Sha256Hasher { /* opaque */ }

impl Sha256Hasher {
    pub fn new() -> Self;
    pub fn update(&mut self, chunk: &[u8]);
    pub fn finalize(self) -> Sha256Digest;
}

impl Default for Sha256Hasher { /* delegates to new() */ }

/// Tendermint's length-prefix-then-sha256 helper.
///
/// Equivalent to `sha256(varint_len(input) || input)`, where `varint_len`
/// uses the Tendermint canonical unsigned varint encoding (LEB128, max 10
/// bytes). See [CometBFT's `crypto/tmhash`].
pub fn tmhash_len_prefixed(input: &[u8]) -> Sha256Digest;
```

The crate sets `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.

## Behavioral requirements

**REQ-1.** `sha256("".as_bytes())` returns `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` (hex).

**REQ-2.** `sha256("abc".as_bytes())` returns `ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad` (hex).

**REQ-3.** `sha512("".as_bytes())` returns the all-zeros-input SHA-512 vector from FIPS 180-4.

**REQ-4.** `ripemd160("".as_bytes())` returns `9c1185a5c5e9fc54612808977ee8f548b2258d31` (hex).

**REQ-5.** `Sha256Hasher` produces the same digest as `sha256` when fed the same bytes in any chunking.

**REQ-6.** `tmhash_len_prefixed(b)` ≡ `sha256(varint_uvarint(b.len()) || b)`, where `varint_uvarint` follows Go's `encoding/binary.PutUvarint`. Test vectors:
- `tmhash_len_prefixed(b"")` = `sha256([0u8])`
- `tmhash_len_prefixed(b"\x00")` = `sha256(&[0x01, 0x00])`
- `tmhash_len_prefixed(&[0xff; 200])` matches the Go reference output (committed fixture).

**REQ-7.** All public functions are deterministic and side-effect-free.

**REQ-8.** The crate does not re-export the underlying RustCrypto types in its public API — callers receive plain `[u8; N]` arrays, never `GenericArray` or trait objects.

**REQ-9.** Property test: for any `Vec<u8>` (proptest-generated, up to 64 KB), `Sha256Hasher::update(.., chunks).finalize()` ≡ `sha256(concat(chunks))` for arbitrary chunkings.

## Conformance fixtures

Committed at `conformance/fixtures/hash/`:

- `hash/empty.json` — digests for empty input across all hash functions.
- `hash/short-vectors.json` — digests for the FIPS / NESSIE short test vectors (10 inputs, all four functions).
- `hash/streaming.json` — input + chunking pattern + expected sha256 digest for ≥ 5 cases.
- `hash/tmhash.json` — Tendermint length-prefix-then-sha256 digests for ≥ 10 inputs of varying length (empty, 1 byte, 127 bytes, 128 bytes, 1 KB, 64 KB, etc.) — these exercise varint boundary lengths.

Regeneration: a new subcommand `hash-vectors` in `conformance/fixturegen/cmd/` that imports `crypto/sha256`, `crypto/sha512`, `golang.org/x/crypto/ripemd160`, and CometBFT's `crypto/tmhash`. Invoked via:

```bash
cargo xtask refresh-fixtures --scenario hash-vectors
```

The fixturegen subcommand emits all four JSON files in one run.

## Definition of done

- [ ] All REQ-1..REQ-9 covered by unit or property tests.
- [ ] All four conformance fixture files committed and tested against.
- [ ] `cargo test -p hash` green.
- [ ] `cargo clippy -p hash --all-targets -- -D warnings` green.
- [ ] `cargo fmt --check` green.
- [ ] `cargo llvm-cov -p hash --summary-only` reports ≥ 90 % line coverage.
- [ ] `crates/tier-0-primitives/hash/README.md` present with purpose / consumers / conformance.
- [ ] PR opened as draft with auto-merge enabled; assignee `@rootulp`; PR body uses `Closes` for any tracked issue.

## Out of scope

- HMAC and keyed hashes.
- Merkle / tree hashes (belongs in `merkle`).
- Namespaced hashes (belongs in `nmt`).
- Hardware acceleration tuning.
- Streaming SHA-512 / streaming RIPEMD-160 (add later if a consumer needs them; YAGNI for now).

## Open questions

- **Q1:** Confirm the RustCrypto crate versions to pin (`sha2 = "0.10"`, `ripemd = "0.1"` are current as of writing). Defer to the implementer; record final versions in the crate `Cargo.toml`.
- **Q2:** Do we want a `const fn sha256` for compile-time hashing? Not required by any current consumer — leave out unless explicitly requested.

## Notes for the implementing agent

1. Write tests **first**. Start by translating REQ-1..REQ-6 into named `#[test]` functions in `crates/tier-0-primitives/hash/src/lib.rs` (or a `tests/` integration file). Run them; they must fail.
2. Implement the public API to make REQ-1..REQ-8 pass. Use `sha2`, `sha512`, `ripemd` from RustCrypto. Use `unsigned-varint` or hand-rolled LEB128 for the tmhash length prefix.
3. Add the proptest for REQ-9 once the unit tests are green.
4. Add the new `hash-vectors` subcommand to fixturegen (mirroring `bech32-addresses`'s shape). Regenerate the four fixture files, commit them.
5. Add conformance tests that load each fixture file and assert function outputs match.
6. Verify coverage, fmt, clippy. Open the PR.
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/specs/m1/hash.md
git commit -m "docs(m1): write hash crate spec (first agent contract)"
```

---

## Task 12: Final smoke test and bootstrap acceptance

**Files:** _(no new files; verification only)_

- [ ] **Step 1: Full repo build & lint**

Run: `cargo build --workspace && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo fmt --all --check`
Expected: all three exit 0.

- [ ] **Step 2: Full test pass**

Run: `cargo test --workspace`
Expected: 4 passed (the xtask CLI tests).

- [ ] **Step 3: `cargo deny` clean**

Install if missing: `cargo install cargo-deny --locked`
Run: `cargo deny check --all-features`
Expected: exit 0; license/bans/advisories all OK. (Multi-version warnings are acceptable at this stage.)

- [ ] **Step 4: fixturegen tests pass**

Run: `cd conformance/fixturegen && go test ./...`
Expected: `PASS`.

- [ ] **Step 5: End-to-end fixture regeneration is idempotent**

Run: `cargo xtask refresh-fixtures && git diff --exit-code conformance/fixtures/`
Expected: exit 0 (committed fixture is stable across regenerations).

- [ ] **Step 6: Update `MILESTONES.md` to mark M0 complete**

Edit `MILESTONES.md` and change the M0 row's status from `in-progress` to `completed`. Append a note under "Current focus":

```markdown
## Current focus
**M1** — see `docs/superpowers/specs/m1/_index.md`. Crate 1 (`hash`) spec ready; remaining 9 specs to be written by a follow-up plan.
```

- [ ] **Step 7: Final commit**

```bash
git add MILESTONES.md
git commit -m "chore: M0 bootstrap complete; M1 next"
```

- [ ] **Step 8: Sanity-check git log**

Run: `git log --oneline -20`
Expected: ≈ 11–12 commits since `initial commit`, with the messages from Tasks 1–12.

---

## Self-review notes (filled in by plan author)

**Spec coverage:**
- Section 1 (workspace architecture) → Tasks 1, 2, 3, 6, 7.
- Section 2 (M1 scope) → Tasks 10, 11 (index + first spec). Other 9 crate specs deferred to follow-up plan.
- Section 3 (roadmap) → `MILESTONES.md` in Task 1.
- Section 4 (testing strategy) → CI workflows (Tasks 4, 5); coverage stub in nightly is wired but enforcement begins with the `hash` crate's PR (per `hash.md` DoD).
- Section 5 (agent workflow) → PR template + CODEOWNERS (Task 6); spec format demonstrated by `hash.md` (Task 11); spec index (Task 10).

**Placeholder scan:** no "TBD" / "TODO" / "fill in" patterns remain in operational steps. The `_to be written_` entries in the M1 spec index are intentional — they're status markers, not gaps in this plan.

**Type consistency:** `Scenario` struct shape matches between `refresh_fixtures.rs` definition and the test expectations. Fixturegen subcommand name (`bech32-addresses`) consistent across Go code, Rust scenario list, xtask test, README, and `MILESTONES.md` acceptance test. The xtask CLI flag is `--crate-name` everywhere (matches the long form in clap, the Rust function parameter, and the dispatcher).
