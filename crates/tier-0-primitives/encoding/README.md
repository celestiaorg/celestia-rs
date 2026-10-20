# encoding

Tier-0 byte encoding primitives for celestia-rs.

## Purpose

Provides the byte encoding helpers used across the workspace:

- protobuf helpers backed by `prost`
- Tendermint-compatible signed and unsigned varints
- canonical lowercase hex
- padded standard and URL-safe base64
- Celestia bech32 address helpers for `celestia`, `celestiavaloper`, and
  `celestiavalcons`

## Depends on

None. This is a pure-primitive Tier-0 crate with no celestia-rs dependencies.

It wraps commodity Rust crates (`prost`, `bytes`, `hex`, `base64`, `bech32`)
behind one workspace API so downstream crates do not choose encoding behavior
independently.

## Consumed by

- `merkle`
- `crypto`
- `tendermint-types`
- `cosmos-types`
- `abci-types`
- `nmt`
- `shares`
- `square`
- downstream app/runtime crates

## Public API

```rust
pub mod protobuf;
pub mod varint;
pub mod hex;
pub mod base64;
pub mod bech32;
```

See [`docs/specs/m1/encoding.md`](../../../docs/specs/m1/encoding.md)
for the locked API contract.

## Conformance

Fixture files live at
[`conformance/fixtures/encoding/`](../../../conformance/fixtures/encoding/):

| File | Scenario | Description |
|---|---|---|
| `protobuf.json` | `encoding-protobuf` | Go `protowire` bytes for raw and length-delimited fixture messages |
| `varint.json` | `encoding-varint` | Go `encoding/binary` signed and unsigned varint boundaries |
| `hex-base64.json` | `encoding-hex-base64` | Go hex, standard base64, and URL-safe base64 vectors |
| `bech32-addresses.json` | `encoding-bech32-addresses` | Cosmos SDK bech32 Celestia address vectors |

Regenerate fixtures:

```bash
cargo xtask refresh-fixtures --crate-name encoding
```

Run conformance tests:

```bash
cargo test -p encoding --test conformance
```

## Out of scope

- Defining or generating protocol protobuf types
- Amino, JSON, YAML, TOML, SSZ, SCALE, or other codecs
- Key parsing or address derivation from public keys
- Bech32m
- Base64 no-padding helpers
