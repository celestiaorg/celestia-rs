# abci-types

Tier-1 CometBFT ABCI++ protocol types for celestia-rs.

## Purpose

Provides request and response envelopes, proposal-processing messages,
vote-extension helpers, commit information, validator updates, events,
transaction execution results, misbehavior descriptors, and state-sync snapshot
messages for the CometBFT v0.38 ABCI++ wire surface.

This crate is a protocol compatibility layer. It preserves protobuf field
ordering and byte semantics without executing transactions, running an
application state machine, verifying vote-extension signatures, storing
snapshots, or serving an ABCI transport.

## Depends on

- `encoding`
- `tendermint-types`

The crate uses pinned CometBFT prost types through `tendermint-proto` and keeps
provider details behind the public domain API.

## Consumed by

- `m1-integration`
- M2 state and replay crates
- M3 transaction and ante machinery
- tier-4 runtime crates
- downstream ABCI server adapters

## Public API

```rust
pub mod error;
pub mod event;
pub mod exec;
pub mod proto;
pub mod request;
pub mod response;
pub mod snapshot;
pub mod validator;
pub mod vote_extension;
```

See
[`docs/superpowers/specs/m1/abci-types.md`](../../../docs/superpowers/specs/m1/abci-types.md)
for the full API contract.

## Conformance

Focused API tests live under [`tests/api.rs`](tests/api.rs). Go-generated
conformance fixtures are planned under
[`conformance/fixtures/abci-types/`](../../../conformance/fixtures/abci-types/).

Regenerate fixtures once scenarios are registered:

```bash
cargo xtask refresh-fixtures --crate-name abci-types
```

Run API tests:

```bash
cargo test -p abci-types --test api
```

## Out of scope

- Application state execution and module keepers
- ABCI server transport, sockets, gRPC services, and runtime dispatch loops
- Mempool admission policy, gas metering, fee handling, and signature
  verification
- Vote-extension signature verification policy
- Snapshot storage, chunk persistence, and state-sync orchestration
- Cosmos SDK transaction decoding beyond preserving opaque tx bytes
