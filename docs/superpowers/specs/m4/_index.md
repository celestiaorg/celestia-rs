# M4 - Core SDK Modules

**Goal:** Ship the Cosmos SDK core state machines needed to replace M3's
fixture-backed account and balance adapters: `x/auth`, `x/bank`, and
`x/staking`.

**Acceptance test:** Given committed mainnet state at block N and the N+1
transactions, execute the supported core-module messages and reproduce Go's
`acc`, `bank`, and `staking` subtree roots at N+1.

**Top-level design:** [`../2026-05-14-celestia-rs-design.md`](../2026-05-14-celestia-rs-design.md)
Section 3.

## Crates

| # | Crate | Tier | Spec | Status |
|---|---|---|---|---|
| 1 | `auth` | 3 | [auth.md](auth.md) | in progress |
| 2 | `bank` | 3 | _to be written_ | pending |
| 3 | `staking` | 3 | _to be written_ | pending |

## Acceptance Scaffold

M4 will add `crates/m4-integration/`. Its acceptance runner will load a
Go-generated block-replay fixture containing the relevant pre-state, decoded
messages, expected writes, and post-block subtree roots. The first `auth`
checkpoint uses focused Go vectors for the SDK v0.50 account collections
encoding; block replay arrives after the three keepers expose their message
execution boundaries.

## Dependency DAG

```text
cosmos-types -> auth -> bank -> staking -> m4-integration
kvstore -> auth -> bank -> staking -> m4-integration
ante -> auth -> bank -> m4-integration
multistore -> m4-integration
```

## Sequencing

Strict: `auth` base accounts -> `bank` balances and sends -> `staking`
delegation state -> M4 block-replay acceptance.

## Per-crate Definition Of Done

Every M4 crate must satisfy these gates before its implementation is complete:

1. Public API documented (`#![warn(missing_docs)]` clean).
2. `cargo test`, `cargo clippy --all-targets -- -D warnings`, and
   `cargo fmt --check` all green.
3. Line coverage >= 90% via `cargo-llvm-cov`.
4. Go conformance fixtures are committed and regenerable with
   `cargo xtask refresh-fixtures --crate-name <name>`.
5. The crate README documents purpose, dependencies, consumers, and
   conformance.
6. CHANGELOG entry; semver 0.x.
