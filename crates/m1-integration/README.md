# m1-integration

M1 acceptance tests for the foundational `celestia-rs` libraries.

This crate verifies cross-crate behavior that is larger than a single protocol
type crate. The current checkpoint reconstructs cached Celestia mainnet app-v6
data squares from committed transaction bytes and checks that each computed
data availability header hash matches the Go fixture.

## Depends On

- `square`

## Consumers

- M1 milestone acceptance gate
- CI

## Conformance

The test consumes fixtures under `conformance/fixtures/square/`, including the
100-block `mainnet-blocks.json` corpus. Regenerate the source fixture with:

```bash
cargo xtask refresh-fixtures --scenario square-mainnet-blocks
```
