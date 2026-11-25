//! The part of celestia-app v1's `x/blobstream` (store `qgb`) that
//! genesis needs: InitGenesis (attestation nonces and params) and the
//! staking hook that gives every new validator a default EVM address
//! while the app version is 1, including the validators that genesis
//! transactions create.
//!
//! Reference: celestia-app v3.13.0 `x/blobstream/genesis.go`,
//! `keeper/hooks.go`, `keeper/keeper_valset.go`,
//! `keeper/keeper_attestation.go`.

use sdk::{Context, Result};

/// Store key and module name.
pub const STORE_KEY: &str = "qgb";
/// `types.LatestAttestationNonce`.
const LATEST_ATTESTATION_NONCE: &[u8] = b"LatestAttestationNonce";
/// `types.EarliestAvailableAttestationNonce`.
const EARLIEST_AVAILABLE_ATTESTATION_NONCE: &[u8] = b"EarliestAvailableAttestationNonce";
/// `types.EVMAddress`: prefix of the EVM address of each validator.
const EVM_ADDRESS: &[u8] = b"EVMAddress";

/// `GetEVMKey`: the prefix and the validator operator address.
fn evm_key(val: &[u8]) -> Vec<u8> {
    [EVM_ADDRESS, val].concat()
}

/// `InitGenesis`: latest attestation nonce 0, earliest available nonce 1,
/// then the params (`genesis.params`, a pointer).
pub fn init_genesis(ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    let mut f = super::params::Fields::new("blobstream GenesisState", genesis)?;
    let params = f.take("params").unwrap_or_default();
    f.finish()?;
    ctx.set(
        STORE_KEY,
        LATEST_ATTESTATION_NONCE.to_vec(),
        0u64.to_be_bytes().to_vec(),
    )?;
    ctx.set(
        STORE_KEY,
        EARLIEST_AVAILABLE_ATTESTATION_NONCE.to_vec(),
        1u64.to_be_bytes().to_vec(),
    )?;
    super::params::set_qgb(ctx, &params)
}

/// `IsEVMAddressUnique`: no validator has `evm` yet.
fn is_evm_address_unique(ctx: &mut Context<'_>, evm: &[u8]) -> Result<bool> {
    let mut it = ctx.prefix_iter(STORE_KEY, EVM_ADDRESS)?;
    while let Some((_, v)) = it.next(ctx)? {
        if v == evm {
            return Ok(false);
        }
    }
    Ok(true)
}

/// `DefaultEVMAddress`: go-ethereum `BytesToAddress` of the operator
/// address (its last 20 bytes, left-padded with zeros when shorter).
fn default_evm_address(val: &[u8]) -> [u8; 20] {
    let mut out = [0u8; 20];
    let tail = &val[val.len().saturating_sub(20)..];
    out[20 - tail.len()..].copy_from_slice(tail);
    out
}

/// The blobstream staking hooks (`Hooks`). Only `AfterValidatorCreated`
/// writes state; the unbonding hook belongs with the end blocker.
#[derive(Debug, Clone, Copy, Default)]
pub struct Hooks;

impl x_staking::StakingHooks for Hooks {
    /// `AfterValidatorCreated`: at app version 1, store the default EVM
    /// address unless another validator has it. A no-op where the `qgb`
    /// store is not mounted (every release but v1) or at a later version.
    fn after_validator_created(&self, ctx: &mut Context<'_>, val: &[u8]) -> Result<()> {
        let app = ctx.consensus_params.version.as_ref().map_or(0, |v| v.app);
        if app != 1 || !ctx.multistore().is_mounted(STORE_KEY) {
            return Ok(());
        }
        let evm = default_evm_address(val);
        if !is_evm_address_unique(ctx, &evm)? {
            // `ErrEVMAddressAlreadyExists` (codespace `blobstream`, 37).
            return Err(sdk::SdkError::new(
                "blobstream",
                37,
                format!(
                    "create a validator with a different operator address to {} (pubkey collision): the provided evm address already exists",
                    x_staking::addr::val_to_string(val)?
                ),
            ));
        }
        ctx.set(STORE_KEY, evm_key(val), evm.to_vec())
    }
}
