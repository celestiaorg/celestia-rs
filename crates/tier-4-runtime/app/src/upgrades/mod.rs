//! Upgrade handlers (`app/upgrades.go`) and the app-version modifier.
//!
//! Each celestia-app binary registers the handlers listed for its release in
//! [`sdk::version::Release::upgrade_handlers`]; [`Handlers`] knows those of
//! the release that executes the app version. A due plan without a handler
//! halts the chain in `x/upgrade`'s `PreBlocker`, exactly like Go. Each
//! ported handler lives in its own module, named after the plan.

use celestia_proto::celestia_core_proto::tendermint::types::VersionParams;
use celestia_proto::cosmos::upgrade::v1beta1::Plan;
use sdk::version::Release;
use sdk::{Context, Result};
use x_upgrade::VersionMap;

mod v10;
mod v11;
mod v12;
mod v9;

/// `appconsts.Version`.
pub const APP_VERSION: u64 = 10;
/// Patch-release version supported alongside historical v9/v10 replay.
pub const V11_APP_VERSION: u64 = 11;
/// App version declared by current celestia-app main.
pub const V12_APP_VERSION: u64 = 12;
/// `appconsts.MaxAgeDuration` (337 h).
pub const EVIDENCE_MAX_AGE_SECONDS: i64 = 337 * 3600;
/// `appconsts.MaxAgeNumBlocks`.
pub const EVIDENCE_MAX_AGE_NUM_BLOCKS: i64 = 404_400;

/// The upgrade handlers registered by the binary that executes
/// `app_version` (ADR 0004): celestia-app registers one handler,
/// `v{appconsts.Version}`, so app version 9 (v9.0.8) knows `v9` and app
/// version 10 knows `v10`. `x/upgrade`'s first `PreBlocker` checks the
/// last completed plan against this set (mainnet's is `v9`). App versions
/// 1 to 8 run as 9 ([`Release::runs_as`]), so they know `v9` too.
#[derive(Debug, Clone, Copy, Default)]
pub struct Handlers {
    /// App version being executed (0: current).
    pub app_version: u64,
}

impl Handlers {
    /// The handlers of the binary executing `app_version`.
    #[must_use]
    pub const fn at(app_version: u64) -> Self {
        Self { app_version }
    }

    const fn names(self) -> &'static [&'static str] {
        Release::executing(self.app_version).upgrade_handlers
    }
}

impl x_upgrade::UpgradeHandlers for Handlers {
    fn has_handler(&self, name: &str) -> bool {
        self.names().contains(&name)
    }

    fn apply(&self, ctx: &mut Context<'_>, plan: &Plan, from: VersionMap) -> Result<VersionMap> {
        match plan.name.as_str() {
            "v10" if self.has_handler("v10") => v10::handler(ctx, from),
            "v11" if self.has_handler("v11") => v11::handler(ctx, from),
            "v12" if self.has_handler("v12") => v12::handler(ctx, from),
            "v9" => v9::handler(ctx, from),
            other => Err(sdk::error::internal(format!(
                "ApplyUpgrade should never be called without first checking HasHandler ({other})"
            ))),
        }
    }
}

/// A signal changes consensus params, not the executable already running.
/// Unset version 0 is the current v10 binary; historical v9 replay can still
/// cross into v10 before pinning that release. v11 is selected by the loader.
pub(crate) fn select_execution_version(selected: &mut Option<u64>, consensus: u64) -> u64 {
    let release = selected.unwrap_or(consensus);
    if selected.is_none() && !crate::stores::is_v9(consensus) {
        *selected = Some(if release == 0 { APP_VERSION } else { release });
    }
    release
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

/// `ModuleManager.RunMigrations` (`DefaultMigrationsOrder`: modules by
/// name). Run the migrations required by the target release. Modules added
/// in v10 run `InitGenesis` when missing from the previous version map.
fn run_migrations(ctx: &mut Context<'_>, from: &VersionMap, target: u64) -> Result<VersionMap> {
    let mut out = VersionMap::new();
    for (name, to) in crate::modules::version_map(target) {
        match from.get(name).copied() {
            Some(v) if v == to => {}
            Some(2) if name == "blob" && to == 3 => x_blob::migrate_params(ctx)?,
            Some(1) if name == "minfee" && to == 2 => x_minfee::migrate_params(ctx)?,
            Some(1) if name == "fibre" && to == 2 => x_fibre::migrate_escrows(ctx)?,
            Some(v) => {
                return Err(sdk::error::sdk::not_supported(format!(
                    "migration of module {name} from version {v} to {to}"
                )));
            }
            None => match name {
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
        out.insert(name.to_owned(), to);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use x_upgrade::UpgradeHandlers as _;

    #[test]
    fn signals_do_not_select_a_new_executable_during_replay() {
        let mut selected = None;
        assert_eq!(select_execution_version(&mut selected, 9), 9);
        assert_eq!(select_execution_version(&mut selected, 10), 10);
        assert_eq!(select_execution_version(&mut selected, 11), 10);
        let mut selected = None;
        assert_eq!(select_execution_version(&mut selected, 0), 0);
        assert_eq!(select_execution_version(&mut selected, 11), 10);
        let mut selected = Some(11);
        assert_eq!(select_execution_version(&mut selected, 10), 11);
    }

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

    /// The release table keeps the handler sets of the hand-written table
    /// it replaced: v9 for app versions 1 to 9 (1 to 8 run as 9), v11 and
    /// v12 for theirs, v10 otherwise (0 and unknown versions).
    #[test]
    fn handler_names_are_unchanged_for_every_app_version() {
        for v in 0..=13 {
            let expected: &[&str] = match v {
                1..=9 => &["v9"],
                11 => &["v11"],
                12 => &["v12"],
                _ => &["v10"],
            };
            assert_eq!(Handlers::at(v).names(), expected, "v{v}");
        }
    }
}
