# Crate: `shares`

**Tier:** 1 (protocol types)
**Depends on:** `encoding`, `cosmos-types`
**Consumed by:** `square`, M1 integration tests, transaction/blob data
reconstruction, and downstream data-availability proof machinery.
**Source of truth (Go):** `github.com/celestiaorg/go-square` v1.0.1 packages
`namespace`, `shares`, and `blob`, as used by current local
celestia-app / go-square module snapshots.
**Crate location:** `crates/tier-1-protocol-types/shares/`

## Purpose

Provide Celestia share-format primitives: 29-byte namespaces, share info bytes,
sequence lengths, compact-share reserved bytes, fixed 512-byte shares, compact
transaction share splitting/parsing, sparse blob share splitting/parsing, share
ranges, and reserved/tail/namespace padding shares.

This crate is the byte-compatibility layer for Celestia's share format. It does
not construct data squares, erasure-code rows/columns, compute data
availability headers, verify NMT proofs, execute transactions, or validate
Cosmos SDK message semantics. Those belong to `square`, `nmt`, `cosmos-types`,
and later execution crates.

## Public API

The crate exports exactly these root modules:

```rust
//! Celestia share encoding and decoding primitives.
//!
//! See `docs/superpowers/specs/m1/shares.md` for the API contract.

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

pub use blob::{Blob, IndexWrapper};
pub use builder::ShareBuilder;
pub use compact::{CompactShareSplitter, SplitTxsResult, marshal_delimited_tx, split_txs};
pub use error::{Error, Result};
pub use info_byte::InfoByte;
pub use namespace::{
    MAX_PRIMARY_RESERVED_NAMESPACE, MIN_SECONDARY_RESERVED_NAMESPACE, NAMESPACE_ID_SIZE,
    NAMESPACE_SIZE, NAMESPACE_VERSION_MAX, NAMESPACE_VERSION_SIZE, NAMESPACE_VERSION_ZERO,
    NAMESPACE_VERSION_ZERO_ID_SIZE, NAMESPACE_VERSION_ZERO_PREFIX_SIZE,
    PARITY_SHARES_NAMESPACE, PAY_FOR_BLOB_NAMESPACE, PRIMARY_RESERVED_PADDING_NAMESPACE,
    Namespace, TAIL_PADDING_NAMESPACE, TX_NAMESPACE,
};
pub use padding::{
    namespace_padding_share, namespace_padding_shares, reserved_padding_share,
    reserved_padding_shares, tail_padding_share, tail_padding_shares,
};
pub use parse::{parse_blobs, parse_compact_shares, parse_sparse_shares, parse_txs};
pub use range::{ShareRange, get_share_range_for_namespace};
pub use reserved::{ReservedBytes, parse_reserved_bytes};
pub use share::{
    COMPACT_SHARE_RESERVED_BYTES, CONTINUATION_COMPACT_SHARE_CONTENT_SIZE,
    CONTINUATION_SPARSE_SHARE_CONTENT_SIZE, DEFAULT_SHARE_VERSION,
    FIRST_COMPACT_SHARE_CONTENT_SIZE, FIRST_SPARSE_SHARE_CONTENT_SIZE,
    MAX_SHARE_VERSION, MIN_SHARE_COUNT, MIN_SQUARE_SIZE, SEQUENCE_LEN_BYTES,
    SHARE_INFO_BYTES, SHARE_SIZE, SHARE_VERSION_ZERO, SUPPORTED_SHARE_VERSIONS,
    Share, shares_from_bytes, shares_to_bytes,
};
pub use sparse::{SparseShareSplitter, split_blobs};
```

The crate sets `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.

### `namespace`

Share namespaces are **29 bytes**, unlike the M1 `nmt::Namespace` helper which
is fixed at 8 bytes. A share namespace is `version || id`.

```rust
pub const NAMESPACE_VERSION_SIZE: usize = 1;
pub const NAMESPACE_ID_SIZE: usize = 28;
pub const NAMESPACE_SIZE: usize = NAMESPACE_VERSION_SIZE + NAMESPACE_ID_SIZE;
pub const NAMESPACE_VERSION_ZERO: u8 = 0;
pub const NAMESPACE_VERSION_MAX: u8 = 255;
pub const NAMESPACE_VERSION_ZERO_PREFIX_SIZE: usize = 18;
pub const NAMESPACE_VERSION_ZERO_ID_SIZE: usize =
    NAMESPACE_ID_SIZE - NAMESPACE_VERSION_ZERO_PREFIX_SIZE;

