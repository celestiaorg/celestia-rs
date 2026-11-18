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
/// last completed plan against this set (mainnet's is `v9`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Handlers {
    /// App version being executed (0: current).
    pub app_version: u64,
}

/// Plan names with a handler at app version 10.
pub const HANDLER_NAMES: &[&str] = &["v10"];
/// Plan names understood by the v11 patch binary.
pub const V11_HANDLER_NAMES: &[&str] = &["v11"];
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
        } else if self.app_version >= V11_APP_VERSION {
            V11_HANDLER_NAMES
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
            "v10" if self.names() == HANDLER_NAMES => v10(ctx, from),
            "v11" if self.names() == V11_HANDLER_NAMES => v11(ctx, from),
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
            Some(2) if *name == "blob" && *to == 3 => x_blob::migrate_params(ctx)?,
            Some(1) if *name == "minfee" && *to == 2 => x_minfee::migrate_params(ctx)?,
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

/// v11 has no new modules and no frozen-account store migration. Corto alone
/// shortens future unbondings, after module migrations (app #8621).
fn v11(ctx: &mut Context<'_>, from: VersionMap) -> Result<VersionMap> {
    let versions = run_migrations(ctx, &from)?;
    if ctx.header.chain_id == "corto-1" {
        let mut params = x_staking::store::get_params(ctx)?;
        params.unbonding_time = pbjson_types::Duration {
            seconds: 600,
            nanos: 0,
        };
        x_staking::store::set_params(ctx, &params)?;
    }
    Ok(versions)
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

    #[test]
    fn v11_handler_migrates_legacy_blob_and_minfee_parameters() -> Result<()> {
        let mut ms = sdk::MultiStore::new();
        ms.mount(x_blob::STORE_KEY, sdk::Kind::Iavl)?;
        ms.mount(x_minfee::STORE_KEY, sdk::Kind::Iavl)?;
        ms.mount(x_params::STORE_KEY, sdk::Kind::Iavl)?;
        ms.mount(x_params::TSTORE_KEY, sdk::Kind::Transient)?;
        let mut ctx = Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
        let mut versions: VersionMap = crate::modules::VERSION_MAP
            .iter()
            .map(|(name, version)| ((*name).to_owned(), *version))
            .collect();
        versions.insert("blob".into(), 2);
        versions.insert("minfee".into(), 1);
        let handler = Handlers::at(11);
        let plan = Plan {
            name: "v11".into(),
            ..Plan::default()
        };
        x_params::set_raw(&mut ctx, "blob", b"GasPerBlobByte", b"12".to_vec())?;
        x_params::set_raw(&mut ctx, "blob", b"GovMaxSquareSize", br#""0""#.to_vec())?;
        x_params::set_raw(
            &mut ctx,
            "minfee",
            b"NetworkMinGasPrice",
            br#""0.000005""#.to_vec(),
        )?;
        let error = handler
            .apply(&mut ctx, &plan, versions.clone())
            .expect_err("zero square size");
        assert_eq!(error.log, "gov max square size cannot be zero");
        assert!(!ctx.has(x_blob::STORE_KEY, x_blob::PARAMS_KEY)?);

        x_params::set_raw(&mut ctx, "blob", b"GovMaxSquareSize", br#""64""#.to_vec())?;
        let migrated = handler.apply(&mut ctx, &plan, versions)?;
        assert_eq!(migrated.get("blob"), Some(&3));
        assert_eq!(migrated.get("minfee"), Some(&2));
        assert!(ctx.has(x_minfee::STORE_KEY, x_minfee::PARAMS_KEY)?);
        x_params::set_raw(
            &mut ctx,
            "minfee",
            b"NetworkMinGasPrice",
            br#""0.01""#.to_vec(),
        )?;
        assert!(ctx.has(x_blob::STORE_KEY, x_blob::PARAMS_KEY)?);
        x_params::set_raw(&mut ctx, "blob", b"GasPerBlobByte", b"8".to_vec())?;
        handler.apply(&mut ctx, &plan, migrated)?;
        assert_eq!(
            x_minfee::get_params(&mut ctx)?
                .network_min_gas_price
                .to_string(),
            "0.000005000000000000"
        );
        let params = x_blob::get_params(&mut ctx)?;
        assert_eq!(
            (params.gas_per_blob_byte, params.gov_max_square_size),
            (12, 64)
        );
        Ok(())
    }

    #[test]
    fn v11_migration_is_corto_only_and_does_not_reset_accounts() -> Result<()> {
        for chain in ["corto-1", "mocha-5", "celestia", "test"] {
            let mut ms = sdk::MultiStore::new();
            ms.mount(x_staking::STORE_KEY, sdk::Kind::Iavl)?;
            let mut ctx = Context::new(
                &mut ms,
                sdk::Header {
                    chain_id: chain.into(),
                    ..Default::default()
                },
                sdk::ExecMode::Finalize,
            );
            let mut p = x_staking::types::Params {
                unbonding_time: Duration {
                    seconds: 337 * 3600,
                    nanos: 0,
                },
                ..Default::default()
            };
            x_staking::store::set_params(&mut ctx, &p)?;
            let versions: VersionMap = crate::modules::VERSION_MAP
                .iter()
                .map(|(n, v)| ((*n).into(), *v))
                .collect();
            let out = Handlers::at(11).apply(
                &mut ctx,
                &Plan {
                    name: "v11".into(),
                    ..Default::default()
                },
                versions.clone(),
            )?;
            assert_eq!(out, versions);
            let after = x_staking::store::get_params(&mut ctx)?;
            p.unbonding_time.seconds = if chain == "corto-1" { 600 } else { 337 * 3600 };
            assert_eq!(after, p);
        }
        assert!(Handlers::at(11).has_handler("v11"));
        assert!(!Handlers::at(11).has_handler("v10"));
        assert!(!Handlers::at(10).has_handler("v11"));
        Ok(())
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
}
