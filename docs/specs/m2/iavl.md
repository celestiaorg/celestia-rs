# Crate: `iavl`

**Tier:** 2 (state and store)
**Depends on:** `hash`
**Consumed by:** `kvstore`, `ics23-proofs`, `multistore`, M2 integration tests, and later SDK module stores.
**Source of truth (Go):** `github.com/cosmos/iavl` v1.2.0 as pulled by `cosmossdk.io/store` v1.1.1, plus Cosmos SDK store tests for iterator and commit semantics.
**Crate location:** `crates/tier-2-state-and-store/iavl/`

## Purpose

Provide a deterministic Rust implementation of the Cosmos IAVL+ tree shape used
for Celestia application state roots. The crate owns key/value mutation,
balanced tree structure, node hashing, versioned commits, and proof path
material. It does not own module routing, gas accounting, or multi-store root
composition; those are downstream M2 crates.

The first implementation target is an in-memory tree with byte-exact roots and
proof material against Go fixtures. Persistent node databases and pruning can
be layered in after the root/proof contract is stable.

## Public API

The crate exports exactly these items from its root module:

```rust
//! Cosmos IAVL+ state tree primitives for celestia-rs.
//!
//! See `docs/specs/m2/iavl.md` for the API contract.

pub const HASH_SIZE: usize = 32;

pub type Hash = [u8; HASH_SIZE];
pub type Version = i64;

pub struct Tree { /* opaque */ }

impl Tree {
    pub fn new() -> Self;
    pub fn version(&self) -> Version;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn root_hash(&self) -> Hash;
    pub fn get(&self, key: &[u8]) -> Option<&[u8]>;
    pub fn set(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<Option<Vec<u8>>>;
    pub fn remove(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>>;
    pub fn save_version(&mut self) -> Result<CommitInfo>;
    pub fn iter<'a>(&'a self, start: Option<&'a [u8]>, end: Option<&'a [u8]>) -> Iter<'a>;
    pub fn reverse_iter<'a>(&'a self, start: Option<&'a [u8]>, end: Option<&'a [u8]>) -> Iter<'a>;
    pub fn get_with_proof(&self, key: &[u8]) -> Result<Option<Proof>>;
    pub fn verify_membership(root: Hash, proof: &Proof, key: &[u8], value: &[u8]) -> Result<()>;
    pub fn verify_non_membership(root: Hash, proof: &Proof, key: &[u8]) -> Result<()>;
}

impl Default for Tree { /* delegates to new() */ }

pub struct CommitInfo {
    pub version: Version,
    pub root_hash: Hash,
    pub size: usize,
}

pub struct Entry<'a> {
    pub key: &'a [u8],
    pub value: &'a [u8],
}

pub struct Iter<'a> { /* opaque */ }

impl<'a> Iterator for Iter<'a> {
    type Item = Entry<'a>;
    fn next(&mut self) -> Option<Self::Item>;
}

pub struct Proof { /* opaque */ }

impl Proof {
    pub fn key(&self) -> &[u8];
    pub fn value(&self) -> Option<&[u8]>;
    pub fn is_membership(&self) -> bool;
    pub fn ops(&self) -> &[ProofOp];
}

pub struct ProofOp { /* opaque */ }

impl ProofOp {
    pub fn field_type(&self) -> ProofOpType;
    pub fn key(&self) -> &[u8];
    pub fn data(&self) -> &[u8];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofOpType {
    IavlValue,
    IavlAbsence,
    IavlCommitment,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("empty key")]
    EmptyKey,
    #[error("invalid version {0}")]
    InvalidVersion(Version),
    #[error("invalid range")]
    InvalidRange,
    #[error("invalid proof: {0}")]
    InvalidProof(&'static str),
    #[error("malformed node: {0}")]
    MalformedNode(&'static str),
}

pub type Result<T> = core::result::Result<T, Error>;
```