pub const TX_NAMESPACE: Namespace;
pub const INTERMEDIATE_STATE_ROOTS_NAMESPACE: Namespace;
pub const PAY_FOR_BLOB_NAMESPACE: Namespace;
pub const PRIMARY_RESERVED_PADDING_NAMESPACE: Namespace;
pub const MAX_PRIMARY_RESERVED_NAMESPACE: Namespace;
pub const MIN_SECONDARY_RESERVED_NAMESPACE: Namespace;
pub const TAIL_PADDING_NAMESPACE: Namespace;
pub const PARITY_SHARES_NAMESPACE: Namespace;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Namespace([u8; NAMESPACE_SIZE]);

impl Namespace {
    pub const fn from_bytes_unchecked(bytes: [u8; NAMESPACE_SIZE]) -> Self;
    pub fn new(version: u8, id: [u8; NAMESPACE_ID_SIZE]) -> Result<Self>;
    pub fn new_v0(sub_id: &[u8]) -> Result<Self>;
    pub fn from_slice(bytes: &[u8]) -> Result<Self>;
    pub const fn as_bytes(&self) -> &[u8; NAMESPACE_SIZE];
    pub const fn into_bytes(self) -> [u8; NAMESPACE_SIZE];
    pub const fn version(self) -> u8;
    pub fn id(self) -> [u8; NAMESPACE_ID_SIZE];
    pub fn validate_basic(&self) -> Result<()>;
    pub fn is_reserved(self) -> bool;
    pub fn is_primary_reserved(self) -> bool;
    pub fn is_secondary_reserved(self) -> bool;
    pub fn is_parity_shares(self) -> bool;
    pub fn is_tail_padding(self) -> bool;
    pub fn is_primary_reserved_padding(self) -> bool;
    pub fn is_tx(self) -> bool;
    pub fn is_pay_for_blob(self) -> bool;
    pub fn repeat(self, times: usize) -> Vec<Self>;
}

impl TryFrom<&[u8]> for Namespace;
impl From<Namespace> for [u8; NAMESPACE_SIZE];
impl core::fmt::LowerHex for Namespace;
```

### `info_byte`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InfoByte(u8);

impl InfoByte {
    pub fn new(version: u8, is_sequence_start: bool) -> Result<Self>;
    pub fn parse(byte: u8) -> Result<Self>;
    pub const fn version(self) -> u8;
    pub const fn is_sequence_start(self) -> bool;
    pub const fn into_byte(self) -> u8;
}

impl TryFrom<u8> for InfoByte;
impl From<InfoByte> for u8;
```

### `reserved`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReservedBytes(u32);

impl ReservedBytes {
    pub fn new(byte_index: u32) -> Result<Self>;
    pub fn parse(bytes: &[u8]) -> Result<Self>;
    pub const fn byte_index(self) -> u32;
    pub fn to_bytes(self) -> [u8; COMPACT_SHARE_RESERVED_BYTES];
}

pub fn parse_reserved_bytes(bytes: &[u8]) -> Result<ReservedBytes>;
```

### `range`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShareRange {
    pub start: usize,
    pub end: usize,
}

impl ShareRange {
    pub const fn new(start: usize, end: usize) -> Self;
    pub const fn empty() -> Self;
    pub const fn is_empty(self) -> bool;
    pub fn add(&mut self, value: usize) -> Result<()>;
}

pub fn get_share_range_for_namespace(
    shares: &[Share],
    namespace: Namespace,
) -> Result<ShareRange>;
```

### `share`

