# Crate: `crypto`

**Tier:** 0 (primitives)
**Depends on:** `hash`
**Consumed by:** `tendermint-types`, `cosmos-types`, `abci-types`,
`blobstream`/bridge-related crates, transaction validation, and downstream
runtime crates.
**Source of truth (Go):** CometBFT v0.38.12
`github.com/cometbft/cometbft/crypto/ed25519`, Cosmos SDK v0.50.10
`github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1`, and
`github.com/supranational/blst/bindings/go` for BLS12-381 ciphersuite test
vectors.
**Crate location:** `crates/tier-0-primitives/crypto/`

## Purpose

Provide the public-key cryptography primitives used by Celestia consensus,
accounts, and Blobstream-related verification code. This crate owns Ed25519
consensus signatures, Cosmos-compatible secp256k1 account signatures, and
BLS12-381 signature and aggregation helpers behind one workspace API.

The crate does not own protobuf `Any` packing, bech32 address rendering,
keyring storage, mnemonic derivation, transaction sign bytes, validator-set
semantics, or Blobstream message construction. Those belong to downstream
protocol-type, account, and runtime crates.

## Public API

The crate exports exactly these root modules:

```rust
//! Public-key cryptography primitives for celestia-rs.
//!
//! See `docs/specs/m1/crypto.md` for the API contract.

pub mod bls12381;
pub mod ed25519;
pub mod secp256k1;
```

The crate sets `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.

### `ed25519`

```rust
/// Size in bytes of a CometBFT Ed25519 private-key seed.
pub const SEED_LENGTH: usize = 32;

/// Size in bytes of a CometBFT Ed25519 private key.
pub const PRIVATE_KEY_LENGTH: usize = 64;

/// Size in bytes of a CometBFT Ed25519 public key.
pub const PUBLIC_KEY_LENGTH: usize = 32;

/// Size in bytes of an Ed25519 signature.
pub const SIGNATURE_LENGTH: usize = 64;

/// Size in bytes of a CometBFT public-key address.
pub const ADDRESS_LENGTH: usize = 20;

/// A 32-byte Ed25519 private-key seed.
pub type SeedBytes = [u8; SEED_LENGTH];

/// A 64-byte CometBFT Ed25519 private key, encoded as seed || public_key.
pub type PrivateKeyBytes = [u8; PRIVATE_KEY_LENGTH];

/// A 32-byte Ed25519 public key.
pub type PublicKeyBytes = [u8; PUBLIC_KEY_LENGTH];

/// A 64-byte Ed25519 signature.
pub type SignatureBytes = [u8; SIGNATURE_LENGTH];

/// A 20-byte CometBFT public-key address.
pub type AddressBytes = [u8; ADDRESS_LENGTH];

/// Errors returned by Ed25519 helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The private key was not encoded as seed || matching public_key.
    MalformedPrivateKey,
    /// The public key could not be parsed by the ZIP-215 verifier.
    MalformedPublicKey,
    /// The signature could not be parsed.
    MalformedSignature,
}

impl core::fmt::Display for Error;
impl std::error::Error for Error;

/// Derive the CometBFT Ed25519 private key from a 32-byte seed.
pub fn private_key_from_seed(seed: &SeedBytes) -> PrivateKeyBytes;

/// Derive the CometBFT Ed25519 private key from an arbitrary secret.
pub fn private_key_from_secret(secret: &[u8]) -> PrivateKeyBytes;

/// Return the public key embedded in a well-formed CometBFT private key.
pub fn public_key_from_private_key(private_key: &PrivateKeyBytes) -> Result<PublicKeyBytes, Error>;

/// Sign `msg` with a CometBFT Ed25519 private key.
pub fn sign(private_key: &PrivateKeyBytes, msg: &[u8]) -> Result<SignatureBytes, Error>;

/// Verify `signature` using CometBFT ZIP-215 Ed25519 semantics.
pub fn verify(public_key: &PublicKeyBytes, msg: &[u8], signature: &SignatureBytes) -> bool;

/// Return CometBFT's Ed25519 address: first 20 bytes of SHA-256(public_key).
pub fn address(public_key: &PublicKeyBytes) -> AddressBytes;
```

### `secp256k1`

```rust
/// Size in bytes of a secp256k1 secret key.
pub const SECRET_KEY_LENGTH: usize = 32;

