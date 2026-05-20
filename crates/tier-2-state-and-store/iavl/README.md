# iavl

Cosmos IAVL+ state tree primitives for `celestia-rs`.

This crate is the first M2 state-and-storage layer. It provides the Rust API
surface for key/value mutation, deterministic root hashes, versioned commits,
range iteration, and future proof material. The current implementation
checkpoint is an in-memory tree with basic mutation and iterator behavior; Go
IAVL conformance fixtures are the next step before downstream store crates use
it for state roots.

## Depends On

- `hash`

## Consumers

- `kvstore`
- `ics23-proofs`
- `multistore`
- M2 integration tests

## Conformance

The implementation spec is `docs/superpowers/specs/m2/iavl.md`. Go-generated
conformance fixtures will be committed under `conformance/fixtures/iavl/` and
regenerated with:

```bash
cargo xtask refresh-fixtures --crate-name iavl
```
