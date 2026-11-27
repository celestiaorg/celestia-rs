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
pub mod v4;
pub mod v6;
mod v8;
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
/// last completed plan against this set (mainnet's is `v9`). App version
/// 4 runs as itself and knows `v4`; app versions 2, 3 and 5 to 8 run as 9
/// ([`Release::runs_as`]), so they know `v9` too; app version 1 runs on
/// the v0.46 driver, whose binary (celestia-app v3.13.0) registers none.
#[derive(Debug, Clone, Copy, Default)]
pub struct Handlers {
    /// App version being executed (0: current).
    pub app_version: u64,
    /// Whether the handler set is that of `app_version`'s own mainnet
    /// release ([`Release::for_app_version`]) rather than the release that
    /// executes it ([`Release::executing`]).
    pub own_release: bool,
}

impl Handlers {
    /// The handlers of the binary executing `app_version`.
    #[must_use]
    pub const fn at(app_version: u64) -> Self {
        Self {
            app_version,
            own_release: false,
        }
    }

    /// The handlers of the mainnet release of `app_version` (its
    /// [`Release::upgrade_handlers`]), whichever release executes it: app
    /// version 4's binary, celestia-app v4.1.0, knows `v4`.
    #[must_use]
    pub const fn of_release(app_version: u64) -> Self {
        Self {
            app_version,
            own_release: true,
        }
    }

    const fn names(self) -> &'static [&'static str] {
        if self.own_release {
            return match Release::for_app_version(self.app_version) {
                Some(r) => r.upgrade_handlers,
                None => &[],
            };
        }
        Release::executing(self.app_version).upgrade_handlers
    }

    /// Whether [`x_upgrade::UpgradeHandlers::apply`] runs plan `name` here.
    /// A release can register a plan (Go's `HasHandler`, which x/upgrade's
    /// PreBlocker checks) before celestia-rs ports its handler (`v7` and
    /// `v8`).
    #[must_use]
    pub fn is_ported(&self, name: &str) -> bool {
        use x_upgrade::UpgradeHandlers as _;
        match name {
            "v9" => true,
            "v4" | "v5" | "v6" | "v8" | "v10" | "v11" | "v12" => self.has_handler(name),
            _ => false,
        }
    }
}

impl x_upgrade::UpgradeHandlers for Handlers {
    fn has_handler(&self, name: &str) -> bool {
        self.names().contains(&name)
    }

