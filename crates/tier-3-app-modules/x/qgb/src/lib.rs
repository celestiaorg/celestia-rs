//! celestia-app v1 `x/blobstream` (store and module name `qgb`), as
//! celestia-app v3.13.0 runs it at app version 1: the attestation store
//! (validator sets and data commitments for the Blobstream EVM
//! contracts), each validator's EVM address, the staking hooks and the
//! EndBlocker. The module only exists at app version 1; its store is
//! dropped at the v1 to v2 upgrade.
//!
//! Store layout (raw string prefixes, `types/keys.go`):
//! `"AttestationRequestKey" | nonce (8 bytes BE)` → the attestation as an
//! `Any`; `"LatestAttestationNonce"`, `"EarliestAvailableAttestationNonce"`
//! and `"LatestUnBondingBlockHeight"` → 8-byte big-endian integers;
//! `"EVMAddress" | operator address` → the 20-byte EVM address. The one
//! parameter, `DataCommitmentWindow`, lives in the cosmos-sdk v0.46
//! parameter subspace `qgb`.
//!
//! Reference: celestia-app v3.13.0 `x/blobstream/{abci.go,genesis.go}`,
//! `keeper/*.go`, `types/*.go`.

pub mod abci;
pub mod errors;
pub mod msgs;
pub mod types;
pub mod valset;

pub use abci::{
    end_block, get_current_valset, get_latest_data_commitment, get_latest_valset,
    has_data_commitment,
};
pub use msgs::register_evm_address;
pub use types::{Attestation, DataCommitment, EvmAddress, Valset};

use sdk::{Context, Result};

/// Store key and module name.
pub const STORE_KEY: &str = "qgb";
/// Parameter subspace.
pub const SUBSPACE: &str = "qgb";
/// The `DataCommitmentWindow` parameter key.
pub const PARAM_DATA_COMMITMENT_WINDOW: &str = "DataCommitmentWindow";
/// `types.AttestationRequestKey`.
pub const ATTESTATION_REQUEST_KEY: &[u8] = b"AttestationRequestKey";
/// `types.LatestUnBondingBlockHeight`.
pub const LATEST_UNBONDING_BLOCK_HEIGHT: &[u8] = b"LatestUnBondingBlockHeight";
/// `types.LatestAttestationNonce`.
pub const LATEST_ATTESTATION_NONCE: &[u8] = b"LatestAttestationNonce";
/// `types.EarliestAvailableAttestationNonce`.
pub const EARLIEST_AVAILABLE_ATTESTATION_NONCE: &[u8] = b"EarliestAvailableAttestationNonce";
/// `types.EVMAddress`: prefix of each validator's EVM address.
pub const EVM_ADDRESS: &[u8] = b"EVMAddress";

/// `GetAttestationKey`.
#[must_use]
pub fn attestation_key(nonce: u64) -> Vec<u8> {
    [ATTESTATION_REQUEST_KEY, &nonce.to_be_bytes()].concat()
}

/// `GetEVMKey`: the prefix and the operator address.
#[must_use]
pub fn evm_key(val: &[u8]) -> Vec<u8> {
    [EVM_ADDRESS, val].concat()
}

/// `UInt64FromBytes` (Go's `binary.BigEndian.Uint64` panics on fewer than
/// 8 bytes; extra bytes are ignored).
fn u64_from_bytes(b: &[u8]) -> Result<u64> {
    b.get(..8)
        .and_then(|x| <[u8; 8]>::try_from(x).ok())
        .map(u64::from_be_bytes)
        .ok_or_else(|| sdk::error::sdk::logic("index out of range decoding a uint64"))
}

fn get_u64(ctx: &mut Context<'_>, key: &[u8]) -> Result<Option<u64>> {
    ctx.get(STORE_KEY, key)?
        .map(|b| u64_from_bytes(&b))
        .transpose()
}

