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

/// The upgrade handlers registered by the binary that executes
/// `app_version` (ADR 0004): celestia-app registers one handler,
/// `v{appconsts.Version}`, so app version 9 (v9.0.8) knows `v9` and app
/// version 10 knows `v10`. `x/upgrade`'s first `PreBlocker` checks the
/// last completed plan against this set (mainnet's is `v9`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Handlers {
    /// App version being executed (0: current).
    pub app_version: u64,
}

/// Plan names with a handler at app version 10.
pub const HANDLER_NAMES: &[&str] = &["v10"];
/// Plan names with a handler at app version 9 (celestia-app v9.0.8).
pub const V9_HANDLER_NAMES: &[&str] = &["v9"];

impl Handlers {
    /// The handlers of the binary executing `app_version`.
    #[must_use]
    pub const fn at(app_version: u64) -> Self {
        Self { app_version }
    }

    const fn names(self) -> &'static [&'static str] {
        if crate::stores::is_v9(self.app_version) {
            V9_HANDLER_NAMES
        } else {
            HANDLER_NAMES
        }
    }
}

impl x_upgrade::UpgradeHandlers for Handlers {
    fn has_handler(&self, name: &str) -> bool {
        self.names().contains(&name)
    }

    fn apply(&self, ctx: &mut Context<'_>, plan: &Plan, from: VersionMap) -> Result<VersionMap> {
        match plan.name.as_str() {
            "v10" if !crate::stores::is_v9(self.app_version) => v10(ctx, from),
            // v9.0.8's own handler (the v8 -> v9 upgrade) only ran on chains
            // that are past it; it is registered for the last-completed-plan
            // check but not ported.
            "v9" => Err(sdk::error::internal(
                "the v9 upgrade handler (v8 -> v9) is not implemented",
            )),
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

#[cfg(test)]
mod tests {
    use super::*;
    use x_upgrade::UpgradeHandlers as _;

    /// celestia-app v9.0.8 `TestUpgrades`: the v9 binary has a `v9`
    /// handler and no `v8` one; v10.4.0's has `v10`. Mainnet's last
    /// completed plan is `v9`, so executing it at app version 9 must
    /// not fail the first `PreBlocker`'s downgrade check (regression:
    /// "wrong app version 9, upgrade handler is missing for v9 upgrade
    /// plan" at the first block after a mainnet snapshot).
    #[test]
    fn handlers_follow_the_executing_app_version() {
        let v9 = Handlers::at(9);
        assert!(v9.has_handler("v9"));
        assert!(!v9.has_handler("v8"));
        assert!(!v9.has_handler("v10"));
        for v10 in [Handlers::at(10), Handlers::at(0)] {
            assert!(v10.has_handler("v10"));
            assert!(!v10.has_handler("v9"));
        }
    }
}
