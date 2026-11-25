//! The `v9` handler (the v8 -> v9 upgrade, celestia-app v9.0.8).

use sdk::{Context, Result};
use x_upgrade::VersionMap;

/// v9.0.8's own handler only ran on chains that are past it; it is
/// registered for the last-completed-plan check but not ported.
pub(super) fn handler(_ctx: &mut Context<'_>, _from: VersionMap) -> Result<VersionMap> {
    Err(sdk::error::internal(
        "the v9 upgrade handler (v8 -> v9) is not implemented",
    ))
}
