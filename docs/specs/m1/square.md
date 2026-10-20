# Crate: `square`

**Tier:** 1 (protocol types)
**Depends on:** `shares`, `nmt`, `merkle`, `encoding`, `hash`
**Consumed by:** M1 integration tests, block data reconstruction,
data-availability header verification, share proofs, and later consensus /
RPC crates.
**Source of truth (Go):** `github.com/celestiaorg/go-square` v1.0.1 packages
`square` and `inclusion`, plus celestia-app v1.x `pkg/da`, `pkg/wrapper`, and
`github.com/celestiaorg/rsmt2d` for EDS / DAH behavior.
**Crate location:** `crates/tier-1-protocol-types/square/`

## Purpose

Provide Celestia original data square construction and data-availability
header primitives. The crate turns ordered block transactions into a square of
fixed-size shares, lays out ordinary transactions, wrapped PFB transactions,
reserved padding, blob shares, namespace padding, and tail padding exactly like
Go, extends the original data square with Reed-Solomon parity shares, and
computes the row/column roots that form a data availability header.

This crate is the layout and DAH layer above `shares`. It does not validate
Cosmos SDK transaction messages, fetch blocks, execute state transitions,
verify signatures, implement DAS sampling, or expose node RPC types. It may
include the minimal blob transaction wrapper format needed to identify and
round-trip blob transactions during square construction and deconstruction.

## Public API

The crate exports exactly these root modules:

```rust
//! Celestia data-square layout and data-availability header primitives.
//!
//! See `docs/specs/m1/square.md` for the API contract.

pub mod blob_tx;
pub mod builder;
pub mod commitment;
pub mod dah;
pub mod erasure;
pub mod error;
pub mod inclusion;
pub mod square;

pub use blob_tx::{BlobTx, PROTO_BLOB_TX_TYPE_ID};
pub use builder::{
    BuildResult, Builder, Element, build, blob_share_range, construct,
    deconstruct, tx_share_range, worst_case_share_indexes,
};
pub use commitment::{
    MerkleRootFn, create_commitment, create_commitments,
    merkle_mountain_range_sizes,
};
pub use dah::{
    DAH_HASH_SIZE, DAH_ROOT_SIZE, DataAvailabilityHeader,
    data_availability_header_from_proto, min_data_availability_header,
    new_data_availability_header,
};
pub use erasure::{
    ErasureCodec, ExtendedDataSquare, LeoRsCodec, compute_extended_data_square,
    extend_shares, import_extended_data_square,
};
pub use error::{Error, Result};
pub use inclusion::{
    blob_min_square_size, blob_shares_used_non_interactive_defaults,
    is_power_of_two, next_share_index, round_down_power_of_two,
    round_up_by_multiple_of, round_up_power_of_two,
    round_up_power_of_two_strict, sub_tree_width,
};
pub use square::{
    DEFAULT_MAX_SQUARE_SIZE, DEFAULT_SUBTREE_ROOT_THRESHOLD, Square,
    empty_square, size, write_square,
};
```

The crate sets `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.

### `blob_tx`

`BlobTx` is the minimal byte wrapper needed by `square`. It is not a Cosmos SDK
semantic validator. Its blob protobuf shape preserves the optional
`shares::Blob` signer bytes in field 5 for share-version-one blobs.

```rust
pub const PROTO_BLOB_TX_TYPE_ID: &str = "BLOB";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobTx {
    pub tx: Vec<u8>,
    pub blobs: Vec<shares::Blob>,
}

