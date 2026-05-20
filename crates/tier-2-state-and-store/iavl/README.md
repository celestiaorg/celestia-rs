# iavl

Cosmos IAVL+ state tree primitives for `celestia-rs`.

This crate is the first M2 state-and-storage layer. It provides the Rust API
surface for key/value mutation, deterministic root hashes, versioned commits,
range iteration, and future proof material. The current implementation
checkpoint is an in-memory tree with Go-compatible committed root hashes and
iterator behavior. Proof generation and verification still use placeholder
material while the committed ICS-23 proof fixtures are wired into the next
implementation step.

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
