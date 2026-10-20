# Crate: `abci-types`

**Tier:** 1 (protocol types)
**Depends on:** `encoding`, `tendermint-types`
**Consumed by:** M1 integration tests, M2 state/replay crates, M3
transaction/ante machinery, tier-4 runtime crates, and downstream ABCI server
adapters.
**Source of truth (Go):** CometBFT v0.38.12
`github.com/cometbft/cometbft/abci/types` and
`github.com/cometbft/cometbft/proto/tendermint/abci`.
**Crate location:** `crates/tier-1-protocol-types/abci-types/`

## Purpose

Provide the CometBFT ABCI++ wire types Celestia needs before state execution:
request/response envelopes, proposal-processing messages, vote-extension
messages, commit information, validator updates, events, transaction execution
results, evidence/misbehavior descriptors, and state-sync snapshot messages.

This crate is a protocol compatibility layer. It does not execute transactions,
run the app state machine, verify vote-extension signatures, store snapshots,
serve sockets, or implement an ABCI server loop. Those belong to later runtime
and module crates.

## Public API

The crate exports exactly these root modules:

```rust
//! CometBFT ABCI++ protocol types for celestia-rs.
//!
//! See `docs/specs/m1/abci-types.md` for the API contract.

pub mod error;
pub mod event;
pub mod exec;
pub mod proto;
pub mod request;
pub mod response;
pub mod snapshot;
pub mod validator;
pub mod vote_extension;

pub use error::{Error, Result};
pub use event::{Event, EventAttribute};
pub use exec::ExecTxResult;
pub use request::{Request, RequestValue};
pub use response::{Response, ResponseValue};
pub use snapshot::Snapshot;
pub use validator::{Misbehavior, MisbehaviorType, Validator, ValidatorUpdate};
pub use vote_extension::{CommitInfo, ExtendedCommitInfo, ExtendedVoteInfo, VoteInfo};
```

The crate sets `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.

### `proto`

The crate re-exports the pinned CometBFT v0.38.12 prost modules needed by
ABCI++:

```rust
pub mod abci;    // tendermint.abci
pub mod crypto;  // tendermint.crypto
pub mod google;  // google.protobuf.Timestamp
pub mod types;   // tendermint.types
```

The generated modules are public only for wire-codec boundaries and
conformance tests. Domain types below are the preferred API.

### `event`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventAttribute {
    pub key: String,
    pub value: String,
    pub index: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub ty: String,
    pub attributes: Vec<EventAttribute>,
}

impl EventAttribute {
    pub fn validate_basic(&self) -> Result<()>;
    pub fn from_proto(proto: proto::abci::EventAttribute) -> Result<Self>;
    pub fn to_proto(&self) -> proto::abci::EventAttribute;
}

impl Event {
    pub fn validate_basic(&self) -> Result<()>;
    pub fn from_proto(proto: proto::abci::Event) -> Result<Self>;
    pub fn to_proto(&self) -> proto::abci::Event;
}
```

### `exec`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecTxResult {
    pub code: u32,
    pub data: Vec<u8>,
    pub log: String,
    pub info: String,
    pub gas_wanted: i64,
    pub gas_used: i64,
    pub events: Vec<Event>,
    pub codespace: String,
}