impl BlobTx {
    pub fn new(tx: Vec<u8>, blobs: Vec<shares::Blob>) -> Result<Self>;
    pub fn validate_basic(&self) -> Result<()>;
    pub fn encode(&self) -> Result<Vec<u8>>;
    pub fn decode(bytes: &[u8]) -> Result<Option<Self>>;
}
```

### `builder`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub blob: shares::Blob,
    pub pfb_index: usize,
    pub blob_index: usize,
    pub num_shares: usize,
    pub max_padding: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildResult {
    pub square: Square,
    pub included_txs: Vec<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct Builder {
    // fields are private
}

impl Builder {
    pub fn new(
        max_square_size: usize,
        subtree_root_threshold: usize,
        txs: &[Vec<u8>],
    ) -> Result<Self>;

    pub fn append_tx(&mut self, tx: Vec<u8>) -> bool;
    pub fn append_blob_tx(&mut self, blob_tx: BlobTx) -> bool;
    pub fn export(&mut self) -> Result<Square>;
    pub fn find_tx_share_range(&mut self, tx_index: usize) -> Result<shares::ShareRange>;
    pub fn find_blob_starting_index(
        &mut self,
        pfb_index: usize,
        blob_index: usize,
    ) -> Result<usize>;
    pub fn blob_share_length(
        &self,
        pfb_index: usize,
        blob_index: usize,
    ) -> Result<usize>;
    pub fn get_wrapped_pfb(&mut self, tx_index: usize) -> Result<shares::IndexWrapper>;
    pub const fn current_size(&self) -> usize;
    pub const fn subtree_root_threshold(&self) -> usize;
    pub fn num_pfbs(&self) -> usize;
    pub fn num_txs(&self) -> usize;
    pub fn is_empty(&self) -> bool;
}

pub fn build(
    txs: &[Vec<u8>],
    max_square_size: usize,
    subtree_root_threshold: usize,
) -> Result<BuildResult>;

pub fn construct(
    txs: &[Vec<u8>],
    max_square_size: usize,
    subtree_root_threshold: usize,
) -> Result<Square>;

pub fn deconstruct<D>(square: &Square, decoder: D) -> Result<Vec<Vec<u8>>>
where
    D: Fn(&[u8]) -> Result<Vec<u32>>;

pub fn tx_share_range(
    txs: &[Vec<u8>],
    tx_index: usize,
    max_square_size: usize,
    subtree_root_threshold: usize,
) -> Result<shares::ShareRange>;

pub fn blob_share_range(
    txs: &[Vec<u8>],
    tx_index: usize,
    blob_index: usize,
    max_square_size: usize,
    subtree_root_threshold: usize,
) -> Result<shares::ShareRange>;

pub fn worst_case_share_indexes(blob_count: usize) -> Vec<u32>;
```

### `square`

```rust
pub const DEFAULT_MAX_SQUARE_SIZE: usize = 128;
pub const DEFAULT_SUBTREE_ROOT_THRESHOLD: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Square {
    shares: Vec<shares::Share>,
}

impl Square {
    pub fn new(shares: Vec<shares::Share>) -> Result<Self>;
    pub fn from_shares_unchecked(shares: Vec<shares::Share>) -> Self;
    pub fn as_shares(&self) -> &[shares::Share];
    pub fn into_shares(self) -> Vec<shares::Share>;
    pub fn to_bytes(&self) -> Vec<[u8; shares::SHARE_SIZE]>;
    pub fn size(&self) -> usize;
    pub fn len(&self) -> usize;
    pub fn is_empty_square(&self) -> bool;
    pub fn wrapped_pfbs(&self) -> Result<Vec<Vec<u8>>>;
}

impl AsRef<[shares::Share]> for Square;
impl TryFrom<Vec<shares::Share>> for Square;

pub fn size(share_count: usize) -> usize;
pub fn empty_square() -> Square;

pub fn write_square(
    tx_writer: shares::CompactShareSplitter,
    pfb_writer: shares::CompactShareSplitter,
    blob_writer: shares::SparseShareSplitter,
    non_reserved_start: usize,
    square_size: usize,
) -> Result<Square>;
```

### `inclusion`

```rust
pub fn blob_shares_used_non_interactive_defaults(
    cursor: usize,
    subtree_root_threshold: usize,
    blob_share_lengths: &[usize],
) -> Result<(usize, Vec<u32>)>;

pub fn next_share_index(
    cursor: usize,
    blob_share_len: usize,
    subtree_root_threshold: usize,
) -> Result<usize>;

pub fn round_up_by_multiple_of(cursor: usize, value: usize) -> Result<usize>;
pub fn blob_min_square_size(share_count: usize) -> Result<usize>;
pub fn sub_tree_width(share_count: usize, subtree_root_threshold: usize) -> Result<usize>;
pub fn round_up_power_of_two(input: usize) -> usize;
pub fn round_down_power_of_two(input: usize) -> Result<usize>;
pub fn round_up_power_of_two_strict(input: usize) -> usize;
pub fn is_power_of_two(input: usize) -> bool;
```

