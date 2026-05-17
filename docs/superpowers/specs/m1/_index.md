# M1 — Foundational Libraries

**Goal:** Ship the bottom two crate tiers (primitives + protocol types) with conformance fixtures against Go celestia-app.

**Acceptance test:** `m1-integration::parse_real_mainnet_block_produces_matching_dah` passes.

**Top-level design:** [`../2026-05-14-celestia-rs-design.md`](../2026-05-14-celestia-rs-design.md) Section 2.

## Crates

| # | Crate | Tier | Spec | Status |
|---|---|---|---|---|
| 1 | `hash` | 0 | [hash.md](hash.md) | implemented |
| 2 | `encoding` | 0 | [encoding.md](encoding.md) | implemented |
| 3 | `crypto` | 0 | [crypto.md](crypto.md) | implemented |
| 4 | `merkle` | 0 | [merkle.md](merkle.md) | implemented |
| 5 | `tendermint-types` | 1 | [tendermint-types.md](tendermint-types.md) | implemented |
| 6 | `cosmos-types` | 1 | [cosmos-types.md](cosmos-types.md) | implemented |
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
