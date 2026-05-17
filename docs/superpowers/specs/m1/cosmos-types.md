# Crate: `cosmos-types`

**Tier:** 1 (protocol types)
**Depends on:** `hash`, `encoding`, `crypto`, `tendermint-types`
**Consumed by:** `abci-types`, `shares`, `square`, M1 integration tests, M3
transaction/ante machinery, and downstream app/runtime crates.
**Source of truth (Go):** Cosmos SDK v0.50.10
`github.com/cosmos/cosmos-sdk/types`,
`github.com/cosmos/cosmos-sdk/types/tx`,
`github.com/cosmos/cosmos-sdk/types/tx/signing`,
`github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1`, and Celestia bech32
prefix configuration from celestia-app.
**Crate location:** `crates/tier-1-protocol-types/cosmos-types/`

## Purpose

Provide the Cosmos SDK protocol types that Celestia needs before module logic:
transaction raw bytes, direct sign docs, auth-info structures, signer info,
coins, protobuf `Any`, secp256k1 account public keys, and typed Celestia
addresses.

This crate is a wire-compatibility layer. It does not execute messages, run the
ante handler, maintain account state, price gas, route messages, or implement
Cosmos SDK modules. Those belong to M3 and later crates.

## Public API

The crate exports exactly these root modules:

```rust
//! Cosmos SDK protocol types for celestia-rs.
//!
//! See `docs/superpowers/specs/m1/cosmos-types.md` for the API contract.

pub mod address;
pub mod any;
pub mod coin;
pub mod error;
pub mod proto;
pub mod tx;

pub use address::{
    AccountAddress, ValconsAddress, ValoperAddress, ADDRESS_LENGTH, account_address_from_bech32,
    account_address_to_bech32, valcons_address_from_bech32, valcons_address_to_bech32,
    valoper_address_from_bech32, valoper_address_to_bech32,
};
pub use any::Any;
pub use coin::Coin;
pub use error::{Error, Result};
pub use tx::{
    AuthInfo, Fee, ModeInfo, SignDoc, SignMode, SignerInfo, Tip, Tx, TxBody, TxRaw,
    secp256k1_public_key_any,
};
```

The crate sets `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.

### `address`

```rust
/// Size in bytes of Cosmos account and validator addresses.
pub const ADDRESS_LENGTH: usize = 20;

/// A 20-byte Celestia account address.
pub type AccountAddress = [u8; ADDRESS_LENGTH];

/// A 20-byte Celestia validator-operator address.
pub type ValoperAddress = [u8; ADDRESS_LENGTH];

/// A 20-byte Celestia validator-consensus address.
pub type ValconsAddress = [u8; ADDRESS_LENGTH];

/// Encode an account address with the `celestia` bech32 prefix.
pub fn account_address_to_bech32(address: &AccountAddress) -> String;

/// Decode and validate a `celestia` bech32 account address.
pub fn account_address_from_bech32(value: &str) -> Result<AccountAddress>;

/// Encode a validator-operator address with the `celestiavaloper` prefix.
pub fn valoper_address_to_bech32(address: &ValoperAddress) -> String;

/// Decode and validate a `celestiavaloper` bech32 validator-operator address.
pub fn valoper_address_from_bech32(value: &str) -> Result<ValoperAddress>;

/// Encode a validator-consensus address with the `celestiavalcons` prefix.
pub fn valcons_address_to_bech32(address: &ValconsAddress) -> String;

/// Decode and validate a `celestiavalcons` bech32 validator-consensus address.
pub fn valcons_address_from_bech32(value: &str) -> Result<ValconsAddress>;
```

### `any`

```rust
/// Cosmos protobuf `Any`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Any {
    /// Type URL, for example `/cosmos.crypto.secp256k1.PubKey`.
    pub type_url: String,
    /// Raw protobuf value bytes.
    pub value: Vec<u8>,
}

impl Any {
    /// Perform Cosmos SDK stateless `Any` validation.
    pub fn validate_basic(&self) -> Result<()>;

    /// Convert protobuf into a domain `Any`.
    pub fn from_proto(proto: proto::google::Any) -> Result<Self>;

    /// Convert this value to protobuf.
    pub fn to_proto(&self) -> proto::google::Any;
}
```

### `coin`

```rust
/// Cosmos SDK coin with a canonical decimal integer amount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coin {
    /// Denomination.
    pub denom: String,
    /// Base-10 integer amount string.
    pub amount: String,
}