### `commitment`

Blob commitments follow go-square `inclusion.CreateCommitment`. The `MerkleRootFn`
callback is intentionally byte-oriented so M1 can use the existing `merkle`
crate for the simple Merkle root over subtree roots.

```rust
pub type MerkleRootFn = fn(&[Vec<u8>]) -> [u8; DAH_HASH_SIZE];

pub fn create_commitment(
    blob: &shares::Blob,
    merkle_root_fn: MerkleRootFn,
    subtree_root_threshold: usize,
) -> Result<[u8; DAH_HASH_SIZE]>;

pub fn create_commitments(
    blobs: &[shares::Blob],
    merkle_root_fn: MerkleRootFn,
    subtree_root_threshold: usize,
) -> Result<Vec<[u8; DAH_HASH_SIZE]>>;

pub fn merkle_mountain_range_sizes(
    total_size: u64,
    max_tree_size: u64,
) -> Result<Vec<u64>>;
```

### `erasure`

The public erasure API mirrors `rsmt2d` but stays Rust-native. M1 targets the
Go default `LeoRSCodec` behavior from celestia-app v1.x
(`rsmt2d.NewLeoRSCodec`, which uses klauspost Leopard Reed-Solomon with equal
data/parity shard counts). The implementation should use a maintained Rust
crate only if conformance fixtures prove byte-identical parity output; otherwise
the crate must include a small compatibility adapter with fixtures guarding the
behavior.

```rust
pub trait ErasureCodec {
    fn encode(&self, data: &[Vec<u8>]) -> Result<Vec<Vec<u8>>>;
    fn decode(&self, data: &mut [Option<Vec<u8>>]) -> Result<Vec<Vec<u8>>>;
    fn max_chunks(&self) -> usize;
    fn name(&self) -> &'static str;
    fn validate_chunk_size(&self, chunk_size: usize) -> Result<()>;
}

#[derive(Debug, Default, Clone)]
pub struct LeoRsCodec;

impl LeoRsCodec {
    pub const fn new() -> Self;
}

impl ErasureCodec for LeoRsCodec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtendedDataSquare {
    original_width: usize,
    width: usize,
    shares: Vec<Vec<u8>>,
    codec_name: &'static str,
}

impl ExtendedDataSquare {
    pub fn new(
        codec: &dyn ErasureCodec,
        width: usize,
        share_size: usize,
    ) -> Result<Self>;
    pub fn import(
        shares: Vec<Vec<u8>>,
        codec: &dyn ErasureCodec,
    ) -> Result<Self>;
    pub const fn original_width(&self) -> usize;
    pub const fn width(&self) -> usize;
    pub fn row(&self, row_index: usize) -> Result<Vec<Vec<u8>>>;
    pub fn column(&self, column_index: usize) -> Result<Vec<Vec<u8>>>;
    pub fn flattened(&self) -> Vec<Vec<u8>>;
    pub fn flattened_ods(&self) -> Vec<Vec<u8>>;
    pub fn row_roots(&self) -> Result<Vec<[u8; DAH_ROOT_SIZE]>>;
    pub fn column_roots(&self) -> Result<Vec<[u8; DAH_ROOT_SIZE]>>;
    pub fn roots(&self) -> Result<Vec<[u8; DAH_ROOT_SIZE]>>;
}

pub fn compute_extended_data_square(
    shares: &[shares::Share],
    codec: &dyn ErasureCodec,
) -> Result<ExtendedDataSquare>;

pub fn extend_shares(shares: &[shares::Share]) -> Result<ExtendedDataSquare>;

pub fn import_extended_data_square(
    shares: Vec<Vec<u8>>,
    codec: &dyn ErasureCodec,
) -> Result<ExtendedDataSquare>;
```

### `dah`

DAH roots are raw Go-compatible NMT root bytes:
`min_namespace || max_namespace || digest`, where both namespaces are the
29-byte share namespace from `shares`, and the digest is SHA-256. Therefore
`DAH_ROOT_SIZE == 90`, not the current 8-byte-namespace `nmt::NODE_SIZE`.

