# multistore

Cosmos SDK commit multi-store primitives for celestia-rs.

## Purpose

`multistore` composes mounted module store roots into one application hash. It
owns in-memory store mounting, commit info values, Cosmos SDK commit-info
protobuf bytes, simple Merkle store-root proof ops, and cache multi-store
write-back.

## Depends On

- `encoding` for Go-compatible varint leaf encoding.
- `hash` and `merkle` for SDK simple-map app hashes.
- `iavl` and `kvstore` for mounted IAVL-backed KV stores.
- `ics23-proofs` for proof-op routing and verification.

## Consumers

The crate is consumed by the M2 integration test scaffold and later SDK module
keepers, ABCI query proof surfaces, and app-hash replay.

## Conformance

The crate contract is specified in
`docs/superpowers/specs/m2/multistore.md`. Go conformance fixtures live under
`conformance/fixtures/multistore/` and are regenerated with:

```bash
cargo xtask refresh-fixtures --crate-name multistore
```
