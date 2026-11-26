//! The params migrations of the cosmos-sdk modules: x/params subspace to
//! module store (bank, distribution, slashing, blob), and slashing's
//! missed-block cleanup.

use sdk::{Context, Result};

/// bank 3 to 4: `MigrateSendEnabledParams` (each `SendEnabled` entry of
/// the subspace to its own key), then the params with `default_send_enabled`
/// only (`types.NewParams`).
pub fn migrate_bank(ctx: &mut Context<'_>) -> Result<()> {
    // `x_bank::set_params` moves the deprecated list into send-enabled
    // entries and clears it, as the two Go steps together do.
    let params = super::legacy(ctx, x_bank::get_params)?;
    x_bank::set_params(ctx, params)
}

/// distribution 2 to 3 (`migrations/v3`): the params with the unused
/// proposer rewards reset to zero; 3 to 4 is a no-op (CIP-30 layout bump).
pub fn migrate_distribution(ctx: &mut Context<'_>) -> Result<()> {
    let mut params = super::legacy(ctx, x_distribution::store::get_params)?;
    params.base_proposer_reward = Some(math::LegacyDec::zero());
    params.bonus_proposer_reward = Some(math::LegacyDec::zero());
    x_distribution::store::set_params(ctx, &params)
}

/// slashing 2 to 3 (params out of the subspace) and 3 to 4
/// (`migrations/v4.Migrate(ctx, store, nil, 10)`: see
/// [`x_slashing::remove_legacy_missed_blocks`], which removes nothing and
/// records a cursor after 10 validators).
pub fn migrate_slashing(ctx: &mut Context<'_>) -> Result<()> {
    let params = super::legacy(ctx, x_slashing::get_params)?;
    x_slashing::set_params(ctx, &params)?;
    x_slashing::remove_legacy_missed_blocks(ctx, None)
}

/// blob 2 to 3: `MigrateParams` (subspace `blob` to store).
pub fn migrate_blob(ctx: &mut Context<'_>) -> Result<()> {
    x_blob::migrate_params(ctx)
}