```rust
pub const SHARE_SIZE: usize = 512;
pub const SHARE_INFO_BYTES: usize = 1;
pub const SEQUENCE_LEN_BYTES: usize = 4;
pub const SHARE_VERSION_ZERO: u8 = 0;
pub const DEFAULT_SHARE_VERSION: u8 = SHARE_VERSION_ZERO;
pub const COMPACT_SHARE_RESERVED_BYTES: usize = 4;
pub const FIRST_COMPACT_SHARE_CONTENT_SIZE: usize =
    SHARE_SIZE - NAMESPACE_SIZE - SHARE_INFO_BYTES - SEQUENCE_LEN_BYTES
        - COMPACT_SHARE_RESERVED_BYTES;
pub const CONTINUATION_COMPACT_SHARE_CONTENT_SIZE: usize =
    SHARE_SIZE - NAMESPACE_SIZE - SHARE_INFO_BYTES - COMPACT_SHARE_RESERVED_BYTES;
pub const FIRST_SPARSE_SHARE_CONTENT_SIZE: usize =
    SHARE_SIZE - NAMESPACE_SIZE - SHARE_INFO_BYTES - SEQUENCE_LEN_BYTES;
pub const CONTINUATION_SPARSE_SHARE_CONTENT_SIZE: usize =
    SHARE_SIZE - NAMESPACE_SIZE - SHARE_INFO_BYTES;
pub const MIN_SQUARE_SIZE: usize = 1;
pub const MIN_SHARE_COUNT: usize = MIN_SQUARE_SIZE * MIN_SQUARE_SIZE;
pub const MAX_SHARE_VERSION: u8 = 127;
pub const SUPPORTED_SHARE_VERSIONS: &[u8] = &[SHARE_VERSION_ZERO];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share([u8; SHARE_SIZE]);

impl Share {
    pub fn from_bytes(bytes: [u8; SHARE_SIZE]) -> Result<Self>;
    pub fn from_slice(bytes: &[u8]) -> Result<Self>;
    pub fn validate(&self) -> Result<()>;
    pub const fn as_bytes(&self) -> &[u8; SHARE_SIZE];
    pub const fn into_bytes(self) -> [u8; SHARE_SIZE];
    pub const fn len(&self) -> usize;
    pub fn namespace(&self) -> Result<Namespace>;
    pub fn info_byte(&self) -> Result<InfoByte>;
    pub fn version(&self) -> Result<u8>;
    pub fn supports_versions(&self, supported_share_versions: &[u8]) -> Result<()>;
    pub fn is_sequence_start(&self) -> Result<bool>;
    pub fn is_compact_share(&self) -> Result<bool>;
    pub fn sequence_len(&self) -> Result<Option<u32>>;
    pub fn is_padding(&self) -> Result<bool>;
    pub fn raw_data(&self) -> Result<&[u8]>;
    pub fn raw_data_using_reserved(&self) -> Result<&[u8]>;
}

pub fn shares_to_bytes(shares: &[Share]) -> Vec<[u8; SHARE_SIZE]>;
pub fn shares_from_bytes(bytes: &[[u8; SHARE_SIZE]]) -> Result<Vec<Share>>;
```

### `blob`

```rust
pub const PROTO_BLOB_TX_TYPE_ID: &str = "BLOB";
pub const PROTO_INDEX_WRAPPER_TYPE_ID: &str = "INDX";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    pub namespace: Namespace,
    pub data: Vec<u8>,
    pub share_version: u8,
}

impl Blob {
    pub fn new(namespace: Namespace, data: Vec<u8>, share_version: u8) -> Result<Self>;
    pub fn new_v0(namespace: Namespace, data: Vec<u8>) -> Result<Self>;
    pub fn validate_basic(&self) -> Result<()>;
    pub fn namespace(&self) -> Namespace;
    pub fn data(&self) -> &[u8];
    pub fn data_len(&self) -> usize;
    pub fn share_version(&self) -> u8;
    pub fn is_empty(&self) -> bool;
    pub fn to_shares(&self) -> Result<Vec<Share>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexWrapper {
    pub tx: Vec<u8>,
    pub share_indexes: Vec<u32>,
}

impl IndexWrapper {
    pub fn new(tx: Vec<u8>, share_indexes: Vec<u32>) -> Self;
    pub fn encode(&self) -> Result<Vec<u8>>;
    pub fn decode(bytes: &[u8]) -> Result<Option<Self>>;
}
```

`IndexWrapper::decode` returns `Ok(None)` when bytes are not an index wrapper,
matching Go's `blob.UnmarshalIndexWrapper` classifier behavior.

### `builder`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareBuilder {
    /* private fields */
}

