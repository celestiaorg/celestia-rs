# hash

Tier-0 cryptographic hash primitives for celestia-rs.

## Purpose

Provides the small set of cryptographic hash functions every other celestia-rs
crate uses, with two goals:

1. **Byte-exact compatibility** with Go celestia-app / CometBFT for sha256,
   sha512, ripemd160, and Tendermint's length-prefix variant (`tmhash`).
2. **One canonical Rust API** — no other crate in the workspace pulls in
   `sha2`, `sha3`, or `ripemd` directly.

## Depends on

None. This is a pure-primitive Tier-0 crate with no celestia-rs dependencies.

## Consumed by

- `merkle`
- `crypto`
- `encoding`
- `tendermint-types`
- `cosmos-types`
- `nmt`
- `shares`
- `square`
- effectively every Tier ≥ 1 crate

## Public API

```rust
pub type Sha256Digest  = [u8; 32];
pub type Sha512Digest  = [u8; 64];
pub type Ripemd160Digest = [u8; 20];

pub fn sha256(input: &[u8]) -> Sha256Digest;
pub fn sha512(input: &[u8]) -> Sha512Digest;
pub fn ripemd160(input: &[u8]) -> Ripemd160Digest;

pub struct Sha256Hasher { /* opaque */ }
impl Sha256Hasher {
    pub fn new() -> Self;
    pub fn update(&mut self, chunk: &[u8]);
    pub fn finalize(self) -> Sha256Digest;
}
impl Default for Sha256Hasher { /* delegates to new() */ }

pub fn tmhash_len_prefixed(input: &[u8]) -> Sha256Digest;
```

All functions return plain `[u8; N]` arrays — no RustCrypto types are
re-exported.

## Conformance

Fixture files live at [`conformance/fixtures/hash/`](../../../conformance/fixtures/hash/):

| File | Scenario | Description |
|---|---|---|
| `empty.json` | `hash-empty` | Digests for empty input across all four functions |
| `short-vectors.json` | `hash-short-vectors` | FIPS/NESSIE vectors for ≥ 10 short inputs |
| `streaming.json` | `hash-streaming` | SHA-256 with explicit chunking patterns |
| `tmhash.json` | `hash-tmhash` | Tendermint length-prefixed SHA-256, varint boundaries |

Regenerate fixtures:

```bash
cargo xtask refresh-fixtures --crate-name hash
```

Run conformance tests:

```bash
cargo test -p hash --test conformance
```

## Out of scope

- HMAC, keyed hashes
- Merkle / tree hashes
- Namespaced hashes
- Hardware acceleration tuning
- Streaming SHA-512 / streaming RIPEMD-160
- Compile-time `const fn sha256`
