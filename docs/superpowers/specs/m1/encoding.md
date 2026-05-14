# Crate: `encoding`

**Tier:** 0 (primitives)
**Depends on:** _(none - pure primitive)_
**Consumed by:** `merkle`, `crypto`, `tendermint-types`, `cosmos-types`, `abci-types`, `nmt`, `shares`, `square`, and downstream app/runtime crates.
**Source of truth (Go):** `encoding/binary`, `encoding/hex`, `encoding/base64`, `google.golang.org/protobuf/encoding/protowire`, and Cosmos SDK bech32 helpers at `github.com/cosmos/cosmos-sdk/types/bech32`.
**Crate location:** `crates/tier-0-primitives/encoding/`

## Purpose

Provide the byte-encoding helpers used throughout celestia-rs: protobuf via
`prost`, Tendermint-compatible varints, canonical hex, standard and URL-safe
base64, and Celestia bech32 address strings.

This crate is a thin facade over commodity Rust crates plus small, audited
helpers where wire compatibility matters. It does not define protocol messages
or address semantics beyond encoding bytes with the Celestia account,
validator-operator, and validator-consensus prefixes.

## Public API

The crate exports exactly these root modules:

```rust
//! Encoding primitives for celestia-rs.
//!
//! See `docs/superpowers/specs/m1/encoding.md` for the API contract.

pub mod protobuf;
pub mod varint;
pub mod hex;
pub mod base64;
pub mod bech32;
```

The crate sets `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.

### `protobuf`

```rust
/// Re-export the bytes traits and buffer types used by prost.
pub use bytes::{Buf, BufMut, Bytes, BytesMut};

/// Re-export the prost types used across the workspace.
pub use prost::{DecodeError, EncodeError, Message};

/// Encode a prost message into a new byte vector.
pub fn encode_to_vec<M: Message>(message: &M) -> Vec<u8>;

/// Encode a prost message with a protobuf length delimiter into a new byte vector.
pub fn encode_length_delimited_to_vec<M: Message>(message: &M) -> Vec<u8>;

/// Decode a prost message from a complete byte slice.
pub fn decode<M>(bytes: &[u8]) -> Result<M, DecodeError>
where
    M: Message + Default;

/// Decode a length-delimited prost message from a complete byte slice.
pub fn decode_length_delimited<M>(bytes: &[u8]) -> Result<M, DecodeError>
where
    M: Message + Default;
```

### `varint`

```rust
/// Maximum number of bytes in a Go-compatible 64-bit varint.
pub const MAX_VARINT_LEN64: usize = 10;

/// Errors returned by canonical varint decoding and bounded-buffer writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The caller-provided output buffer is too small.
    BufferTooSmall { required: usize, available: usize },
    /// The byte slice ended before a terminating byte was found.
    Incomplete,
    /// The encoded integer exceeds the 64-bit varint range.
    Overflow,
    /// The byte sequence is valid LEB128 but not the shortest canonical form.
    NonCanonical,
}

impl core::fmt::Display for Error;
impl std::error::Error for Error;

/// Return the canonical byte length of `value` when encoded as a uvarint.
pub const fn encoded_uvarint_len(value: u64) -> usize;

/// Return the canonical byte length of `value` when encoded as a signed varint.
pub const fn encoded_varint_len(value: i64) -> usize;

/// Encode `value` as Go `binary.PutUvarint` bytes.
pub fn encode_uvarint(value: u64) -> Vec<u8>;

/// Encode `value` as Go `binary.PutVarint` bytes.
pub fn encode_varint(value: i64) -> Vec<u8>;

/// Encode `value` into `dst`, returning the number of bytes written.
pub fn put_uvarint(dst: &mut [u8], value: u64) -> Result<usize, Error>;

/// Encode `value` into `dst`, returning the number of bytes written.
pub fn put_varint(dst: &mut [u8], value: i64) -> Result<usize, Error>;

/// Decode a canonical Go `binary.Uvarint` byte sequence.
pub fn decode_uvarint(src: &[u8]) -> Result<(u64, usize), Error>;

/// Decode a canonical Go `binary.Varint` byte sequence.
pub fn decode_varint(src: &[u8]) -> Result<(i64, usize), Error>;
```

### `hex`

```rust
/// Re-export the hex decoding error type.
pub use hex::FromHexError;

/// Encode bytes to canonical lowercase hex.
pub fn encode(bytes: &[u8]) -> String;

/// Decode lowercase or uppercase hex into bytes.
pub fn decode(input: &str) -> Result<Vec<u8>, FromHexError>;
```

### `base64`

```rust
/// Re-export the base64 decoding error type.
pub use base64::DecodeError;

/// Encode bytes with padded RFC 4648 standard base64.
pub fn encode_standard(bytes: &[u8]) -> String;

/// Decode padded RFC 4648 standard base64.
pub fn decode_standard(input: &str) -> Result<Vec<u8>, DecodeError>;

