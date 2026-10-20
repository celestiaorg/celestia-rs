# Crate: `ante`

**Tier:** 3 (transaction and ante machinery)
**Depends on:** `abci-types`, `cosmos-types`, `crypto`, `hash`, `kvstore`
**Consumed by:** `m3-integration`, M4 `x/auth` / `x/bank` keepers, later
runtime and ABCI app crates.
**Source of truth (Go):** Cosmos SDK v0.50.10 ante handlers as pulled by
celestia-app app-v6, plus celestia-app fee/min-gas configuration for mainnet
transaction validation.
**Crate location:** `crates/tier-3-app-modules/ante/`

## Purpose

Provide the Cosmos SDK ante surface needed to validate Celestia transactions
before module message execution. The crate owns stateless transaction checks,
SIGN_MODE_DIRECT sign bytes, secp256k1 signature verification, fee/gas
validation, account number and sequence checks, and a small trait boundary for
account and fee state.

The first implementation target is a deterministic ante pipeline that can run
against fixture-backed account and bank snapshots. It should not implement
`x/auth`, `x/bank`, or message execution; those crates will later implement the
traits defined here.

## Public API

The crate exports exactly these items from its root module:

```rust
//! Cosmos SDK transaction ante machinery for celestia-rs.
//!
//! See `docs/specs/m3/ante.md` for the API contract.

pub use cosmos_types::tx::{AuthInfo, Fee, ModeInfo, SignDoc, SignMode, SignerInfo, Tx, TxRaw};

pub type Gas = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunTxMode {
    Check,
    Recheck,
    Deliver,
    Simulate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnteConfig {
    pub chain_id: String,
    pub min_gas_prices: Vec<cosmos_types::Coin>,
    pub tx_size_cost_per_byte: Gas,
    pub sig_verify_cost_secp256k1: Gas,
    pub max_memo_characters: usize,
    pub consume_signature_verification_gas: bool,
}

impl AnteConfig {
    pub fn celestia_mainnet_app_v6(chain_id: impl Into<String>) -> Self;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub address: cosmos_types::AccountAddress,
    pub account_number: u64,
    pub sequence: u64,
    pub public_key: Option<cosmos_types::Any>,
}

pub trait AccountReader {
    fn account(&self, address: &cosmos_types::AccountAddress) -> Result<Option<Account>>;
}

pub trait AccountWriter: AccountReader {
    fn set_sequence(&mut self, address: &cosmos_types::AccountAddress, sequence: u64)
        -> Result<()>;
    fn set_public_key(
        &mut self,
        address: &cosmos_types::AccountAddress,
        public_key: cosmos_types::Any,
    ) -> Result<()>;
}

pub trait FeeKeeper {
    fn spendable_balance(
        &self,
        address: &cosmos_types::AccountAddress,
        denom: &str,
    ) -> Result<Option<cosmos_types::Coin>>;

    fn deduct_fees(
        &mut self,
        payer: &cosmos_types::AccountAddress,
        fees: &[cosmos_types::Coin],
    ) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GasMeter {
    pub limit: Gas,
    pub consumed: Gas,
}

impl GasMeter {
    pub fn new(limit: Gas) -> Self;
    pub fn infinite() -> Self;
    pub fn consume(&mut self, amount: Gas, descriptor: &'static str) -> Result<()>;
    pub fn remaining(&self) -> Option<Gas>;
    pub fn is_out_of_gas(&self) -> bool;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnteContext {
    pub mode: RunTxMode,
    pub block_height: i64,
    pub gas_meter: GasMeter,
    pub events: Vec<abci_types::Event>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignerData {
    pub address: cosmos_types::AccountAddress,
    pub account_number: u64,
    pub sequence: u64,
    pub public_key: cosmos_types::Any,
    pub mode: SignMode,
    pub sign_bytes: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnteOutcome {
    pub gas_wanted: Gas,
    pub gas_used: Gas,
    pub events: Vec<abci_types::Event>,
    pub signer_sequences: Vec<(cosmos_types::AccountAddress, u64)>,
}

pub struct AnteHandler { /* opaque */ }

impl AnteHandler {
    pub fn new(config: AnteConfig) -> Self;

    pub fn validate_tx_stateless(&self, tx_bytes: &[u8]) -> Result<TxRaw>;

    pub fn signer_data<A: AccountReader>(
        &self,
        tx: &TxRaw,
        accounts: &A,
    ) -> Result<Vec<SignerData>>;

    pub fn run<A, F>(
        &self,
        ctx: AnteContext,
        tx_bytes: &[u8],
        accounts: &mut A,
        fees: &mut F,
    ) -> Result<AnteOutcome>
    where
        A: AccountWriter,
        F: FeeKeeper;
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("malformed transaction")]
    MalformedTx,
    #[error("unsupported sign mode")]
    UnsupportedSignMode,
    #[error("missing signer")]
    MissingSigner,
    #[error("signature verification failed")]
    SignatureVerification,
    #[error("account not found")]
    AccountNotFound,
    #[error("account number mismatch")]
    AccountNumberMismatch,
    #[error("sequence mismatch")]
    SequenceMismatch,
    #[error("insufficient fee")]
    InsufficientFee,
    #[error("insufficient funds")]
    InsufficientFunds,
    #[error("out of gas")]
    OutOfGas,
    #[error("gas overflow")]
    GasOverflow,
    #[error("invalid memo")]
    InvalidMemo,
    #[error("invalid public key")]
    InvalidPublicKey,
    #[error("state error: {0}")]
    State(String),
}

pub type Result<T> = std::result::Result<T, Error>;
```

