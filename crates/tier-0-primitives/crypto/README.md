# crypto

Tier-0 public-key cryptography primitives for celestia-rs.

## Purpose

Provides the signature primitives used by consensus, accounts, and
Blobstream-related verification:

- `CometBFT` Ed25519 consensus signatures with ZIP-215 verification
- Cosmos SDK secp256k1 account signatures and addresses
- BLS12-381 minimal-public-key-size signatures and aggregation

## Depends on

- `hash`

Commodity provider crates (`ed25519-consensus`, `k256`, and `blst`) stay behind
this crate so downstream crates use one workspace API.

## Consumed by

- `tendermint-types`
- `cosmos-types`
- `abci-types`
- Blobstream and bridge-related crates
- transaction validation and runtime crates

## Public API

```rust
pub mod bls12381;
pub mod ed25519;
pub mod secp256k1;
```

See [`docs/superpowers/specs/m1/crypto.md`](../../../docs/superpowers/specs/m1/crypto.md)
for the locked API contract.

## Conformance

Fixture files live at
[`conformance/fixtures/crypto/`](../../../conformance/fixtures/crypto/):

| File | Scenario | Description |
|---|---|---|
| `ed25519.json` | `crypto-ed25519` | `CometBFT` Ed25519 seed, secret, signature, verify, and address vectors |
| `secp256k1.json` | `crypto-secp256k1` | Cosmos SDK secp256k1 key, signature, verify, and address vectors |
| `bls12381.json` | `crypto-bls12381` | `blst` BLS12-381 key, signature, aggregate, and verify vectors |

Regenerate fixtures:

```bash
cargo xtask refresh-fixtures --crate-name crypto
```

Run conformance tests:

```bash
cargo test -p crypto --test conformance
```

## Out of scope

- Random key generation
- HD wallets, mnemonics, keyrings, and bech32 rendering
- Transaction sign-byte construction
- Protobuf `Any` packing
- Recoverable secp256k1 signatures
- Blobstream attestation message construction
