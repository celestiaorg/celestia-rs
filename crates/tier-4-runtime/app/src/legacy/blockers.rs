//! celestia-app v1's BeginBlocker and EndBlocker: each v1 module's begin
//! and end blocker in `SetOrderBeginBlockers` / `SetOrderEndBlockers`
//! order (`app/modules.go`, `app/app.go` at v3.13.0), on cosmos-sdk
//! v0.46.
//!
//! The shared module crates take their v0.46 path where the stored bytes
//! or events differ (`Context::is_sdk_v046`): the v1 inflation schedule,
//! distribution's proposer reward, slashing's per-index missed blocks,
//! staking's full-header `HistoricalInfo` and unbonding without IDs.
//! Paths that no v1 block has reached in the oracle and whose v0.46
//! behavior is not ported return not-supported instead of running v0.50
//! code: evidence, downtime jailing, governance proposal processing, the
//! ibc-go v6 upgrade and localhost client paths, and the v1 to v2
//! upgrade.

use celestia_proto::celestia_core_proto::abci::ValidatorUpdate;
use celestia_proto::celestia_core_proto::tendermint::types::Header as TmHeader;
use sdk::{Context, Result};

/// `SetOrderBeginBlockers` restricted to the modules of app version 1.
pub const V1_BEGIN_BLOCKERS: &[&str] = &[
    "capability",
    "mint",
    "distribution",
    "slashing",
    "evidence",
    "staking",
    "ibc",
    "transfer",
    "feegrant",
    "auth",
    "bank",
    "crisis",
    "gov",
    "genutil",
    "blob",
    "qgb",
    "params",
    "authz",
    "vesting",
];

/// `SetOrderEndBlockers` restricted to the modules of app version 1 (the
/// upgrade module is not one of them).
pub const V1_END_BLOCKERS: &[&str] = &[
    "crisis",
    "gov",
    "staking",
    "capability",
    "mint",
    "distribution",
    "slashing",
    "evidence",
    "ibc",
    "transfer",
    "feegrant",
    "auth",
    "bank",
    "genutil",
    "blob",
    "qgb",
    "params",
    "authz",
    "vesting",
];

/// The last app version 1 height of each chain whose v1 to v2 upgrade
/// height celestia-app knows (`appconsts.*UpgradeHeightV2 - 1`).
const V2_UPGRADE_END_HEIGHTS: &[(&str, i64)] = &[
    ("celestia", 2_371_494),
    ("mocha-4", 2_585_030),
    ("arabica-11", 1_751_706),
];

/// Not ported: the v0.46 behavior of `what`.
fn not_ported(what: &str) -> sdk::SdkError {
    sdk::error::sdk::not_supported(format!("app version 1: {what}"))
}

/// ibc-go v6 `BeginBlocker` (02-client): it only acts with a scheduled
/// upgrade plan or a localhost client, neither of which v1 can create.
fn ibc_begin_block(ctx: &mut Context<'_>) -> Result<()> {
    if let Ok(Some(_)) = x_upgrade::get_upgrade_plan(ctx) {
        return Err(not_ported("ibc-go v6 upgraded consensus state"));
    }
    if x_ibc::client::client_state(ctx, x_ibc::client::LOCALHOST_CLIENT_ID)?.is_some() {
        return Err(not_ported("ibc-go v6 localhost client"));
    }
    Ok(())
}

/// One v1 module's BeginBlock.
fn begin_module(ctx: &mut Context<'_>, module: &str, header: &TmHeader) -> Result<()> {
    match module {
        "capability" => x_capability::begin_block(ctx),
        "mint" => x_mint::begin_block(ctx),
        "distribution" => x_distribution::begin_block(ctx),
        "slashing" => x_slashing::begin_block(ctx),
        "evidence" if ctx.misbehavior.is_empty() => Ok(()),
        "evidence" => Err(not_ported("evidence")),
        "staking" => x_staking::abci::begin_block_v046(ctx, header),
        "ibc" => ibc_begin_block(ctx),
        // cosmos-sdk v0.46 has no limit on the grants pruned per block.
        "authz" => x_authz::dequeue_and_delete_expired_grants(ctx, usize::MAX),
        // No BeginBlock.
        "transfer" | "feegrant" | "auth" | "bank" | "crisis" | "gov" | "genutil" | "blob"
        | "qgb" | "params" | "vesting" => Ok(()),
        _ => Err(sdk::error::sdk::logic(format!(
            "unknown v1 module {module}"
        ))),
    }
}

/// cosmos-sdk v0.46 gov `EndBlocker`: only proposals whose deposit or
/// voting period ended do anything, and their v0.46 processing is not
/// ported.
fn gov_end_block(ctx: &mut Context<'_>) -> Result<()> {
    let now = ctx.header.time;
    for prefix in [x_gov::INACTIVE_QUEUE_PREFIX, x_gov::ACTIVE_QUEUE_PREFIX] {
        if !x_gov::expired_queue_entries(ctx, prefix, &now)?.is_empty() {
            return Err(not_ported("governance proposal processing"));
        }
    }
    Ok(())
}

/// One v1 module's EndBlock; staking returns the validator updates.
fn end_module(ctx: &mut Context<'_>, module: &str) -> Result<Vec<ValidatorUpdate>> {
    match module {
        "staking" => return x_staking::end_block(ctx),
        "gov" => gov_end_block(ctx)?,
        // cosmos-sdk v0.46 has no limit on the allowances pruned per block.
        "feegrant" => x_feegrant::remove_expired_allowances(ctx, usize::MAX)?,
        "qgb" => x_qgb::end_block(ctx)?,
        // No EndBlock; crisis checks invariants only with a non-zero
        // `--inv-check-period` node flag (0, never, by default).
        "crisis" | "capability" | "mint" | "distribution" | "slashing" | "evidence" | "ibc"
        | "transfer" | "auth" | "bank" | "genutil" | "blob" | "params" | "authz" | "vesting" => {}
        _ => {
            return Err(sdk::error::sdk::logic(format!(
                "unknown v1 module {module}"
            )));
        }
    }
    Ok(Vec::new())
}

/// celestia-app v1 `BeginBlocker`: the module manager's BeginBlock.
pub fn begin_block(ctx: &mut Context<'_>, header: &TmHeader) -> Result<()> {
    for &module in V1_BEGIN_BLOCKERS {
        begin_module(ctx, module, header)?;
    }
    Ok(())
}

/// celestia-app v1 `EndBlocker`: the module manager's EndBlock (at most
/// one module may return validator updates), then the height-based v1 to
/// v2 upgrade, which is not ported.
pub fn end_block(ctx: &mut Context<'_>) -> Result<Vec<ValidatorUpdate>> {
    let mut updates = Vec::new();
    for &module in V1_END_BLOCKERS {
        let u = end_module(ctx, module)?;
        if !u.is_empty() {
            if !updates.is_empty() {
                return Err(sdk::error::sdk::logic(
                    "validator EndBlock updates already set by a previous module",
                ));
            }
            updates = u;
        }
    }
    let chain = ctx.header.chain_id.as_str();
    if V2_UPGRADE_END_HEIGHTS.contains(&(chain, ctx.header.height)) {
        return Err(not_ported("the v1 to v2 upgrade"));
    }
    Ok(updates)
}
