# ics23-proofs

Cosmos SDK ICS-23 proof operation primitives for celestia-rs.

This crate wraps IAVL proof bytes as SDK `ics23:*` commitment operations,
verifies membership and non-membership proofs, and exposes CometBFT
`ProofOp` / `ProofOps` protobuf-compatible structs for query proof surfaces.

## Depends On

- `iavl` for local proof material and root hash types.
- `ics23` for Cosmos ICS-23 proof specs and verification rules.
- `prost` for protobuf wire compatibility.

## Consumers

- `multistore`
- M2 integration tests
- ABCI query proof responses
- Later light-client and IBC verification surfaces

## Conformance

The crate is specified by `docs/specs/m2/ics23-proofs.md`.
Go-generated fixtures live under `conformance/fixtures/ics23-proofs/` and are
regenerated with:

```bash
cargo xtask refresh-fixtures --crate-name ics23-proofs
```
