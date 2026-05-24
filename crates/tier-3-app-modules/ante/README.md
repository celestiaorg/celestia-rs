# ante

Cosmos SDK transaction ante machinery for `celestia-rs`.

## Purpose

This crate is the M3 transaction-validation layer. It owns stateless
transaction checks, ante gas accounting, signer extraction scaffolding, and the
trait boundary that later `x/auth` and `x/bank` keepers will implement.

The current checkpoint provides the crate skeleton, gas-meter behavior, and
stateless `TxRaw` validation. Signature execution, fee deduction fixtures, and
the mainnet transaction corpus are next.

## Depends On

- `abci-types`
- `cosmos-types`
- `crypto`
- `hash`
- `kvstore`

## Consumers

- M3 integration tests
- M4 account and bank module keepers
- Later ABCI app/runtime crates

## Conformance

The implementation spec is `docs/superpowers/specs/m3/ante.md`.
Go-generated fixtures will live under `conformance/fixtures/ante/` and will be
regenerated with:

```bash
cargo xtask refresh-fixtures --crate-name ante
```

