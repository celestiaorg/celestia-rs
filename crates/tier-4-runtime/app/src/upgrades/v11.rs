//! The `v11` handler (the v11 patch release).

use sdk::{Context, Result};
use x_upgrade::VersionMap;

use super::{V11_APP_VERSION, run_migrations};

/// v11 has no new modules and no frozen-account store migration. Corto alone
/// shortens future unbondings, after module migrations (app #8621).
pub(super) fn handler(ctx: &mut Context<'_>, from: VersionMap) -> Result<VersionMap> {
    let versions = run_migrations(ctx, &from, V11_APP_VERSION)?;
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
    use super::super::Handlers;
    use celestia_proto::cosmos::upgrade::v1beta1::Plan;
    use pbjson_types::Duration;
    use sdk::{Context, Result};
    use x_upgrade::{UpgradeHandlers as _, VersionMap};

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
}