## Behavioral Requirements

**REQ-1.** `validate_tx_stateless` decodes `cosmos.tx.v1beta1.TxRaw` through
`cosmos-types`, rejects malformed protobuf, missing body bytes, missing auth
info bytes, mismatched signature counts, empty messages, unsupported extension
options, and memos longer than `AnteConfig::max_memo_characters`.

**REQ-2.** The ante handler supports `SIGN_MODE_DIRECT` for single secp256k1
signers and `SIGN_MODE_LEGACY_AMINO_JSON` for the delegate and withdraw
delegator reward message shapes present in the mainnet corpus. It rejects
`Unspecified`, `Textual`, `DirectAux`, `Eip191`, other legacy Amino message
types, and all multisig mode infos until fixtures prove the exact Go behavior
needed by Celestia mainnet.

**REQ-3.** Signer addresses are derived from compressed secp256k1 public keys
using the `crypto` and `cosmos-types` address helpers. If an account already
has a public key, the transaction public key must match it. If the account has
no public key and the signature verifies, `run` stores the public key.

**REQ-4.** `signer_data` constructs one sign document per signer. Direct sign
bytes use the original body bytes, auth-info bytes, configured chain ID, and
account number from state. Legacy Amino JSON sign bytes use the SDK canonical
JSON document and message names. Both forms must byte-match Cosmos SDK.

**REQ-5.** Signature verification uses Cosmos SDK secp256k1 lower-S semantics
from the local `crypto` crate. Invalid public keys, malformed signatures, and
verification failures return typed errors without mutating state.

**REQ-6.** Account number and sequence checks match Cosmos SDK ante behavior:
account number mismatch rejects the transaction; sequence mismatch rejects the
transaction; successful `Deliver` mode increments every signer sequence by one.

**REQ-7.** `GasMeter::consume` uses checked arithmetic. Finite meters return
`OutOfGas` after recording consumed gas; overflow returns `GasOverflow`;
infinite meters never return `OutOfGas`.

**REQ-8.** `run` initializes gas wanted from `AuthInfo.fee.gas_limit`, preserves
gas already consumed on the supplied context by fixture-backed keeper access,
consumes transaction-size gas using `tx_size_cost_per_byte`, and consumes
configured signature verification gas for each verified signer when enabled.

**REQ-9.** Fee validation compares fee amount against `min_gas_prices *
gas_limit` using SDK coin semantics. Missing fees are valid only when all
configured minimum gas prices are zero.

**REQ-10.** Fee payer defaults to the first signer unless `Fee.payer` is set.
`run` validates the payer address, checks spendable balances through
`FeeKeeper`, and deducts fees only after stateless checks, account checks, and
signature verification have all succeeded.

**REQ-11.** `Simulate` mode runs stateless checks, signer extraction, and gas
estimation without sequence mutation or fee deduction. `Check`, `Recheck`, and
`Deliver` modes must match the Go fixtures for sequence mutation and fee
deduction behavior.

**REQ-12.** The crate emits deterministic ante events for fee deduction and
signature verification only when the Go fixtures show the corresponding events
for the selected run mode. Event ordering and attributes must match fixtures.

**REQ-13.** Error mapping preserves enough information for
`m3-integration` to compare Go `code`, `codespace`, `log`, `gas_wanted`, and
`gas_used` outputs. The public `Error` variants remain stable even if internal
messages change.

