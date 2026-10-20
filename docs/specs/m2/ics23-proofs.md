# Crate: `ics23-proofs`

**Tier:** 2 (state and store)
**Depends on:** `iavl`
**Consumed by:** `multistore`, M2 integration tests, ABCI query proofs, and
later light-client / IBC verification surfaces.
**Source of truth (Go):** `cosmossdk.io/store/types` v1.1.1 proof operators,
`github.com/cosmos/ics23/go` v0.11.0, and CometBFT v0.38
`tendermint.crypto.ProofOp` / `ProofOps` protobuf shapes.
**Crate location:** `crates/tier-2-state-and-store/ics23-proofs/`

## Purpose

Provide the Cosmos SDK proof-operation layer that sits between raw IAVL proof
material and ABCI / multi-store query proofs. The crate owns ICS-23 commitment
proof wrappers, CometBFT proof-op type routing, membership / non-membership
verification entry points, and the simple Merkle commitment operation used by
multi-store commit info.

The first implementation target is a wire-compatible in-memory proof surface:
decode and encode the ICS-23 commitment proof protobuf bytes produced by the
local `iavl` crate, wrap them as SDK `ics23:*` proof ops, verify them against
roots, and generate Go-compatible fixtures. It must not duplicate IAVL tree
mutation, key ordering, or node hashing.

## Public API

The crate exports exactly these items from its root module:

```rust
//! Cosmos SDK ICS-23 proof operation primitives.
//!
//! See `docs/specs/m2/ics23-proofs.md` for the API contract.

pub use iavl::Hash;

pub const PROOF_OP_IAVL_COMMITMENT: &str = "ics23:iavl";
pub const PROOF_OP_SIMPLE_MERKLE_COMMITMENT: &str = "ics23:simple";
pub const PROOF_OP_SMT_COMMITMENT: &str = "ics23:smt";

pub struct ProofOp {
    pub field_type: String,
    pub key: Vec<u8>,
    pub data: Vec<u8>,
}

pub struct ProofOps {
    pub ops: Vec<ProofOp>,
}

pub struct CommitmentProof { /* opaque */ }

impl CommitmentProof {
    pub fn from_proto_bytes(bytes: Vec<u8>) -> Result<Self>;
    pub fn from_iavl_proof(proof: &iavl::Proof) -> Result<Self>;
    pub fn as_proto_bytes(&self) -> &[u8];
    pub fn kind(&self) -> CommitmentProofKind;
    pub fn key(&self) -> &[u8];
    pub fn value(&self) -> Option<&[u8]>;
    pub fn calculate_root(&self, spec: ProofSpec) -> Result<Hash>;
    pub fn verify_membership(
        &self,
        spec: ProofSpec,
        root: Hash,
        key: &[u8],
        value: &[u8],
    ) -> Result<()>;
    pub fn verify_non_membership(
        &self,
        spec: ProofSpec,
        root: Hash,
        key: &[u8],
    ) -> Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitmentProofKind {
    Existence,
    NonExistence,
    Batch,
    Compressed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofSpec {
    Iavl,
    Tendermint,
    Smt,
}

pub struct CommitmentOp { /* opaque */ }

impl CommitmentOp {
    pub fn new_iavl(key: Vec<u8>, proof: CommitmentProof) -> Self;
    pub fn new_simple_merkle(key: Vec<u8>, proof: CommitmentProof) -> Self;
    pub fn new_smt(key: Vec<u8>, proof: CommitmentProof) -> Self;
    pub fn from_proof_op(op: ProofOp) -> Result<Self>;
    pub fn from_iavl_proof(proof: &iavl::Proof) -> Result<Self>;
    pub fn field_type(&self) -> &'static str;
    pub fn spec(&self) -> ProofSpec;
    pub fn key(&self) -> &[u8];
    pub fn proof(&self) -> &CommitmentProof;
    pub fn to_proof_op(&self) -> ProofOp;
    pub fn run(&self, args: &[&[u8]]) -> Result<Vec<Hash>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    EmptyKey,
    InvalidProofOpType(String),
    InvalidProof(&'static str),
    MalformedProto(&'static str),
    UnsupportedProofKind(&'static str),
    Iavl(iavl::Error),
}

impl From<iavl::Error> for Error { /* wraps Error::Iavl */ }

pub type Result<T> = core::result::Result<T, Error>;
```