impl ShareBuilder {
    pub fn new_empty() -> Self;
    pub fn new(namespace: Namespace, share_version: u8, is_first_share: bool)
        -> Result<Self>;
    pub fn available_bytes(&self) -> usize;
    pub fn import_raw_share(&mut self, raw_bytes: Vec<u8>) -> Result<()>;
    pub fn add_data(&mut self, raw_data: &[u8]) -> Option<Vec<u8>>;
    pub fn build(&self) -> Result<Share>;
    pub fn is_empty_share(&self) -> bool;
    pub fn zero_pad_if_necessary(&mut self) -> usize;
    pub fn maybe_write_reserved_bytes(&mut self) -> Result<()>;
    pub fn write_sequence_len(&mut self, sequence_len: u32) -> Result<()>;
    pub fn flip_sequence_start(&mut self);
}
```

### `compact`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactShareSplitter {
    /* private fields */
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitTxsResult {
    pub tx_shares: Vec<Share>,
    pub pfb_shares: Vec<Share>,
    pub share_ranges: std::collections::BTreeMap<[u8; 32], ShareRange>,
}

impl CompactShareSplitter {
    pub fn new(namespace: Namespace, share_version: u8) -> Result<Self>;
    pub fn write_tx(&mut self, tx: &[u8]) -> Result<()>;
    pub fn export(&mut self) -> Result<Vec<Share>>;
    pub fn count(&self) -> usize;
    pub fn share_ranges(
        &self,
        share_range_offset: usize,
    ) -> Result<std::collections::BTreeMap<[u8; 32], ShareRange>>;
}

pub fn marshal_delimited_tx(tx: &[u8]) -> Vec<u8>;
pub fn split_txs(txs: &[Vec<u8>]) -> Result<SplitTxsResult>;
```

### `sparse`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SparseShareSplitter {
    /* private fields */
}

impl SparseShareSplitter {
    pub fn new() -> Self;
    pub fn write(&mut self, blob: &Blob) -> Result<()>;
    pub fn write_namespace_padding_shares(&mut self, count: usize) -> Result<()>;
    pub fn export(&self) -> Vec<Share>;
    pub fn count(&self) -> usize;
}

impl Default for SparseShareSplitter;

pub fn split_blobs(blobs: &[Blob]) -> Result<Vec<Share>>;
```

### `parse`

```rust
pub fn parse_txs(shares: &[Share]) -> Result<Vec<Vec<u8>>>;
pub fn parse_blobs(shares: &[Share]) -> Result<Vec<Blob>>;
pub fn parse_compact_shares(
    shares: &[Share],
    supported_share_versions: &[u8],
) -> Result<Vec<Vec<u8>>>;
pub fn parse_sparse_shares(
    shares: &[Share],
    supported_share_versions: &[u8],
) -> Result<Vec<Blob>>;
```

### `padding`

```rust
pub fn namespace_padding_share(namespace: Namespace, share_version: u8) -> Result<Share>;
pub fn namespace_padding_shares(
    namespace: Namespace,
    share_version: u8,
    count: usize,
) -> Result<Vec<Share>>;
pub fn reserved_padding_share() -> Share;
pub fn reserved_padding_shares(count: usize) -> Vec<Share>;
pub fn tail_padding_share() -> Share;
pub fn tail_padding_shares(count: usize) -> Vec<Share>;
```

### `error`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidNamespaceLength { actual: usize, expected: usize },
    InvalidNamespaceVersion { version: u8 },
    InvalidNamespaceIdLength { actual: usize, expected: usize },
    InvalidNamespaceV0Prefix,
    InvalidShareLength { actual: usize, expected: usize },
    InvalidShareVersion { version: u8, max: u8 },
    UnsupportedShareVersion { version: u8, supported: Vec<u8> },
    InvalidInfoByte,
    InvalidReservedBytesLength { actual: usize, expected: usize },
    InvalidReservedByteIndex { actual: u32, max_exclusive: u32 },
    InvalidSequenceLen,
    InvalidShareRange,
    InvalidPadding,
    InvalidBlob,
    InvalidCompactShare,
    InvalidSparseShare,
    InvalidIndexWrapper,
    Decode(String),
    Encode(String),
}

impl core::fmt::Display for Error;
impl std::error::Error for Error;

pub type Result<T> = core::result::Result<T, Error>;
```

## Behavioral requirements

**REQ-1.** Namespace bytes are exactly 29 bytes: one version byte followed by a
28-byte namespace ID. Ordering is lexicographic over the full 29 bytes.

**REQ-2.** Only namespace versions `0` and `255` are accepted. Version `0`
namespace IDs must be exactly 28 bytes and must start with 18 zero bytes.

