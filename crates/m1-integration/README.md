# m1-integration

M1 acceptance tests for the foundational `celestia-rs` libraries.

This crate verifies cross-crate behavior that is larger than a single protocol
type crate. The current checkpoint reconstructs a cached Celestia mainnet
app-v6 block's data square from committed transaction bytes and checks that the
computed data availability header hash matches the Go fixture.

## Depends On

- `square`

## Consumers

- M1 milestone acceptance gate
- CI once the mainnet fixture corpus is expanded

## Conformance

The test consumes fixtures under `conformance/fixtures/square/`, currently
including `mainnet-blocks.json`. Regenerate the source fixture with:

```bash
cargo xtask refresh-fixtures --scenario square-mainnet-blocks
```
