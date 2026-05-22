# Crate: `kvstore`

**Tier:** 2 (state and store)
**Depends on:** `encoding`, `iavl`
**Consumed by:** `multistore`, M2 integration tests, and later SDK module
keepers.
**Source of truth (Go):** `cosmossdk.io/store` v1.1.1 packages
`types`, `cachekv`, `prefix`, `gaskv`, and `iavl`, as pulled by Cosmos SDK
v0.50.10.
**Crate location:** `crates/tier-2-state-and-store/kvstore/`

## Purpose

Provide the Cosmos SDK key/value store surface used by module keepers before
multi-store composition. The crate owns basic KV operations, write-back cache
branches, prefix-scoped stores, deterministic iteration, and gas accounting for
store access. It uses the local `iavl` crate for committed state roots but does
not own multi-store commit info, module routing, or ICS-23 proof conversion.

The first implementation target is an in-memory, IAVL-backed store whose public
behavior matches the SDK store wrappers closely enough for later `multistore`
and module crates to build on it without reworking the API.

## Public API

The crate exports exactly these items from its root module:

```rust
//! Cosmos SDK key/value store primitives for celestia-rs.
//!
//! See `docs/superpowers/specs/m2/kvstore.md` for the API contract.

pub use iavl::{CommitInfo, Hash, Version};

pub type Gas = u64;

pub struct Store { /* opaque */ }

impl Store {
    pub fn new() -> Self;
    pub fn from_tree(tree: iavl::Tree) -> Self;
    pub fn into_tree(self) -> iavl::Tree;
    pub fn version(&self) -> Version;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn root_hash(&self) -> Hash;
    pub fn save_version(&mut self) -> Result<CommitInfo>;

    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;
    pub fn has(&self, key: &[u8]) -> Result<bool>;
    pub fn set(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<Option<Vec<u8>>>;
    pub fn delete(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>>;
    pub fn iter(&self, start: Option<&[u8]>, end: Option<&[u8]>) -> Result<Iter>;
    pub fn reverse_iter(&self, start: Option<&[u8]>, end: Option<&[u8]>) -> Result<Iter>;

    pub fn cache(&mut self) -> CacheStore<'_>;
    pub fn prefix(&mut self, prefix: Vec<u8>) -> PrefixStore<'_>;
    pub fn with_gas<'a>(
        &'a mut self,
        meter: &'a mut GasMeter,
        config: GasConfig,
    ) -> GasStore<'a>;
}

impl Default for Store { /* delegates to new() */ }

pub struct CacheStore<'a> { /* opaque */ }

impl<'a> CacheStore<'a> {
    pub fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>>;
    pub fn has(&mut self, key: &[u8]) -> Result<bool>;
    pub fn set(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<Option<Vec<u8>>>;
    pub fn delete(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>>;
    pub fn iter(&mut self, start: Option<&[u8]>, end: Option<&[u8]>) -> Result<Iter>;
    pub fn reverse_iter(&mut self, start: Option<&[u8]>, end: Option<&[u8]>) -> Result<Iter>;
    pub fn write(self) -> Result<()>;
}

pub struct PrefixStore<'a> { /* opaque */ }

impl<'a> PrefixStore<'a> {
    pub fn prefix(&self) -> &[u8];
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;
    pub fn has(&self, key: &[u8]) -> Result<bool>;
    pub fn set(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<Option<Vec<u8>>>;
    pub fn delete(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>>;
    pub fn iter(&self, start: Option<&[u8]>, end: Option<&[u8]>) -> Result<Iter>;
    pub fn reverse_iter(&self, start: Option<&[u8]>, end: Option<&[u8]>) -> Result<Iter>;
}

pub struct GasStore<'a> { /* opaque */ }

impl<'a> GasStore<'a> {
    pub fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>>;
    pub fn has(&mut self, key: &[u8]) -> Result<bool>;
    pub fn set(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<Option<Vec<u8>>>;
    pub fn delete(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>>;
    pub fn iter(&mut self, start: Option<&[u8]>, end: Option<&[u8]>) -> Result<GasIter<'_>>;
    pub fn reverse_iter(
        &mut self,
        start: Option<&[u8]>,
        end: Option<&[u8]>,
    ) -> Result<GasIter<'_>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

pub struct Iter { /* opaque */ }

impl Iterator for Iter {
    type Item = Entry;
    fn next(&mut self) -> Option<Self::Item>;
}

pub struct GasIter<'a> { /* opaque */ }

impl<'a> Iterator for GasIter<'a> {
    type Item = Result<Entry>;
    fn next(&mut self) -> Option<Self::Item>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GasConfig {
    pub has_cost: Gas,
    pub delete_cost: Gas,
    pub read_cost_flat: Gas,
    pub read_cost_per_byte: Gas,
    pub write_cost_flat: Gas,
    pub write_cost_per_byte: Gas,
    pub iter_next_cost_flat: Gas,
}

impl GasConfig {
    pub const fn kv() -> Self;
    pub const fn transient() -> Self;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GasMeter { /* opaque */ }

impl GasMeter {
    pub const fn new(limit: Gas) -> Self;
    pub const fn infinite() -> Self;
    pub fn limit(&self) -> Option<Gas>;
    pub fn consumed(&self) -> Gas;
    pub fn consumed_to_limit(&self) -> Gas;
    pub fn remaining(&self) -> Option<Gas>;
    pub fn is_past_limit(&self) -> bool;
    pub fn is_out_of_gas(&self) -> bool;
    pub fn consume(&mut self, amount: Gas, descriptor: &'static str) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    EmptyKey,
    InvalidRange,
    OutOfGas {
        descriptor: &'static str,
        limit: Gas,
        consumed: Gas,
    },
    GasOverflow,
    Iavl(iavl::Error),
}

impl From<iavl::Error> for Error { /* wraps Error::Iavl */ }

pub type Result<T> = core::result::Result<T, Error>;

pub fn prefix_end_bytes(prefix: &[u8]) -> Option<Vec<u8>>;
pub fn inclusive_end_bytes(bytes: &[u8]) -> Vec<u8>;
pub fn u64_to_big_endian(value: u64) -> [u8; 8];
pub fn u64_from_big_endian(bytes: &[u8]) -> Result<u64>;
```