fn set_u64(ctx: &mut Context<'_>, key: &[u8], v: u64) -> Result<()> {
    ctx.set(STORE_KEY, key.to_vec(), v.to_be_bytes().to_vec())
}

/// `InitGenesis`: latest attestation nonce 0 and earliest available
/// nonce 1. The params go to subspace `qgb` first (the caller).
pub fn init_genesis(ctx: &mut Context<'_>) -> Result<()> {
    set_u64(ctx, LATEST_ATTESTATION_NONCE, 0)?;
    set_u64(ctx, EARLIEST_AVAILABLE_ATTESTATION_NONCE, 1)
}

/// `GetDataCommitmentWindowParam`.
pub fn data_commitment_window(ctx: &mut Context<'_>) -> Result<u64> {
    x_params::v046::get_u64(ctx, SUBSPACE, PARAM_DATA_COMMITMENT_WINDOW)
}

/// `CheckLatestAttestationNonce` and `GetLatestAttestationNonce`: `None`
/// before initialization.
pub fn latest_attestation_nonce(ctx: &mut Context<'_>) -> Result<Option<u64>> {
    get_u64(ctx, LATEST_ATTESTATION_NONCE)
}

/// `SetLatestAttestationNonce`: once set, the nonce may only grow by one
/// (Go panics otherwise).
pub fn set_latest_attestation_nonce(ctx: &mut Context<'_>, nonce: u64) -> Result<()> {
    if let Some(cur) = latest_attestation_nonce(ctx)?
        && cur.wrapping_add(1) != nonce
    {
        return Err(sdk::error::sdk::logic(
            "not incrementing latest attestation nonce correctly",
        ));
    }
    set_u64(ctx, LATEST_ATTESTATION_NONCE, nonce)
}

/// `CheckEarliestAvailableAttestationNonce` and its getter: `None` before
/// initialization.
pub fn earliest_available_attestation_nonce(ctx: &mut Context<'_>) -> Result<Option<u64>> {
    get_u64(ctx, EARLIEST_AVAILABLE_ATTESTATION_NONCE)
}

/// `SetEarliestAvailableAttestationNonce`.
pub fn set_earliest_available_attestation_nonce(ctx: &mut Context<'_>, nonce: u64) -> Result<()> {
    set_u64(ctx, EARLIEST_AVAILABLE_ATTESTATION_NONCE, nonce)
}

/// `GetAttestationByNonce`: `None` when absent.
pub fn get_attestation(ctx: &mut Context<'_>, nonce: u64) -> Result<Option<Attestation>> {
    ctx.get(STORE_KEY, &attestation_key(nonce))?
        .map(|b| Attestation::decode(&b))
        .transpose()
}

/// `DeleteAttestation` (a no-op when absent).
pub fn delete_attestation(ctx: &mut Context<'_>, nonce: u64) -> Result<()> {
    let key = attestation_key(nonce);
    if ctx.has(STORE_KEY, &key)? {
        ctx.delete(STORE_KEY, &key)?;
    }
    Ok(())
}

/// `SetAttestationRequest`: stores the attestation (never over an existing
/// one), advances the latest nonce and emits `AttestationRequest`.
pub fn set_attestation_request(ctx: &mut Context<'_>, at: &Attestation) -> Result<()> {
    let nonce = at.nonce();
    let key = attestation_key(nonce);
    if ctx.has(STORE_KEY, &key)? {
        return Err(sdk::error::sdk::logic(
            "trying to overwrite existing attestation request",
        ));
    }
    ctx.set(STORE_KEY, key, at.encode())?;
    set_latest_attestation_nonce(ctx, nonce)?;
    ctx.events.emit(sdk::event::event(
        "AttestationRequest",
        [
            ("module", STORE_KEY.to_owned()),
            ("nonce", nonce.to_string()),
        ],
    ));
    Ok(())
}

