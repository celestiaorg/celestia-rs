# Crate: `multistore`

**Tier:** 2 (state and store)
**Depends on:** `encoding`, `iavl`, `kvstore`, `ics23-proofs`, `merkle`, `hash`
**Consumed by:** M2 integration tests, later SDK module keepers, ABCI query
proofs, and app-hash replay.
**Source of truth (Go):** `cosmossdk.io/store` v1.1.1 packages `rootmulti`,
`cachemulti`, and `types`, as pulled by Cosmos SDK v0.50.10.
**Crate location:** `crates/tier-2-state-and-store/multistore/`

## Purpose

Provide the Cosmos SDK commit multi-store surface used to compose module store
roots into a single application hash. The crate owns store mounting by name,
deterministic multi-store commits, `CommitInfo` / `StoreInfo` / `CommitID`
protobuf compatibility, simple Merkle proof ops for store-root membership, and
cache multi-store write-back.

The first implementation target is an in-memory multi-store over local
`kvstore::Store` IAVL-backed stores. It must produce byte-exact commit info,
store-root proofs, and app hashes against Go fixtures. Persistent databases,
store upgrades, pruning, snapshots, tracing, listeners, and inter-block caches
are out of scope until the app-hash contract is stable.

## Public API

The crate exports exactly these items from its root module:

```rust
//! Cosmos SDK commit multi-store primitives for celestia-rs.
//!
//! See `docs/superpowers/specs/m2/multistore.md` for the API contract.

pub use iavl::{Hash, Version};

pub type KvStore = kvstore::Store;

pub struct CommitId {
    pub version: Version,
    pub hash: Hash,
}

impl CommitId {
    pub fn zero() -> Self;
    pub fn is_zero(&self) -> bool;
}

pub struct StoreInfo {
    pub name: String,
    pub commit_id: CommitId,
}

pub struct CommitInfo {
    pub version: Version,
    pub store_infos: Vec<StoreInfo>,
    pub timestamp: Option<prost_types::Timestamp>,
}

impl CommitInfo {
    pub fn new(version: Version, store_infos: Vec<StoreInfo>) -> Result<Self>;
    pub fn hash(&self) -> Result<Hash>;
    pub fn commit_id(&self) -> Result<CommitId>;
    pub fn proof_op(&self, store_name: &str) -> Result<ics23_proofs::ProofOp>;
    pub fn store_info(&self, store_name: &str) -> Option<&StoreInfo>;
    pub fn to_proto_bytes(&self) -> Result<Vec<u8>>;
    pub fn from_proto_bytes(bytes: &[u8]) -> Result<Self>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreKind {
    Iavl,
}

pub struct Store { /* opaque */ }

impl Store {
    pub fn new() -> Self;
    pub fn with_initial_version(initial_version: Version) -> Result<Self>;
    pub fn mount_store(&mut self, name: impl Into<String>, kind: StoreKind) -> Result<()>;
    pub fn has_store(&self, name: &str) -> bool;
    pub fn store_names(&self) -> Vec<&str>;
    pub fn get_store(&self, name: &str) -> Result<&KvStore>;
    pub fn get_store_mut(&mut self, name: &str) -> Result<&mut KvStore>;
    pub fn version(&self) -> Version;
    pub fn last_commit_id(&self) -> Result<CommitId>;
    pub fn last_commit_info(&self) -> Option<&CommitInfo>;
    pub fn working_hash(&self) -> Result<Hash>;
    pub fn commit(&mut self) -> Result<CommitId>;
    pub fn get_commit_info(&self, version: Version) -> Result<&CommitInfo>;
    pub fn cache_multi_store(&self) -> CacheMultiStore;
    pub fn apply_cache(&mut self, cache: CacheMultiStore) -> Result<()>;
}

impl Default for Store { /* delegates to new() */ }

pub struct CacheMultiStore { /* opaque */ }

impl CacheMultiStore {
    pub fn has_store(&self, name: &str) -> bool;
    pub fn store_names(&self) -> Vec<&str>;
    pub fn get_store(&self, name: &str) -> Result<&KvStore>;
    pub fn get_store_mut(&mut self, name: &str) -> Result<&mut KvStore>;
    pub fn write(self, parent: &mut Store) -> Result<()>;
    pub fn discard(self);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    EmptyStoreName,
    DuplicateStoreName(String),
    UnknownStore(String),
    InvalidVersion {
        version: Version,
    },
    VersionNotFound {
        version: Version,
    },
    InvalidCommitInfo(&'static str),
    MalformedProto(&'static str),
    Kv(kvstore::Error),
    Iavl(iavl::Error),
    Merkle(merkle::Error),
    Proof(ics23_proofs::Error),
}

impl From<kvstore::Error> for Error { /* wraps Error::Kv */ }
impl From<iavl::Error> for Error { /* wraps Error::Iavl */ }
impl From<merkle::Error> for Error { /* wraps Error::Merkle */ }
impl From<ics23_proofs::Error> for Error { /* wraps Error::Proof */ }

pub type Result<T> = core::result::Result<T, Error>;

pub fn commit_info_hash(store_infos: &[StoreInfo]) -> Result<Hash>;
pub fn proof_op_from_store_infos(
    store_infos: &[StoreInfo],
    store_name: &str,
) -> Result<ics23_proofs::ProofOp>;
```