**REQ-3.** `Namespace::new_v0(sub_id)` rejects `sub_id` longer than 10 bytes
and otherwise left-pads it to the 10-byte version-zero sub-ID field.

**REQ-4.** Reserved namespace constants match Go `go-square/namespace`:
`TX_NAMESPACE` is primary reserved last byte `0x01`,
`INTERMEDIATE_STATE_ROOTS_NAMESPACE` is `0x02`, `PAY_FOR_BLOB_NAMESPACE` is
`0x04`, primary reserved padding / max primary reserved is `0xFF`, tail
padding is secondary reserved last byte `0xFE`, and parity shares is secondary
reserved last byte `0xFF`.

**REQ-5.** `InfoByte` encodes version in the high 7 bits and the sequence-start
indicator in the low bit. Versions above 127 are rejected.

**REQ-6.** `ReservedBytes` is a 4-byte big-endian offset and rejects offsets
greater than or equal to `SHARE_SIZE`.

**REQ-7.** A `Share` is exactly 512 bytes. The namespace occupies bytes
`0..29`; byte `29` is the info byte.

**REQ-8.** `Share::sequence_len()` returns `None` for continuation shares and
returns the big-endian 4-byte sequence length for sequence-start shares.

**REQ-9.** Compact shares are shares whose namespace is `TX_NAMESPACE` or
`PAY_FOR_BLOB_NAMESPACE`. Compact shares reserve 4 bytes for the byte offset of
the first unit that starts in the share.

**REQ-10.** Sparse shares are all non-compact shares. First sparse shares
include a 4-byte sequence length; continuation sparse shares do not.

**REQ-11.** `Share::raw_data()` starts after namespace, info byte, optional
sequence length, and compact-share reserved bytes exactly like Go
`Share.rawDataStartIndex`.

**REQ-12.** `Share::raw_data_using_reserved()` uses compact reserved bytes for
the first compact share in a sequence and returns an empty slice when reserved
bytes point to zero.

**REQ-13.** `ShareBuilder::new` pre-populates namespace, info byte, optional
sequence length placeholder, and compact reserved-byte placeholder exactly like
Go `Builder.prepareCompactShare` / `prepareSparseShare`.

**REQ-14.** `ShareBuilder::add_data` appends as much input as fits and returns
the remaining bytes when the share fills.

**REQ-15.** `CompactShareSplitter::write_tx` prefixes each tx with Go unsigned
varint length bytes, writes reserved bytes before the first unit in each share,
and records share ranges keyed by `sha256(tx)`.

**REQ-16.** `CompactShareSplitter::export` zero-pads the final pending share,
writes sequence length to the first share, is idempotent, and returns no shares
for an empty splitter.

**REQ-17.** `split_txs` writes ordinary txs to `TX_NAMESPACE` and index-wrapper
txs to `PAY_FOR_BLOB_NAMESPACE`, then offsets PFB share ranges by the number of
ordinary tx shares.

**REQ-18.** `SparseShareSplitter::write` validates blobs, rejects unsupported
share versions, writes sequence length equal to blob data length, emits
continuation shares as needed, and zero-pads the last share.

**REQ-19.** Namespace padding shares use the preceding blob namespace and
share version, set sequence length to zero, and fill the first sparse content
region with zeroes. Reserved and tail padding use their reserved namespaces.

**REQ-20.** `parse_compact_shares` validates share versions, concatenates raw
data with reserved-byte handling for the first share, parses varint-delimited
units, stops on zero-length delimiter padding, and stops without error when the
remaining bytes contain only a partial next unit.

**REQ-21.** `parse_sparse_shares` skips padding shares, starts a new blob on
sequence-start shares, appends continuation raw data to the previous blob, and
trims each blob to its declared sequence length.

**REQ-22.** `parse_sparse_shares` rejects a continuation share before any
sequence-start share and rejects unsupported share versions.

**REQ-23.** `get_share_range_for_namespace` assumes sorted shares, returns an
empty range when the namespace is outside the first/last namespace or absent,
and returns the end-exclusive contiguous range when present.

**REQ-24.** `Blob::validate_basic` rejects empty data, invalid namespace, and
share versions above `u8::MAX`; `Blob::to_shares` delegates to sparse splitting.