The crate sets `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.

## Behavioral Requirements

**REQ-1.** Keys are arbitrary non-empty byte strings ordered by lexicographic
byte comparison. Empty keys are rejected by mutation and proof APIs.

**REQ-2.** Values are arbitrary byte strings, including empty values. Setting an
existing key replaces the value and returns the previous value.

**REQ-3.** `remove` deletes an existing key and returns its prior value. Removing
a missing key is a no-op that returns `Ok(None)`.

**REQ-4.** `root_hash` for an empty tree matches Go IAVL's empty root for the
selected source version.

**REQ-5.** For any committed fixture operation sequence, the Rust root hash
after every `save_version` equals the Go IAVL root hash for the same sequence.

**REQ-6.** `save_version` increments the tree version by exactly one and returns
the committed version, root hash, and item count. Version zero is reserved for a
new uncommitted tree.

**REQ-7.** The tree remains balanced according to Go IAVL height/size update
rules after insertion and deletion.

**REQ-8.** Iterators are half-open ranges `[start, end)` when both bounds are
present. `None` start means the first key; `None` end means past the last key.

**REQ-9.** Forward iteration yields strictly increasing keys; reverse iteration
yields strictly decreasing keys. Iterator output matches Go for empty, exact
bound, missing-bound, prefix-like, and full-range cases.

**REQ-10.** Membership proofs for existing keys verify against the root hash and
the exact value bytes from the fixture.

**REQ-11.** Non-membership proofs for missing keys verify against the root hash
and encode the same neighboring proof shape as Go IAVL.

**REQ-12.** Proof operation order and operation type names match the Go IAVL /
Cosmos SDK proof material consumed by ICS-23 downstream.

**REQ-13.** Node hashes are deterministic and byte-exact with Go IAVL for leaf,
inner, single-leaf, balanced, unbalanced, and post-delete trees.

**REQ-14.** The public API does not expose internal node references or permit
callers to mutate keys/values through borrowed entries.

**REQ-15.** The crate does not perform disk IO in M2. Persistence and pruning
are out of scope until root/proof compatibility is established.

## Conformance Fixtures

Committed under `conformance/fixtures/iavl/`:

- `roots.json` - operation sequences with commit versions, tree sizes, and
  expected root hashes.
- `iterators.json` - forward and reverse range iterator cases.
- `proofs.json` - membership and non-membership proofs with expected operation
  shape and verification outcomes.
- `malformed.json` - empty keys, invalid ranges, malformed proof operations,
  and impossible node encodings.

Regenerate fixtures:

```bash
cargo xtask refresh-fixtures --crate-name iavl
```

Equivalent individual scenarios:

```bash
cargo xtask refresh-fixtures --scenario iavl-roots
cargo xtask refresh-fixtures --scenario iavl-iterators
cargo xtask refresh-fixtures --scenario iavl-proofs
cargo xtask refresh-fixtures --scenario iavl-malformed
```

Each scenario must be registered in BOTH the Go fixturegen
(`conformance/fixturegen/cmd/`) and the Rust `SCENARIOS` const in
`xtask/src/refresh_fixtures.rs`.

## Definition Of Done

- [x] Crate created at `crates/tier-2-state-and-store/iavl/` with a
      `Cargo.toml` inheriting workspace metadata, and the crate path is added
      to root `Cargo.toml` `[workspace]` members.
- [x] Public API above implemented with documented modules and
      `#![forbid(unsafe_code)]`.
- [x] All REQ-1..REQ-15 covered by unit, property, or conformance tests.
- [x] All four conformance fixture files committed with `.meta.json` sidecars.
- [x] `cargo test -p iavl` green.
- [x] `cargo clippy -p iavl --all-targets -- -D warnings` green.
- [x] `cargo fmt --check` green.
- [x] `cargo llvm-cov -p iavl --summary-only` reports >= 90 % line coverage.
- [x] `crates/tier-2-state-and-store/iavl/README.md` present with purpose /
      consumers / conformance.
- [x] IAVL fixturegen scenarios registered in BOTH the Go fixturegen and the
      Rust `SCENARIOS` const.

## Out Of Scope

- Persistent node database, pruning, snapshots, and fast-node indexes.
- Multi-store root composition.
- Gas metering and cache wrappers.
- Module store prefixes and object codecs.
- Executing Cosmos SDK messages or replaying blocks.

## Notes For The Implementing Agent

- Start with red tests for deterministic roots over small fixed operation
  sequences: empty tree, one key, two sorted keys, two reverse-inserted keys,
  replacement, and deletion.
- Keep node encoding private until Go fixtures lock the byte format. The public
  API should expose roots and proof operations, not internal node layouts.
- Implement iterator semantics before proofs; proof generation depends on
  stable key ordering and neighbor discovery.
- Use the Go fixtures to decide edge behavior whenever a Go IAVL comment and a
  Cosmos SDK store test appear to disagree.
- Leave persistence out of the first implementation. A correct in-memory IAVL
  root is the strict prerequisite for `kvstore` and `multistore`.