The crate sets `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.

## Behavioral Requirements

**REQ-1.** Store names are non-empty UTF-8 strings. Mounting an empty name
returns `Error::EmptyStoreName`; mounting the same name twice returns
`Error::DuplicateStoreName`.

**REQ-2.** `StoreKind::Iavl` mounts a fresh `kvstore::Store` backed by the local
`iavl` crate. Transient, memory, trace, listen, and inter-block cache store
kinds are out of scope for M2.

**REQ-3.** `store_names` returns mounted store names in lexicographic order,
matching Go `StoreKey.Name()` sorting.

**REQ-4.** `get_store` and `get_store_mut` return `Error::UnknownStore` for
unmounted names and never create stores implicitly.

**REQ-5.** A new store has version zero and `last_commit_id()` returns version
zero with `sha256([])` as the hash, matching Cosmos SDK `LastCommitID` when no
commit info exists.

**REQ-6.** `with_initial_version(n)` rejects `n <= 0`. The first commit uses
`n` when `n > 1`; otherwise the first commit uses version one.

**REQ-7.** `commit` commits mounted IAVL stores in lexicographic name order and
records `StoreInfo { name, commit_id }` for each mounted non-transient store.

**REQ-8.** Every committed `StoreInfo.commit_id.version` equals the multi-store
commit version for stores committed through this crate.

**REQ-9.** `CommitInfo::new` validates positive versions, non-empty unique
store names, and 32-byte hashes, then stores `store_infos` in lexicographic
name order.

**REQ-10.** `CommitInfo::hash` matches Cosmos SDK `CommitInfo.Hash`: it builds
the map `store name -> store commit hash`, converts each map entry to the SDK
simple-map leaf bytes, sorts by key, and returns the CometBFT simple Merkle
root. Empty store info lists return `sha256([])`.

**REQ-11.** `CommitInfo::commit_id` returns the commit info version plus
`CommitInfo::hash()`.

**REQ-12.** `Store::commit` returns the same `CommitId` as
`last_commit_info().unwrap().commit_id()` and stores the commit info in
in-memory version history.

**REQ-13.** `Store::working_hash` computes the app hash from current mounted
store working roots without incrementing any store or multi-store version.

**REQ-14.** `get_commit_info(version)` returns the historical commit info for a
committed version and `Error::VersionNotFound` for unknown versions.

**REQ-15.** `CommitInfo::to_proto_bytes` and `from_proto_bytes` encode and
decode the Cosmos SDK `cosmos.store.v1beta1.CommitInfo`,
`StoreInfo`, and `CommitID` protobuf fields: `version`, `store_infos`,
`timestamp`, `name`, `commit_id`, and `hash`.

**REQ-16.** Commit info protobuf decoding rejects malformed protobuf, non-
positive commit versions, empty or duplicate store names, missing commit IDs,
and commit hashes that are not exactly 32 bytes.

**REQ-17.** `CommitInfo::proof_op(store_name)` and
`proof_op_from_store_infos` match Cosmos SDK `CommitInfo.ProofOp`: they produce
an `ics23:simple` proof op keyed by `store_name` proving the store hash as the
leaf value in the multi-store app hash.

**REQ-18.** Store-root proof ops for unknown store names return
`Error::UnknownStore`.

**REQ-19.** `CacheMultiStore` owns a write-back clone of every mounted store.
Mutations through a cache never affect the parent until `write` or
`Store::apply_cache`.

**REQ-20.** `CacheMultiStore::write` flushes each cached store back to the
parent by matching store name. It returns `Error::UnknownStore` if the parent no
longer has a store that exists in the cache.

**REQ-21.** `CacheMultiStore::discard` drops all cached mutations without
touching the parent.

**REQ-22.** The crate performs no disk IO in M2. Commit history is in-memory
only, but its protobuf bytes must be compatible with the records that Go
`rootmulti.Store` writes under `s/<version>`.

## Conformance Fixtures

Committed under `conformance/fixtures/multistore/`:

- `commit.json` - store mounting, deterministic commit order, per-store commit
  IDs, commit-info protobuf bytes, empty-store app hash, and app hash after
  multi-version mutations.
- `cache.json` - cache multi-store write-back and discard behavior across
  multiple mounted stores.
- `proofs.json` - simple Merkle store-root proof ops for every committed store
  and verification of the combined store-root plus IAVL key proof path.
- `malformed.json` - empty store names, duplicate stores, unknown stores,
  invalid initial versions, missing historical commit info, malformed commit
  info protobuf, bad commit hashes, and unknown proof-op store names.

Regenerate fixtures:

```bash
cargo xtask refresh-fixtures --crate-name multistore
```

Equivalent individual scenarios:

```bash
cargo xtask refresh-fixtures --scenario multistore-commit
cargo xtask refresh-fixtures --scenario multistore-cache
cargo xtask refresh-fixtures --scenario multistore-proofs
cargo xtask refresh-fixtures --scenario multistore-malformed
```

Each scenario must be registered in BOTH the Go fixturegen
(`conformance/fixturegen/cmd/`) and the Rust `SCENARIOS` const in
`xtask/src/refresh_fixtures.rs`.

## Definition Of Done

- [x] Crate created at `crates/tier-2-state-and-store/multistore/` with a
      `Cargo.toml` inheriting workspace metadata, and the crate path is added
      to root `Cargo.toml` `[workspace]` members.
- [x] Public API above implemented with documented modules and
      `#![forbid(unsafe_code)]`.