/// Size in bytes of a compressed secp256k1 public key.
pub const PUBLIC_KEY_LENGTH: usize = 33;

/// Size in bytes of a Cosmos secp256k1 signature, encoded as r || s.
pub const SIGNATURE_LENGTH: usize = 64;

/// Size in bytes of a Cosmos account address.
pub const ADDRESS_LENGTH: usize = 20;

/// A 32-byte secp256k1 secret key.
pub type SecretKeyBytes = [u8; SECRET_KEY_LENGTH];

/// A 33-byte compressed secp256k1 public key.
pub type PublicKeyBytes = [u8; PUBLIC_KEY_LENGTH];

/// A 64-byte secp256k1 signature, encoded as r || s.
pub type SignatureBytes = [u8; SIGNATURE_LENGTH];

/// A 20-byte Cosmos account address.
pub type AddressBytes = [u8; ADDRESS_LENGTH];

/// Errors returned by secp256k1 helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The secret key is zero or outside the secp256k1 scalar field.
    InvalidSecretKey,
    /// The compressed public key is malformed or not on curve.
    InvalidPublicKey,
    /// The signature scalar pair is malformed or not in lower-S form.
    InvalidSignature,
}

impl core::fmt::Display for Error;
impl std::error::Error for Error;

/// Derive the Cosmos SDK secp256k1 secret key from an arbitrary secret.
pub fn secret_key_from_secret(secret: &[u8]) -> SecretKeyBytes;

/// Parse a raw 32-byte secp256k1 secret key.
pub fn secret_key_from_bytes(bytes: &SecretKeyBytes) -> Result<SecretKeyBytes, Error>;

/// Derive the compressed secp256k1 public key for `secret_key`.
pub fn public_key_from_secret_key(secret_key: &SecretKeyBytes) -> Result<PublicKeyBytes, Error>;

/// Sign `msg` using Cosmos SDK secp256k1 semantics.
pub fn sign(secret_key: &SecretKeyBytes, msg: &[u8]) -> Result<SignatureBytes, Error>;

/// Verify a Cosmos SDK lower-S r || s secp256k1 signature.
pub fn verify(public_key: &PublicKeyBytes, msg: &[u8], signature: &SignatureBytes) -> bool;

/// Return Cosmos's secp256k1 address: RIPEMD-160(SHA-256(compressed_public_key)).
pub fn address(public_key: &PublicKeyBytes) -> AddressBytes;
```

### `bls12381`

```rust
/// BLS signature ciphersuite used for Blobstream-compatible fixtures.
pub const CIPHERSUITE: &str = "BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_";

/// Size in bytes of a BLS12-381 scalar.
pub const SECRET_KEY_LENGTH: usize = 32;

/// Size in bytes of a compressed G1 public key.
pub const PUBLIC_KEY_LENGTH: usize = 48;

/// Size in bytes of a compressed G2 signature.
pub const SIGNATURE_LENGTH: usize = 96;

/// A 32-byte BLS12-381 secret scalar.
pub type SecretKeyBytes = [u8; SECRET_KEY_LENGTH];

/// A compressed G1 public key.
pub type PublicKeyBytes = [u8; PUBLIC_KEY_LENGTH];

/// A compressed G2 signature.
pub type SignatureBytes = [u8; SIGNATURE_LENGTH];

/// Errors returned by BLS12-381 helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The secret key is zero or outside the BLS12-381 scalar field.
    InvalidSecretKey,
    /// The public key is not a valid compressed G1 point in the prime-order subgroup.
    InvalidPublicKey,
    /// The signature is not a valid compressed G2 point in the prime-order subgroup.
    InvalidSignature,
    /// Aggregation was requested for an empty input list.
    EmptyAggregate,
}

impl core::fmt::Display for Error;
impl std::error::Error for Error;

/// Derive a BLS12-381 secret key from input key material.
pub fn secret_key_from_ikm(ikm: &[u8]) -> Result<SecretKeyBytes, Error>;

/// Parse a raw 32-byte BLS12-381 secret key.
pub fn secret_key_from_bytes(bytes: &SecretKeyBytes) -> Result<SecretKeyBytes, Error>;

