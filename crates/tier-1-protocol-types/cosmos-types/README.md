# cosmos-types

Tier-1 Cosmos SDK protocol types for celestia-rs.

## Purpose

Provides Celestia bech32 address helpers, Cosmos coins, protobuf `Any`,
secp256k1 public-key packing, transaction body/auth-info structures, raw
transaction bytes, direct sign docs, and transaction hashing.

This crate is a wire-compatibility layer. It preserves protobuf field ordering
and byte semantics for the M1 transaction surface without executing messages,
running ante handlers, pricing gas, maintaining account state, or verifying
application-specific signatures.

## Depends on

- `crypto`
- `encoding`
- `hash`
- `tendermint-types`

The crate uses pinned Cosmos SDK prost types through `cosmos-sdk-proto` and
keeps them behind the public domain API.

## Consumed by

- `abci-types`
- `shares`
- `square`
- `m1-integration`
- M3 transaction and ante machinery
- downstream app and runtime crates

## Public API

```rust
pub mod address;
pub mod any;
pub mod coin;
pub mod error;
pub mod proto;
pub mod tx;
```

See
[`docs/superpowers/specs/m1/cosmos-types.md`](../../../docs/superpowers/specs/m1/cosmos-types.md)
for the full API contract.

## Conformance

The initial crate implementation has focused API tests under
[`tests/api.rs`](tests/api.rs). Go-generated conformance fixtures are tracked by
the crate spec and are the next remaining hardening slice:

| Planned file | Scenario | Description |
|---|---|---|
| `addresses.json` | `cosmos-addresses` | Celestia bech32 address vectors |
| `coins.json` | `cosmos-coins` | Cosmos SDK coin validation vectors |
| `public-keys.json` | `cosmos-public-keys` | secp256k1 public-key `Any` packing vectors |
| `sign-docs.json` | `cosmos-sign-docs` | SIGN_MODE_DIRECT sign-byte vectors |
| `tx-raw.json` | `cosmos-tx-raw` | Raw transaction encoding and hash vectors |
| `malformed.json` | `cosmos-malformed` | Malformed protobuf and validation vectors |

Run API tests:

```bash
cargo test -p cosmos-types
```

## Out of scope

- Cosmos SDK module message implementations
- Ante-handler execution
- Account lookup, sequence checks, fee deduction, and gas metering
- Legacy Amino JSON sign-byte generation
- Textual sign-mode rendering
- Keyring, mnemonic, HD derivation, and CLI transaction building