**REQ-25.** `IndexWrapper` encoding/decoding is byte-compatible with Go
`blob.IndexWrapper` protobuf shape and uses type ID `INDX`. Non-wrapper bytes
classify as `Ok(None)`.

## Conformance fixtures

Committed under `conformance/fixtures/shares/`:

- `namespaces.json` - namespace constructors, reserved namespace constants,
  ordering, and malformed namespace cases.
- `info-reserved.json` - info byte and compact reserved-byte encoding,
  boundaries, and malformed inputs.
- `builders.json` - compact/sparse builder initialization, data append,
  sequence length writing, reserved-byte writing, zero padding, and raw-data
  offsets.
- `compact.json` - tx delimiter encoding, compact share splitting, parsing,
  share ranges, PFB index-wrapper classification, and malformed compact shares.
- `sparse.json` - blob splitting, parsing, namespace padding, sparse
  continuation shares, and malformed sparse share sequences.
- `padding.json` - namespace, reserved, and tail padding shares.
- `malformed.json` - invalid share lengths, unsupported versions, invalid
  reserved offsets, invalid blobs, invalid ranges, and parse failures.

Regenerate fixtures:

```bash
cargo xtask refresh-fixtures --crate-name shares
```

Equivalent individual scenarios:

```bash
cargo xtask refresh-fixtures --scenario shares-namespaces
cargo xtask refresh-fixtures --scenario shares-info-reserved
cargo xtask refresh-fixtures --scenario shares-builders
cargo xtask refresh-fixtures --scenario shares-compact
cargo xtask refresh-fixtures --scenario shares-sparse
cargo xtask refresh-fixtures --scenario shares-padding
cargo xtask refresh-fixtures --scenario shares-malformed
```

Each scenario must be registered in BOTH the Go fixturegen
(`conformance/fixturegen/cmd/`) and the Rust `SCENARIOS` const in
`xtask/src/refresh_fixtures.rs`.

## Definition of Done

- [ ] Crate created at `crates/tier-1-protocol-types/shares/` with a
      `Cargo.toml` inheriting workspace metadata, and the crate path is added
      to root `Cargo.toml` `[workspace]` members.
- [ ] Public API above implemented with documented modules and
      `#![forbid(unsafe_code)]`.
- [ ] All REQ-1..REQ-25 covered by unit, property, or conformance tests.
- [ ] All seven conformance fixture files committed with `.meta.json` sidecars.
- [ ] `cargo test -p shares` green.
- [ ] `cargo clippy -p shares --all-targets -- -D warnings` green.
- [ ] `cargo fmt --check` green.
- [ ] `cargo llvm-cov -p shares --summary-only` reports >= 90 % line coverage.
- [ ] `crates/tier-1-protocol-types/shares/README.md` present with purpose /
      consumers / conformance.
- [ ] Share fixturegen scenarios registered in BOTH the Go fixturegen and the
      Rust `SCENARIOS` const.
- [ ] PR opened as draft with assignee `@rootulp`; PR body uses `Closes` for
      any tracked issue, or `Closes N/A` when no issue exists.

## Out of scope

- Data square layout, original data square sizing, reserved share placement
  across square rows, row/column roots, DAH construction, and erasure coding.
- NMT root/proof verification for shares.
- Cosmos SDK transaction decoding beyond raw bytes and index-wrapper
  classification.
- Blob transaction stateless validation against MsgPayForBlobs.
- Signer fields from newer go-square blob APIs; M1 targets go-square v1.0.1.
- Alternative share sizes or namespace versions beyond Go-supported versions.

## Notes for the implementing agent

- Start with red tests for namespace constants, info-byte bit packing,
  reserved-byte parsing, and fixed share length before adding splitters.
- Keep share namespace types local to this crate. Do not reuse the M1
  `nmt::Namespace`, which has a different width.
- Use `encoding` for Go-compatible unsigned varint helpers in compact shares.
- Use `hash::sha256` for share-range transaction keys.
- Prefer `BTreeMap<[u8; 32], ShareRange>` over `HashMap` in public results so
  fixture output and debug snapshots are deterministic.
- Implement compact shares and parsing before sparse blobs; compact reserved
  bytes drive several subtle parser behaviors.
- Treat padding constructors as ordinary share constructors plus fixed
  namespace/sequence-length rules so all parsing paths exercise the same
  validation code.
