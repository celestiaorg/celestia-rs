# iavl

Cosmos IAVL+ state tree primitives for `celestia-rs`.

This crate is the first M2 state-and-storage layer. It provides the Rust API
surface for key/value mutation, deterministic root hashes, versioned commits,
range iteration, and proof material. The current implementation checkpoint is
an in-memory tree with Go-compatible committed root hashes, iterator behavior,
and ICS-23 membership/non-membership proof operation bytes for the committed
fixtures.

## Depends On

- `hash`

## Consumers

- `kvstore`
- `ics23-proofs`
- `multistore`
- M2 integration tests

## Conformance

The implementation spec is `docs/superpowers/specs/m2/iavl.md`. Go-generated
conformance fixtures live under `conformance/fixtures/iavl/` and are
regenerated with:

```bash
cargo xtask refresh-fixtures --crate-name iavl
```
