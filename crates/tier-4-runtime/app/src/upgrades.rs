//! Upgrade handlers (`app/upgrades.go`) and the app-version modifier.
//!
//! celestia-app v10 registers one handler, `v10`. Handlers for future
//! versions are added to [`Handlers`] as they are ported; a due plan without
//! a handler halts the chain in `x/upgrade`'s `PreBlocker`, exactly like Go.

use celestia_proto::celestia_core_proto::tendermint::types::{EvidenceParams, VersionParams};
use celestia_proto::cosmos::upgrade::v1beta1::Plan;
use pbjson_types::Duration;
use sdk::{Context, Result};
use x_upgrade::VersionMap;

/// `appconsts.Version`.
pub const APP_VERSION: u64 = 10;
/// `appconsts.MaxAgeDuration` (337 h).
pub const EVIDENCE_MAX_AGE_SECONDS: i64 = 337 * 3600;
/// `appconsts.MaxAgeNumBlocks`.
pub const EVIDENCE_MAX_AGE_NUM_BLOCKS: i64 = 404_400;

/// The registered upgrade handlers.
#[derive(Debug, Clone, Copy, Default)]
pub struct Handlers;

/// Plan names with a handler in this binary.
pub const HANDLER_NAMES: &[&str] = &["v10"];

impl x_upgrade::UpgradeHandlers for Handlers {
    fn has_handler(&self, name: &str) -> bool {
        HANDLER_NAMES.contains(&name)
    }

    fn apply(&self, ctx: &mut Context<'_>, plan: &Plan, from: VersionMap) -> Result<VersionMap> {
        match plan.name.as_str() {
            "v10" => v10(ctx, from),
            other => Err(sdk::error::internal(format!(
                "ApplyUpgrade should never be called without first checking HasHandler ({other})"
            ))),
        }
    }
}

/// `BaseApp.SetAppVersion` via the consensus version modifier.
pub fn set_app_version(ctx: &mut Context<'_>, version: u64) -> Result<()> {
    let mut cp = x_consensus::get_params(ctx)?.ok_or_else(|| {
        sdk::error::internal(
            "collections: not found: key 'no_key' of type tendermint.types.ConsensusParams",
        )
    })?;
    cp.version = Some(VersionParams { app: version });
    x_consensus::set_params(ctx, &cp)?;
    Ok(())
}

/// `App.SetEvidenceParams`.
fn set_evidence_params(ctx: &mut Context<'_>) -> Result<()> {
    let mut cp = x_consensus::get_params(ctx)?.ok_or_else(|| {
        sdk::error::internal(
            "collections: not found: key 'no_key' of type tendermint.types.ConsensusParams",
        )
    })?;
    let ev = cp.evidence.get_or_insert_with(EvidenceParams::default);
    ev.max_age_duration = Some(Duration {
        seconds: EVIDENCE_MAX_AGE_SECONDS,
        nanos: 0,
    });
    ev.max_age_num_blocks = EVIDENCE_MAX_AGE_NUM_BLOCKS;
    x_consensus::set_params(ctx, &cp)
}

/// `App.ensureFibreModuleAccount`: replaces any account at fibre's address
/// with a fresh fibre module account.
fn ensure_fibre_module_account(ctx: &mut Context<'_>) -> Result<()> {
    let addr = x_auth::module_address("fibre");
    if let Some(acc) = x_auth::get_account(ctx, &addr)? {
        x_auth::remove_account(ctx, &acc)?;
    }
    x_auth::get_module_account(ctx, "fibre")?;
    Ok(())
}

/// `ModuleManager.RunMigrations` (`DefaultMigrationsOrder`: modules by
/// name). No module changed its consensus version since v9; modules missing
/// from `from` (fibre and valaddr, added by v10) run `InitGenesis` with
/// their default genesis.
fn run_migrations(ctx: &mut Context<'_>, from: &VersionMap) -> Result<VersionMap> {
    let mut out = VersionMap::new();
    for (name, to) in crate::modules::VERSION_MAP {
        match from.get(*name) {
            Some(v) if v == to => {}
            Some(v) => {
                return Err(sdk::error::sdk::not_supported(format!(
                    "migration of module {name} from version {v} to {to}"
                )));
            }
            None => match *name {
                "fibre" => x_fibre::init_genesis(
                    ctx,
                    &celestia_proto::celestia::fibre::v1::GenesisState {
                        params: Some(x_fibre::default_params()),
                        ..Default::default()
                    },
                )?,
                "valaddr" => x_valaddr::init_genesis(
                    ctx,
                    &celestia_proto::celestia::valaddr::v1::GenesisState::default(),
                )?,
                _ => {
                    return Err(sdk::error::sdk::not_supported(format!(
                        "adding module {name} during an upgrade"
                    )));
                }
            },
        }
        out.insert((*name).to_owned(), *to);
    }
    Ok(out)
}

/// The `v10` upgrade handler.
fn v10(ctx: &mut Context<'_>, from: VersionMap) -> Result<VersionMap> {
    set_evidence_params(ctx)?;
    ensure_fibre_module_account(ctx)?;
    run_migrations(ctx, &from)
}