```rust
pub const DAH_HASH_SIZE: usize = 32;
pub const DAH_ROOT_SIZE: usize = (2 * shares::NAMESPACE_SIZE) + DAH_HASH_SIZE;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataAvailabilityHeader {
    pub row_roots: Vec<[u8; DAH_ROOT_SIZE]>,
    pub column_roots: Vec<[u8; DAH_ROOT_SIZE]>,
}

impl DataAvailabilityHeader {
    pub fn new(
        row_roots: Vec<[u8; DAH_ROOT_SIZE]>,
        column_roots: Vec<[u8; DAH_ROOT_SIZE]>,
    ) -> Result<Self>;
    pub fn from_proto_bytes(bytes: &[u8]) -> Result<Self>;
    pub fn to_proto_bytes(&self) -> Result<Vec<u8>>;
    pub fn validate_basic(&self) -> Result<()>;
    pub fn hash(&self) -> [u8; DAH_HASH_SIZE];
    pub fn is_zero(&self) -> bool;
    pub fn equals(&self, other: &Self) -> bool;
}

pub fn new_data_availability_header(
    eds: &ExtendedDataSquare,
) -> Result<DataAvailabilityHeader>;

pub fn data_availability_header_from_proto(
    bytes: &[u8],
) -> Result<DataAvailabilityHeader>;

pub fn min_data_availability_header() -> DataAvailabilityHeader;
```

## Behavioral requirements

**REQ-1.** `size(share_count)` returns the power-of-two side length equal to
`RoundUpPowerOfTwo(ceil(sqrt(share_count)))`. `size(0)` returns `1`, matching
Go's helper behavior through `RoundUpPowerOfTwo`.

**REQ-2.** `Square::new` rejects non-power-of-two side lengths and malformed
shares. It accepts the one-share tail-padding empty square.

**REQ-3.** `empty_square()` returns a 1x1 square with exactly one tail-padding
share from `shares::tail_padding_shares(shares::MIN_SHARE_COUNT)`.

**REQ-4.** `Builder::new` rejects `max_square_size == 0`, rejects
non-power-of-two maximum square sizes, and rejects any ordinary transaction
that appears after the first blob transaction in the provided exact tx list.

**REQ-5.** `build` accepts an arbitrary prioritized tx list, appends ordinary
transactions before blob transactions, silently drops transactions that do not
fit, and returns the included txs in the exact order committed to the square.

**REQ-6.** `construct` treats the tx list as exact, preserves tx order, and
returns an error instead of dropping any tx that does not fit.

**REQ-7.** `append_tx` uses compact-share counting for the unsigned
varint-delimited tx bytes. It commits the tx and updates `current_size` only
when the tx fits within `max_square_size * max_square_size`; otherwise it
reverts the counter and returns `false`.

**REQ-8.** `append_blob_tx` wraps the original PFB tx bytes in
`shares::IndexWrapper`, estimates wrapper protobuf size using worst-case share
indexes of `128 * 128`, preserves version-one blob signer bytes when encoding
or decoding `BlobTx`, and tracks one `Element` for each blob.

**REQ-9.** `Element::num_shares` is `shares::SparseShareSplitter`'s required
share count for the blob data length. `Element::max_padding` is
`sub_tree_width(num_shares, subtree_root_threshold) - 1`.

**REQ-10.** Builder fitting for blob transactions uses the compact-share delta
for the worst-case wrapped PFB plus each blob's `num_shares + max_padding`.

**REQ-11.** `Builder::export` returns `empty_square()` when the builder has no
ordinary tx shares and no PFB shares.

**REQ-12.** `Builder::export` computes the original data square side length as
`blob_min_square_size(current_size)` and never exceeds `max_square_size`.

**REQ-13.** Blob elements are stable-sorted by their full 29-byte namespace.
Blobs with the same namespace preserve tx priority order.

**REQ-14.** Ordinary transactions are written to compact shares in
`shares::TX_NAMESPACE`. Wrapped PFB transactions are written to compact shares
in `shares::PAY_FOR_BLOB_NAMESPACE`.