/// Derive the compressed G1 public key for `secret_key`.
pub fn public_key_from_secret_key(secret_key: &SecretKeyBytes) -> Result<PublicKeyBytes, Error>;

/// Sign `msg` into a compressed G2 signature.
pub fn sign(secret_key: &SecretKeyBytes, msg: &[u8]) -> Result<SignatureBytes, Error>;

/// Verify a compressed G2 signature against a compressed G1 public key.
pub fn verify(public_key: &PublicKeyBytes, msg: &[u8], signature: &SignatureBytes) -> bool;

/// Aggregate compressed G1 public keys.
pub fn aggregate_public_keys(public_keys: &[PublicKeyBytes]) -> Result<PublicKeyBytes, Error>;

/// Aggregate compressed G2 signatures.
pub fn aggregate_signatures(signatures: &[SignatureBytes]) -> Result<SignatureBytes, Error>;

/// Fast-aggregate-verify signatures over one common message.
pub fn fast_aggregate_verify(
    public_keys: &[PublicKeyBytes],
    msg: &[u8],
    signature: &SignatureBytes,
) -> bool;
```

## Behavioral requirements

**REQ-1.** `ed25519::private_key_from_seed(seed)` matches CometBFT
`ed25519.NewKeyFromSeed(seed)` byte-for-byte: 64 bytes encoded as
`seed || public_key`.

**REQ-2.** `ed25519::private_key_from_secret(secret)` matches CometBFT
`GenPrivKeyFromSecret`: SHA-256 the secret to a 32-byte seed, then derive the
private key from that seed.

**REQ-3.** `ed25519::public_key_from_private_key` returns the last 32 bytes of a
well-formed CometBFT private key and returns `Error::MalformedPrivateKey` if the
public-key suffix does not match the seed-derived public key.

**REQ-4.** `ed25519::sign` matches CometBFT Ed25519 signatures for fixture
private keys and messages.

**REQ-5.** `ed25519::verify` uses CometBFT's ZIP-215 verification semantics,
not stricter RFC 8032-only verification.

**REQ-6.** `ed25519::address(public_key)` returns the first 20 bytes of
SHA-256 over the raw 32-byte public key.

**REQ-7.** `secp256k1::secret_key_from_secret(secret)` matches Cosmos SDK
`GenPrivKeyFromSecret`: SHA-256 the secret, interpret it as a big-endian
integer `c`, compute `(c mod (n - 1)) + 1`, and left-pad the result to 32
bytes.

**REQ-8.** `secp256k1::secret_key_from_bytes` rejects zero and values greater
than or equal to the secp256k1 curve order.

**REQ-9.** `secp256k1::public_key_from_secret_key` returns the 33-byte
compressed public key accepted by Cosmos SDK `PubKey.Bytes()`.

**REQ-10.** `secp256k1::sign` signs SHA-256(message) using deterministic ECDSA
nonce generation, returns `r || s`, omits the recovery byte, and normalizes `s`
to lower-S form.

**REQ-11.** `secp256k1::verify` verifies SHA-256(message) and rejects malformed
public keys, malformed signatures, and signatures whose `s` value is not
lower-S.

**REQ-12.** `secp256k1::address(public_key)` returns
RIPEMD-160(SHA-256(compressed_public_key)).

**REQ-13.** BLS12-381 uses the minimal-public-key-size variant: public keys are
compressed G1 points, signatures are compressed G2 points, and the ciphersuite
domain separation tag is `bls12381::CIPHERSUITE`.

**REQ-14.** `bls12381::secret_key_from_ikm`, public-key derivation, signing,
verification, public-key aggregation, signature aggregation, and
fast-aggregate verification match `blst` Go binding fixtures.

**REQ-15.** BLS public-key and signature parsing rejects non-canonical
encodings, infinity where it is not valid, off-curve points, and points outside
the prime-order subgroup.

**REQ-16.** All public functions are deterministic and side-effect-free.
Random key generation is intentionally out of scope for this crate.

**REQ-17.** No crate outside `crypto` should depend directly on
`ed25519-consensus`, `k256`, `secp256k1`, `blst`, or other signature-provider
crates for these primitive helpers unless a future crate spec explicitly grants
an exception.

## Conformance fixtures

Committed at `conformance/fixtures/crypto/`:

- `crypto/ed25519.json` - CometBFT v0.38.12 Ed25519 vectors for seed-derived
  private keys, secret-derived private keys, public keys, signatures, valid and
  invalid verification cases, and 20-byte addresses.
- `crypto/secp256k1.json` - Cosmos SDK v0.50.10 secp256k1 vectors for
  secret-derived keys, raw key parsing, compressed public keys, deterministic
  lower-S signatures, valid and invalid verification cases, and 20-byte
  addresses.
- `crypto/bls12381.json` - `blst` Go binding vectors for IKM-derived keys,
  G1 public keys, G2 signatures, aggregation, fast-aggregate verification, and
  malformed point encodings.

Regeneration is split into one-file scenarios:

```bash
cargo xtask refresh-fixtures --crate-name crypto
```

Equivalent individual scenarios:

```bash
cargo xtask refresh-fixtures --scenario crypto-ed25519
cargo xtask refresh-fixtures --scenario crypto-secp256k1
cargo xtask refresh-fixtures --scenario crypto-bls12381
```

Each scenario must be registered in BOTH the Go fixturegen
(`conformance/fixturegen/cmd/`) and the Rust `SCENARIOS` const in
`xtask/src/refresh_fixtures.rs`.

## Definition of done

- [ ] Crate created at `crates/tier-0-primitives/crypto/` with a
      `Cargo.toml` that inherits workspace metadata (`edition.workspace = true`,
      etc.), and the crate path is added to the root `Cargo.toml` `[workspace]`
      members list.
- [ ] All REQ-1..REQ-17 covered by unit, property, or conformance tests.
- [ ] All three conformance fixture files committed with `.meta.json` sidecars.
- [ ] `cargo test -p crypto` green.
- [ ] `cargo clippy -p crypto --all-targets -- -D warnings` green.
- [ ] `cargo fmt --check` green.
- [ ] `cargo llvm-cov -p crypto --summary-only` reports >= 90 % line coverage.
- [ ] `crates/tier-0-primitives/crypto/README.md` present with purpose /
      consumers / conformance.
- [ ] Crypto fixturegen scenario(s) registered in BOTH the Go fixturegen
      (`conformance/fixturegen/cmd/`) AND the Rust `SCENARIOS` const in
      `xtask/src/refresh_fixtures.rs`.
- [ ] PR opened as draft with assignee `@rootulp`; PR body uses `Closes` for
      any tracked issue, or `Closes N/A` when no issue exists.

## Out of scope

- Random key generation.
- HD wallet, mnemonic, or keyring support.
- Bech32 rendering of account or validator addresses.
- Transaction sign-byte construction.
- Protobuf `Any` packing for public keys.
- Recoverable secp256k1 signatures and public-key recovery.
- Amino JSON or legacy marshaling.
- Batch Ed25519 verification. Add it later only if validator-set verification
  needs it for performance.
- Blobstream attestation message construction or proof verification.

## Open questions

- None. The current fixturegen module pins the CometBFT and Cosmos SDK versions
  for Ed25519 and secp256k1, and BLS12-381 vectors are pinned to `blst` to avoid
  inventing curve behavior in this repository.

## Notes for the implementing agent

1. **Scaffold the crate.** Create
   `crates/tier-0-primitives/crypto/{Cargo.toml,src/lib.rs}` and add the path
   to the root `Cargo.toml` `[workspace] members`.
2. **Pick provider crates conservatively.** Prefer `ed25519-consensus` or an
   equivalent ZIP-215-capable verifier for Ed25519, `k256` for secp256k1, and
   `blst` for BLS12-381. Keep provider types private.
3. **Write tests first.** Translate REQ-1..REQ-15 into named API tests. Include
   negative tests for malformed keys, high-S secp256k1 signatures, invalid BLS
   point encodings, and empty BLS aggregates.
4. **Add fixturegen scenarios.** Reuse the already-pinned Go modules for
   CometBFT/Cosmos vectors. Add `blst` only for BLS fixtures.
5. **Regenerate fixtures** with
   `cargo xtask refresh-fixtures --crate-name crypto` and commit the JSON files
   plus static `.meta.json` sidecars.
6. **Add conformance tests** under
   `crates/tier-0-primitives/crypto/tests/conformance.rs` following the hash
   and encoding workspace-root helper pattern.
7. **Verify the full crate DoD** before opening the draft PR.