impl Coin {
    /// Create and validate a coin.
    pub fn new(denom: impl Into<String>, amount: impl Into<String>) -> Result<Self>;

    /// Perform Cosmos SDK stateless coin validation.
    pub fn validate_basic(&self) -> Result<()>;

    /// Convert protobuf into a domain coin.
    pub fn from_proto(proto: proto::base::Coin) -> Result<Self>;

    /// Convert this coin to protobuf.
    pub fn to_proto(&self) -> proto::base::Coin;
}
```

### `tx`

```rust
/// Cosmos SDK sign modes needed by M1/M3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum SignMode {
    /// Unspecified sign mode.
    Unspecified = 0,
    /// SIGN_MODE_DIRECT.
    Direct = 1,
    /// SIGN_MODE_TEXTUAL.
    Textual = 2,
    /// SIGN_MODE_DIRECT_AUX.
    DirectAux = 3,
    /// SIGN_MODE_LEGACY_AMINO_JSON.
    LegacyAminoJson = 127,
    /// SIGN_MODE_EIP_191.
    Eip191 = 191,
}

impl TryFrom<i32> for SignMode;
impl From<SignMode> for i32;

/// Cosmos SDK mode info.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModeInfo {
    /// Single-signature mode.
    Single { mode: SignMode },
    /// Multisig mode with nested mode infos and compact bit array bytes.
    Multi {
        bitarray_extra_bits_stored: u32,
        bitarray_elems: Vec<u8>,
        mode_infos: Vec<ModeInfo>,
    },
}

/// Signer info for one transaction signer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerInfo {
    /// Optional public key `Any`.
    pub public_key: Option<Any>,
    /// Signing mode info.
    pub mode_info: ModeInfo,
    /// Account sequence.
    pub sequence: u64,
}

/// Transaction fee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fee {
    /// Fee amount.
    pub amount: Vec<Coin>,
    /// Gas limit.
    pub gas_limit: u64,
    /// Optional payer address string.
    pub payer: String,
    /// Optional granter address string.
    pub granter: String,
}

/// Optional transaction tip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tip {
    /// Tip amount.
    pub amount: Vec<Coin>,
    /// Tipper address string.
    pub tipper: String,
}

/// Cosmos SDK auth info.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthInfo {
    /// Signer infos in protobuf order.
    pub signer_infos: Vec<SignerInfo>,
    /// Fee.
    pub fee: Option<Fee>,
    /// Optional tip.
    pub tip: Option<Tip>,
}

/// Cosmos SDK transaction body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxBody {
    /// Message `Any` values in protobuf order.
    pub messages: Vec<Any>,
    /// Memo.
    pub memo: String,
    /// Timeout height.
    pub timeout_height: u64,
    /// Extension options.
    pub extension_options: Vec<Any>,
    /// Non-critical extension options.
    pub non_critical_extension_options: Vec<Any>,
}

/// Decoded Cosmos SDK transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tx {
    /// Transaction body.
    pub body: Option<TxBody>,
    /// Auth info.
    pub auth_info: Option<AuthInfo>,
    /// Signatures in signer order.
    pub signatures: Vec<Vec<u8>>,
}

/// Raw Cosmos SDK transaction bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxRaw {
    /// Encoded `TxBody` bytes.
    pub body_bytes: Vec<u8>,
    /// Encoded `AuthInfo` bytes.
    pub auth_info_bytes: Vec<u8>,
    /// Signatures in signer order.
    pub signatures: Vec<Vec<u8>>,
}

impl TxRaw {
    /// Decode protobuf `cosmos.tx.v1beta1.TxRaw`.
    pub fn decode(bytes: &[u8]) -> Result<Self>;

    /// Encode protobuf `cosmos.tx.v1beta1.TxRaw`.
    pub fn encode_to_vec(&self) -> Result<Vec<u8>>;

    /// Return the Cosmos SDK transaction hash: SHA-256 over encoded `TxRaw`.
    pub fn hash(&self) -> Result<hash::Hash>;
}

