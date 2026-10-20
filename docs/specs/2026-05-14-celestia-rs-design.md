# celestia-rs — Top-Level Design

**Date:** 2026-05-14
**Status:** Approved (top-level design); per-milestone specs to follow.
**Author:** Rootul Patel (with Claude)

## Purpose

Port [celestia-app](https://github.com/celestiaorg/celestia-app) into Rust as a wire-compatible Celestia consensus node. The end goal is a Rust binary that can validate on Celestia mainnet alongside Go validators, producing identical block hashes and app hashes.

The project is sized for many engineer-years of work, so this document defines a decomposition into milestones, an architecture that supports parallel autonomous-agent work, and a testing discipline that makes correctness verifiable at every step.

## High-level decisions (settled during brainstorming)

| Decision | Choice | Rationale |
|---|---|---|
| Compatibility target | **Full wire-compatible validator** | Mainnet participation is the north star; conformance tests against Go produce a clear correctness signal. |
| External Rust crate posture | **Hybrid — vendor protocol-critical, depend on commodity** | Vendor anything where byte-exact Cosmos/Celestia behavior matters (tendermint-rs, ibc-rs, lumina/celestia-types, cosmos protos, NMT, IAVL). Depend normally on commodity crates (tokio, prost, serde, ed25519-dalek, sha2, hex). |
| First milestone shape | **Foundational libraries with conformance vectors** | Best fit for autonomous agents: well-scoped, locally testable, clear DoD. |
| Network target | **Current Celestia mainnet app version** | The most valuable conformance reference. Versioned design allows handling upgrade history later. |
| Test discipline | **Red/green TDD + ≥ 90 % line coverage + Go-driven conformance fixtures** | Wire-compat requires byte-exact comparison against Go outputs; TDD + coverage prevents regressions in long unattended runs. |

## Section 1 — Workspace architecture

A single Cargo workspace organized in **strict dependency tiers**. A tier-N crate can depend only on tier-≤ N crates. Enforced by review + a `cargo-deny`/custom CI check.

```
celestia-rs/
├── Cargo.toml                  # workspace manifest
├── README.md
├── LICENSE-APACHE / LICENSE-MIT
├── rust-toolchain.toml         # pin stable
├── deny.toml                   # cargo-deny config (licenses + dep direction)
├── crates/
│   ├── tier-0-primitives/
│   │   ├── hash/               # sha256, sha512, ripemd160, tendermint helpers
│   │   ├── encoding/           # protobuf, varint, hex, base64, bech32
│   │   ├── crypto/             # ed25519, secp256k1, BLS12-381
│   │   └── merkle/             # Tendermint binary Merkle tree
│   ├── tier-1-protocol-types/
│   │   ├── tendermint-types/   # vendored & trimmed from tendermint-rs
│   │   ├── cosmos-types/       # vendored cosmos protos + helpers
│   │   ├── abci-types/         # ABCI++ Request/Response, vote extensions
│   │   ├── nmt/                # namespaced Merkle tree (port from lumina)
│   │   ├── shares/             # share encoding/decoding (port from go-square)
│   │   └── square/             # square layout, DAH, erasure encoding
│   ├── tier-2-state-and-store/ # M2: iavl, kvstore, proofs (ICS-23)
│   ├── tier-3-app-modules/     # M3–M7: x/auth, x/bank, …, x/blob, x/blobstream
│   ├── tier-4-runtime/         # M8–M11: ABCI server, mempool, consensus
│   └── tier-5-node/            # M12+: celestia-rs binary, CLI, keyring
├── conformance/
│   ├── fixtures/               # committed golden vectors (JSON / zstd / LFS)
│   └── fixturegen/             # Go tool: uses celestia-app to emit fixtures
├── docs/
│   ├── specs/                  # design docs + per-milestone specs
│   └── adr/                    # architecture decision records
├── .github/workflows/          # CI: build, test, clippy, fmt, deny, coverage
└── xtask/                      # cargo-xtask: regen fixtures, vendor bump
```

### Principles

- **Strict tiering** enforced by `deny.toml` and a CI check.
- **One crate, one responsibility.** Each crate has a 1-paragraph README: purpose, depends-on, consumed-by.
- **Conformance is first-class.** `conformance/fixturegen/` is the only Go in the repo; it imports celestia-app as a library and emits golden vectors. Vectors are committed; regeneration is a deliberate human step via `cargo xtask refresh-fixtures`.
- **No top-level `src/`.** Everything is a workspace member.

## Section 2 — Milestone 1 scope and "definition of done"

M1 ships tiers 0 + 1: ten foundational crates with conformance fixtures.

### M1 crates

| # | Crate | Scope | Done criterion |
|---|---|---|---|
| 1 | `hash` | sha256, sha512, ripemd160, tendermint `len-prefix-then-sha256` helpers | RustCrypto re-exports + thin wrappers; ≥ 50 golden vectors from Go pass |
| 2 | `encoding` | protobuf via prost; varint; hex; base64; bech32 | Round-trip property tests; bech32 addresses match `celestia1…` from Go |
| 3 | `crypto` | ed25519 (consensus + accounts), secp256k1 (accounts), BLS12-381 G1/G2 (blobstream attestations) | Sign/verify against Go fixtures; deterministic signatures (RFC 6979 for secp256k1) |
| 4 | `merkle` | Tendermint's binary Merkle tree, proof generation + verification | Roots match Go for ≥ 100 vector sets incl. edge cases |
| 5 | `tendermint-types` | `Hash`, `BlockId`, `Header`, `Commit`, `Validator`, `ValidatorSet`, `Vote`, `Proposal`, canonical encoding | Header-hash fixtures from mainnet blocks 1, 1M, current-tip all match |
| 6 | `cosmos-types` | proto bindings + helpers for `Tx`, `TxBody`, `AuthInfo`, `SignDoc`, addresses, coins | Tx hash for ≥ 50 real mainnet txs matches Go |
| 7 | `abci-types` | ABCI++ Request/Response, vote extensions, `ExtendedCommitInfo` | Proto + JSON round-trip |
| 8 | `nmt` | Namespaced Merkle tree: node hashing, inclusion + absence proofs, root computation | Roots & proofs for ≥ 100 mainnet block data-roots match Go go-square |
| 9 | `shares` | Share construction, padding, info/sequence bytes, namespace, share-version | Decode mainnet block data → re-encode → byte-identical for ≥ 50 blocks |
| 10 | `square` | Square layout, ODS, DAH, erasure encoding (Reed-Solomon GF(2^8)) | DAH for ≥ 100 mainnet blocks matches Go; ODS round-trips |

### Ordering & parallelism

Strict sequential: 1 → 2 → {3, 4} → 5 → {6, 7} → 8 → 9 → 10. Parallel-safe pairs marked `{}`.

### Definition of done — per crate

1. Public API documented (`#![warn(missing_docs)]`).
2. `cargo test` green; `cargo clippy --all-targets -- -D warnings` green; `cargo fmt --check` green.
3. Line coverage ≥ 90 % via `cargo-llvm-cov` in CI. Carve-outs allowed only for unreachable error paths, reviewed.
4. Conformance suite passes; vectors committed under `conformance/fixtures/<crate>/`; regenerable via `cargo xtask refresh-fixtures --crate-name <name>`.
5. README: purpose, depends-on, consumers, `## Conformance` section.
6. CHANGELOG entry; semver 0.x.

### Definition of done — for M1 overall

- All 10 crates ship.
- `m1-integration` test crate: takes real mainnet block bytes, parses through `tendermint-types` + `shares` + `nmt` + `square`, verifies DAH matches Go fixture.
- `MILESTONES.md` at repo root records M1 complete.

### Explicitly NOT in M1

IAVL tree (M2); IBC / ICS-23 proofs (M2); anything network-touching; any `x/*` module; block production / consensus; node binary.

### M1 risks

- **Reed-Solomon erasure encoding** — Celestia uses RS over GF(2^8) with a specific Vandermonde matrix. Plan: depend on a maintained crate like `reed-solomon-erasure` with a thin adapter; vendor a fallback only if behavior diverges. To be confirmed in the `square` crate spec.
- **Square padding rules** have changed across app versions. M1 targets the current mainnet version; the `square` crate documents version assumptions.
- **NMT** — `lumina` has a maintained Rust impl. Plan: vendor it as the starting point, attribute, evolve in-tree.

## Section 3 — Long-term milestone roadmap

Each milestone gets its own spec → plan → implementation cycle. Numbers are sequence, not durations.

| # | Milestone | Scope | Acceptance test |
|---|---|---|---|
| **M1** | Foundational libraries | tier 0 + tier 1 crates | Parse + validate real mainnet block → DAH matches Go |
| **M2** | State & storage | IAVL+ tree, multi-store, ICS-23 proofs, gas-metered KV, cache wrappers | KV roots match Go after applying fixture-recorded writes |
| **M3** | Tx & ante machinery | Tx codec, sig verify, ante chain (sig/gas/fee/sequence), keeper framework, msg router, gas meter | Tx-validation outcome matches Go for ≥ 1 k mainnet txs |
| **M4** | Core SDK modules | `x/auth`, `x/bank`, `x/staking` | Given block N state + N+1 txs, produce N+1 state with matching subtree roots |
| **M5** | Remaining SDK modules *(splittable)* | `x/slashing`, `x/distribution`, `x/gov`, `x/evidence`, `x/mint`, `x/params`, `x/upgrade`, `x/feegrant`, `x/authz`, `x/crisis`, `x/vesting` | Per-module state roots match Go |
| **M6** | Celestia-specific modules | `x/blob`, `x/blobstream`, celestia `x/mint`, `x/signal`, `x/minfee`, `x/tokenfilter`, `x/paramfilter` | Apply mainnet blocks with blob txs → app hash matches |
| **M7** | IBC *(splittable)* | ICS-02 (tendermint LC), ICS-03/04, ICS-23, ICS-20 | IBC-touching mainnet blocks replay; LC verifies headers |
| **M8** | ABCI++ app integration | Wire modules into `App`; run behind vendored Go CometBFT; replay mainnet from checkpoint | **App hash matches Go for 1 000 consecutive mainnet blocks** — first "celestia-rs works" milestone |
| **M9** | Consensus engine *(splittable)* | Tendermint BFT in Rust — propose, prevote, precommit, commit, evidence, validator-set updates (vendor malachite as starting point) | Two celestia-rs nodes produce a private network; mixed Rust-among-Go testnet works |
| **M10** | P2P networking | Tendermint P2P (MConn, secret-connection, PEX), mempool gossip, block gossip | Connects to Go peers on a private network, gossips a block |
| **M11** | RPC & gRPC | Tendermint RPC (JSON-RPC + WS), gRPC, REST gateway | RPC responses match Go for a canned request corpus |
| **M12** | Node binary, CLI, ops | `celestia-rs` binary, CLI, keyring, snapshots, state-sync client | `cargo install celestia-rs && celestia-rs start --network mainnet` works |
| **M13** | Mainnet sync from genesis | End-to-end: sync mainnet block 1 → tip; app hashes match throughout | Sync completes; final app hash equals current mainnet's |
| **M14** | Mainnet validator | Sign blocks on mainnet | Produces signed blocks; no double-signs; no evidence |

### Sequencing

- **Strict prereqs:** M1 → M2 → M3 → M4 → (M5 ∥ M6) → M7 → M8 → M9 → M10 → M11 → M12 → M13 → M14.
- **Parallel-safe:** after M4, M5 ∥ M6. After M8, M11 ∥ M9.
- **Splittable:** M5, M7, M9 are each large enough to decompose at spec time.

### "Easy out of the box" — M12 UX commitments

- Config precedence: code defaults > config file > env vars > flags. No required config to start.
- `celestia-rs start` joins mainnet by default.
- `--network mainnet | mocha | arabica | local` wires seeds, chain-id, genesis hash.
- Single static binary; no plugin system; no required external services.
- First-run UX: auto-init keystore + node identity to `~/.celestia-rs/`.

### Explicitly out of scope

- Data Availability Sampling (`celestia-node` territory).
- Rollkit / sovereign rollup support.
- CLI/SDK parity with Go celestia-app's user-facing tooling beyond M12.

## Section 4 — Testing strategy

Three layers.

### Layer 1 — Red/green TDD at the unit level

- **CI gate:** every PR must include a test change or a documented `// no test: <reason>` justification.
- **Coverage gate:** ≥ 90 % line coverage per crate via `cargo-llvm-cov`. Carve-outs via `#[cfg_attr(coverage, coverage(off))]` only for unreachable branches, reviewed.
- **Mutation testing** (post-M1, weekly): `cargo-mutants`.
- **Property tests** (`proptest`) for codecs, Merkle proofs, NMT proofs; committed regression seeds.
- **Fuzz harnesses** (`cargo-fuzz`) for every deserializer, gradual rollout; OSS-Fuzz integration once stable.

### Layer 2 — Conformance against Go celestia-app

**Where fixtures live**
- `conformance/fixtures/<crate>/<scenario>.json` — small (≤ 100 KB) committed directly.
- `conformance/fixtures/<crate>/<scenario>.bin.zst` — large binaries committed compressed; Git LFS for files > 10 MB.
- Each fixture has a sibling `<scenario>.meta.json` recording: source chain, height, app version, generating commit SHA, timestamp.

**How fixtures are generated**
- `conformance/fixturegen/` (Go) — imports celestia-app as a library. Subcommands:
  - `fixturegen mainnet-blocks --start <h> --count <n>`
  - `fixturegen tx-corpus --count 1000`
  - `fixturegen unit <type> --seed <s>`
  - `fixturegen replay-state --start <h> --count <n>` (M2+)
- Invoked via `cargo xtask refresh-fixtures [--crate-name <name>] [--scenario <name>]`.

**Conformance test pattern**
```rust
#[test]
fn conformance_mainnet_header_hash() {
    for fixture in load_fixtures::<HeaderFixture>("tendermint-types/headers/") {
        let computed = Header::decode(&fixture.bytes).unwrap().hash();
        assert_eq!(computed, fixture.expected_hash, "mismatch at height {}", fixture.height);
    }
}
```

**Fixture coverage targets per crate**
- ≥ 50 deterministic unit vectors (edge cases: empty, max, min, malformed).
- ≥ 100 real-mainnet vectors sampled across chain history.
- ≥ 10 regression vectors added per discovered discrepancy.

### Layer 3 — End-to-end milestone acceptance tests

One named acceptance test per milestone, e.g.:

- M1: `m1_integration::parse_real_mainnet_block_produces_matching_dah`
- M2: `m2_integration::apply_mainnet_block_diff_produces_matching_kv_root`
- M8: `m8_integration::replay_1000_mainnet_blocks_app_hashes_match`
- M13: `m13_integration::sync_mainnet_from_genesis_to_tip_app_hashes_match` (nightly only)

These live in `crates/<tier>-integration/` workspace members named `m1-integration`, etc.

### CI pipeline

Per-PR (target < 10 min):
1. `cargo fmt --check`
2. `cargo clippy --all-targets --all-features -- -D warnings`
3. `cargo test --workspace --exclude '*-nightly'`
4. `cargo deny check`
5. `cargo llvm-cov` with per-crate threshold

Nightly:
6. Full conformance corpus.
7. `cargo mutants` (post-M1).
8. Mainnet replay from latest acceptance level (M8: 10 k blocks; M13: genesis-to-tip weekly).

Pre-release (per-milestone tag):
9. Run milestone acceptance test on clean machine.

### Required accepts

- Go toolchain is a **build-time dev dependency** (fixture regeneration only); not a runtime or Rust dep.
- Fixture corpus grows over time (~hundreds of MB eventually); Git LFS or zstd compression manages repo size.
- Slow tests are tagged "nightly only" to keep PR cycle fast.

## Section 5 — Autonomous-agent development workflow

The unit of agent work is **one crate, one PR**.

### Per-crate spec format (contract the agent works against)

Each crate has a spec at `docs/specs/<milestone>/<crate>.md`:

```markdown
# Crate: <name>
**Tier:** 0 | 1 | 2 | …
**Depends on:** <list of crates>
**Consumed by:** <list of crates / "TBD">
**Source of truth (Go):** <celestia-app path or upstream Go module>

## Purpose
<2–4 sentences>

## Public API
<exact signatures the crate must expose; locked at spec time>

## Behavioral requirements
<numbered list: REQ-1, REQ-2, …>

## Conformance fixtures
<list, plus regen command>

## Definition of done
<DoD checklist>

## Out of scope
<explicit non-goals>

## Open questions
<things the agent must surface back to a human>
```

The spec is a contract: an agent satisfying all numbered requirements and shipping the public API as specified is done.

### Dispatch loop

A meta-process (you, `/loop`, or a scheduled agent) runs:

1. Read `MILESTONES.md` → current milestone.
2. Read `docs/specs/<milestone>/_index.md` → crate dependency DAG.
3. Compute "ready" crates: unmerged, all deps merged.
4. For each ready crate without an open PR: dispatch a sub-agent ("implement crate X per its spec, open a PR").
5. For each open `agent-built` PR: if CI green & approved, auto-merge handles it; if CI red, dispatch a "fix CI" sub-agent.
6. Sleep, repeat.

Codified as a `/celestia-rs-tick` slash command in `.claude/commands/` once M1 begins.

### Resumability

- **State lives in files, not agent memory.** Crate spec + committed code + open PR = full state.
- **WIP commits** on agent branches mid-work; next agent reads the branch, runs `cargo test`, picks up.
- **No coordination via chat.** All handoff is through git + GitHub PR state.
- **Idempotent commands.** `cargo xtask refresh-fixtures` and the dispatch loop are safe to re-run.

### Repo conventions

- **One PR per crate** (never bundle).
- **Commit format:** `crate(<name>): <imperative summary>`; body explains *why*.
- **Branch naming:** `agent/<crate-name>`.
- **PR template** at `.github/PULL_REQUEST_TEMPLATE.md` requires DoD checkoff + spec link.
- **CODEOWNERS** auto-tags @rootulp.
- **All PRs start as drafts with auto-merge enabled** (matches user CLAUDE.md).
- **PR description** uses "Closes" with Linear issue + GitHub issue (matches user CLAUDE.md).

### Human review checkpoints

| Checkpoint | What you do | Frequency |
|---|---|---|
| PR review | Skim agent-opened PRs; approve or request changes | When you have time |
| Milestone gate | Manually verify acceptance criteria before the milestone-acceptance PR merges | Once per milestone |
| Spec-change review | Resolve agent-surfaced open questions that require spec changes | Ad hoc |
| Conformance regen | After mainnet upgrades, regenerate fixtures against the new Go version | Upstream-driven |

### Failure modes the system survives

- Agent loses context → resumes from committed state.
- Agent makes wrong choice → caught at PR review or CI.
- Two agents on same crate → second sees open PR and exits.
- Fixtures go stale → nightly CI flags mismatches; refresh task created.
- Upstream Go changes wire format → fixture-regen produces new vectors; conformance tests flag divergences; "port upgrade" milestone scheduled.
- Spec is wrong → agent flags via "Open questions"; meta-agent or human resolves.

### Commitments required

- Agents auto-merge their own PRs when CI is green; you retain veto via PR review.
- Spec quality is invested in upfront. M1 has 10 crates → 10 specs written before any code.
- `MILESTONES.md` + per-milestone spec index is the single source of truth for "what is next."

## Next steps

After this design is approved:

1. **Write per-crate specs for M1** — 10 specs, one per crate listed in Section 2. Each spec uses the format in Section 5.
2. **Scaffold the workspace** — root `Cargo.toml`, `deny.toml`, `rust-toolchain.toml`, `xtask/`, empty `crates/` tree, CI workflows, PR template, CODEOWNERS, `MILESTONES.md`.
3. **Stand up `conformance/fixturegen/`** — minimal Go program that can emit at least one fixture type (e.g., header hashes), to validate the regeneration path before crate work starts.
4. **Begin M1 crate 1 (`hash`)** — the first agent assignment. Spec → red tests → green impl → PR.

Steps 1–3 belong in an implementation plan generated by the `writing-plans` skill. Step 4 begins the long-running-agent loop.
