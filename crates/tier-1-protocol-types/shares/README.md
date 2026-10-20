# shares

Tier-1 Celestia share encoding and decoding primitives for celestia-rs.

## Purpose

Provides 29-byte share namespaces, info-byte and reserved-byte helpers,
fixed 512-byte shares, compact transaction splitting/parsing, sparse blob
splitting/parsing, share ranges, and padding share constructors.

This crate is a compatibility layer for `github.com/celestiaorg/go-square`
v1.0.1 share-format behavior. It does not construct data squares, erasure-code
shares, compute data availability headers, verify NMT proofs, or decode Cosmos
SDK transaction semantics.

## Depends on

- `encoding`
- `hash`
- `prost`

The crate uses the workspace `encoding` crate for Go-compatible unsigned
varints and protobuf helpers, and the workspace `hash` crate for SHA-256
transaction range keys.

## Consumed by

- `square`
- M1 integration tests
- transaction/blob reconstruction
- downstream data-availability proof machinery

## Public API

```rust
pub mod blob;
pub mod builder;
pub mod compact;
pub mod error;
pub mod info_byte;
pub mod namespace;
pub mod padding;
pub mod parse;
pub mod range;
pub mod reserved;
pub mod share;
pub mod sparse;
```

See [`docs/specs/m1/shares.md`](../../../docs/specs/m1/shares.md)
for the locked API contract.

## Conformance

Fixture files live at
[`conformance/fixtures/shares/`](../../../conformance/fixtures/shares/):

| File | Scenario | Description |
|---|---|---|
| `namespaces.json` | `shares-namespaces` | namespace constructors, constants, ordering, and malformed namespace cases |
| `info-reserved.json` | `shares-info-reserved` | info-byte and compact reserved-byte vectors |
| `builders.json` | `shares-builders` | compact and sparse builder layout vectors |
| `compact.json` | `shares-compact` | compact transaction splitting, parsing, ranges, and index wrappers |
| `sparse.json` | `shares-sparse` | sparse blob splitting and parsing |
| `padding.json` | `shares-padding` | namespace, reserved, and tail padding shares |
| `malformed.json` | `shares-malformed` | invalid share length, sparse, compact, and version cases |

Regenerate fixtures:

```bash
cargo xtask refresh-fixtures --crate-name shares
```

Run focused tests:

```bash
cargo test -p shares
```

## Out of scope

- Original data-square sizing and reserved-share placement
- Row/column roots, data availability headers, and erasure coding
- NMT root/proof verification for shares
- Cosmos SDK transaction decoding beyond raw bytes and index-wrapper detection