The crate sets `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.

## Behavioral Requirements

**REQ-1.** Keys are arbitrary non-empty byte strings ordered by lexicographic
byte comparison. Empty keys are rejected by `get`, `has`, `set`, `delete`, and
gas/prefix/cache wrappers with `Error::EmptyKey`.

**REQ-2.** Values are arbitrary byte strings, including empty values. Setting an
existing key replaces the value and returns the previous value.

**REQ-3.** `delete` removes an existing key and returns its previous value.
Deleting a missing key returns `Ok(None)`.

**REQ-4.** `Store::root_hash` and `Store::save_version` delegate to the
underlying `iavl::Tree`, and roots match `iavl` for the same key/value
mutation sequence.

**REQ-5.** `iter(start, end)` yields entries in strictly increasing key order
over the half-open range `[start, end)`. `reverse_iter(start, end)` yields the
same domain in strictly decreasing key order.

**REQ-6.** Iterator ranges with both bounds present and `start > end` return
`Error::InvalidRange`. Equal bounds are valid and produce an empty iterator.

**REQ-7.** `CacheStore` reads through to its parent on misses, stores read
results in the cache, and never mutates the parent until `write`.

**REQ-8.** `CacheStore::set` and `CacheStore::delete` are write-back
operations. `write` flushes dirty entries to the parent in ascending key order,
including delete tombstones.

**REQ-9.** Cache iteration merges parent entries and dirty cache entries in
lexicographic order. Dirty cache values override parent values, and deleted
cache entries are omitted.

**REQ-10.** `PrefixStore` maps every visible key `k` to parent key
`prefix || k`, strips the prefix from iterator output, and never exposes keys
outside its prefix.

**REQ-11.** `prefix_end_bytes` matches Cosmos SDK `PrefixEndBytes`: empty
prefix returns `None`, otherwise the returned end bound increments the final
non-`0xff` byte and truncates trailing `0xff` bytes; an all-`0xff` prefix
returns `None`.

**REQ-12.** `GasConfig::kv()` returns the SDK default KV gas schedule:
`has=1000`, `delete=1000`, `read_flat=1000`, `read_per_byte=3`,
`write_flat=2000`, `write_per_byte=30`, and `iter_next_flat=30`.

**REQ-13.** `GasStore::get` consumes read flat gas plus read-per-byte gas for
both key length and returned value length. `has` consumes only `has_cost`.

**REQ-14.** `GasStore::set` validates key/value first, then consumes write flat
gas plus write-per-byte gas for key length and value length before mutating the
parent. `delete` consumes `delete_cost` before delegating to the parent.

**REQ-15.** `GasIter` consumes seek gas at creation and on each `next`: if the
underlying iterator is valid, it consumes read-per-byte gas for current key and
current value; it always consumes `iter_next_cost_flat`.

**REQ-16.** `GasMeter::consume` uses checked addition. Consuming past a finite
limit records the consumed total and returns `Error::OutOfGas`; arithmetic
overflow returns `Error::GasOverflow`. Infinite meters never return
`OutOfGas`.

**REQ-17.** `u64_to_big_endian` and `u64_from_big_endian` match the SDK helper
semantics. Empty input decodes to zero; non-empty input must be exactly eight
bytes or return `Error::InvalidRange`.

**REQ-18.** The crate does not perform disk IO in M2. Persistent databases,
trace stores, listen stores, and inter-block caches are out of scope until
`multistore` needs them.

## Conformance Fixtures

Committed under `conformance/fixtures/kvstore/`:

- `basic.json` - basic get/has/set/delete behavior, replacement results,
  committed roots, and iterator ranges.
- `cache.json` - write-back cache reads, dirty writes, delete tombstones,
  merge iteration, and flush order.
- `prefix.json` - prefix key mapping, prefix iterator bounds, `0xff` prefix
  end edge cases, and big-endian helpers.
- `gas.json` - default gas config, exact gas consumed by get/has/set/delete,
  iterator seek/next costs, infinite meters, and out-of-gas errors.
- `malformed.json` - empty keys, invalid ranges, malformed big-endian input,
  and gas overflow/out-of-gas cases.

Regenerate fixtures:

```bash
cargo xtask refresh-fixtures --crate-name kvstore
```

Equivalent individual scenarios:

```bash
cargo xtask refresh-fixtures --scenario kvstore-basic
cargo xtask refresh-fixtures --scenario kvstore-cache
cargo xtask refresh-fixtures --scenario kvstore-prefix
cargo xtask refresh-fixtures --scenario kvstore-gas
cargo xtask refresh-fixtures --scenario kvstore-malformed
```

Each scenario must be registered in BOTH the Go fixturegen
(`conformance/fixturegen/cmd/`) and the Rust `SCENARIOS` const in
`xtask/src/refresh_fixtures.rs`.

## Definition Of Done

- [x] Crate created at `crates/tier-2-state-and-store/kvstore/` with a
      `Cargo.toml` inheriting workspace metadata, and the crate path is added
      to root `Cargo.toml` `[workspace]` members.
- [x] Public API above implemented with documented modules and
      `#![forbid(unsafe_code)]`.
