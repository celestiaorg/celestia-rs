# merkle

Tier-0 CometBFT simple Merkle tree primitives for celestia-rs.

## Purpose

Provides the deterministic binary Merkle tree used by CometBFT for short
ordered byte-slice lists, including root computation and inclusion proof
verification.

This crate implements the simple `crypto/merkle` tree only. It is not IAVL,
ICS-23, NMT, or Celestia's data availability square.

## Depends on

- `hash`

The crate uses `hash::sha256` and `hash::Sha256Hasher` so SHA-256 behavior
stays centralized in the workspace.

## Consumed by

- `tendermint-types`
- `cosmos-types`
- `abci-types`
- `nmt`
- `shares`
- `square`
- downstream proof-verification crates

## Public API

```rust
pub const HASH_LENGTH: usize = 32;
pub const MAX_AUNTS: usize = 100;

pub type Hash = [u8; HASH_LENGTH];

pub enum Error { /* opaque variants in docs */ }

pub struct Proof {
    pub total: u64,
    pub index: u64,
    pub leaf_hash: Hash,
    pub aunts: Vec<Hash>,
}

impl Proof {
    pub fn new(total: u64, index: u64, leaf_hash: Hash, aunts: Vec<Hash>) -> Result<Self, Error>;
    pub fn validate_basic(&self) -> Result<(), Error>;
    pub fn compute_root_hash(&self) -> Result<Hash, Error>;
    pub fn verify(&self, root: &Hash, leaf: &[u8]) -> Result<(), Error>;
}

pub fn empty_hash() -> Hash;
pub fn leaf_hash(leaf: &[u8]) -> Hash;
pub fn inner_hash(left: &Hash, right: &Hash) -> Hash;
pub fn root_from_byte_slices<T: AsRef<[u8]>>(items: &[T]) -> Hash;
pub fn proofs_from_byte_slices<T: AsRef<[u8]>>(items: &[T]) -> (Hash, Vec<Proof>);
```

See [`docs/superpowers/specs/m1/merkle.md`](../../../docs/superpowers/specs/m1/merkle.md)
for the locked API contract.

## Conformance

Fixture files live at
[`conformance/fixtures/merkle/`](../../../conformance/fixtures/merkle/):

| File | Scenario | Description |
|---|---|---|
| `roots.json` | `merkle-roots` | CometBFT simple Merkle root vectors and primitive hash vectors |
| `proofs.json` | `merkle-proofs` | CometBFT inclusion proofs for representative ordered lists |
| `malformed-proofs.json` | `merkle-malformed-proofs` | Invalid proof cases for verifier error paths |

Regenerate fixtures:

```bash
cargo xtask refresh-fixtures --crate-name merkle
```

Run conformance tests:

```bash
cargo test -p merkle --test conformance
```

## Out of scope

- IAVL, ICS-23, and mutable or versioned Merkle stores
- Namespaced Merkle trees
- Data availability headers and erasure-coded square construction
- CometBFT generic proof operators and protobuf proof conversion
- Streaming or incremental tree construction
