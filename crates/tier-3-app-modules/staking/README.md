# staking

Cosmos SDK `x/staking` state for `celestia-rs`.

## Purpose

This crate provides SDK v0.50 validator and delegation storage plus exact
`MsgDelegate` execution. Delegations move bank balances into the appropriate
staking pool, update fixed-precision shares and validator power indexes, and
emit SDK-compatible events.

## Depends On

- `cosmos-types`
- `kvstore`
- `bank`
- `abci-types`

## Consumers

- M4 integration tests
- Later distribution, slashing, and runtime crates

## Conformance

The implementation spec is `docs/superpowers/specs/m4/staking.md`.
Go-generated fixtures live under `conformance/fixtures/staking/` and are
regenerated with:

```bash
cargo xtask refresh-fixtures --crate-name staking
```