- [x] All REQ-1..REQ-18 covered by unit, property, or conformance tests.
- [x] All five conformance fixture files committed with `.meta.json` sidecars.
- [x] `cargo test -p kvstore` green.
- [x] `cargo clippy -p kvstore --all-targets -- -D warnings` green.
- [x] `cargo fmt --check` green.
- [x] `cargo llvm-cov -p kvstore --summary-only` reports >= 90 % line
      coverage.
- [x] `crates/tier-2-state-and-store/kvstore/README.md` present with purpose /
      consumers / conformance.
- [x] KVStore fixturegen scenarios registered in BOTH the Go fixturegen and the
      Rust `SCENARIOS` const.

## Out Of Scope

- Root multi-store composition and commit info protobufs.
- ICS-23 proof conversion and proof op routing.
- TraceKVStore, ListenKVStore, streaming listeners, pruning, and snapshots.
- Persistent database backends beyond the `iavl::Tree` wrapper.
- SDK context, ante handlers, module keepers, and message routing.

## Notes For The Implementing Agent

- Start with red tests for `prefix_end_bytes`, basic store mutations, cache
  write-back behavior, and gas accounting before adding fixturegen.
- Keep the first implementation single-threaded and in-memory. The Go
  `cachekv` store uses a mutex because Go IAVL is not concurrency-safe, but
  this crate should avoid synchronization until a Rust caller actually needs
  shared access.
- Implement cache iteration before gas iteration; `GasIter` can wrap the final
  merged iterator once its ordering is stable.
- Treat Go panics from nil keys/values as Rust `Result` errors. Rust cannot
  represent nil values, so empty values must remain valid.
- Use `iavl::Tree` as the only committed-state backend. Do not duplicate root
  hashing or proof logic in this crate.