- [x] All REQ-1..REQ-22 covered by unit, property, or conformance tests.
- [x] All four conformance fixture files committed with `.meta.json` sidecars.
- [x] `cargo test -p multistore` green.
- [x] `cargo clippy -p multistore --all-targets -- -D warnings` green.
- [x] `cargo fmt --check` green.
- [x] `cargo llvm-cov -p multistore --summary-only` reports >= 90 % line
      coverage.
- [x] `crates/tier-2-state-and-store/multistore/README.md` present with
      purpose / consumers / conformance.
- [x] Multistore fixturegen scenarios registered in BOTH the Go fixturegen and
      the Rust `SCENARIOS` const.

## Out Of Scope

- Persistent databases, pruning, snapshots, rollback, and store upgrades.
- Transient stores, memory stores, trace stores, listen stores, and
  inter-block caches.
- Loading historical immutable IAVL stores from disk.
- ABCI query path parsing and response envelopes.
- Executing Cosmos SDK messages or replaying blocks.

## Notes For The Implementing Agent

- Start with red tests for `CommitInfo::hash` over zero, one, two, and
  three-store fixtures before implementing `Store::commit`.
- Reuse the local `merkle` crate for CometBFT simple Merkle roots and proof
  shape; do not reimplement binary tree hashing in this crate.
- Reuse `ics23-proofs` for proof-op routing and verification. Multistore only
  owns the store-name map conversion and `ics23:simple` proof construction.
- Keep cache multi-store behavior by-value for the first implementation. A
  clone-backed cache is acceptable in M2 because persistent databases and
  inter-block caches are explicitly out of scope.
- Treat Go fixtures as the final authority for protobuf edge cases, especially
  zero timestamps and malformed commit info bytes.