**REQ-15.** Wrapped PFB compact shares are written only after blob start
indexes are finalized so each `shares::IndexWrapper.share_indexes` entry
contains the actual blob start index.

**REQ-16.** `non_reserved_start` starts at the counted ordinary tx share count
plus the counted PFB share count, then moves to the first blob's aligned start
index.

**REQ-17.** Each blob start index is
`next_share_index(cursor, num_shares, subtree_root_threshold)`. Inter-blob gaps
are filled with namespace padding shares belonging to the previous blob's
namespace and share version.

**REQ-18.** `write_square` lays out shares in this order: ordinary tx compact
shares at index `0`, PFB compact shares immediately after ordinary txs,
primary-reserved padding until `non_reserved_start`, sparse blob shares and
namespace padding from `non_reserved_start`, then tail padding through
`square_size * square_size`.

**REQ-19.** `write_square` rejects a `non_reserved_start` that cannot fit all
ordinary tx and PFB compact shares and rejects a square size that cannot fit all
blob shares.

**REQ-20.** `find_tx_share_range` forces export when needed, rejects
out-of-range tx indexes, and returns an end-exclusive share range for the
ordinary tx or wrapped PFB at the combined tx index.

**REQ-21.** `find_blob_starting_index`, `blob_share_length`, and
`blob_share_range` reject indexes that do not identify a PFB, reject
out-of-range blob indexes, and return end-exclusive share ranges.

**REQ-22.** `deconstruct` returns an empty tx list for `empty_square()`.

**REQ-23.** `deconstruct` requires ordinary tx shares to start at index `0`.
If PFB shares are present, they must begin immediately after ordinary tx shares.

**REQ-24.** `deconstruct` parses ordinary txs and wrapped PFB txs with
`shares::parse_txs`, rejects non-wrapper PFB payloads in the PFB namespace, and
rejects wrapped PFBs with zero blob share indexes.

**REQ-25.** `deconstruct` calls the provided decoder with each original PFB tx
bytes payload, requires the decoder's blob-size count to equal the wrapped
share-index count, parses exactly one blob from each indexed sparse range, and
reconstructs `BlobTx` bytes.

**REQ-26.** `blob_shares_used_non_interactive_defaults` iterates blob share
lengths from the provided cursor, applies `next_share_index` to each blob, and
returns both total shares used and the `u32` start indexes.

**REQ-27.** `round_up_by_multiple_of(cursor, value)` rejects `value == 0`,
returns `cursor` when already divisible, and otherwise rounds up to the next
multiple without overflow.

**REQ-28.** `blob_min_square_size(share_count)` rejects `share_count == 0` and
returns `RoundUpPowerOfTwo(ceil(sqrt(share_count)))`.

**REQ-29.** `sub_tree_width(share_count, subtree_root_threshold)` rejects zero
inputs, computes `ceil(share_count / subtree_root_threshold)`, rounds that up
to a power of two, and returns the minimum of that value and
`blob_min_square_size(share_count)`.

**REQ-30.** `round_down_power_of_two` rejects zero and returns the greatest
power of two less than or equal to the input. `round_up_power_of_two_strict`
returns the next strictly greater power of two when the input is already a
power of two.

**REQ-31.** `create_commitment` validates the blob, splits it using
`shares::split_blobs`, calculates MMR tree sizes using
`merkle_mountain_range_sizes(len(shares), sub_tree_width(...))`, and computes
one NMT subtree root per MMR part.

**REQ-32.** Blob commitment NMT leaves are `blob.namespace || share_bytes`.
The namespace is prepended even though the share already contains it. NMT
subtree roots use a 29-byte namespace size and `ignore_max_namespace = true`.

**REQ-33.** `create_commitment` returns `merkle_root_fn(subtree_roots)` with
the subtree roots in MMR order.

**REQ-34.** `merkle_mountain_range_sizes(total_size, max_tree_size)` repeatedly
uses `max_tree_size` chunks while possible, then consumes the greatest power of
two less than or equal to the remaining size. It rejects zero `max_tree_size`.

**REQ-35.** `extend_shares` rejects input whose share count is not a power of
two, computes the original square size with `square::size`, and uses the
celestia-app wrapper tree constructor semantics for row and column roots.

