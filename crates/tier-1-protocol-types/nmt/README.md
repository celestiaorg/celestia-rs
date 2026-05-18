# nmt

Tier-1 Celestia namespaced Merkle tree primitives for celestia-rs.

## Purpose

Provides fixed-width namespace IDs, namespaced leaf and inner-node hashing,
ordered tree construction, namespace inclusion and absence proofs, and subroot
path calculation.

This crate is a compatibility layer for `github.com/celestiaorg/nmt` v0.24.3.
It does not define share layout, data-square construction, erasure coding, or
data availability headers.

## Depends on

- `encoding`
- `hash`

The crate uses the workspace `hash` crate for SHA-256 so hash behavior stays
centralized.

## Consumed by

- `shares`
- `square`
- M1 integration tests
- downstream proof-verification crates
- data-availability sampling helpers

## Public API

```rust
pub mod error;
pub mod namespace;
pub mod node;
pub mod proof;
pub mod subroot;
pub mod tree;

pub use error::{Error, Result};
pub use namespace::{Namespace, MAX_NAMESPACE, MIN_NAMESPACE, NAMESPACE_SIZE};
pub use node::{
    HASH_SIZE, LEAF_PREFIX, NODE_PREFIX, NODE_SIZE, NamespacedHash, empty_root,
    hash_leaf, hash_node,
};
pub use proof::{Proof, ProofKind};
pub use subroot::{SubrootPath, subroot_paths};
pub use tree::{NamespacedMerkleTree, TreeOptions};
```

See [`docs/superpowers/specs/m1/nmt.md`](../../../docs/superpowers/specs/m1/nmt.md)
for the locked API contract.

## Conformance

Fixture files live at
[`conformance/fixtures/nmt/`](../../../conformance/fixtures/nmt/):

| File | Scenario | Description |
|---|---|---|
| `hashes.json` | `nmt-hashes` | NMT hash vectors and malformed node hash cases |
| `roots.json` | `nmt-roots` | Ordered tree root vectors |
| `proofs.json` | `nmt-proofs` | Namespace and range proof vectors |
| `subroot-paths.json` | `nmt-subroot-paths` | `GetSubrootPaths` compatible cases |
| `malformed.json` | `nmt-malformed` | Invalid namespace, leaf, range, proof, and subroot cases |

Regenerate fixtures:

```bash
cargo xtask refresh-fixtures --crate-name nmt
```

Run focused tests:

```bash
cargo test -p nmt
```

## Out of scope

- Celestia share info bytes and reserved namespace policy
- Data square construction, row/column root ordering, and DAH construction
- Reed-Solomon erasure coding
- Network transport, DAS sampling, and light-client RPC formats
- Alternative namespace sizes
