# staking

Cosmos SDK `x/staking` state for `celestia-rs`.

## Purpose

This crate is the third M4 core-module checkpoint. Its first slice provides
SDK v0.50 validator and delegation primary storage plus the validator-to-
delegator reverse index needed by later delegation execution.

## Depends On

- `cosmos-types`
- `kvstore`

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
