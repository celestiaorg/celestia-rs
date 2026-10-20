# Crate: `tendermint-types`

**Tier:** 1 (protocol types)
**Depends on:** `hash`, `encoding`, `crypto`, `merkle`
**Consumed by:** `cosmos-types`, `abci-types`, `shares`, `square`,
M1 integration tests, and downstream consensus/runtime crates.
**Source of truth (Go):** CometBFT v0.38.12
`github.com/cometbft/cometbft/types`,
`github.com/cometbft/cometbft/proto/tendermint/types`,
`github.com/cometbft/cometbft/proto/tendermint/version`, and
`github.com/cometbft/cometbft/proto/tendermint/crypto`.
**Crate location:** `crates/tier-1-protocol-types/tendermint-types/`

## Purpose

Provide the CometBFT consensus and block data structures that must be
byte-compatible with Celestia mainnet blocks. This crate owns block IDs,
headers, commits, validator sets, votes, proposals, canonical sign bytes, and
the trimmed protobuf surface needed by later M1 crates.

The crate does not own ABCI request/response types, Cosmos SDK transaction
types, evidence verification, consensus state machines, networking, RPC, or
mainnet block replay orchestration. Those belong to later protocol, runtime,
and integration crates.

## Public API

The crate exports exactly these root modules:

```rust
//! CometBFT protocol types for celestia-rs.
//!
//! See `docs/specs/m1/tendermint-types.md` for the API contract.

pub mod block;
pub mod canonical;
pub mod commit;
pub mod error;
pub mod hash;
pub mod proposal;
pub mod proto;
pub mod validator;
pub mod vote;

pub use block::{BlockId, ConsensusVersion, Header, PartSetHeader};
pub use commit::{BlockIdFlag, Commit, CommitSig};
pub use error::{Error, Result};
pub use hash::{Address, Hash, ADDRESS_LENGTH, HASH_LENGTH};
pub use proposal::Proposal;
pub use validator::{PublicKey, Validator, ValidatorSet};
pub use vote::{SignedMsgType, Vote};
```

The crate sets `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.

### `hash`

```rust
/// Size in bytes of a CometBFT hash.
pub const HASH_LENGTH: usize = 32;

/// Size in bytes of a CometBFT account/validator address.
pub const ADDRESS_LENGTH: usize = 20;

/// A 32-byte CometBFT hash.
pub type Hash = [u8; HASH_LENGTH];

/// A 20-byte CometBFT address.
pub type Address = [u8; ADDRESS_LENGTH];

/// Parse a protobuf hash field. Empty bytes map to `None`; 32 bytes map to
/// `Some(Hash)`; all other lengths are rejected.
pub fn optional_hash_from_bytes(bytes: &[u8]) -> Result<Option<Hash>>;

/// Encode an optional CometBFT hash as the protobuf bytes representation.
pub fn optional_hash_to_vec(hash: Option<&Hash>) -> Vec<u8>;

/// Parse a 20-byte address.
pub fn address_from_bytes(bytes: &[u8]) -> Result<Address>;
```

### `error`

```rust
/// Errors returned by CometBFT protocol type helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidHashLength { actual: usize },
    InvalidAddressLength { actual: usize },
    InvalidChainIdLength { actual: usize, max: usize },
    InvalidHeight,
    InvalidRound,
    InvalidBlockId,
    InvalidPartSetHeader,
    InvalidCommit,
    InvalidCommitSig,
    InvalidValidator,
    InvalidValidatorSet,
    InvalidVote,
    InvalidProposal,
    InvalidPublicKey,
    InvalidTimestamp,
    Decode(String),
    Encode(String),
}

impl core::fmt::Display for Error;
impl std::error::Error for Error;

/// Result alias for this crate.
pub type Result<T> = core::result::Result<T, Error>;
```

### `block`

```rust
/// Maximum CometBFT chain ID length.
pub const MAX_CHAIN_ID_LENGTH: usize = 50;

/// CometBFT consensus version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsensusVersion {
    pub block: u64,
    pub app: u64,
}

