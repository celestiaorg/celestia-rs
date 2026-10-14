# M3 - Tx & Ante Machinery

> **Superseded (2026-10-14).** The app-v6 prototype crates this spec
> describes (`crates/tier-3-app-modules/ante`, `crates/m3-integration`) were
> removed: the v10 port `crates/tier-3-app-modules/x/ante` replaced them
> (Go vectors in `conformance/antevectors`), and the 1,062-tx mainnet corpus
> now runs through it (`x/ante/tests/mainnet_corpus.rs`). Kept for history.

**Goal:** Ship the transaction validation layer that turns decoded Cosmos SDK
transactions into deterministic ante outcomes: signer extraction, signature
verification, fee and gas checks, account sequence checks, ante state updates,
message routing scaffolds, and a Go-generated transaction corpus.

**Acceptance test:** `m3-integration::mainnet_tx_validation_matches_go`
matches Go celestia-app validation outcomes for at least 1000 mainnet
transactions.

**Top-level design:** [`../2026-05-14-celestia-rs-design.md`](../2026-05-14-celestia-rs-design.md)
Section 3.

## Crates

| # | Crate | Tier | Spec | Status |
|---|---|---|---|---|
| 1 | `ante` | 3 | [ante.md](ante.md) | complete |

## Acceptance Scaffold

M3 includes `crates/m3-integration/` with the non-ignored
`mainnet_tx_validation_matches_go` acceptance entry point. The test loads the
committed corpus emitted by
`fixturegen ante-mainnet-tx-corpus`, runs every transaction through the Rust
ante pipeline, and compares the resulting code, codespace, gas wanted, gas
used, events, signer sequence updates, and fee movement summary to the Go
reference output. The fixture contract is specified in
[`mainnet-tx-corpus.md`](mainnet-tx-corpus.md).

The fixture corpus must include both `CheckTx`-style and `FinalizeBlock`-style
contexts when Go behavior differs. The first corpus can use a small account and
bank-state snapshot embedded in the fixture instead of a full module store
replay; M4 will replace those snapshot adapters with real `x/auth` and
`x/bank` keepers.

## Dependency DAG

```
hash -> crypto -> cosmos-types -> ante -> m3-integration
encoding -> cosmos-types -> ante -> m3-integration
kvstore -> ante -> m3-integration
abci-types -> ante -> m3-integration
```

## Sequencing

Strict: `ante` spec -> red API tests -> fixturegen `tx-corpus` -> green ante
implementation -> `m3-integration` acceptance scaffold.

## Per-crate Definition Of Done

Every M3 crate must satisfy these gates before its implementation is
considered complete:

1. Public API documented (`#![warn(missing_docs)]` clean).
2. `cargo test`, `cargo clippy --all-targets -- -D warnings`, and
   `cargo fmt --check` all green.
3. Line coverage >= 90 % via `cargo-llvm-cov`.
4. Go conformance suite passes; vectors committed under
   `conformance/fixtures/<crate>/`; regenerable via
   `cargo xtask refresh-fixtures --crate-name <name>`.
5. README at `crates/tier-3-app-modules/<crate>/README.md`: purpose,
   depends-on, consumers, and `## Conformance` section.
6. CHANGELOG entry; semver 0.x.
