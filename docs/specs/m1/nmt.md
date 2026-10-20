# Crate: `nmt`

**Tier:** 1 (protocol types)
**Depends on:** `encoding`, `hash`
**Consumed by:** `shares`, `square`, M1 integration tests, M2 proof
verification, data-availability sampling helpers, and downstream light-client
proof machinery.
**Source of truth (Go):** `github.com/celestiaorg/nmt` v0.24.3, as used by
current Celestia Core / celestia-app module snapshots in the local Go module
cache.
**Crate location:** `crates/tier-1-protocol-types/nmt/`

## Purpose

Provide Celestia's namespaced Merkle tree primitives: namespace IDs,
namespaced leaf and inner-node hashing, ordered tree construction, root
computation, namespace inclusion proofs, namespace absence proofs, subtree root
helpers, and subroot path calculation for later square/DAH construction.

This crate is a byte-compatibility layer for Celestia NMT behavior. It does not
define Celestia share formats, reserved namespace policy, data-square layout,
erasure coding, DAH encoding, or block parsing. Those belong to `shares`,
`square`, and integration crates.

## Public API

The crate exports exactly these root modules:

```rust
//! Celestia namespaced Merkle tree primitives.
//!
//! See `docs/specs/m1/nmt.md` for the API contract.

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

The crate sets `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.

### `namespace`

```rust
pub const NAMESPACE_SIZE: usize = 8;
pub const MIN_NAMESPACE: Namespace;
pub const MAX_NAMESPACE: Namespace;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Namespace([u8; NAMESPACE_SIZE]);

impl Namespace {
    pub const fn new(bytes: [u8; NAMESPACE_SIZE]) -> Self;
    pub fn from_slice(bytes: &[u8]) -> Result<Self>;
    pub const fn as_bytes(&self) -> &[u8; NAMESPACE_SIZE];
    pub const fn into_bytes(self) -> [u8; NAMESPACE_SIZE];
    pub const fn is_min(self) -> bool;
    pub const fn is_max(self) -> bool;
}

impl TryFrom<&[u8]> for Namespace;
impl From<[u8; NAMESPACE_SIZE]> for Namespace;
impl From<Namespace> for [u8; NAMESPACE_SIZE];
impl core::fmt::LowerHex for Namespace;
```

### `node`

```rust
pub const HASH_SIZE: usize = 32;
pub const NODE_SIZE: usize = (2 * NAMESPACE_SIZE) + HASH_SIZE;
pub const LEAF_PREFIX: u8 = 0;
pub const NODE_PREFIX: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NamespacedHash {
    pub min_namespace: Namespace,
    pub max_namespace: Namespace,
    pub digest: [u8; HASH_SIZE],
}

impl NamespacedHash {
    pub const fn new(
        min_namespace: Namespace,
        max_namespace: Namespace,
        digest: [u8; HASH_SIZE],
    ) -> Self;
    pub fn validate_basic(&self) -> Result<()>;
    pub fn from_bytes(bytes: &[u8]) -> Result<Self>;
    pub fn to_bytes(&self) -> [u8; NODE_SIZE];
}

pub fn empty_root() -> NamespacedHash;
pub fn hash_leaf(leaf: &[u8]) -> Result<NamespacedHash>;
pub fn hash_node(
    left: &NamespacedHash,
    right: &NamespacedHash,
    options: TreeOptions,
) -> Result<NamespacedHash>;
```

`leaf` is namespace-prefixed data: the first `NAMESPACE_SIZE` bytes are the
namespace ID and all remaining bytes are opaque payload.

### `tree`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeOptions {
    pub ignore_max_namespace: bool,
}

impl Default for TreeOptions;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespacedMerkleTree {
    /* private fields */
}

impl NamespacedMerkleTree {
    pub fn new() -> Self;
    pub fn with_options(options: TreeOptions) -> Self;
    pub fn options(&self) -> TreeOptions;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn leaves(&self) -> &[Vec<u8>];
    pub fn push(&mut self, leaf: Vec<u8>) -> Result<()>;
    pub fn extend<I>(&mut self, leaves: I) -> Result<()>
    where
        I: IntoIterator<Item = Vec<u8>>;
    pub fn root(&self) -> Result<NamespacedHash>;
    pub fn min_namespace(&self) -> Result<Namespace>;
    pub fn max_namespace(&self) -> Result<Namespace>;
    pub fn get(&self, namespace: Namespace) -> Vec<Vec<u8>>;
    pub fn prove_range(&self, start: usize, end: usize) -> Result<Proof>;
    pub fn prove_namespace(&self, namespace: Namespace) -> Result<Proof>;
    pub fn subtree_root(&self, start: usize, end: usize) -> Result<NamespacedHash>;
    pub fn reset(&mut self);
}

