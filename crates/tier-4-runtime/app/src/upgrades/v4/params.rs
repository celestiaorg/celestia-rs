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

/// slashing `ValidatorSigningInfoKeyPrefix`.
const SIGNING_INFO_PREFIX: u8 = 0x01;
/// slashing v0.46 missed-block bit array prefix.
const LEGACY_MISSED_BLOCKS_PREFIX: u8 = 0x02;
/// slashing `NextMigrateValidatorMissedBlocksKey`.
const NEXT_MIGRATE_MISSED_BLOCKS: &[u8] = &[0xFF];
/// `v4.Migrate`'s limit in `Migrate3to4`.
const MISSED_BLOCKS_MIGRATION_LIMIT: usize = 10;

/// slashing 2 to 3 (params out of the subspace) and 3 to 4
/// (`migrations/v4.Migrate(ctx, store, nil, 10)` of celestiaorg cosmos-sdk
/// v1.29.4-sdk-v0.50.14).
///
/// The 3 to 4 step walks the signing infos to delete each validator's
/// v0.46 missed-block entries. Its iterator runs over a store prefixed with
/// 0x01, so each key is `len || address`, but `ValidatorSigningInfoAddress`
/// drops two bytes (`key[2:]`): the address loses its first byte, and the
/// prefix it deletes (`0x02 || len(short) || short`) matches no v0.46 entry,
/// whose length byte is the full address length. After 10 signing infos it
/// records the next one's key (`len || address`) at 0xFF; when the walk ends
/// first, it deletes 0xFF.
pub fn migrate_slashing(ctx: &mut Context<'_>) -> Result<()> {
    let params = super::legacy(ctx, x_slashing::get_params)?;
    x_slashing::set_params(ctx, &params)?;

    let mut keys = Vec::new();
    {
        let mut it = ctx.prefix_iter(x_slashing::STORE_KEY, &[SIGNING_INFO_PREFIX])?;
        while let Some((k, _)) = it.next(ctx)? {
            keys.push(k);
        }
    }
    let mut next = None;
    for (i, key) in keys.iter().enumerate() {
        let stripped = &key[1..];
        if i >= MISSED_BLOCKS_MIGRATION_LIMIT {
            next = Some(stripped.to_vec());
            break;
        }
        if stripped.len() < 3 {
            return Err(sdk::error::internal(format!(
                "key length should be at least 3; got {}",
                stripped.len()
            )));
        }
        let short = &stripped[2..];
        let len =
            u8::try_from(short.len()).map_err(|_| sdk::error::internal("address too long"))?;
        let prefix = [&[LEGACY_MISSED_BLOCKS_PREFIX, len][..], short].concat();
        let mut doomed = Vec::new();
        {
            let mut it = ctx.prefix_iter(x_slashing::STORE_KEY, &prefix)?;
            while let Some((k, _)) = it.next(ctx)? {
                doomed.push(k);
            }
        }
        for k in doomed {
            ctx.delete(x_slashing::STORE_KEY, &k)?;
        }
    }
    match next {
        Some(key) => ctx.set(
            x_slashing::STORE_KEY,
            NEXT_MIGRATE_MISSED_BLOCKS.to_vec(),
            key,
        ),
        None => ctx.delete(x_slashing::STORE_KEY, NEXT_MIGRATE_MISSED_BLOCKS),
    }
}

/// blob 2 to 3: `MigrateParams` (subspace `blob` to store).
pub fn migrate_blob(ctx: &mut Context<'_>) -> Result<()> {
    x_blob::migrate_params(ctx)
}
