# Crate: `merkle`

**Tier:** 0 (primitives)
**Depends on:** `hash`
**Consumed by:** `tendermint-types`, `cosmos-types`, `abci-types`, `nmt`,
`shares`, `square`, and downstream proof-verification crates.
**Source of truth (Go):** CometBFT v0.38.12
`github.com/cometbft/cometbft/crypto/merkle`.
**Crate location:** `crates/tier-0-primitives/merkle/`

## Purpose

Provide CometBFT's deterministic simple Merkle tree for short ordered byte
slices, including root computation and inclusion proofs. This is the binary
Merkle tree used for Tendermint/CometBFT protocol data such as transaction
lists and validator signatures.

This crate is not an authenticated key-value store, an IAVL implementation, an
ICS-23 proof system, or a namespaced Merkle tree. It intentionally implements
only the simple ordered tree in CometBFT `crypto/merkle`.

## Public API

The crate exports exactly these root items:

```rust
//! CometBFT simple Merkle tree primitives for celestia-rs.
//!
//! See `docs/specs/m1/merkle.md` for the API contract.

/// Size in bytes of a simple Merkle hash.
pub const HASH_LENGTH: usize = 32;

/// Maximum number of aunt hashes accepted in a proof.
pub const MAX_AUNTS: usize = 100;

/// A 32-byte simple Merkle hash.
pub type Hash = [u8; HASH_LENGTH];

/// Errors returned by simple Merkle proof helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The proof has `total == 0`, or an operation requiring leaves was called
    /// for an empty tree.
    InvalidTotal,
    /// The proof index is outside `0..total`.
    InvalidIndex,
    /// The proof contains more than [`MAX_AUNTS`] aunt hashes.
    TooManyAunts { max: usize, actual: usize },
    /// The proof has fewer or more aunt hashes than the tree shape requires.
    UnexpectedAuntCount,
    /// The proof leaf hash does not match the provided leaf bytes.
    LeafHashMismatch,
    /// The computed root does not match the expected root.
    RootMismatch,
}

impl core::fmt::Display for Error;
impl std::error::Error for Error;

/// An inclusion proof for one leaf in a CometBFT simple Merkle tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proof {
    /// Total number of leaves in the tree.
    pub total: u64,
    /// Zero-based index of the proved leaf.
    pub index: u64,
    /// CometBFT leaf hash of the proved leaf.
    pub leaf_hash: Hash,
    /// Sibling hashes from leaf-adjacent to root-adjacent.
    pub aunts: Vec<Hash>,
}

impl Proof {
    /// Construct and validate a proof from raw parts.
    pub fn new(total: u64, index: u64, leaf_hash: Hash, aunts: Vec<Hash>) -> Result<Self, Error>;

    /// Validate proof shape without checking a root or leaf preimage.
    pub fn validate_basic(&self) -> Result<(), Error>;

    /// Compute the root hash implied by this proof.
    pub fn compute_root_hash(&self) -> Result<Hash, Error>;

    /// Verify that this proof proves `leaf` is included in `root`.
    pub fn verify(&self, root: &Hash, leaf: &[u8]) -> Result<(), Error>;
}

/// Return CometBFT's empty-tree hash: SHA-256([]).
pub fn empty_hash() -> Hash;

/// Return CometBFT's leaf hash: SHA-256(0x00 || leaf).
pub fn leaf_hash(leaf: &[u8]) -> Hash;

/// Return CometBFT's inner hash: SHA-256(0x01 || left || right).
pub fn inner_hash(left: &Hash, right: &Hash) -> Hash;

/// Return the root hash for an ordered list of byte slices.
pub fn root_from_byte_slices<T: AsRef<[u8]>>(items: &[T]) -> Hash;

/// Return the root hash and inclusion proof for every item.
pub fn proofs_from_byte_slices<T: AsRef<[u8]>>(items: &[T]) -> (Hash, Vec<Proof>);
```

