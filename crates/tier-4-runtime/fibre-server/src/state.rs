//! What the server needs from the node: chain state (Go `state.Client`)
//! and the validator's consensus key (the parts of Go `core.PrivValidator`
//! the server uses).
//!
//! # Async, object-safe
//!
//! Every query returns a [`BoxFuture`], so the traits are object safe
//! (`Arc<dyn ChainState>`, `Arc<dyn Signer>`) without a macro crate, and the
//! server can bound each call with a timeout (health probes) or abandon it
//! when the client cancels. The node's implementations read in-process
//! state, but some of that state sits behind locks the block executor holds
//! for a whole block (the app's query path), and the signer may be a remote
//! privval: a synchronous call there would park a runtime worker and could
//! not be timed out. An implementation that has to block must do so in
//! `tokio::task::spawn_blocking`, never on the calling task.
//!
//! [`ChainState::chain_id`] is the one synchronous call: the chain ID is
//! fixed for the node's lifetime.
//!
//! # Error texts
//!
//! Errors are carried as text because Go puts them, wrapped, into gRPC
//! status messages that clients see (`payment promise verification failed:
//! stateful validation: <text>`). For byte parity with Go's server, the
//! node should produce the text Go's app client would see from its gRPC
//! query, e.g. `rpc error: code = InvalidArgument desc = <message>` for a
//! failed `ValidatePaymentPromise` query.

use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, SystemTime};

pub use celestia_proto::celestia::fibre::v1::PaymentPromise;

/// A boxed, `Send` future (the return type of the async trait methods).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A failed chain-state query.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StateError {
    /// The app has no Fibre module (Go: the query failed with gRPC
    /// `Unimplemented`). The `fibre_module` health check reports it as
    /// `fibre_module_unavailable`.
    #[error("{0}")]
    ModuleUnavailable(String),
    /// Any other failure; the text reaches clients and logs.
    #[error("{0}")]
    Other(String),
}

impl StateError {
    /// A [`StateError::Other`] with this text.
    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }
}

/// One validator of a validator set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validator {
    /// Consensus address: the first 20 bytes of SHA-256 of the public key.
    pub address: [u8; 20],
    /// Ed25519 consensus public key.
    pub pub_key: [u8; 32],
    /// Voting power.
    pub voting_power: i64,
}

impl Validator {
    /// A validator with the address derived from `pub_key`.
    #[must_use]
    pub fn new(pub_key: [u8; 32], voting_power: i64) -> Self {
        Self {
            address: crypto::ed25519::address(&pub_key),
            pub_key,
            voting_power,
        }
    }
}

/// A validator set at a height (Go `validator.Set`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ValidatorSet {
    /// The validators in CometBFT order (voting power descending, then
    /// address ascending), as `core.ValidatorSet.Validators` holds them.
    /// Row assignment depends on this order.
    pub validators: Vec<Validator>,
    /// The height the set is valid at.
    pub height: u64,
}

impl ValidatorSet {
    /// Go `Set.GetByAddress`: the position and entry of the validator with
    /// this consensus address.
    #[must_use]
    pub fn get_by_address(&self, address: &[u8; 20]) -> Option<(usize, &Validator)> {
        self.validators
            .iter()
            .enumerate()
            .find(|(_, v)| &v.address == address)
    }

    /// Voting powers in set order (the input of
    /// [`crate::assignment::assign`]).
    #[must_use]
    pub fn voting_powers(&self) -> Vec<i64> {
        self.validators.iter().map(|v| v.voting_power).collect()
    }

    /// `TotalVotingPower` (wrapping, as the assignment arithmetic is).
    #[must_use]
    pub fn total_voting_power(&self) -> i64 {
        self.validators
            .iter()
            .fold(0i64, |a, v| a.wrapping_add(v.voting_power))
    }
}

/// The result of a successful stateful promise check (Go
/// `state.VerifiedPromise`, from the `ValidatePaymentPromise` query).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedPromise {
    /// When the payment promise expires.
    pub expires_at: SystemTime,
    /// The on-chain minimum local retention of the shard.
    pub shard_retention: Duration,
}

