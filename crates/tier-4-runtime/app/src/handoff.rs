//! The hand-off from the cosmos-sdk v0.46 driver to the v0.50 app at the
//! `v4` upgrade (mainnet height 6,680,340), as the celestia-app v4
//! multiplexer does it: the embedded celestia-app v3.13.0 commits the last
//! app version 3 block, whose EndBlocker sets app version 4 and schedules
//! plan `v4` at the next height; celestia-app v4.1.0 then opens the same
//! database. Its store loader (`UpgradeStoreLoader(height, Added: circuit,
//! consensus, hyperlane, warp, minfee; Deleted: crisis, blobstream)`)
//! adds the new stores with that height as their first version, the
//! v0.50 commit info no longer lists the memory store `mem_capability`,
//! and the x/upgrade PreBlocker of the block runs the `v4` handler.

use sdk::Result;
use sdk::version::Release;

use crate::{App, legacy, store_err};

/// The app version whose binary runs on cosmos-sdk v0.50 first.
pub(crate) const V4: u64 = 4;

impl App {
    /// Hands off to the v0.50 app when block `height` is the first v0.50
    /// block ([`Self::v4_handoff_due`]); returns whether it did.
    pub(crate) fn start_v4_handoff(&mut self, height: i64) -> Result<bool> {
        let due = self.v4_handoff_due(height)?;
        if due {
            self.hand_off_to_v050(height)?;
        }
        self.v4_handoff_block = due;
        Ok(due)
    }

    /// Whether block `height` is the first cosmos-sdk v0.50 block: the
    /// committed v0.46 state is at app version 4 with plan `v4` scheduled
    /// at `height` (celestia-app v4.1.0 applies its store upgrades only
    /// when `upgrade-info.json` names plan `v4` at the next height; the v3
    /// EndBlocker writes both). Without the plan the v0.46 state still
    /// stops at app version 4.
    fn v4_handoff_due(&mut self, height: i64) -> Result<bool> {
        if self.legacy.is_none() || self.last_height == 0 || legacy::app_version(&self.ms) != V4 {
            return Ok(false);
        }
        let mut ctx = sdk::Context::new(
            &mut self.ms,
            sdk::Header::default(),
            sdk::ExecMode::Finalize,
        );
        let plan = x_upgrade::get_upgrade_plan(&mut ctx)?;
        Ok(plan.is_some_and(|p| p.name == "v4" && p.height == height))
    }

    /// Switches a v0.46 state at app version 4 to the v0.50 app before
    /// block `height`: the v0.46 commit-memory entries go, every store of
    /// the app version 4 release that is not mounted is added with
    /// `height` as its first version, and the v0.46 driver is dropped.
    /// The `Deleted` stores of the loader (crisis, blobstream) were never
    /// mounted.
    fn hand_off_to_v050(&mut self, height: i64) -> Result<()> {
        let v4 = Release::for_app_version(V4)
            .ok_or_else(|| sdk::error::sdk::logic("no app version 4 release"))?;
        let v3 = Release::for_app_version(V4 - 1)
            .ok_or_else(|| sdk::error::sdk::logic("no app version 3 release"))?;
        for name in v3.commit_memory_stores {
            self.ms.unmount_iavl(name).map_err(store_err)?;
        }
        for name in v4.stores {
            if !self.ms.is_mounted(name) {
                tracing::info!(store = name, height, "adding store (v4 upgrade)");
                self.ms.mount_iavl_at(name, height).map_err(store_err)?;
            }
        }
        self.legacy = None;
        Ok(())
    }
}

/// The consensus app version of a block and the version that executes it,
/// which it sets on the thread ([`sdk::version::set_app_version`]): at the
/// hand-off x/consensus is still empty, and celestia-app v4.1.0 runs.
pub(crate) fn block_versions(
    ctx: &sdk::Context<'_>,
    selected: &mut Option<u64>,
    v4_handoff: bool,
) -> (u64, u64) {
    let consensus = ctx.consensus_params.version.as_ref().map_or(0, |v| v.app);
    let execution = if v4_handoff {
        V4
    } else {
        crate::upgrades::select_execution_version(selected, consensus)
    };
    sdk::version::set_app_version(execution);
    (consensus, execution)
}

/// One module's `PreBlock`. A block at app version 4 runs on celestia-app
/// v4.1.0, which registers the `v4` handler: x/upgrade applies plan `v4`
/// at the hand-off and, at the next blocks, finds a handler for the last
/// completed plan. Every other app version keeps the handlers of the
/// release that executes it ([`crate::modules::pre_block`]).
pub(crate) fn pre_block(
    ctx: &mut sdk::Context<'_>,
    module: &str,
    execution_version: u64,
    upgrade: &mut x_upgrade::NodeState,
) -> Result<bool> {
    if execution_version == V4 && module == "upgrade" {
        return x_upgrade::pre_block(ctx, &crate::upgrades::Handlers::of_release(V4), upgrade);
    }
    crate::modules::pre_block(ctx, module, execution_version, upgrade)
}