/// Encode bytes with padded RFC 4648 URL-safe base64.
pub fn encode_url_safe(bytes: &[u8]) -> String;

/// Decode padded RFC 4648 URL-safe base64.
pub fn decode_url_safe(input: &str) -> Result<Vec<u8>, DecodeError>;
```

### `bech32`

```rust
/// Celestia account address human-readable prefix.
pub const ACCOUNT_PREFIX: &str = "celestia";

/// Celestia validator operator address human-readable prefix.
pub const VALIDATOR_OPERATOR_PREFIX: &str = "celestiavaloper";

/// Celestia validator consensus address human-readable prefix.
pub const VALIDATOR_CONSENSUS_PREFIX: &str = "celestiavalcons";

/// Canonical Celestia address byte length.
pub const ADDRESS_LENGTH: usize = 20;

/// Canonical Celestia address bytes.
pub type AddressBytes = [u8; ADDRESS_LENGTH];

/// Errors returned by bech32 helpers.
#[derive(Debug)]
pub enum Error {
    /// The bech32 string failed checksum, character-set, or variant validation.
    Invalid(::bech32::Error),
    /// The decoded human-readable prefix did not match the expected prefix.
    WrongPrefix { expected: &'static str, actual: String },
    /// The decoded data length did not match the expected address length.
    WrongLength { expected: usize, actual: usize },
}

impl core::fmt::Display for Error;
impl std::error::Error for Error;
impl From<::bech32::Error> for Error;

/// Encode arbitrary bytes with an explicit bech32 human-readable prefix.
pub fn encode(prefix: &str, bytes: &[u8]) -> Result<String, Error>;

/// Decode a bech32 string into its human-readable prefix and data bytes.
pub fn decode(input: &str) -> Result<(String, Vec<u8>), Error>;

/// Encode a 20-byte Celestia account address.
pub fn encode_account_address(bytes: &AddressBytes) -> Result<String, Error>;

/// Decode a Celestia account address, enforcing prefix and 20-byte length.
pub fn decode_account_address(input: &str) -> Result<AddressBytes, Error>;

/// Encode a 20-byte Celestia validator operator address.
pub fn encode_validator_operator_address(bytes: &AddressBytes) -> Result<String, Error>;

/// Decode a Celestia validator operator address, enforcing prefix and 20-byte length.
pub fn decode_validator_operator_address(input: &str) -> Result<AddressBytes, Error>;

/// Encode a 20-byte Celestia validator consensus address.
pub fn encode_validator_consensus_address(bytes: &AddressBytes) -> Result<String, Error>;

/// Decode a Celestia validator consensus address, enforcing prefix and 20-byte length.
pub fn decode_validator_consensus_address(input: &str) -> Result<AddressBytes, Error>;
```

## Behavioral requirements

**REQ-1.** `protobuf::encode_to_vec` and `protobuf::decode` round-trip any
`prost::Message + Default` value without altering `prost`'s wire bytes.

**REQ-2.** `protobuf::encode_length_delimited_to_vec` and
`protobuf::decode_length_delimited` use protobuf's canonical unsigned varint
length prefix and match Go `protowire` length-delimited bytes for fixture
messages.

**REQ-3.** `varint::encode_uvarint` exactly matches Go
`encoding/binary.PutUvarint` for all `u64` values. Boundary vectors must include
`0`, `1`, `127`, `128`, `255`, `300`, `16_383`, `16_384`, `u32::MAX`, and
`u64::MAX`.

**REQ-4.** `varint::encode_varint` exactly matches Go
`encoding/binary.PutVarint` for all `i64` values. Boundary vectors must include
`0`, `1`, `-1`, `63`, `64`, `-64`, `-65`, `i32::MIN`, `i32::MAX`,
`i64::MIN`, and `i64::MAX`.

**REQ-5.** `varint::decode_uvarint` and `varint::decode_varint` accept only
canonical encodings emitted by the corresponding encode function. They return
`Error::Incomplete` for unterminated byte slices, `Error::Overflow` for values
outside the 64-bit range, and `Error::NonCanonical` for overlong encodings such
as `[0x80, 0x00]`.

**REQ-6.** `varint::put_uvarint` and `varint::put_varint` must not partially
write when `dst` is too small; they return `Error::BufferTooSmall` with the
required and available lengths.

**REQ-7.** `hex::encode` always returns lowercase hex. `hex::decode` accepts
lowercase and uppercase input and rejects odd-length or non-hex input through
`hex::FromHexError`.

**REQ-8.** `base64::encode_standard` and `base64::decode_standard` use padded
RFC 4648 standard base64. `base64::encode_url_safe` and
`base64::decode_url_safe` use padded RFC 4648 URL-safe base64.

**REQ-9.** `bech32::encode` and `bech32::decode` use original bech32, not
bech32m, matching Cosmos SDK address encoding.

**REQ-10.** Account, validator-operator, and validator-consensus bech32 helpers
must enforce the exact prefixes `celestia`, `celestiavaloper`, and
`celestiavalcons`, respectively, and must enforce 20-byte address payloads.

**REQ-11.** All public functions are deterministic and side-effect-free.

**REQ-12.** No crate outside `encoding` should depend directly on `prost`,
`bytes`, `hex`, `base64`, or `bech32` for these primitive helpers unless a
future crate spec explicitly grants an exception.

## Conformance fixtures

Committed at `conformance/fixtures/encoding/`:

- `encoding/protobuf.json` - Go `protowire` bytes for a small deterministic
  fixture message, including raw and length-delimited encodings.
- `encoding/varint.json` - signed and unsigned varint boundary vectors generated
  with Go `encoding/binary.PutUvarint` and `encoding/binary.PutVarint`, plus
  malformed canonical-decoder edge cases.
- `encoding/hex-base64.json` - byte vectors with lowercase hex, uppercase hex
  input, padded standard base64, and padded URL-safe base64 generated by Go
  `encoding/hex` and `encoding/base64`.
- `encoding/bech32-addresses.json` - the existing bootstrap bech32 address
  vectors reused for `celestia` and `celestiavaloper`, extended with
  `celestiavalcons`.

Regeneration is split into one-file scenarios:

```bash
cargo xtask refresh-fixtures --crate-name encoding
```

Equivalent individual scenarios:

```bash
cargo xtask refresh-fixtures --scenario encoding-protobuf
cargo xtask refresh-fixtures --scenario encoding-varint
cargo xtask refresh-fixtures --scenario encoding-hex-base64
cargo xtask refresh-fixtures --scenario encoding-bech32-addresses
```

Each scenario must be registered in BOTH the Go fixturegen
(`conformance/fixturegen/cmd/`) and the Rust `SCENARIOS` const in
`xtask/src/refresh_fixtures.rs`.

## Definition of done

- [ ] Crate created at `crates/tier-0-primitives/encoding/` with a
      `Cargo.toml` that inherits workspace metadata (`edition.workspace = true`,
      etc.), and the crate path is added to the root `Cargo.toml` `[workspace]`
      members list.
- [ ] All REQ-1..REQ-12 covered by unit, property, or conformance tests.
- [ ] All four conformance fixture files committed with `.meta.json` sidecars.
- [ ] `cargo test -p encoding` green.
- [ ] `cargo clippy -p encoding --all-targets -- -D warnings` green.
- [ ] `cargo fmt --check` green.
- [ ] `cargo llvm-cov -p encoding --summary-only` reports >= 90 % line
      coverage.
- [ ] `crates/tier-0-primitives/encoding/README.md` present with purpose /
      consumers / conformance.
- [ ] Encoding fixturegen scenario(s) registered in BOTH the Go fixturegen
      (`conformance/fixturegen/cmd/`) AND the Rust `SCENARIOS` const in
      `xtask/src/refresh_fixtures.rs`.
- [ ] PR opened as draft with assignee `@rootulp`; PR body uses `Closes` for
      any tracked issue, or `Closes N/A` when no issue exists.

## Out of scope

- Defining or generating Celestia, Cosmos, CometBFT, or ABCI protobuf types.
- Amino, JSON, YAML, TOML, SSZ, SCALE, or other codecs.
- Key parsing, address derivation from public keys, or checksum schemes beyond
  bech32.
- Bech32m.
- Base64 no-padding helpers unless a downstream crate spec requires them.
- Replacing `hash::tmhash_len_prefixed`'s private varint helper in this PR.
  A future cleanup may switch it to `encoding::varint` once the dependency
  direction is explicitly accepted.

## Open questions

- None. `prost` is the chosen protobuf implementation in the top-level M1
  design and this crate intentionally keeps the wrapper thin.

## Notes for the implementing agent

1. **Scaffold the crate.** Create
   `crates/tier-0-primitives/encoding/{Cargo.toml,src/lib.rs}` and add the path
   to the root `Cargo.toml` `[workspace] members`.
2. **Write tests first.** Translate REQ-1..REQ-10 into named unit tests. Add
   proptests for varint signed/unsigned round-trips and protobuf round-trips.
3. **Implement module by module.** Keep wrappers thin. Do not add a custom
   serialization framework on top of `prost`.
4. **Keep varint canonical.** Encoding must match Go exactly. Decoding should
   reject overlong encodings even if Go's `binary.Uvarint` can parse them.
5. **Add fixturegen scenarios.** Use Go standard library packages for varint,
   hex, and base64; use `protowire` for protobuf; use Cosmos SDK bech32 helpers
   for address strings.
6. **Regenerate fixtures** with
   `cargo xtask refresh-fixtures --crate-name encoding` and commit the JSON
   files plus static `.meta.json` sidecars.
7. **Add conformance tests** under
   `crates/tier-0-primitives/encoding/tests/conformance.rs` following the hash
   crate's workspace-root helper pattern.
8. **Verify the full crate DoD** before opening the draft PR.