**REQ-14.** The crate performs no disk IO. All account and fee state comes
through `AccountReader`, `AccountWriter`, and `FeeKeeper`; fixture-backed
implementations live in tests or `m3-integration`.

## Conformance Fixtures

Committed under `conformance/fixtures/ante/`:

- `stateless.json` - raw transaction decode, memo, signature count, extension
  option, empty-message, and unsupported sign-mode cases.
- `signatures.json` - secp256k1 signer extraction, sign-doc bytes, account
  number checks, sequence checks, invalid public keys, malformed signatures,
  and verification failures.
- `fees.json` - min-gas-price decisions, explicit fee payer handling,
  insufficient funds, zero-fee transactions, and fee deduction summaries.
- `gas.json` - tx-size gas, signature verification gas, out-of-gas and
  overflow cases, and simulate-mode gas accounting.
- `mainnet-tx-corpus.json.zst` or `mainnet-tx-corpus.json` - at least 1000
  mainnet transaction validation outcomes generated by Go celestia-app using
  the schema in [`mainnet-tx-corpus.md`](mainnet-tx-corpus.md).
- `malformed.json` - protobuf and fixture-shape errors not covered above.

Regenerate fixtures:

```bash
cargo xtask refresh-fixtures --crate-name ante
```

Equivalent individual scenarios:

```bash
cargo xtask refresh-fixtures --scenario ante-stateless
cargo xtask refresh-fixtures --scenario ante-signatures
cargo xtask refresh-fixtures --scenario ante-fees
cargo xtask refresh-fixtures --scenario ante-gas
cargo xtask refresh-fixtures --scenario ante-malformed
```

The `ante-mainnet-tx-corpus` fixturegen command is intentionally not registered
in `xtask refresh-fixtures` yet because it requires external mainnet RPC and
local application DB inputs. Register it in `xtask/src/refresh_fixtures.rs`
only once it can produce the committed corpus fixture from explicit inputs.

## Definition Of Done

- [x] Crate created at `crates/tier-3-app-modules/ante/` with a `Cargo.toml`
      inheriting workspace metadata, and the crate path is added to root
      `Cargo.toml` `[workspace]` members.
- [x] Public API above implemented with documented modules and
      `#![forbid(unsafe_code)]`.
- [x] All REQ-1..REQ-14 covered by unit, property, or conformance tests.
- [x] All six conformance fixture files committed with `.meta.json` sidecars
      (`stateless.json`, `signatures.json`, `fees.json`, `gas.json`,
      `malformed.json`, and the mainnet transaction corpus).
- [x] `m3-integration::mainnet_tx_validation_matches_go` passes for at least
      1000 mainnet transactions.
- [x] `cargo test -p ante` green.
- [x] `cargo clippy -p ante --all-targets -- -D warnings` green.
- [x] `cargo fmt --check` green.
- [x] `cargo llvm-cov -p ante --summary-only` reports >= 90 % line coverage.
- [x] `crates/tier-3-app-modules/ante/README.md` present with purpose /
      consumers / conformance.
- [x] Ante fixturegen scenarios registered in BOTH the Go fixturegen and the
      Rust `SCENARIOS` const.

## Out Of Scope

- `x/auth` account store encoding and genesis initialization.
- `x/bank` balances, supply, sends, blocked addresses, and metadata.
- Message service routing, keeper execution, and module state transitions.
- Legacy Amino JSON message shapes outside the committed corpus, textual
  signing, SIGN_MODE_DIRECT_AUX, multisig, and feegrant/authz semantics.
- Mempool priority, replacement policy, peer scoring, and gossip.
- Persistent stores, snapshots, pruning, and ABCI server integration.

## Notes For The Implementing Agent

- Start with red tests for malformed `TxRaw`, unsupported sign modes,
  secp256k1 sign-doc verification, fee payer selection, and sequence mutation.
- Keep account and fee state traits small. Do not pre-implement `x/auth` or
  `x/bank`; fixture adapters are enough for M3.
- Reuse `cosmos-types::SignDoc::sign_bytes` and `crypto::secp256k1::verify`
  rather than duplicating encoding or signature logic.
- Fixturegen should use Go celestia-app ante decorators directly where
  possible. If constructing a full app is required, keep the emitted fixture
  schema independent of Go internals so Rust tests remain stable across app
  upgrades.
- Prefer explicit fixture cases over broad random generation for error
  surfaces; mainnet corpus coverage should be the broad integration signal.