impl Default for NamespacedMerkleTree;
```

### `proof`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofKind {
    Empty,
    Inclusion,
    Absence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proof {
    pub start: usize,
    pub end: usize,
    pub nodes: Vec<NamespacedHash>,
    pub leaf_hash: Option<NamespacedHash>,
    pub is_max_namespace_ignored: bool,
}

impl Proof {
    pub fn empty(ignore_max_namespace: bool) -> Self;
    pub fn inclusion(
        start: usize,
        end: usize,
        nodes: Vec<NamespacedHash>,
        ignore_max_namespace: bool,
    ) -> Result<Self>;
    pub fn absence(
        start: usize,
        end: usize,
        nodes: Vec<NamespacedHash>,
        leaf_hash: NamespacedHash,
        ignore_max_namespace: bool,
    ) -> Result<Self>;
    pub fn kind(&self) -> ProofKind;
    pub fn validate_basic(&self) -> Result<()>;
    pub fn nodes_as_bytes(&self) -> Vec<[u8; NODE_SIZE]>;
    pub fn verify_namespace(
        &self,
        namespace: Namespace,
        leaves: &[Vec<u8>],
        root: &NamespacedHash,
    ) -> Result<bool>;
    pub fn verify_leaf_hashes(
        &self,
        namespace: Namespace,
        leaf_hashes: &[NamespacedHash],
        root: &NamespacedHash,
        verify_completeness: bool,
    ) -> Result<bool>;
}
```

### `subroot`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubrootPath {
    pub row: usize,
    pub path: Vec<bool>,
}

pub fn subroot_paths(
    square_size: usize,
    share_start: usize,
    share_count: usize,
) -> Result<Vec<Vec<SubrootPath>>>;
```

`SubrootPath::path` stores a Merkle branch from the row root toward the covered
subroot. `false` means left and `true` means right. An empty path means the
covered range spans the entire row root.

### `error`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidNamespaceLength { actual: usize },
    InvalidLeafLength { actual: usize, min: usize },
    InvalidNodeLength { actual: usize, expected: usize },
    InvalidNodeNamespaceOrder,
    UnorderedSiblings,
    UnorderedLeaves,
    InvalidRange,
    InvalidProof,
    FailedCompletenessCheck,
    InvalidSubrootPath,
    Decode(String),
    Encode(String),
}

impl core::fmt::Display for Error;
impl std::error::Error for Error;

pub type Result<T> = core::result::Result<T, Error>;
```

## Behavioral requirements

**REQ-1.** Namespace IDs are exactly 8 bytes for M1. All namespace comparisons
are lexicographic byte comparisons matching Go `namespace.ID.Less`.

**REQ-2.** `empty_root()` returns `0x00 * 16 || sha256([])`, matching Go
`NmtHasher.EmptyRoot()` with `NamespaceIDSize(8)`.

**REQ-3.** `hash_leaf(leaf)` rejects leaves shorter than 8 bytes and otherwise
returns `ns || ns || sha256([LEAF_PREFIX] || leaf)`, where `ns = leaf[..8]`.

**REQ-4.** `NamespacedHash::from_bytes` accepts only 48-byte node hashes,
splits them as `min_namespace || max_namespace || digest`, and rejects
`max_namespace < min_namespace`.

**REQ-5.** `hash_node(left, right, options)` rejects invalid node namespace
order when `right.min_namespace < left.max_namespace`. Otherwise it returns
`parent_min || parent_max || sha256([NODE_PREFIX] || left_bytes || right_bytes)`.

**REQ-6.** When `TreeOptions::ignore_max_namespace` is true, `hash_node`
ignores a right child whose namespace range starts at `MAX_NAMESPACE` for
parent max-namespace calculation, matching Go `computeNsRange`.

**REQ-7.** `TreeOptions::default()` sets `ignore_max_namespace = true`.

**REQ-8.** `NamespacedMerkleTree::push` requires namespace-prefixed leaves and
rejects any leaf whose namespace is lexicographically less than the previously
pushed leaf namespace. Equal adjacent namespaces are allowed.

**REQ-9.** `root()` computes roots recursively with Go's split point: the left
subtree size is the largest power of two strictly less than the leaf count,
except for leaf counts that are not powers of two where it is the largest power
of two less than the count.

**REQ-10.** `root()` of an empty tree equals `empty_root()`. `root()` of a
single-leaf tree equals that leaf's namespaced hash.

**REQ-11.** `get(namespace)` returns all namespace-prefixed leaves for exactly
that namespace, preserving original leaf order and returning an empty vector
when absent.

**REQ-12.** `prove_range(start, end)` rejects invalid ranges (`start >= end` or
`end > len`) and returns proof nodes in the same order as Go
`Proof.Nodes()`: in-order traversal nodes needed to reconstruct the range root.

