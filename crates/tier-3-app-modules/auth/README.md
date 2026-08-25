# auth

Cosmos SDK `x/auth` account storage for `celestia-rs`.

## Purpose

This crate is the first M4 core-module checkpoint. It provides the base-account
keeper, Cosmos SDK v0.50 account key/value encoding, global account-number
allocation, unique account-number indexing, ordered account iteration, and the `ante::AccountReader` /
`ante::AccountWriter` implementation used by transaction validation.

This checkpoint intentionally supports `BaseAccount` only. Module and vesting
account variants will be added when their owning M4/M5 execution paths require
them.

## Depends On

- `ante`
- `cosmos-types`
- `kvstore`

## Consumers

- M4 integration tests
- `x/bank` and `x/staking`
- Later ABCI app/runtime crates

## Conformance

The implementation spec is `docs/superpowers/specs/m4/auth.md`. Go-generated
fixtures live under `conformance/fixtures/auth/` and are regenerated with:

```bash
cargo xtask refresh-fixtures --crate-name auth
```

`accounts.json` fixes the SDK v0.50 account store keys, interface-wrapped base
account bytes, unique account-number index, and global account-number sequence
encoding.
