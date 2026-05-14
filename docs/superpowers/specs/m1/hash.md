# Crate: `hash`

**Tier:** 0 (primitives)
**Depends on:** _(none — pure primitive)_
**Consumed by:** `merkle`, `crypto`, `encoding`, `tendermint-types`, `cosmos-types`, `nmt`, `shares`, `square`, and effectively every tier ≥ 1 crate.
**Source of truth (Go):** `crypto/sha256`, `golang.org/x/crypto/ripemd160`, `crypto/sha512`, and the Tendermint helper at `github.com/cometbft/cometbft/crypto/tmhash` (length-prefix-then-sha256).
**Crate location:** `crates/tier-0-primitives/hash/`

## Purpose

Provide the small set of cryptographic hash primitives every other celestia-rs crate uses, with two goals:

1. **Byte-exact compatibility** with Go celestia-app / CometBFT for sha256, sha512, ripemd160, and Tendermint's length-prefix variant.
2. **One canonical Rust API** for hashing throughout the workspace — no other crate may pull in `sha2`, `sha3`, or `ripemd` directly.

This crate is a thin façade over the [RustCrypto](https://github.com/RustCrypto/hashes) family, vendored only conceptually (we depend on the crates and re-export the types we need). It does **not** include keyed hashes (HMAC), tree hashes (Merkle), or namespaced hashes (NMT) — those are downstream crates.

## Public API

The crate exports exactly these items from its root module:

```rust
//! Cryptographic hash primitives for celestia-rs.
//!
//! See `docs/superpowers/specs/m1/hash.md` for the API contract.

/// A 32-byte SHA-256 digest.
pub type Sha256Digest = [u8; 32];

/// A 64-byte SHA-512 digest.
pub type Sha512Digest = [u8; 64];

/// A 20-byte RIPEMD-160 digest.
pub type Ripemd160Digest = [u8; 20];

/// Compute SHA-256 of `input`.
pub fn sha256(input: &[u8]) -> Sha256Digest;

/// Compute SHA-512 of `input`.
pub fn sha512(input: &[u8]) -> Sha512Digest;

/// Compute RIPEMD-160 of `input`.
pub fn ripemd160(input: &[u8]) -> Ripemd160Digest;

/// Streaming SHA-256 builder. Equivalent to feeding successive chunks to a
/// single `sha256` call.
pub struct Sha256Hasher { /* opaque */ }

impl Sha256Hasher {
    pub fn new() -> Self;
    pub fn update(&mut self, chunk: &[u8]);
    pub fn finalize(self) -> Sha256Digest;
}

impl Default for Sha256Hasher { /* delegates to new() */ }

/// Tendermint's length-prefix-then-sha256 helper.
///
/// Equivalent to `sha256(varint_len(input) || input)`, where `varint_len`
/// uses the Tendermint canonical unsigned varint encoding (LEB128, max 10
/// bytes). See [CometBFT's `crypto/tmhash`].
pub fn tmhash_len_prefixed(input: &[u8]) -> Sha256Digest;
```

The crate sets `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.

## Behavioral requirements

**REQ-1.** `sha256("".as_bytes())` returns `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` (hex).

**REQ-2.** `sha256("abc".as_bytes())` returns `ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad` (hex).

**REQ-3.** `sha512("".as_bytes())` returns `cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e` (hex). This is the SHA-512 of the empty byte string, per FIPS 180-4.

**REQ-4.** `ripemd160("".as_bytes())` returns `9c1185a5c5e9fc54612808977ee8f548b2258d31` (hex).

**REQ-5.** `Sha256Hasher` produces the same digest as `sha256` when fed the same bytes in any chunking.

**REQ-6.** `tmhash_len_prefixed(b)` ≡ `sha256(varint_uvarint(b.len()) || b)`, where `varint_uvarint` follows Go's `encoding/binary.PutUvarint`. Test vectors:
- `tmhash_len_prefixed(b"")` = `sha256([0u8])`
- `tmhash_len_prefixed(b"\x00")` = `sha256(&[0x01, 0x00])`
- `tmhash_len_prefixed(&[0xff; 200])` matches the Go reference output (committed fixture).

**REQ-7.** All public functions are deterministic and side-effect-free.

**REQ-8.** The crate does not re-export the underlying RustCrypto types in its public API — callers receive plain `[u8; N]` arrays, never `GenericArray` or trait objects.

**REQ-9.** Property test: for any `Vec<u8>` (proptest-generated, up to 64 KB), `Sha256Hasher::update(.., chunks).finalize()` ≡ `sha256(concat(chunks))` for arbitrary chunkings.

## Conformance fixtures

Committed at `conformance/fixtures/hash/`:

- `hash/empty.json` — digests for empty input across all hash functions.
- `hash/short-vectors.json` — digests for the FIPS / NESSIE short test vectors (10 inputs, all four functions).
- `hash/streaming.json` — input + chunking pattern + expected sha256 digest for ≥ 5 cases.
- `hash/tmhash.json` — Tendermint length-prefix-then-sha256 digests for ≥ 10 inputs of varying length (empty, 1 byte, 127 bytes, 128 bytes, 1 KB, 64 KB, etc.) — these exercise varint boundary lengths.

Regeneration: a new subcommand `hash-vectors` in `conformance/fixturegen/cmd/` that imports `crypto/sha256`, `crypto/sha512`, `golang.org/x/crypto/ripemd160`, and CometBFT's `crypto/tmhash`. Invoked via:

```bash
cargo xtask refresh-fixtures --scenario hash-vectors
```

The fixturegen subcommand emits all four JSON files in one run.

## Definition of done

- [ ] Crate created at `crates/tier-0-primitives/hash/` with a `Cargo.toml` that inherits workspace metadata (`edition.workspace = true`, etc.), and the crate path is added to the root `Cargo.toml` `[workspace] members` list.
- [ ] All REQ-1..REQ-9 covered by unit or property tests.
- [ ] All four conformance fixture files committed and tested against.
- [ ] `cargo test -p hash` green.
- [ ] `cargo clippy -p hash --all-targets -- -D warnings` green.
- [ ] `cargo fmt --check` green.
- [ ] `cargo llvm-cov -p hash --summary-only` reports ≥ 90 % line coverage.
- [ ] `crates/tier-0-primitives/hash/README.md` present with purpose / consumers / conformance.
- [ ] Hash fixturegen scenario(s) registered in BOTH the Go fixturegen (`conformance/fixturegen/cmd/`) AND the Rust `SCENARIOS` const in `xtask/src/refresh_fixtures.rs`. Either a single `hash-vectors` scenario (extending the `Scenario` struct to emit multiple files) OR four scenarios `hash-empty`, `hash-short-vectors`, `hash-streaming`, `hash-tmhash` is acceptable.
- [ ] PR opened as draft with auto-merge enabled; assignee `@rootulp`; PR body uses `Closes` for any tracked issue.

## Out of scope

- HMAC and keyed hashes.
- Merkle / tree hashes (belongs in `merkle`).
- Namespaced hashes (belongs in `nmt`).
- Hardware acceleration tuning.
- Streaming SHA-512 / streaming RIPEMD-160 (add later if a consumer needs them; YAGNI for now).

## Open questions

- **Q1:** Confirm the RustCrypto crate versions to pin (`sha2 = "0.10"`, `ripemd = "0.1"` are current as of writing). Defer to the implementer; record final versions in the crate `Cargo.toml`.
- **Q2:** Do we want a `const fn sha256` for compile-time hashing? Not required by any current consumer — leave out unless explicitly requested.

## Notes for the implementing agent

1. **Scaffold the crate.** Create `crates/tier-0-primitives/hash/{Cargo.toml,src/lib.rs}` with workspace-inherited metadata. Add `"crates/tier-0-primitives/hash"` to the root `Cargo.toml` `[workspace] members` list. Confirm `cargo build -p hash` works (with an empty `lib.rs` it will succeed).
2. **Write tests first.** Translate REQ-1..REQ-6 into named `#[test]` functions in `src/lib.rs` (or a `tests/` integration file). Run them; they must fail (TDD red).
3. **Implement the public API** to make REQ-1..REQ-8 pass. Use `sha2`, `sha512`, `ripemd` from RustCrypto. Use `unsigned-varint` or hand-rolled LEB128 for the tmhash length prefix.
4. **Add the proptest** for REQ-9 once the unit tests are green.
5. **Add the `hash-vectors` fixturegen scenario.** Both sides:
   - In Go: create `conformance/fixturegen/cmd/hash_vectors.go` mirroring `bech32.go`'s shape. Register it in `cmd.All` in `cmd/cmd.go`.
   - In Rust: append a `Scenario` entry to `SCENARIOS` in `xtask/src/refresh_fixtures.rs` with `name: "hash-vectors"`, `crate_name: "hash"`, and an `output_path` pointing to `conformance/fixtures/hash/`.
   - Note: the current `Scenario` struct emits one output file per scenario. `hash-vectors` writes four files (empty.json, short-vectors.json, streaming.json, tmhash.json). Either extend the `Scenario` shape to support multiple outputs, or break `hash-vectors` into four separate `Scenario` entries that share one Go subcommand. Either approach is acceptable; pick one and document it in the PR.
6. **Regenerate fixtures** with `cargo xtask refresh-fixtures --scenario hash-vectors` and commit the JSON files plus their `.meta.json` sidecars.
7. **Add conformance tests** that load each fixture file and assert function outputs match.
8. **Verify coverage, fmt, clippy.** Open the PR.