/// CometBFT part-set header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartSetHeader {
    pub total: u32,
    pub hash: Option<Hash>,
}

impl PartSetHeader {
    pub fn is_zero(&self) -> bool;
    pub fn validate_basic(&self) -> Result<()>;
    pub fn from_proto(proto: proto::types::PartSetHeader) -> Result<Self>;
    pub fn to_proto(&self) -> proto::types::PartSetHeader;
}

/// CometBFT block ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockId {
    pub hash: Option<Hash>,
    pub part_set_header: PartSetHeader,
}

impl BlockId {
    pub fn is_zero(&self) -> bool;
    pub fn is_complete(&self) -> bool;
    pub fn validate_basic(&self) -> Result<()>;
    pub fn from_proto(proto: proto::types::BlockId) -> Result<Self>;
    pub fn to_proto(&self) -> proto::types::BlockId;
}

/// CometBFT block header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub version: ConsensusVersion,
    pub chain_id: String,
    pub height: i64,
    pub time: prost_types::Timestamp,
    pub last_block_id: BlockId,
    pub last_commit_hash: Option<Hash>,
    pub data_hash: Option<Hash>,
    pub validators_hash: Option<Hash>,
    pub next_validators_hash: Option<Hash>,
    pub consensus_hash: Option<Hash>,
    pub app_hash: Vec<u8>,
    pub last_results_hash: Option<Hash>,
    pub evidence_hash: Option<Hash>,
    pub proposer_address: Address,
}

impl Header {
    pub fn validate_basic(&self) -> Result<()>;
    pub fn hash(&self) -> Option<Hash>;
    pub fn from_proto(proto: proto::types::Header) -> Result<Self>;
    pub fn to_proto(&self) -> proto::types::Header;
    pub fn decode(bytes: &[u8]) -> Result<Self>;
    pub fn encode_to_vec(&self) -> Result<Vec<u8>>;
}
```

### `commit`

```rust
/// CometBFT block ID flag used inside commit signatures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BlockIdFlag {
    Absent = 1,
    Commit = 2,
    Nil = 3,
}

impl TryFrom<i32> for BlockIdFlag;
impl From<BlockIdFlag> for i32;

/// A signature from one validator inside a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitSig {
    pub block_id_flag: BlockIdFlag,
    pub validator_address: Option<Address>,
    pub timestamp: Option<prost_types::Timestamp>,
    pub signature: Vec<u8>,
}

impl CommitSig {
    pub fn absent() -> Self;
    pub fn block_id(&self, commit_block_id: &BlockId) -> Result<BlockId>;
    pub fn validate_basic(&self) -> Result<()>;
    pub fn from_proto(proto: proto::types::CommitSig) -> Result<Self>;
    pub fn to_proto(&self) -> proto::types::CommitSig;
}

/// CometBFT commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub height: i64,
    pub round: i32,
    pub block_id: BlockId,
    pub signatures: Vec<CommitSig>,
}

impl Commit {
    pub fn validate_basic(&self) -> Result<()>;
    pub fn hash(&self) -> Hash;
    pub fn size(&self) -> usize;
    pub fn get_vote(&self, validator_index: i32) -> Result<Option<Vote>>;
    pub fn vote_sign_bytes(&self, chain_id: &str, validator_index: i32) -> Result<Vec<u8>>;
    pub fn from_proto(proto: proto::types::Commit) -> Result<Self>;
    pub fn to_proto(&self) -> proto::types::Commit;
}
```

### `validator`

```rust
/// CometBFT public key variants used by validator and consensus types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicKey {
    Ed25519(crypto::ed25519::PublicKeyBytes),
    Secp256k1(crypto::secp256k1::PublicKeyBytes),
}

impl PublicKey {
    pub fn address(&self) -> Address;
    pub fn verify_signature(&self, msg: &[u8], signature: &[u8]) -> bool;
    pub fn from_proto(proto: proto::crypto::PublicKey) -> Result<Self>;
    pub fn to_proto(&self) -> proto::crypto::PublicKey;
}

/// A CometBFT validator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validator {
    pub address: Address,
    pub public_key: PublicKey,
    pub voting_power: i64,
    pub proposer_priority: i64,
}

