//! The migrations of an app version change, which the celestia-app fork
//! of cosmos-sdk v0.46 runs in `BaseApp.Commit` before it commits the
//! block (so they are part of that block's app hash):
//!
//! 1. `migrateCommitStore`: stores of the old version that the new one
//!    lacks are deleted, new stores are added with the next version as
//!    their initial IAVL version, their first nodes versioned 1 as iavl
//!    v0.19 does ([`stores`]).
//! 2. `migrateModules` (`Manager.RunMigrations`): each module of both
//!    versions runs its consensus-version migrations (none between app
//!    versions 1, 2 and 3: every module keeps its version), and each new
//!    module runs `InitGenesis` with its module's default genesis, in
//!    `defaultMigrationsOrder` (by name, `auth` last) ([`modules`]).
//!
//! The change from 3 to 4 migrates nothing here: the v4 upgrade handler
//! runs on the v0.50 app.

use sdk::{Context, MultiStore, Result};

/// The stores a change from `from` to `to` deletes and adds
/// (`migrateCommitStore` over `versionedStoreKeys`).
fn store_changes(from: u64, to: u64) -> Result<(Vec<&'static str>, Vec<&'static str>)> {
    let release = |v| {
        sdk::version::Release::for_app_version(v)
            .ok_or_else(|| sdk::error::sdk::logic(format!("unknown app version {v}")))
    };
    let (old, new) = (release(from)?, release(to)?);
    let deleted = old
        .stores
        .iter()
        .copied()
        .filter(|s| !new.stores.contains(s))
        .collect();
    let added = new
        .stores
        .iter()
        .copied()
        .filter(|s| !old.stores.contains(s))
        .collect();
    Ok((deleted, added))
}

/// `migrateCommitStore` and `LoadLatestVersionAndUpgrade`: deletes and
/// adds the stores of the change from `from` to `to` (the block's writes
/// are already flushed). Added stores start at the version being
/// committed.
pub fn stores(ms: &mut MultiStore, from: u64, to: u64) -> Result<()> {
    if to == 4 {
        return Ok(());
    }
    let (deleted, added) = store_changes(from, to)?;
    let store_err = |e: sdk::store::StoreError| sdk::error::sdk::logic(format!("store: {e}"));
    for name in deleted {
        ms.remove_iavl(name).map_err(store_err)?;
    }
    let next = ms.version().saturating_add(1);
    for name in added {
        ms.mount_iavl_at_v019(name, next).map_err(store_err)?;
    }
    Ok(())
}

/// `migrateModules` from `from` to `to`.
pub fn modules(ctx: &mut Context<'_>, from: u64, to: u64) -> Result<()> {
    match (from, to) {
        (1, 2) => v1_to_v2(ctx),
        (2, 3) | (3, 4) => Ok(()),
        _ => Err(super::not_ported(sdk::error::sdk::not_supported(format!(
            "module migrations from app version {from} to {to}"
        )))),
    }
}

/// The new modules of app version 2, in `defaultMigrationsOrder`:
/// `interchainaccounts`, `minfee`, `packetforward`, `signal`. Each runs
/// `InitGenesis` with its module's `DefaultGenesis`.
fn v1_to_v2(ctx: &mut Context<'_>) -> Result<()> {
    // ibc-go v6 interchain accounts: the host genesis (port `icahost`,
    // host enabled, every message allowed); no controller keeper. Host
    // `InitGenesis`: bind and claim the port unless bound, then
    // `SetParams` in the `icahost` subspace.
    let port = x_icahost::HOST_PORT_ID;
    if x_icahost::SCOPED_KEEPER
        .get_capability(ctx, &x_ibc::host::port_path(port))?
        .is_none()
    {
        x_icahost::set_port(ctx, port)?;
        let cap = x_ibc::port::bind_port(ctx, port)?;
        x_icahost::SCOPED_KEEPER.claim_capability(ctx, &cap, &x_ibc::host::port_path(port))?;
    }
    x_params::set_raw(ctx, "icahost", b"HostEnabled", b"true".to_vec())?;
    x_params::set_raw(ctx, "icahost", b"AllowMessages", br#"["*"]"#.to_vec())?;
    // celestia-app x/minfee: `DefaultNetworkMinGasPrice` (0.000001).
    x_params::set_raw(
        ctx,
        x_minfee::MODULE_NAME,
        x_minfee::KEY_NETWORK_MIN_GAS_PRICE,
        br#""0.000001000000000000""#.to_vec(),
    )?;
    // packet-forward-middleware v6: `FeePercentage` 0 in the
    // `packetfowardmiddleware` subspace, no in-flight packets.
    x_params::set_raw(
        ctx,
        x_packetforward::MODULE_NAME,
        b"FeePercentage",
        br#""0.000000000000000000""#.to_vec(),
    )?;
    // x/signal has no genesis state.
    Ok(())
}