**REQ-36.** The default erasure codec is Go-compatible `LeoRSCodec`, not plain
Vandermonde Reed-Solomon. It must produce parity shares byte-identical to
`rsmt2d.NewLeoRSCodec` fixtures for widths covered by conformance tests.

**REQ-37.** EDS construction extends Q0 to Q1 by encoding each original row,
extends Q0 to Q2 by encoding each original column, then extends Q2 rows to Q3.
`flattened_ods` returns only the original Q0 quadrant.

**REQ-38.** `LeoRsCodec::validate_chunk_size` rejects share sizes that are not
multiples of 64 bytes. Celestia 512-byte shares must pass.

**REQ-39.** DAH row and column roots are computed over every EDS row/column
with the celestia-app erasured NMT wrapper: Q0 cells use their own first
29 bytes as namespace prefix; all non-Q0 cells use
`shares::PARITY_SHARES_NAMESPACE` as the prepended namespace.

**REQ-40.** DAH roots are exactly `DAH_ROOT_SIZE == 90` bytes. Implementations
must not serialize row/column roots as the existing 8-byte-namespace
`nmt::NamespacedHash` format.

**REQ-41.** `DataAvailabilityHeader::hash` returns the CometBFT simple Merkle
root over `row_roots || column_roots`, matching `merkle.HashFromByteSlices`.
Nil/zero behavior follows Go: an empty DAH hashes as the simple Merkle root of
an empty list.

**REQ-42.** `DataAvailabilityHeader::validate_basic` rejects nil-equivalent
headers, row/column count mismatches, fewer than `2 * shares::MIN_SQUARE_SIZE`
roots per axis, more than `2 * DEFAULT_MAX_SQUARE_SIZE` roots per axis, and
headers whose hash is not a valid 32-byte CometBFT hash.

**REQ-43.** `min_data_availability_header()` returns the DAH for
`empty_square()` extended with the default codec.

**REQ-44.** DAH protobuf encoding/decoding uses the celestia
`DataAvailabilityHeader` field order: repeated `row_roots` field 1 and
repeated `column_roots` field 2. Unknown fields are ignored on decode.

**REQ-45.** All public APIs return crate-local `Error` values with enough
context to distinguish invalid sizing, malformed tx/blob wrappers, share
layout violations, erasure codec failures, root-size failures, and protobuf
decode failures.

## Conformance fixtures

Committed under `conformance/fixtures/square/`:

- `powers-of-two.json` - power-of-two helpers, square sizing, and malformed
  zero/overflow cases.
- `inclusion-rules.json` - `sub_tree_width`, `next_share_index`,
  `blob_min_square_size`, and multi-blob share-index allocation.
- `blob-tx.json` - blob transaction wrapper marshal/unmarshal, non-wrapper
  classification, and malformed wrapper bytes.
- `builders.json` - builder fitting, tx ordering, blob namespace sorting,
  wrapped PFB indexes, and `Build` versus `Construct` behavior.
- `squares.json` - empty square, `WriteSquare` layout, reserved padding,
  namespace padding, tail padding, wrapped PFB extraction, and malformed square
  sizes.
- `ranges.json` - tx share ranges, blob share ranges, and error cases for
  non-PFB / out-of-range indexes.
- `deconstruct.json` - square-to-transaction round trips, PFB decoder output
  handling, missing PFB ranges, and malformed sparse blob ranges.
- `commitments.json` - MMR tree sizes, blob commitment subtree roots, and final
  commitments for small and threshold-crossing blobs.
- `erasure.json` - LeoRS parity shares, EDS import/export, Q0 round trips,
  root computation over rows/columns, and malformed chunk-size cases.
- `dah.json` - minimum DAH, DAH roots, DAH hash, protobuf conversion, and
  `ValidateBasic` error cases.
- `mainnet-blocks.json` - at least 100 sampled mainnet block data inputs with
  expected ordered txs, EDS roots, DAH hash, and block height. The fixture
  intentionally omits derived ODS share dumps so the corpus stays below
  repository size warning thresholds.
- `malformed.json` - invalid input combinations not covered by the focused
  fixtures.

Regenerate fixtures:

```bash
cargo xtask refresh-fixtures --crate-name square
```

Equivalent individual scenarios:

```bash
cargo xtask refresh-fixtures --scenario square-powers-of-two
cargo xtask refresh-fixtures --scenario square-inclusion-rules
cargo xtask refresh-fixtures --scenario square-blob-tx
cargo xtask refresh-fixtures --scenario square-builders
cargo xtask refresh-fixtures --scenario square-squares
cargo xtask refresh-fixtures --scenario square-ranges
cargo xtask refresh-fixtures --scenario square-deconstruct
cargo xtask refresh-fixtures --scenario square-commitments
cargo xtask refresh-fixtures --scenario square-erasure
cargo xtask refresh-fixtures --scenario square-dah
cargo xtask refresh-fixtures --scenario square-mainnet-blocks
cargo xtask refresh-fixtures --scenario square-malformed
```

Populate additional cached source blocks for the mainnet corpus:

```bash
cd conformance/fixturegen
go run . square-fetch-mainnet-blocks --rpc <cometbft-rpc-url> --start <height> --count <n>
```

Each scenario must be registered in BOTH the Go fixturegen
(`conformance/fixturegen/cmd/`) and the Rust `SCENARIOS` const in
`xtask/src/refresh_fixtures.rs`.

## Definition of Done

- [ ] Crate created at `crates/tier-1-protocol-types/square/` with a
      `Cargo.toml` inheriting workspace metadata, and the crate path is added
      to root `Cargo.toml` `[workspace]` members.
- [ ] Public API above implemented with documented modules and
      `#![forbid(unsafe_code)]`.
- [ ] All REQ-1..REQ-45 covered by unit, property, or conformance tests.
- [ ] All twelve conformance fixture files committed with `.meta.json`
      sidecars.
- [ ] `cargo test -p square` green.
- [ ] `cargo clippy -p square --all-targets -- -D warnings` green.
- [ ] `cargo fmt --check` green.
- [ ] `cargo llvm-cov -p square --summary-only` reports >= 90 % line coverage.
- [ ] `crates/tier-1-protocol-types/square/README.md` present with purpose /
      consumers / conformance.
- [ ] Square fixturegen scenarios registered in BOTH the Go fixturegen and the
      Rust `SCENARIOS` const.
- [ ] M1 integration acceptance test
      `m1-integration::parse_real_mainnet_block_produces_matching_dah` passes
      for at least 100 mainnet block fixtures.
- [ ] PR opened as draft with assignee `@rootulp`; PR body uses `Closes` for
      any tracked issue, or `Closes N/A` when no issue exists.

## Out of scope

- Cosmos SDK `MsgPayForBlobs` semantic validation, fee checking, signer
  validation, or transaction execution.
- DAS sampling, fraud proof generation, share proof RPC types, and network
  retrieval.
- Stateful block replay, app hash calculation, or module stores.
- Historical square layout versions older than the current mainnet v1.x layout.
- Erasure codec performance tuning beyond correctness against Go fixtures.
- Replacing celestia-app's DAH hash or protobuf shape.

## Notes for the implementing agent

- Start with red tests for power-of-two helpers and `WriteSquare` layout before
  adding builder state. These are the easiest places to catch off-by-one errors.
- Keep `BlobTx` minimal. Its job is to classify and round-trip wrapper bytes for
  square construction, not to validate Cosmos SDK messages.
- Implement builder/export/deconstruct before DAH. That produces ODS fixtures
  that can be reused by the EDS and DAH scenarios.
- Do not use `nmt::NamespacedHash` directly for DAH row/column roots unless the
  `nmt` crate has first been generalized to 29-byte namespaces without breaking
  its existing API. A private 29-byte NMT adapter inside `square` is acceptable
  for M1 if it is fully fixture-tested.
- For blob commitments and DAH wrapper roots, the NMT leaf payload intentionally
  prepends the namespace to bytes that already include the namespace. Preserve
  that duplication.
- Verify any Rust erasure dependency against Go-generated LeoRS fixtures before
  committing to it. Plain Reed-Solomon crates are not acceptable unless parity
  bytes match `rsmt2d.NewLeoRSCodec`.
- Use deterministic fixture ordering and include both compact human-readable
  fields and raw hex bytes in Go fixtures so layout failures are easy to debug.