impl Validator {
    pub fn new(public_key: PublicKey, voting_power: i64) -> Self;
    pub fn validate_basic(&self) -> Result<()>;
    pub fn bytes(&self) -> Result<Vec<u8>>;
    pub fn from_proto(proto: proto::types::Validator) -> Result<Self>;
    pub fn to_proto(&self) -> proto::types::Validator;
}

/// A CometBFT validator set sorted by voting power, then address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorSet {
    pub validators: Vec<Validator>,
    pub proposer: Option<Validator>,
}

impl ValidatorSet {
    pub fn new(validators: Vec<Validator>) -> Result<Self>;
    pub fn validate_basic(&self) -> Result<()>;
    pub fn hash(&self) -> Hash;
    pub fn total_voting_power(&self) -> Result<i64>;
    pub fn from_proto(proto: proto::types::ValidatorSet) -> Result<Self>;
    pub fn to_proto(&self) -> proto::types::ValidatorSet;
}
```

### `vote`

```rust
/// CometBFT signed message type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum SignedMsgType {
    Unknown = 0,
    Prevote = 1,
    Precommit = 2,
    Proposal = 32,
}

impl TryFrom<i32> for SignedMsgType;
impl From<SignedMsgType> for i32;

/// A CometBFT vote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vote {
    pub ty: SignedMsgType,
    pub height: i64,
    pub round: i32,
    pub block_id: BlockId,
    pub timestamp: Option<prost_types::Timestamp>,
    pub validator_address: Address,
    pub validator_index: i32,
    pub signature: Vec<u8>,
    pub extension: Vec<u8>,
    pub extension_signature: Vec<u8>,
}

impl Vote {
    pub fn validate_basic(&self) -> Result<()>;
    pub fn commit_sig(&self) -> Result<CommitSig>;
    pub fn sign_bytes(&self, chain_id: &str) -> Result<Vec<u8>>;
    pub fn extension_sign_bytes(&self, chain_id: &str) -> Result<Vec<u8>>;
    pub fn verify(&self, chain_id: &str, public_key: &PublicKey) -> bool;
    pub fn from_proto(proto: proto::types::Vote) -> Result<Self>;
    pub fn to_proto(&self) -> proto::types::Vote;
}
```

### `proposal`

```rust
/// A CometBFT proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub height: i64,
    pub round: i32,
    pub pol_round: i32,
    pub block_id: BlockId,
    pub timestamp: Option<prost_types::Timestamp>,
    pub signature: Vec<u8>,
}

impl Proposal {
    pub fn validate_basic(&self) -> Result<()>;
    pub fn sign_bytes(&self, chain_id: &str) -> Result<Vec<u8>>;
    pub fn verify(&self, chain_id: &str, public_key: &PublicKey) -> bool;
    pub fn from_proto(proto: proto::types::Proposal) -> Result<Self>;
    pub fn to_proto(&self) -> proto::types::Proposal;
}
```

### `canonical`

```rust
/// Return CometBFT's canonical timestamp string.
pub fn canonical_time(timestamp: &prost_types::Timestamp) -> Result<String>;

/// Encode canonical vote sign bytes with the CometBFT length-delimited
/// protobuf wrapper.
pub fn vote_sign_bytes(chain_id: &str, vote: &Vote) -> Result<Vec<u8>>;

/// Encode canonical proposal sign bytes with the CometBFT length-delimited
/// protobuf wrapper.
pub fn proposal_sign_bytes(chain_id: &str, proposal: &Proposal) -> Result<Vec<u8>>;

