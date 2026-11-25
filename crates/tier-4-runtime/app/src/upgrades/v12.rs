//! The `v12` handler (upstream celestia-app v12).

use pbjson_types::Duration;
use sdk::{Context, Result};
use x_upgrade::VersionMap;

use super::{V12_APP_VERSION, run_migrations};

/// Migrations and block-count parameter changes in upstream v12.
pub(super) fn handler(ctx: &mut Context<'_>, from: VersionMap) -> Result<VersionMap> {
    x_blob::ensure_params(ctx)?;
    let versions = run_migrations(ctx, &from, V12_APP_VERSION)?;

    let corto = ctx.header.chain_id == "corto-1";
    let mut consensus = x_consensus::get_params(ctx)?
        .ok_or_else(|| sdk::error::internal("missing consensus parameters"))?;
    let evidence = consensus
        .evidence
        .as_mut()
        .ok_or_else(|| sdk::error::internal("missing evidence parameters"))?;
    evidence.max_age_num_blocks = if corto { 500 } else { 1_011_000 };
    if corto {
        evidence.max_age_duration = Some(Duration {
            seconds: 600,
            nanos: 0,
        });
    }
    x_consensus::set_params(ctx, &consensus)?;

    let mut slashing = x_slashing::get_params(ctx)?;
    slashing.signed_blocks_window = 32_500;
    x_slashing::set_params(ctx, &slashing)?;

    let mut staking = x_staking::store::get_params(ctx)?;
    staking.historical_entries = 32_500;
    if corto {
        staking.unbonding_time = Duration {
            seconds: 600,
            nanos: 0,
        };
    }
    x_staking::store::set_params(ctx, &staking)?;

    let mut fibre = x_fibre::get_params(ctx)?;
    fibre.payment_promise_height_window = 3_250;
    x_fibre::set_params(ctx, &fibre)?;

    let mut connection = x_ibc::connection::get_params(ctx)?;
    connection.max_expected_time_per_block = 4_000_000_000;
    x_ibc::connection::set_params(ctx, &connection)?;
    Ok(versions)
}