The crate sets `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.

## Behavioral requirements

**REQ-1.** `empty_hash()` returns SHA-256 over the empty byte string:
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.

**REQ-2.** `leaf_hash(leaf)` returns SHA-256 over `0x00 || leaf`, matching
CometBFT's `leafHash`.

**REQ-3.** `inner_hash(left, right)` returns SHA-256 over
`0x01 || left || right`, matching CometBFT's `innerHash`.

**REQ-4.** `root_from_byte_slices([])` returns `empty_hash()`.

**REQ-5.** `root_from_byte_slices([leaf])` returns `leaf_hash(leaf)`.

**REQ-6.** For two or more leaves, the tree split point is the largest power of
two strictly less than the leaf count. If the leaf count is itself a power of
two, the split point is half the leaf count. The left subtree receives
`items[..split]` and the right subtree receives `items[split..]`.

**REQ-7.** `root_from_byte_slices` is order-sensitive and byte-exact with
CometBFT `HashFromByteSlices` for all fixture item lists.

**REQ-8.** `proofs_from_byte_slices([])` returns `(empty_hash(), vec![])`.

**REQ-9.** `proofs_from_byte_slices(items)` returns the same root as
`root_from_byte_slices(items)` and returns one proof per item in item order.

**REQ-10.** `Proof::aunts` are ordered from leaf-adjacent sibling to
root-adjacent sibling, matching CometBFT `FlattenAunts`.

**REQ-11.** `Proof::compute_root_hash` reproduces CometBFT
`computeHashFromAunts`: when the proved index is in the left subtree, the last
aunt is used as the right child; when it is in the right subtree, the last aunt
is used as the left child.

**REQ-12.** `Proof::verify(root, leaf)` recomputes `leaf_hash(leaf)`, compares
it to `self.leaf_hash`, computes the root from aunts, and returns an error on
leaf-hash mismatch or root mismatch.

**REQ-13.** `Proof::validate_basic` rejects `total == 0`, `index >= total`, and
more than `MAX_AUNTS` aunt hashes.

**REQ-14.** `Proof::compute_root_hash` rejects proofs with too few or too many
aunts for the declared `(index, total)` tree shape.

**REQ-15.** Leaf-prefix and inner-prefix domain separation must make
`leaf_hash(left || right)` different from `inner_hash(left_hash, right_hash)`
for the committed RFC 6962 collision-regression vectors.

**REQ-16.** All public functions are deterministic and side-effect-free.

**REQ-17.** No crate outside `merkle` should reimplement CometBFT simple
Merkle hashing or proof verification directly unless a future crate spec
explicitly grants an exception.

## Conformance fixtures

Committed at `conformance/fixtures/merkle/`:

- `merkle/roots.json` - CometBFT v0.38.12 root vectors for empty trees,
  single-leaf trees, RFC 6962 prefix vectors, powers of two, off-powers, and
  item lists of lengths 0..32.
- `merkle/proofs.json` - root plus every inclusion proof for representative
  ordered byte-slice lists, including empty, one item, two items, odd lengths,
  powers of two, and larger short lists.
- `merkle/malformed-proofs.json` - invalid proof cases covering bad totals,
  out-of-range indexes, leaf-hash mismatches, root mismatches, missing aunts,
  extra aunts, and excessive aunt counts.

Regeneration is split into one-file scenarios:

```bash
cargo xtask refresh-fixtures --crate-name merkle
```

Equivalent individual scenarios:

```bash
cargo xtask refresh-fixtures --scenario merkle-roots
cargo xtask refresh-fixtures --scenario merkle-proofs
cargo xtask refresh-fixtures --scenario merkle-malformed-proofs
```

Each scenario must be registered in BOTH the Go fixturegen
(`conformance/fixturegen/cmd/`) and the Rust `SCENARIOS` const in
`xtask/src/refresh_fixtures.rs`.

## Definition of done

- [ ] Crate created at `crates/tier-0-primitives/merkle/` with a
      `Cargo.toml` that inherits workspace metadata (`edition.workspace = true`,
      etc.), and the crate path is added to the root `Cargo.toml` `[workspace]`
      members list.
- [ ] All REQ-1..REQ-17 covered by unit, property, or conformance tests.
- [ ] All three conformance fixture files committed with `.meta.json` sidecars.
- [ ] `cargo test -p merkle` green.
- [ ] `cargo clippy -p merkle --all-targets -- -D warnings` green.
- [ ] `cargo fmt --check` green.
- [ ] `cargo llvm-cov -p merkle --summary-only` reports >= 90 % line coverage.
- [ ] `crates/tier-0-primitives/merkle/README.md` present with purpose /
      consumers / conformance.
- [ ] Merkle fixturegen scenario(s) registered in BOTH the Go fixturegen
      (`conformance/fixturegen/cmd/`) AND the Rust `SCENARIOS` const in
      `xtask/src/refresh_fixtures.rs`.
- [ ] PR opened as draft with assignee `@rootulp`; PR body uses `Closes` for
      any tracked issue, or `Closes N/A` when no issue exists.

## Out of scope

- IAVL, ICS-23, and mutable or versioned Merkle stores.
- Namespaced Merkle trees. Those belong to `nmt`.
- Data availability headers, row roots, column roots, and erasure-coded square
  construction. Those belong to `square`.
- CometBFT generic `ProofOp`, `ProofRuntime`, key-path parsing, or protobuf
  proof conversion. Those belong to protocol-type crates once protobuf types
  exist.
- Streaming or incremental tree construction. Add later only if a downstream
  crate needs it.

## Open questions

- None. The simple Merkle tree behavior is fully determined by CometBFT
  v0.38.12 `crypto/merkle`.

## Notes for the implementing agent

1. **Scaffold the crate.** Create
   `crates/tier-0-primitives/merkle/{Cargo.toml,src/lib.rs}` and add the path
   to the root `Cargo.toml` `[workspace] members`.
2. **Use the hash crate.** Implement SHA-256 calls through `hash::sha256`; do
   not depend on `sha2` directly.
3. **Write tests first.** Translate REQ-1..REQ-15 into named unit/API tests.
   Include explicit tests for empty, one-leaf, odd-length, power-of-two, and
   malformed-proof cases.
4. **Implement recursively first.** Match CometBFT's recursive split behavior
   before considering any iterative optimization.
5. **Add fixturegen scenarios.** Use CometBFT `crypto/merkle` from the already
   pinned Go module. Keep malformed fixtures explicit rather than relying only
   on generated random data.
6. **Regenerate fixtures** with
   `cargo xtask refresh-fixtures --crate-name merkle` and commit the JSON files
   plus static `.meta.json` sidecars.
7. **Add conformance tests** under
   `crates/tier-0-primitives/merkle/tests/conformance.rs` following the hash
   and encoding workspace-root helper pattern.
8. **Verify the full crate DoD** before opening the draft PR.