impl ExecTxResult {
    pub fn validate_basic(&self) -> Result<()>;
    pub fn from_proto(proto: proto::abci::ExecTxResult) -> Result<Self>;
    pub fn to_proto(&self) -> proto::abci::ExecTxResult;
}
```

### `validator`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum MisbehaviorType {
    Unknown = 0,
    DuplicateVote = 1,
    LightClientAttack = 2,
}

impl TryFrom<i32> for MisbehaviorType;
impl From<MisbehaviorType> for i32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validator {
    pub address: tendermint_types::hash::Address,
    pub power: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorUpdate {
    pub public_key: tendermint_types::PublicKey,
    pub power: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Misbehavior {
    pub ty: MisbehaviorType,
    pub validator: Validator,
    pub height: i64,
    pub time: proto::google::Timestamp,
    pub total_voting_power: i64,
}

impl Validator {
    pub fn validate_basic(&self) -> Result<()>;
    pub fn from_proto(proto: proto::abci::Validator) -> Result<Self>;
    pub fn to_proto(&self) -> proto::abci::Validator;
}

impl ValidatorUpdate {
    pub fn validate_basic(&self) -> Result<()>;
    pub fn from_proto(proto: proto::abci::ValidatorUpdate) -> Result<Self>;
    pub fn to_proto(&self) -> proto::abci::ValidatorUpdate;
}

impl Misbehavior {
    pub fn validate_basic(&self) -> Result<()>;
    pub fn from_proto(proto: proto::abci::Misbehavior) -> Result<Self>;
    pub fn to_proto(&self) -> proto::abci::Misbehavior;
}
```

### `vote_extension`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoteInfo {
    pub validator: Validator,
    pub block_id_flag: tendermint_types::BlockIdFlag,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtendedVoteInfo {
    pub validator: Validator,
    pub vote_extension: Vec<u8>,
    pub extension_signature: Vec<u8>,
    pub block_id_flag: tendermint_types::BlockIdFlag,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    pub round: i32,
    pub votes: Vec<VoteInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtendedCommitInfo {
    pub round: i32,
    pub votes: Vec<ExtendedVoteInfo>,
}

impl VoteInfo {
    pub fn validate_basic(&self) -> Result<()>;
    pub fn from_proto(proto: proto::abci::VoteInfo) -> Result<Self>;
    pub fn to_proto(&self) -> proto::abci::VoteInfo;
}

impl ExtendedVoteInfo {
    pub fn validate_basic(&self) -> Result<()>;
    pub fn from_proto(proto: proto::abci::ExtendedVoteInfo) -> Result<Self>;
    pub fn to_proto(&self) -> proto::abci::ExtendedVoteInfo;
}

impl CommitInfo {
    pub fn validate_basic(&self) -> Result<()>;
    pub fn from_proto(proto: proto::abci::CommitInfo) -> Result<Self>;
    pub fn to_proto(&self) -> proto::abci::CommitInfo;
}

impl ExtendedCommitInfo {
    pub fn validate_basic(&self) -> Result<()>;
    pub fn from_proto(proto: proto::abci::ExtendedCommitInfo) -> Result<Self>;
    pub fn to_proto(&self) -> proto::abci::ExtendedCommitInfo;
}
```

### `snapshot`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub height: u64,
    pub format: u32,
    pub chunks: u32,
    pub hash: Vec<u8>,
    pub metadata: Vec<u8>,
}

impl Snapshot {
    pub fn validate_basic(&self) -> Result<()>;
    pub fn from_proto(proto: proto::abci::Snapshot) -> Result<Self>;
    pub fn to_proto(&self) -> proto::abci::Snapshot;
}
```

### `request`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub value: RequestValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestValue {
    Echo(proto::abci::RequestEcho),
    Flush,
    Info(proto::abci::RequestInfo),
    InitChain(proto::abci::RequestInitChain),
    Query(proto::abci::RequestQuery),
    CheckTx(proto::abci::RequestCheckTx),
    Commit,
    ListSnapshots,
    OfferSnapshot(proto::abci::RequestOfferSnapshot),
    LoadSnapshotChunk(proto::abci::RequestLoadSnapshotChunk),
    ApplySnapshotChunk(proto::abci::RequestApplySnapshotChunk),
    PrepareProposal(proto::abci::RequestPrepareProposal),
    ProcessProposal(proto::abci::RequestProcessProposal),
    ExtendVote(proto::abci::RequestExtendVote),
    VerifyVoteExtension(proto::abci::RequestVerifyVoteExtension),
    FinalizeBlock(proto::abci::RequestFinalizeBlock),
}