/// SIGN_MODE_DIRECT sign doc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignDoc {
    /// Encoded `TxBody` bytes.
    pub body_bytes: Vec<u8>,
    /// Encoded `AuthInfo` bytes.
    pub auth_info_bytes: Vec<u8>,
    /// Chain ID.
    pub chain_id: String,
    /// Account number.
    pub account_number: u64,
}

impl SignDoc {
    /// Perform stateless sign-doc validation.
    pub fn validate_basic(&self) -> Result<()>;

    /// Decode protobuf `cosmos.tx.v1beta1.SignDoc`.
    pub fn decode(bytes: &[u8]) -> Result<Self>;

    /// Encode protobuf `cosmos.tx.v1beta1.SignDoc`.
    pub fn encode_to_vec(&self) -> Result<Vec<u8>>;

    /// Return bytes signed for SIGN_MODE_DIRECT.
    pub fn sign_bytes(&self) -> Result<Vec<u8>>;
}

/// Pack a compressed secp256k1 public key as Cosmos SDK `Any`.
pub fn secp256k1_public_key_any(public_key: &crypto::secp256k1::PublicKeyBytes) -> Any;
```

### `proto`

The crate owns generated `prost` types for only the protobuf files needed by
M1:

```rust
pub mod base;    // cosmos.base.v1beta1
pub mod crypto;  // cosmos.crypto.secp256k1
pub mod google;  // google.protobuf.Any
pub mod tx;      // cosmos.tx.v1beta1
pub mod signing; // cosmos.tx.signing.v1beta1
```

The generated modules must be private to protocol compatibility work in this
crate and downstream wire-codec boundaries. Public domain types above are the
preferred API.

### `error`

```rust
/// Errors returned by Cosmos SDK protocol type helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidAddressLength { actual: usize },
    InvalidAddress,
    InvalidAddressPrefix { expected: &'static str, actual: String },
    InvalidAny,
    InvalidCoin,
    InvalidDenom,
    InvalidAmount,
    InvalidFee,
    InvalidModeInfo,
    InvalidSignMode,
    InvalidSignerInfo,
    InvalidSignDoc,
    InvalidTx,
    InvalidPublicKey,
    Decode(String),
    Encode(String),
}

impl core::fmt::Display for Error;
impl std::error::Error for Error;

