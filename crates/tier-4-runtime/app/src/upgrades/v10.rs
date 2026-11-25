//! The `v10` handler (the v9 -> v10 upgrade).

use celestia_proto::celestia_core_proto::tendermint::types::EvidenceParams;
use pbjson_types::Duration;
use sdk::{Context, Result};
use x_upgrade::VersionMap;

use super::{APP_VERSION, EVIDENCE_MAX_AGE_NUM_BLOCKS, EVIDENCE_MAX_AGE_SECONDS, run_migrations};

/// The `v10` upgrade handler.
pub(super) fn handler(ctx: &mut Context<'_>, from: VersionMap) -> Result<VersionMap> {
    set_evidence_params(ctx)?;
    ensure_fibre_module_account(ctx)?;
    run_migrations(ctx, &from, APP_VERSION)
}

/// `App.SetEvidenceParams`.
fn set_evidence_params(ctx: &mut Context<'_>) -> Result<()> {
    let mut cp = x_consensus::get_params(ctx)?.ok_or_else(|| {
        sdk::error::internal(
            "collections: not found: key 'no_key' of type tendermint.types.ConsensusParams",
        )
    })?;
    let ev = cp.evidence.get_or_insert_with(EvidenceParams::default);
    ev.max_age_duration = Some(Duration {
        seconds: EVIDENCE_MAX_AGE_SECONDS,
        nanos: 0,
    });
    ev.max_age_num_blocks = EVIDENCE_MAX_AGE_NUM_BLOCKS;
    x_consensus::set_params(ctx, &cp)
}

/// `App.ensureFibreModuleAccount`: replaces any account at fibre's address
/// with a fresh fibre module account.
fn ensure_fibre_module_account(ctx: &mut Context<'_>) -> Result<()> {
    let addr = x_auth::module_address("fibre");
    if let Some(acc) = x_auth::get_account(ctx, &addr)? {
        x_auth::remove_account(ctx, &acc)?;
    }
    x_auth::get_module_account(ctx, "fibre")?;
    Ok(())
}