impl Request {
    pub fn validate_basic(&self) -> Result<()>;
    pub fn from_proto(proto: proto::abci::Request) -> Result<Self>;
    pub fn to_proto(&self) -> proto::abci::Request;
    pub fn decode(bytes: &[u8]) -> Result<Self>;
    pub fn encode_to_vec(&self) -> Result<Vec<u8>>;
}
```

### `response`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub value: ResponseValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseValue {
    Exception(proto::abci::ResponseException),
    Echo(proto::abci::ResponseEcho),
    Flush,
    Info(proto::abci::ResponseInfo),
    InitChain(proto::abci::ResponseInitChain),
    Query(proto::abci::ResponseQuery),
    CheckTx(proto::abci::ResponseCheckTx),
    Commit(proto::abci::ResponseCommit),
    ListSnapshots(proto::abci::ResponseListSnapshots),
    OfferSnapshot(proto::abci::ResponseOfferSnapshot),
    LoadSnapshotChunk(proto::abci::ResponseLoadSnapshotChunk),
    ApplySnapshotChunk(proto::abci::ResponseApplySnapshotChunk),
    PrepareProposal(proto::abci::ResponsePrepareProposal),
    ProcessProposal(proto::abci::ResponseProcessProposal),
    ExtendVote(proto::abci::ResponseExtendVote),
    VerifyVoteExtension(proto::abci::ResponseVerifyVoteExtension),
    FinalizeBlock(proto::abci::ResponseFinalizeBlock),
}

impl Response {
    pub fn validate_basic(&self) -> Result<()>;
    pub fn from_proto(proto: proto::abci::Response) -> Result<Self>;
    pub fn to_proto(&self) -> proto::abci::Response;
    pub fn decode(bytes: &[u8]) -> Result<Self>;
    pub fn encode_to_vec(&self) -> Result<Vec<u8>>;
}
```

### `error`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidAddressLength { actual: usize },
    InvalidHashLength { actual: usize },
    InvalidHeight,
    InvalidRound,
    InvalidEvent,
    InvalidExecTxResult,
    InvalidValidator,
    InvalidValidatorUpdate,
    InvalidVoteInfo,
    InvalidCommitInfo,
    InvalidMisbehavior,
    InvalidSnapshot,
    InvalidRequest,
    InvalidResponse,
    InvalidStatus,
    InvalidPublicKey,
    Decode(String),
    Encode(String),
}

impl core::fmt::Display for Error;
impl std::error::Error for Error;

