//! celestia-app v3.13.0's app version changes (`App.EndBlocker`,
//! `app/app.go`), after the module manager's EndBlock:
//!
//! - App version 1: at the block before the v2 upgrade height
//!   (`--v2-upgrade-height`, with a default per public chain), set app
//!   version 2 in the consensus params (`SetInitialAppVersionInConsensusParams`
//!   then `SetAppVersion`). The params keeper's `DeleteSubspace("blobstream")`
//!   only drops an in-memory subspace: no state changes.
//! - Later versions: the signalled upgrade, once its height is reached.
//!
//! The store and module migrations that the new version brings run in
//! `Commit` in Go, before the commit ([`super::migrate`]).

use sdk::{Context, Result};

use super::consensus_params;

/// The v1 to v2 upgrade height (the first app version 2 block) that
/// celestia-app's `start` command sets by default for a chain ID
/// (`appconsts.*UpgradeHeightV2`); `None` for any other chain, which
/// never upgrades unless the node sets one.
#[must_use]
pub fn default_upgrade_height_v2(chain_id: &str) -> Option<i64> {
    match chain_id {
        "celestia" => Some(2_371_495),
        "mocha-4" => Some(2_585_031),
        "arabica-11" => Some(1_751_707),
        _ => None,
    }
}

/// The app version change at the end of a block executed at `current`:
/// the new version, after its consensus params write, or `None`.
pub fn end_blocker(
    ctx: &mut Context<'_>,
    current: u64,
    upgrade_height_v2: Option<i64>,
) -> Result<Option<u64>> {
    if current == 1 {
        if upgrade_height_v2.is_some_and(|h| ctx.header.height == h - 1) {
            consensus_params::set_initial_app_version(ctx, 2)?;
            consensus_params::set_app_version(ctx, 2)?;
            return Ok(Some(2));
        }
        return Ok(None);
    }
    // `SignalKeeper.ShouldUpgrade`: only an increase applies; a reached
    // upgrade that does not raise the version stays pending.
    let Some(upgrade) = x_signal::should_upgrade(ctx)? else {
        return Ok(None);
    };
    if upgrade.app_version <= current {
        return Ok(None);
    }
    if current == 3 {
        return Err(super::not_ported(sdk::error::sdk::not_supported(
            "app version 3: scheduling the v4 upgrade plan",
        )));
    }
    consensus_params::set_app_version(ctx, upgrade.app_version)?;
    x_signal::reset_tally(ctx)?;
    Ok(Some(upgrade.app_version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::{ExecMode, Header, MultiStore};

    fn ctx_at<'a>(ms: &'a mut MultiStore, chain_id: &str, height: i64) -> Context<'a> {
        let header = Header {
            chain_id: chain_id.to_owned(),
            height,
            ..Header::default()
        };
        Context::new(ms, header, ExecMode::Finalize)
    }

    fn v1_store() -> MultiStore {
        let mut ms = MultiStore::new();
        let v1 = sdk::version::Release::for_app_version(1).unwrap();
        crate::stores::mount_release(&mut ms, v1).unwrap();
        ms
    }

    /// The mainnet EndBlocker of height 2,371,494 sets app version 2;
    /// other heights, and chains with no default height, keep version 1.
    #[test]
    fn v1_upgrades_at_the_block_before_the_v2_height() -> Result<()> {
        let _v046 = sdk::version::SdkV046Scope::enter(1);
        let mainnet = default_upgrade_height_v2("celestia");
        assert_eq!(mainnet, Some(2_371_495));
        assert_eq!(default_upgrade_height_v2("test"), None);
        for (chain, height, h2, want) in [
            ("celestia", 2_371_493, mainnet, None),
            ("celestia", 2_371_495, mainnet, None),
            ("test", 5, None, None),
            ("test", 5, Some(6), Some(2)),
            ("celestia", 2_371_494, mainnet, Some(2)),
        ] {
            let mut ms = v1_store();
            let mut ctx = ctx_at(&mut ms, chain, height);
            assert_eq!(end_blocker(&mut ctx, 1, h2)?, want, "{chain} {height}");
            let stored = x_params::get_raw(&mut ctx, "baseapp", b"VersionParams")?;
            assert_eq!(
                stored,
                want.map(|_| br#"{"app_version":"2"}"#.to_vec()),
                "{chain} {height}"
            );
        }
        Ok(())
    }
}
