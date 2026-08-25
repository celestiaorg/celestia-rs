# bank

Cosmos SDK `x/bank` balance storage for `celestia-rs`.

## Purpose

This crate is the second M4 core-module checkpoint. It provides SDK v0.50
balance and supply storage, denomination/address reverse indexing, transfers,
mint/burn arithmetic, SDK protobuf routing for `MsgSend` and `MsgMultiSend`,
Go-compatible transfer events, and the `ante::FeeKeeper` implementation that
moves fees to the fee collector module account.

## Depends On

- `abci-types`
- `ante`
- `cosmos-types`
- `kvstore`

## Consumers

- M4 integration tests
- `x/staking` and later SDK modules
- Later ABCI app/runtime crates

## Conformance

The implementation spec is `docs/superpowers/specs/m4/bank.md`.
Go-generated fixtures live under `conformance/fixtures/bank/` and are
regenerated with:

```bash
cargo xtask refresh-fixtures --crate-name bank
```