pub type Result<T> = core::result::Result<T, Error>;
```

## Behavioral requirements

**REQ-1.** Protobuf structs match CometBFT v0.38.12 `tendermint.abci` field
numbers, oneof names, enum values, reserved fields, JSON names, and package
paths.

**REQ-2.** `Request::decode`/`encode_to_vec` and
`Response::decode`/`encode_to_vec` round-trip unknown-free protobuf bytes
without changing oneof variants or field order.

**REQ-3.** Request and response envelopes reject missing oneof values and
unknown enum/status values.

**REQ-4.** `CommitInfo` and `ExtendedCommitInfo` preserve vote order, reject
negative rounds, and validate every nested vote info.

**REQ-5.** `VoteInfo` and `ExtendedVoteInfo` validate 20-byte validator
addresses, known `BlockIdFlag` values, non-negative validator power, and
bounded extension-signature sizes.

**REQ-6.** `ValidatorUpdate` converts CometBFT public keys through
`tendermint-types::PublicKey`, validates supported key variants, and preserves
voting power exactly.

**REQ-7.** `Misbehavior` accepts only known CometBFT misbehavior enum values,
rejects non-positive heights, validates validator shape, and preserves
timestamps exactly.

**REQ-8.** `Event` and `EventAttribute` preserve order and UTF-8 strings.
Empty event types and empty attribute keys are rejected.

**REQ-9.** `ExecTxResult`, `ResponseCheckTx`, and `ResponseFinalizeBlock`
preserve event order, gas values, codespace, logs, info strings, and tx-result
ordering exactly. Nondeterministic fields are still carried byte-for-byte.

**REQ-10.** `Snapshot` rejects zero heights, zero chunks, and zero formats; it
preserves hash and metadata bytes exactly.

**REQ-11.** Proposal-processing request/response variants preserve all
transaction byte vectors in order and do not attempt to decode Cosmos SDK txs.

**REQ-12.** Vote-extension request/response variants preserve vote-extension
bytes exactly and accept only known verify statuses.

**REQ-13.** No crate outside `abci-types` should depend directly on
CometBFT-specific ABCI protobuf provider details unless a later crate spec
explicitly grants an exception.

## Conformance fixtures

Committed under `conformance/fixtures/abci-types/`:

- `requests.json` - ABCI++ request-envelope protobuf vectors covering every
  request variant.
- `responses.json` - ABCI++ response-envelope protobuf vectors covering every
  response variant and status enum.
- `vote-extensions.json` - `CommitInfo`, `ExtendedCommitInfo`,
  `RequestExtendVote`, `RequestVerifyVoteExtension`,
  `ResponseExtendVote`, and `ResponseVerifyVoteExtension` vectors.
- `events.json` - event, attribute, check-tx, finalize-block, and exec-tx
  result vectors.
- `malformed.json` - malformed protobuf/domain cases covering missing oneofs,
  bad addresses, unknown enum/status values, negative heights/rounds, invalid
  events, invalid snapshots, and malformed public keys.

Regenerate fixtures:

```bash
cargo xtask refresh-fixtures --crate-name abci-types
```

Equivalent individual scenarios:

```bash
cargo xtask refresh-fixtures --scenario abci-requests
cargo xtask refresh-fixtures --scenario abci-responses
cargo xtask refresh-fixtures --scenario abci-vote-extensions
cargo xtask refresh-fixtures --scenario abci-events
cargo xtask refresh-fixtures --scenario abci-malformed
```

Each scenario must be registered in BOTH the Go fixturegen
(`conformance/fixturegen/cmd/`) and the Rust `SCENARIOS` const in
`xtask/src/refresh_fixtures.rs`.

## Definition of done

- [ ] Crate created at `crates/tier-1-protocol-types/abci-types/` with a
      `Cargo.toml` inheriting workspace metadata, and the crate path is added
      to root `Cargo.toml` `[workspace]` members.
- [ ] Public API above implemented with documented modules and
      `#![forbid(unsafe_code)]`.
- [ ] Protobuf generation or checked-in prost modules cover only the M1 ABCI++
      protobuf surface listed in `proto`.
- [ ] All REQ-1..REQ-13 covered by unit, property, or conformance tests.
- [ ] All five conformance fixture files committed with `.meta.json` sidecars.
- [ ] `cargo test -p abci-types` green.
- [ ] `cargo clippy -p abci-types --all-targets -- -D warnings` green.
- [ ] `cargo fmt --check` green.
- [ ] `cargo llvm-cov -p abci-types --summary-only` reports >= 90 % line
      coverage.
- [ ] `crates/tier-1-protocol-types/abci-types/README.md` present with
      purpose / consumers / conformance.
- [ ] ABCI fixturegen scenarios registered in BOTH the Go fixturegen and the
      Rust `SCENARIOS` const.
- [ ] PR opened as draft with assignee `@rootulp`; PR body uses `Closes` for
      any tracked issue, or `Closes N/A` when no issue exists.

## Out of scope

- Application state execution and module keepers.
- ABCI server transport, sockets, gRPC services, and runtime dispatch loops.
- Mempool admission policy, gas metering, fee handling, and signature
  verification.
- Vote-extension signature verification policy.
- Snapshot storage, chunk persistence, and state-sync orchestration.
- Cosmos SDK transaction decoding beyond preserving opaque tx bytes.