/// Encode canonical vote-extension sign bytes.
pub fn vote_extension_sign_bytes(chain_id: &str, vote: &Vote) -> Result<Vec<u8>>;
```

### `proto`

The crate owns generated `prost` types for only the protobuf files needed by
M1:

```rust
pub mod crypto;
pub mod types;
pub mod version;
```

The generated modules must be private to protocol compatibility work in this
crate and downstream crates. Public domain types above are the preferred API;
direct `proto::*` use is allowed only for wire decoding/encoding boundaries.

## Behavioral requirements

**REQ-1.** Generated protobuf structs match CometBFT v0.38.12 field numbers,
oneof names, enum values, and package paths for the included files.

**REQ-2.** `Header::decode` and `Header::encode_to_vec` round-trip CometBFT
`tendermint.types.Header` protobuf bytes without losing unknown-free fields.

**REQ-3.** `Header::hash` matches CometBFT `Header.Hash()` exactly. It returns
`None` when `validators_hash` is absent, and otherwise hashes the 14 header
fields in Go order with CometBFT's legacy amino/`cdcEncode` behavior where Go
does so.

**REQ-4.** `PartSetHeader`, `BlockId`, `Header`, `Commit`, `CommitSig`,
`Validator`, `ValidatorSet`, `Vote`, and `Proposal` `validate_basic` methods
match CometBFT stateless validation for lengths, negative heights/rounds,
missing signatures, malformed block IDs, and malformed public keys.

**REQ-5.** Optional hash handling matches CometBFT: an empty protobuf `bytes`
field is `None`, a 32-byte field is `Some(Hash)`, and any other length is an
error except `Header::app_hash`, which is arbitrary bytes.

**REQ-6.** `BlockId::is_zero` and `BlockId::is_complete` match CometBFT:
zero means empty block hash plus zero part-set header; complete means 32-byte
block hash, positive part count, and 32-byte part-set hash.

**REQ-7.** `CommitSig::block_id` maps `Absent` and `Nil` to zero `BlockId` and
`Commit` to the commit's block ID. Unknown flags are rejected during protobuf
conversion.

**REQ-8.** `Commit::hash` matches CometBFT `Commit.Hash()` by protobuf-encoding
each `CommitSig` and passing the list to the `merkle` crate's CometBFT simple
Merkle root function.

**REQ-9.** `Commit::get_vote` and `Commit::vote_sign_bytes` reproduce
CometBFT's conversion from commit signature at validator index to precommit
vote sign bytes.

**REQ-10.** `PublicKey::address` matches CometBFT public-key addresses:
Ed25519 uses first 20 bytes of SHA-256 over the raw public key, and secp256k1
uses RIPEMD-160(SHA-256(compressed_public_key)).

**REQ-11.** `Validator::bytes` matches CometBFT `Validator.Bytes()`: encode the
protobuf `SimpleValidator` with public key and voting power only, excluding
address and proposer priority.

**REQ-12.** `ValidatorSet::new` sorts validators by voting power descending,
using address ascending as the tiebreaker exactly as CometBFT does, and rejects
duplicate validator addresses.

**REQ-13.** `ValidatorSet::hash` matches CometBFT `ValidatorSet.Hash()` by
hashing each validator's `bytes()` output with the CometBFT simple Merkle tree.

**REQ-14.** `Vote::sign_bytes`, `Proposal::sign_bytes`, and the free functions
in `canonical` match CometBFT `VoteSignBytes`, `ProposalSignBytes`, and
`VoteExtensionSignBytes`: canonical protobuf form, length-delimited with
unsigned varint prefix.

**REQ-15.** `Vote::verify` and `Proposal::verify` verify canonical sign bytes
using the `crypto` crate and reject signatures whose lengths or providers are
invalid.

**REQ-16.** Canonical timestamp formatting matches CometBFT `CanonicalTime`:
UTC, no monotonic component, RFC3339Nano-compatible formatting.

**REQ-17.** No crate outside `tendermint-types` should depend directly on
CometBFT-specific protobuf provider details for these M1 consensus types unless
a later crate spec explicitly grants an exception.

## Conformance fixtures

Committed under `conformance/fixtures/tendermint-types/`:

- `headers.json` - CometBFT v0.38.12 header protobuf bytes, decoded domain
  fields, validation result, and expected header hash. Include at least genesis
  style, empty optional hashes, and representative non-empty headers. Add real
  mainnet block headers as soon as a committed block corpus is available.
- `commits.json` - commit signatures, commit hashes, `get_vote` outputs, and
  vote sign bytes for absent, nil, and commit flags.
- `validators.json` - public keys, addresses, validator bytes, sorted validator
  sets, duplicate-address failures, and validator set hashes.
- `canonical-sign-bytes.json` - vote, proposal, and vote-extension canonical
  sign bytes including timestamp edge cases.
- `malformed.json` - malformed protobuf/domain cases covering invalid hash
  lengths, invalid address lengths, negative heights/rounds, missing signatures,
  invalid block IDs, invalid public keys, and duplicate validators.

Regeneration is split into one-file scenarios:

```bash
cargo xtask refresh-fixtures --crate-name tendermint-types
```

Equivalent individual scenarios:

```bash
cargo xtask refresh-fixtures --scenario tendermint-headers
cargo xtask refresh-fixtures --scenario tendermint-commits
cargo xtask refresh-fixtures --scenario tendermint-validators
cargo xtask refresh-fixtures --scenario tendermint-canonical-sign-bytes
cargo xtask refresh-fixtures --scenario tendermint-malformed
```

Each scenario must be registered in BOTH the Go fixturegen
(`conformance/fixturegen/cmd/`) and the Rust `SCENARIOS` const in
`xtask/src/refresh_fixtures.rs`.

## Definition of done

- [ ] Crate created at
      `crates/tier-1-protocol-types/tendermint-types/` with a `Cargo.toml` that
      inherits workspace metadata (`edition.workspace = true`, etc.), and the
      crate path is added to the root `Cargo.toml` `[workspace]` members list.
- [ ] Protobuf generation or checked-in generated `prost` modules cover only
      the M1 CometBFT proto files listed in `proto`.
- [ ] All REQ-1..REQ-17 covered by unit, property, or conformance tests.
- [ ] All five conformance fixture files committed with `.meta.json` sidecars.
- [ ] `cargo test -p tendermint-types` green.
- [ ] `cargo clippy -p tendermint-types --all-targets -- -D warnings` green.
- [ ] `cargo fmt --check` green.
- [ ] `cargo llvm-cov -p tendermint-types --summary-only` reports >= 90 % line
      coverage.
- [ ] `crates/tier-1-protocol-types/tendermint-types/README.md` present with
      purpose / consumers / conformance.
- [ ] Tendermint fixturegen scenarios registered in BOTH the Go fixturegen
      (`conformance/fixturegen/cmd/`) AND the Rust `SCENARIOS` const in
      `xtask/src/refresh_fixtures.rs`.
- [ ] PR opened as draft with assignee `@rootulp`; PR body uses `Closes` for
      any tracked issue, or `Closes N/A` when no issue exists.

## Out of scope

- ABCI request/response types and vote-extension application semantics.
- Evidence verification and evidence list hashing beyond header hash fields.
- Full `Block`, `Data`, and transaction execution semantics.
- Validator-set proposer-priority mutation algorithms beyond validation,
  sorting, total voting power, and hashing.
- Consensus reactor/state machine behavior.
- RPC, P2P, mempool, blocksync, and statesync wire messages.
- Mainnet block replay orchestration; that belongs to `m1-integration`.

## Open questions

- None for crate implementation. The repo does not yet contain a committed
  real-mainnet block corpus, so the first implementation should use
  deterministic CometBFT-generated fixtures and leave broad real-mainnet block
  replay to `m1-integration`.

## Notes for the implementing agent

1. **Start from protobuf compatibility.** Generate or vendor `prost` modules
   before implementing hashes; every hash/sign-byte fixture depends on exact
   wire encoding.
2. **Keep Go behavior visible.** For header hash, validator bytes, commit hash,
   and canonical sign bytes, write fixtures first from CometBFT v0.38.12 and
   implement Rust against them.
3. **Prefer domain types at crate boundaries.** `proto::*` should be available
   for codecs, but downstream crates should use `Header`, `Commit`,
   `ValidatorSet`, `Vote`, and `Proposal`.
4. **Do not broaden into ABCI or Cosmos SDK.** If a field is needed only by
   later crates, expose it through the domain types here but leave higher-level
   interpretation to the later crate.