The crate sets `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.

## Behavioral Requirements

**REQ-1.** `ProofOp` and `ProofOps` encode the CometBFT
`tendermint.crypto.ProofOp` / `ProofOps` protobuf field names and ordering:
`type`, `key`, `data`, and repeated `ops`.

**REQ-2.** `CommitmentProof::from_proto_bytes` accepts exactly ICS-23
`CommitmentProof` protobuf bytes and rejects malformed bytes with
`Error::MalformedProto`.

**REQ-3.** `CommitmentProof::as_proto_bytes` returns byte-for-byte the same
canonical protobuf bytes used by Go `CommitmentProof.Marshal`.

**REQ-4.** `CommitmentProof::from_iavl_proof` consumes the single operation in
an `iavl::Proof`, preserves its protobuf bytes, and rejects empty or multi-op
IAVL proof material with `Error::InvalidProof`.

**REQ-5.** IAVL existence proofs verify with `ProofSpec::Iavl` when the
provided key, value, proof root, and expected root all match.

**REQ-6.** IAVL non-existence proofs verify with `ProofSpec::Iavl` when the
provided key is absent and the left/right neighbor proof roots match the
expected root.

**REQ-7.** Verification rejects wrong roots, wrong keys, wrong values, missing
neighbors, and proof-kind mismatches with `Error::InvalidProof`.

**REQ-8.** `CommitmentProof::calculate_root` matches Go ICS-23 `Calculate` for
existence, non-existence, batch, and compressed proofs. Batch calculation may
return the first calculated root, matching Go, but verification must still
validate every relevant embedded proof.

**REQ-9.** `ProofSpec::Iavl` uses the Go ICS-23 IAVL spec:
leaf prefix `0x00`, SHA-256 value prehash, var-proto length op, child order
`[0, 1]`, inner child size `33`, inner prefix length range `4..=12`, and
SHA-256 inner hashing.

**REQ-10.** `ProofSpec::Tendermint` uses the Go ICS-23 Tendermint simple
Merkle spec with SHA-256 hashing, var-proto leaf lengths, child order `[0, 1]`,
child size `32`, and one-byte inner prefixes.

**REQ-11.** `ProofSpec::Smt` is recognized for proof-op type routing and
decoding, but SMT proof generation is out of scope unless downstream crates
need it in M2.

**REQ-12.** `CommitmentOp::from_proof_op` maps only `ics23:iavl`,
`ics23:simple`, and `ics23:smt` to the corresponding `ProofSpec`; all other
operation types return `Error::InvalidProofOpType`.

**REQ-13.** `CommitmentOp::to_proof_op` emits the same type, key, and proof
bytes as Cosmos SDK `CommitmentOp.ProofOp`.

**REQ-14.** `CommitmentOp::run(&[])` verifies non-membership and returns the
calculated root as a single-element vector. `run(&[value])` verifies
membership. Any other argument count returns `Error::InvalidProof`.

**REQ-15.** `CommitmentOp::from_iavl_proof` emits `ics23:iavl` proof ops whose
key and data match the local `iavl` crate's proof operation material.

**REQ-16.** Empty commitment-op keys are rejected with `Error::EmptyKey`.

**REQ-17.** The crate performs no disk IO and owns no persistent proof cache.

## Conformance Fixtures

Committed under `conformance/fixtures/ics23-proofs/`:

- `iavl.json` - IAVL membership and non-membership commitment proof bytes,
  proof-op wrapping, calculated roots, and verification outcomes.
- `simple.json` - simple Merkle commitment ops for multi-store store-name ->
  store-root maps.
- `ops.json` - `ProofOp` / `ProofOps` protobuf encoding, decoding, and
  `CommitmentOp::run` behavior.
- `malformed.json` - malformed protobuf bytes, unknown proof-op types, wrong
  roots/keys/values, empty keys, unsupported proof kinds, and bad argument
  counts.

Regenerate fixtures:

```bash
cargo xtask refresh-fixtures --crate-name ics23-proofs
```

Equivalent individual scenarios:

```bash
cargo xtask refresh-fixtures --scenario ics23-proofs-iavl
cargo xtask refresh-fixtures --scenario ics23-proofs-simple
cargo xtask refresh-fixtures --scenario ics23-proofs-ops
cargo xtask refresh-fixtures --scenario ics23-proofs-malformed
```

Each scenario must be registered in BOTH the Go fixturegen
(`conformance/fixturegen/cmd/`) and the Rust `SCENARIOS` const in
`xtask/src/refresh_fixtures.rs`.

## Definition Of Done

- [x] Crate created at `crates/tier-2-state-and-store/ics23-proofs/` with a
      `Cargo.toml` inheriting workspace metadata, and the crate path is added
      to root `Cargo.toml` `[workspace]` members.
- [x] Public API above implemented with documented modules and
      `#![forbid(unsafe_code)]`.
- [x] All REQ-1..REQ-17 covered by unit, property, or conformance tests.
- [x] All four conformance fixture files committed with `.meta.json` sidecars.
- [x] `cargo test -p ics23-proofs` green.
- [x] `cargo clippy -p ics23-proofs --all-targets -- -D warnings` green.
- [x] `cargo fmt --check` green.
- [x] `cargo llvm-cov -p ics23-proofs --summary-only` reports >= 90 % line
      coverage.
- [x] `crates/tier-2-state-and-store/ics23-proofs/README.md` present with
      purpose / consumers / conformance.
- [x] ICS23 proof fixturegen scenarios registered in BOTH the Go fixturegen and
      the Rust `SCENARIOS` const.

## Out Of Scope

- IAVL tree mutation, node balancing, and root hashing.
- Multi-store root composition and store-key routing beyond simple commitment
  op fixtures.
- ABCI request / response envelopes.
- Persistent proof caches, disk IO, and pruning.
- SMT proof generation unless a later M2/M3 crate requires it.

## Notes For The Implementing Agent

- Start with red tests around the existing `conformance/fixtures/iavl/proofs.json`
  proof bytes: wrap them as `ics23:iavl`, verify membership / non-membership,
  and reject wrong key/value/root variants.
- Reuse the local `iavl::Proof` operation data; do not reimplement IAVL tree
  path generation here.
- Keep the first parser as small as possible but structured: decode protobuf
  fields into typed proof variants instead of searching raw bytes.
- Implement simple Merkle commitment op support before `multistore`; the
  multi-store proof path depends on `ics23:simple` store-name routing.
- Leave batch proof combination for a follow-up unless fixtures require it for
  verification parity.
