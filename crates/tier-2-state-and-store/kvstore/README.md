# kvstore

Cosmos SDK key/value store primitives for `celestia-rs`.

This crate is an M2 state-and-storage layer. It provides IAVL-backed key/value
access, cache branches, prefix stores, and gas accounting surfaces used by later
multi-store and module crates.

## Depends On

- `encoding`
- `iavl`

## Consumers

- `multistore`
- M2 integration tests
- Later SDK module keepers

## Conformance

The implementation spec is `docs/superpowers/specs/m2/kvstore.md`.
Go-generated conformance fixtures will live under
`conformance/fixtures/kvstore/` and are regenerated with:

```bash
cargo xtask refresh-fixtures --crate-name kvstore
```
