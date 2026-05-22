# M2 - State & Storage

**Goal:** Ship the state and storage layer needed by later SDK modules: IAVL+
tree roots, commit multi-store root composition, gas-metered KV access, cache
wrappers, and ICS-23 proof material.

**Acceptance test:** `m2-integration::apply_mainnet_block_diff_produces_matching_kv_root` passes.

**Top-level design:** [`../2026-05-14-celestia-rs-design.md`](../2026-05-14-celestia-rs-design.md) Section 3.

## Crates

| # | Crate | Tier | Spec | Status |
|---|---|---|---|---|
| 1 | `iavl` | 2 | [iavl.md](iavl.md) | complete |
| 2 | `kvstore` | 2 | [kvstore.md](kvstore.md) | specified |
| 3 | `ics23-proofs` | 2 | _to be written_ | pending |
| 4 | `multistore` | 2 | _to be written_ | pending |

## Acceptance Scaffold

Create `crates/m2-integration/` once the first M2 crate is implemented. The
acceptance test consumes Go-generated state-diff fixtures and applies them
through the Rust state layer, then verifies the final store root against the Go
fixture.

## Dependency DAG

```
hash -> iavl -> ics23-proofs -> multistore -> m2-integration
hash -> iavl -> kvstore -> multistore -> m2-integration
encoding -> kvstore -> multistore -> m2-integration
```

## Sequencing

Strict: 1 -> {2, 3} -> 4 -> acceptance scaffold. Parallel-safe pairs in `{}`.

## Per-crate Definition Of Done

Every M2 crate must satisfy these gates before its implementation is considered
complete:

1. Public API documented (`#![warn(missing_docs)]` clean).
2. `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check` all green.
3. Line coverage >= 90 % via `cargo-llvm-cov`.
4. Go conformance suite passes; vectors committed under
   `conformance/fixtures/<crate>/`; regenerable via
   `cargo xtask refresh-fixtures --crate-name <name>`.
5. README at `crates/tier-2-state-and-store/<crate>/README.md`: purpose,
   depends-on, consumers, and `## Conformance` section.
6. CHANGELOG entry; semver 0.x.