**REQ-13.** `prove_namespace(namespace)` supports all Go cases: empty proof
when the namespace is outside the root namespace range, inclusion proof when
matching leaves exist, and absence proof when the namespace falls inside the
tree range but has no matching leaves.

**REQ-14.** `Proof::verify_namespace` verifies both inclusion/absence and
namespace completeness. It rejects leaves that do not have the queried
namespace prefix, mismatched leaf counts for the proof range, malformed proof
nodes, and proofs that leave adjacent same-namespace data unproven.

**REQ-15.** `subroot_paths(square_size, share_start, share_count)` matches Go
`GetSubrootPaths`: `square_size` must be a power of two at least 2,
`share_count` must be nonzero, the range must fit inside
`square_size * square_size`, and returned row/path grouping must be identical.

**REQ-16.** All public verification APIs are deterministic and do not mutate
the tree or proof.

## Conformance fixtures

Committed under `conformance/fixtures/nmt/`:

- `hashes.json` - empty root, leaf hash, inner node hash, namespace parsing,
  max-namespace ignore behavior, and malformed node hash cases.
- `roots.json` - roots for empty, single-leaf, balanced, unbalanced, repeated
  namespace, and max-namespace-padding trees.
- `proofs.json` - namespace inclusion, namespace absence, empty range absence,
  explicit range proof, malformed proof, and negative verification cases.
- `subroot-paths.json` - `GetSubrootPaths` compatible cases including single
  share, partial row, full row, multi-row, and invalid inputs.
- `malformed.json` - invalid namespace lengths, too-short leaves, unordered
  leaves, unordered siblings, invalid ranges, bad proof structure, and invalid
  subroot path inputs.

Regenerate fixtures:

```bash
cargo xtask refresh-fixtures --crate-name nmt
```

Equivalent individual scenarios:

```bash
cargo xtask refresh-fixtures --scenario nmt-hashes
cargo xtask refresh-fixtures --scenario nmt-roots
cargo xtask refresh-fixtures --scenario nmt-proofs
cargo xtask refresh-fixtures --scenario nmt-subroot-paths
cargo xtask refresh-fixtures --scenario nmt-malformed
```

Each scenario must be registered in BOTH the Go fixturegen
(`conformance/fixturegen/cmd/`) and the Rust `SCENARIOS` const in
`xtask/src/refresh_fixtures.rs`.

## Definition of Done

- [ ] Crate created at `crates/tier-1-protocol-types/nmt/` with a
      `Cargo.toml` inheriting workspace metadata, and the crate path is added
      to root `Cargo.toml` `[workspace]` members.
- [ ] Public API above implemented with documented modules and
      `#![forbid(unsafe_code)]`.
- [ ] All REQ-1..REQ-16 covered by unit, property, or conformance tests.
- [ ] All five conformance fixture files committed with `.meta.json` sidecars.
- [ ] `cargo test -p nmt` green.
- [ ] `cargo clippy -p nmt --all-targets -- -D warnings` green.
- [ ] `cargo fmt --check` green.
- [ ] `cargo llvm-cov -p nmt --summary-only` reports >= 90 % line coverage.
- [ ] `crates/tier-1-protocol-types/nmt/README.md` present with purpose /
      consumers / conformance.
- [ ] NMT fixturegen scenarios registered in BOTH the Go fixturegen and the
      Rust `SCENARIOS` const.
- [ ] PR opened as draft with assignee `@rootulp`; PR body uses `Closes` for
      any tracked issue, or `Closes N/A` when no issue exists.

## Out of scope

- Celestia share info bytes, sequence layout, padding shares, and reserved
  namespace interpretation. Those belong to `shares`.
- Data square construction, row/column root ordering, DAH construction, and
  Reed-Solomon erasure coding. Those belong to `square`.
- Tendermint block parsing and data availability header integration tests.
- Network transport, DAS sampling, and light-client RPC formats.
- Alternative namespace sizes. M1 fixes the Celestia namespace size at 8 bytes.

## Notes for the implementing agent

- Start with red tests for `hash_leaf`, `hash_node`, `empty_root`, ordered
  `push`, and the unbalanced split-point root before adding fixturegen.
- Use the workspace `hash` crate for SHA-256 instead of reaching directly for a
  new hashing dependency.
- Keep `NamespacedHash` as a fixed-size value type; convert to `Vec<u8>` only at
  fixture and API boundaries that require owned byte vectors.
- Match Go's `ignoreMaxNamespace = true` default. Add tests that demonstrate
  the different parent max namespace when it is false.
- Implement proof verification against leaf hashes first, then layer
  namespace-prefixed leaf validation and completeness checks on top.
- Treat `subroot_paths` as a pure helper with no dependency on tree state.