/// A snapshot of the node's status (Go `state.NodeStatus`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NodeStatus {
    /// The chain ID the node reports.
    pub chain_id: String,
    /// Latest block height.
    pub height: u64,
    /// Latest block time; `None` is Go's zero time.
    pub block_time: Option<SystemTime>,
    /// Whether the node is still syncing.
    pub catching_up: bool,
}

/// The on-chain Fibre provider registration of a validator (Go
/// `state.ProviderRegistration`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderRegistration {
    /// A registration exists.
    pub found: bool,
    /// The registered `host:port` (only when `found`).
    pub host: String,
}

/// Chain state the server reads (Go `state.Client`).
pub trait ChainState: Send + Sync + 'static {
    /// The chain ID (Go: detected once by `Start`).
    fn chain_id(&self) -> String;

    /// The validator set at `height` (Go `GetByHeight`; height > 0).
    fn validator_set(&self, height: u64) -> BoxFuture<'_, Result<ValidatorSet, StateError>>;

    /// The latest validator set (Go `Head`).
    fn head_validator_set(&self) -> BoxFuture<'_, Result<ValidatorSet, StateError>>;

    /// Go `VerifyPromise`: the `ValidatePaymentPromise` query of the Fibre
    /// module for `promise` as received. An invalid promise is an error
    /// (Go: `payment promise is invalid` when the query says `is_valid =
    /// false`, `expiration time not provided in validation response` when
    /// it lacks the time). The text is client visible.
    fn verify_promise<'a>(
        &'a self,
        promise: &'a PaymentPromise,
    ) -> BoxFuture<'a, Result<VerifiedPromise, StateError>>;

    /// The `FullStakeStorageBudget` governance parameter in bytes, capped
    /// at `i64::MAX` (Go: the module's `Params` query).
    /// [`StateError::ModuleUnavailable`] when the app has no Fibre module.
    fn full_stake_storage_budget(&self) -> BoxFuture<'_, Result<i64, StateError>>;

    /// The node's status (Go `NodeStatus`; health checks only).
    fn node_status(&self) -> BoxFuture<'_, Result<NodeStatus, StateError>>;

    /// The Fibre provider registration of the validator with this
    /// consensus address, never cached (Go `ProviderRegistration`; health
    /// checks only).
    fn provider_registration<'a>(
        &'a self,
        consensus_address: &'a [u8; 20],
    ) -> BoxFuture<'a, Result<ProviderRegistration, StateError>>;
}

/// A failed signer call; the text reaches logs and (for signing) clients.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SignerError(pub String);

/// The validator's consensus key (the parts of Go `core.PrivValidator`
/// the server uses). Only Ed25519 keys can serve Fibre: clients verify
/// 64-byte Ed25519 signatures.
pub trait Signer: Send + Sync + 'static {
    /// The consensus public key (Go `GetPubKey`). Health checks compare it
    /// with the key read at start.
    fn public_key(&self) -> BoxFuture<'_, Result<[u8; 32], SignerError>>;

    /// Go `SignRawBytes`: signs CometBFT's
    /// `RawBytesMessageSignBytes(chain_id, unique_id, raw)` with the
    /// consensus key and returns the signature.
    fn sign_raw_bytes<'a>(
        &'a self,
        chain_id: &'a str,
        unique_id: &'a str,
        raw: &'a [u8],
    ) -> BoxFuture<'a, Result<Vec<u8>, SignerError>>;
}

/// A [`Signer`] over a local Ed25519 key (tests, and nodes that hold the
/// key in process).
impl Signer for crate::tlsid::Ed25519RawBytesSigner {
    fn public_key(&self) -> BoxFuture<'_, Result<[u8; 32], SignerError>> {
        let pk = crate::tlsid::Ed25519RawBytesSigner::public_key(self);
        Box::pin(async move { Ok(pk) })
    }

    fn sign_raw_bytes<'a>(
        &'a self,
        chain_id: &'a str,
        unique_id: &'a str,
        raw: &'a [u8],
    ) -> BoxFuture<'a, Result<Vec<u8>, SignerError>> {
        let res = crate::tlsid::RawBytesSigner::sign_raw_bytes(self, chain_id, unique_id, raw)
            .map_err(SignerError);
        Box::pin(async move { res })
    }
}