/// `SetLatestUnBondingBlockHeight`.
pub fn set_latest_unbonding_block_height(ctx: &mut Context<'_>, height: u64) -> Result<()> {
    set_u64(ctx, LATEST_UNBONDING_BLOCK_HEIGHT, height)
}

/// `GetLatestUnBondingBlockHeight` (0 when unset or empty).
pub fn latest_unbonding_block_height(ctx: &mut Context<'_>) -> Result<u64> {
    match ctx.get(STORE_KEY, LATEST_UNBONDING_BLOCK_HEIGHT)? {
        Some(b) if !b.is_empty() => u64_from_bytes(&b),
        _ => Ok(0),
    }
}

/// `GetEVMAddress`: `BytesToAddress` of the stored value.
pub fn get_evm_address(ctx: &mut Context<'_>, val: &[u8]) -> Result<Option<EvmAddress>> {
    let key = evm_key(val);
    if !ctx.has(STORE_KEY, &key)? {
        return Ok(None);
    }
    Ok(ctx
        .get(STORE_KEY, &key)?
        .map(|b| types::bytes_to_address(&b)))
}

/// `SetEVMAddress`.
pub fn set_evm_address(ctx: &mut Context<'_>, val: &[u8], evm: &EvmAddress) -> Result<()> {
    ctx.set(STORE_KEY, evm_key(val), evm.to_vec())
}

/// `IsEVMAddressUnique`: no stored EVM address equals `evm`. Scans every
/// stored address (one per validator ever created at app version 1), as
/// Go does, so the gas charged for the reads matches.
pub fn is_evm_address_unique(ctx: &mut Context<'_>, evm: &EvmAddress) -> Result<bool> {
    let mut it = ctx.prefix_iter(STORE_KEY, EVM_ADDRESS)?;
    while let Some((_, v)) = it.next(ctx)? {
        if v == evm {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The app version of the block header (`ctx.BlockHeader().Version.App`),
/// taken from the consensus params in effect.
fn header_app_version(ctx: &Context<'_>) -> u64 {
    ctx.consensus_params.version.as_ref().map_or(0, |v| v.app)
}

/// Whether the hooks act: only inside the legacy driver's cosmos-sdk
/// v0.46 scope ([`Context::is_sdk_v046`], checked first so that nothing
/// else reads a store or uses gas), at app version 1 (Go: no-op above 1;
/// the v0.46 driver never runs a header below 1), with the `qgb` store
/// mounted (it is only mounted for app version 1).
fn hooks_active(ctx: &mut Context<'_>) -> bool {
    ctx.is_sdk_v046()
        && header_app_version(ctx) == 1
        && ctx.multistore().is_mounted(STORE_KEY)
}

/// The blobstream staking hooks (`keeper.Hooks`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Hooks;

impl x_staking::StakingHooks for Hooks {
    /// `AfterValidatorCreated`: store the default EVM address unless
    /// another validator has it.
    fn after_validator_created(&self, ctx: &mut Context<'_>, val: &[u8]) -> Result<()> {
        if !hooks_active(ctx) {
            return Ok(());
        }
        let evm = types::default_evm_address(val);
        if !is_evm_address_unique(ctx, &evm)? {
            return Err(errors::evm_address_already_exists(&format!(
                "create a validator with a different operator address to {} (pubkey collision)",
                x_staking::addr::val_to_string(val)?
            )));
        }
        set_evm_address(ctx, val, &evm)
    }

    /// `AfterValidatorBeginUnbonding`: record the height so the EndBlocker
    /// emits a new valset.
    fn after_validator_begin_unbonding(
        &self,
        ctx: &mut Context<'_>,
        _cons: &[u8],
        _val: &[u8],
    ) -> Result<()> {
        if !hooks_active(ctx) {
            return Ok(());
        }
        #[allow(clippy::cast_sign_loss)]
        let height = ctx.header.height as u64;
        set_latest_unbonding_block_height(ctx, height)
    }
}

#[cfg(test)]
mod tests;