    fn apply(&self, ctx: &mut Context<'_>, plan: &Plan, from: VersionMap) -> Result<VersionMap> {
        match plan.name.as_str() {
            // celestia-app v5.0.12's `v5` handler returns `fromVM`.
            "v5" if self.has_handler("v5") => Ok(from),
            "v6" if self.has_handler("v6") => v6::handler(ctx, from),
            "v8" if self.has_handler("v8") => v8::handler(ctx, from),
            "v4" if self.has_handler("v4") => {
                let versions = v4::handler(ctx, &from)?;
                // celestiaorg x/upgrade v0.1.0 `ApplyUpgrade` (celestia-app
                // v4.1.0): after the handler, the app version is raised by
                // one through the consensus version modifier.
                let current = x_consensus::get_params(ctx)?
                    .and_then(|cp| cp.version)
                    .map_or(0, |v| v.app);
                set_app_version(ctx, current + 1)?;
                Ok(versions)
            }
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

/// The module manager's version map of the release that runs as app
/// version `app_version`, where celestia-rs does not derive it from
/// [`crate::modules::VERSION_MAP`]: celestia-app v4.1.0 (no capability,
/// ICA host or forwarding module), v5.0.12 (v4.1.0's plus capability),
/// v6.4.x (v5.0.12's plus interchain accounts) and v8.0.8 (v6.4.x's plus
/// forwarding and zkism).
#[must_use]
pub fn release_version_map(app_version: u64) -> Option<Vec<(&'static str, u64)>> {
    let mut versions: Vec<(&'static str, u64)> = match app_version {
        4..=6 | 8 => v4::VERSION_MAP.to_vec(),
        _ => return None,
    };
    if app_version >= 5 {
        versions.push(("capability", 1));
    }
    if app_version >= 6 {
        versions.push(("interchainaccounts", 3));
    }
    if app_version >= 8 {
        versions.extend([("forwarding", 1), ("zkism", 1)]);
    }
    versions.sort_unstable();
    Some(versions)
}

/// A signal changes consensus params, not the executable already running.
/// Unset version 0 is the current v10 binary; historical v9 replay can still
/// cross into v10 before pinning that release. v11 is selected by the loader.
pub(crate) fn select_execution_version(selected: &mut Option<u64>, consensus: u64) -> u64 {
    let release = selected.unwrap_or(consensus);
    if selected.is_none() && !Release::executing(consensus).before_v10() {
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
    // The target release's module manager: v9.0.8 has no fibre or
    // valaddr module.
    let versions = crate::modules::version_map(target)
        .filter(|(name, _)| crate::stores::exists_at(name, target))
        .collect();
    run_migrations_to(ctx, from, versions)
}

/// `ModuleManager.RunMigrations` to the module versions `versions` (the
/// target release's module manager): migrates each module `from` lists and
/// runs the default genesis of each module it does not.
fn run_migrations_to(
    ctx: &mut Context<'_>,
    from: &VersionMap,
    versions: Vec<(&'static str, u64)>,
) -> Result<VersionMap> {
    let mut out = VersionMap::new();
    for (name, to) in versions {
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
                // celestia-app's `icaModule` default genesis (v6.4.x).
                "interchainaccounts" => x_icahost::init_host_genesis(
                    ctx,
                    &celestia_proto::ibc::applications::interchain_accounts::genesis::v1::HostGenesisState {
                        port: x_icahost::HOST_PORT_ID.to_owned(),
                        params: Some(v6::ica_host_params()),
                        ..Default::default()
                    },
                )?,
                // celestia-app v8.0.8: x/forwarding has no store and
                // x/zkism's default genesis writes no key.
                "forwarding" | "zkism" => {}
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

    /// A release's own handler set, whatever executes its app version:
    /// celestia-app v4.1.0 registers `v4` (and v5.0.12 still does).
    #[test]
    fn own_release_handlers_follow_the_mainnet_release() {
        assert!(Handlers::of_release(4).has_handler("v4"));
        assert!(!Handlers::of_release(4).has_handler("v9"));
        assert!(Handlers::of_release(5).has_handler("v4"));
        assert!(Handlers::of_release(5).has_handler("v5"));
        assert!(!Handlers::of_release(3).has_handler("v4"));
        assert!(!Handlers::of_release(13).has_handler("v4"));
        // App versions 4 to 6 run as themselves; 7 still runs as 9.
        assert!(Handlers::at(4).has_handler("v4"));
        assert!(Handlers::at(5).has_handler("v5"));
        assert!(Handlers::at(6).has_handler("v6"));
        assert!(!Handlers::at(7).has_handler("v4"));
    }

    /// The release table keeps the handler sets of the hand-written table
    /// it replaced: v9 for app versions 2, 3 and 5 to 9 (they run as 9),
    /// v11 and v12 for theirs, v10 otherwise (0 and unknown versions). App
    /// versions 1 and 4 run as themselves: celestia-app v3.13.0 registers
    /// none, v4.1.0 `v4`.
    /// celestia-app v5.0.12 registers `v4` and `v5`; the `v5` handler
    /// returns the version map it is given (no migrations).
    #[test]
    fn v5_handler_returns_the_version_map() {
        let v5 = Handlers::at(5);
        assert!(v5.has_handler("v5") && v5.has_handler("v4"));
        assert!(v5.is_ported("v5"));
        let mut ms = sdk::MultiStore::new();
        let mut ctx = Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
        let from: VersionMap = [("bank".to_owned(), 4), ("blob".to_owned(), 3)]
            .into_iter()
            .collect();
        let plan = Plan {
            name: "v5".into(),
            height: 7,
            ..Default::default()
        };
        assert_eq!(v5.apply(&mut ctx, &plan, from.clone()).unwrap(), from);
    }

    #[test]
    fn handler_names_are_unchanged_for_every_app_version() {
        for v in 0..=13 {
            let expected: &[&str] = match v {
                1 => &[],
                4 => &["v4"],
                5 => &["v4", "v5"],
                6 => &["v6"],
                8 => &["v8"],
                2..=9 => &["v9"],
                11 => &["v11"],
                12 => &["v12"],
                _ => &["v10"],
            };
            assert_eq!(Handlers::at(v).names(), expected, "v{v}");
        }
    }
}
