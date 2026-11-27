//! The `v8` handler (the v6 -> v8 upgrade, celestia-app v8.0.8).

use math::LegacyDec;
use sdk::{Context, Result};
use x_upgrade::VersionMap;

/// `appconsts.MinCommissionRate` (20%, v8.0.8).
fn min_commission_rate() -> LegacyDec {
    LegacyDec::new_with_prec(20, 2).unwrap_or_default()
}

/// The max commission rate the handler raises each validator's to (60%).
fn max_commission_rate() -> LegacyDec {
    LegacyDec::new_with_prec(60, 2).unwrap_or_default()
}

/// The `v8` upgrade handler: `SetMinCommissionRate`,
/// `UpdateValidatorCommissionRates` (a rate below 20% rises to 20% and a
/// max rate below 60% to 60%; `UpdateTime` is the block time), then
/// `RunMigrations`, which adds the forwarding and zkism modules (neither
/// default genesis writes a key).
pub(super) fn handler(ctx: &mut Context<'_>, from: VersionMap) -> Result<VersionMap> {
    let (min, max) = (min_commission_rate(), max_commission_rate());
    let mut params = x_staking::store::get_params(ctx)?;
    params.min_commission_rate = min.clone();
    x_staking::store::set_params(ctx, &params)?;

    for mut v in x_staking::store::get_all_validators(ctx)? {
        if v.commission.rate >= min && v.commission.max_rate >= max {
            continue;
        }
        if v.commission.rate < min {
            v.commission.rate = min.clone();
        }
        if v.commission.max_rate < max {
            v.commission.max_rate = max.clone();
        }
        v.commission.update_time = ctx.header.time;
        x_staking::store::set_validator(ctx, &v)?;
    }

    super::run_migrations_to(
        ctx,
        &from,
        super::release_version_map(8).unwrap_or_default(),
    )
}