/// Result alias for this crate.
pub type Result<T> = core::result::Result<T, Error>;
```

## Behavioral requirements

**REQ-1.** Generated protobuf structs match Cosmos SDK v0.50.10 field numbers,
oneof names, enum values, and package paths for the included files.

**REQ-2.** `TxRaw::decode` and `TxRaw::encode_to_vec` round-trip
`cosmos.tx.v1beta1.TxRaw` bytes without losing unknown-free fields.

**REQ-3.** `TxRaw::hash` returns SHA-256 over the encoded `TxRaw` bytes and
matches Cosmos SDK `Tx.Hash()` / `TxRaw` hashing for the fixture corpus.

**REQ-4.** `TxBody`, `AuthInfo`, `SignerInfo`, `ModeInfo`, `Fee`, `Tip`, and
`Tx` protobuf conversion preserves field order and optional field semantics.

**REQ-5.** `SignDoc::sign_bytes` matches Cosmos SDK SIGN_MODE_DIRECT sign bytes:
the protobuf encoding of `cosmos.tx.v1beta1.SignDoc`, not length-delimited and
not JSON encoded.

**REQ-6.** `SignMode` accepts only known SDK v0.50.10 enum values and rejects
unknown values during protobuf conversion.

**REQ-7.** `secp256k1_public_key_any` uses type URL
`/cosmos.crypto.secp256k1.PubKey` and protobuf value bytes matching Cosmos SDK
account public keys.

**REQ-8.** Address helpers use the Celestia prefixes exactly: `celestia`,
`celestiavaloper`, and `celestiavalcons`. Decoding rejects wrong prefixes and
non-20-byte payloads.

**REQ-9.** `Coin::validate_basic` matches Cosmos SDK stateless coin validation
for empty denoms, malformed denoms, nil or unparsable amounts, negative
amounts, and integer overflow.

**REQ-10.** `Any::validate_basic` rejects empty type URLs and malformed type
URLs. It preserves value bytes exactly.

**REQ-11.** `Fee::validate_basic` rejects malformed coins and empty payer or
granter strings only when those optional fields are present. Empty amount and
zero gas are valid.

**REQ-12.** `SignerInfo::validate_basic` rejects missing or invalid mode info,
malformed public keys, and signatures whose lengths cannot be validated by
their provider when a provider is known.

**REQ-13.** `TxRaw::decode` rejects malformed protobuf bytes, empty body bytes,
empty auth-info bytes, and signature vectors whose count cannot match decoded
signer infos when auth info is decoded.

**REQ-14.** All public functions are deterministic and side-effect-free.

**REQ-15.** No crate outside `cosmos-types` should depend directly on
Cosmos-specific protobuf provider details for these M1 transaction types unless
a later crate spec explicitly grants an exception.

## Conformance fixtures

Committed under `conformance/fixtures/cosmos-types/`:

- `addresses.json` - Celestia account, validator-operator, and
  validator-consensus bech32 vectors.
- `coins.json` - valid and malformed coin cases generated with Cosmos SDK
  v0.50.10 validation.
- `public-keys.json` - secp256k1 public-key `Any` packing vectors.
- `sign-docs.json` - SIGN_MODE_DIRECT `SignDoc` protobuf and sign-byte vectors.
- `tx-raw.json` - `TxRaw` bytes, decoded field bytes, signatures, and
  transaction hash vectors.
- `malformed.json` - malformed protobuf/domain cases covering bad addresses,
  invalid sign modes, malformed `Any`, malformed coins, malformed sign docs,
  and bad transaction raw bytes.

Regenerate fixtures:

```bash
cargo xtask refresh-fixtures --crate-name cosmos-types
```

Equivalent individual scenarios:

```bash
cargo xtask refresh-fixtures --scenario cosmos-addresses
cargo xtask refresh-fixtures --scenario cosmos-coins
cargo xtask refresh-fixtures --scenario cosmos-public-keys
cargo xtask refresh-fixtures --scenario cosmos-sign-docs
cargo xtask refresh-fixtures --scenario cosmos-tx-raw
cargo xtask refresh-fixtures --scenario cosmos-malformed
```

Each scenario must be registered in BOTH the Go fixturegen
(`conformance/fixturegen/cmd/`) and the Rust `SCENARIOS` const in
`xtask/src/refresh_fixtures.rs`.

## Definition of done

- [ ] Crate created at `crates/tier-1-protocol-types/cosmos-types/` with a
      `Cargo.toml` that inherits workspace metadata (`edition.workspace = true`,
      etc.), and the crate path is added to the root `Cargo.toml` `[workspace]`
      members list.
- [ ] Protobuf generation or checked-in generated `prost` modules cover only
      the M1 Cosmos SDK proto files listed in `proto`.
- [ ] All REQ-1..REQ-15 covered by unit, property, or conformance tests.
- [ ] All six conformance fixture files committed with `.meta.json` sidecars.
- [ ] `cargo test -p cosmos-types` green.
- [ ] `cargo clippy -p cosmos-types --all-targets -- -D warnings` green.
- [ ] `cargo fmt --check` green.
- [ ] `cargo llvm-cov -p cosmos-types --summary-only` reports >= 90 % line
      coverage.
- [ ] `crates/tier-1-protocol-types/cosmos-types/README.md` present with
      purpose / consumers / conformance.
- [ ] Cosmos fixturegen scenarios registered in BOTH the Go fixturegen
      (`conformance/fixturegen/cmd/`) AND the Rust `SCENARIOS` const in
      `xtask/src/refresh_fixtures.rs`.
- [ ] PR opened as draft with assignee `@rootulp`; PR body uses `Closes` for
      any tracked issue, or `Closes N/A` when no issue exists.

## Out of scope

- Cosmos SDK module messages beyond opaque protobuf `Any` preservation.
- Ante-handler execution, account lookup, sequence checks, fee deduction,
  gas metering, and signature verification policy.
- SIGN_MODE_LEGACY_AMINO_JSON byte generation beyond enum preservation.
- Textual sign mode rendering.
- Keyring, mnemonic, HD derivation, and CLI transaction building.
- IBC, authz, feegrant, and module keeper state.

## Open questions

- Whether M1 should vendor checked-in Cosmos SDK generated prost modules or use
  a local generation step from pinned `.proto` files.
- Whether the first implementation should include a small bank-send message
  fixture as an opaque `Any`, or defer message-specific coverage to M3.
