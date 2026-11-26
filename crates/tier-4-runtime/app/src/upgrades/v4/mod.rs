//! The `v4` upgrade (celestia-app v4.1.0 `app/upgrades.go`): the first
//! cosmos-sdk v0.50 block after the last cosmos-sdk v0.46 (app version 3)
//! block. Each function ports one step of the Go handler and is checked
//! against the Go oracle (`tests/upgrade_v4_oracle.rs`).
//!
//! Legacy values are read with the modules' own cosmos-sdk v0.46 readers
//! ([`legacy`]), and the migrated values are written with their v0.50
//! writers.

use sdk::version::SdkV046Scope;
use sdk::{Context, Result};

mod auth;
mod gov;
mod handler;
mod ibc;
mod params;
mod staking;
pub(super) mod transfer;

pub use auth::migrate_auth;
pub use gov::migrate_gov;
pub use handler::{VERSION_MAP, handler, run_migrations};
pub use ibc::migrate_ibc;
pub use params::{migrate_bank, migrate_blob, migrate_distribution, migrate_slashing};
pub use staking::migrate_staking;
pub use transfer::migrate_transfer;

/// Runs `f` with the modules' cosmos-sdk v0.46 paths (app version 3), to
/// read a value in its pre-upgrade layout.
fn legacy<T>(ctx: &mut Context<'_>, f: impl FnOnce(&mut Context<'_>) -> Result<T>) -> Result<T> {
    let _v046 = SdkV046Scope::enter(3);
    f(ctx)
}
