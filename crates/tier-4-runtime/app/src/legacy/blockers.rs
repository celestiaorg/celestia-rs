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
//! code: evidence and the ibc-go v6 upgrade and localhost client paths.

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

/// Not ported: the v0.46 behavior of `what`.
fn not_ported(what: &str) -> sdk::SdkError {
    sdk::error::sdk::not_supported(format!("app version 1: {what}"))
}

/// ibc-go v6 `BeginBlocker` (02-client). With an upgrade plan it writes
/// the upgraded consensus state only when the upgrade store holds an
/// upgraded client for the plan's height and this is the block before it.
/// No v1 to v3 message writes an upgraded client (celestia-app refuses the
/// ibc `UpgradeProposal`, and v1 to v3 route no software upgrade
/// proposal), so the plan alone (the v4 plan the last v3 `EndBlock`
/// schedules) changes nothing. The localhost client has the fixed ID
/// `09-localhost`: `MsgCreateClient.ValidateBasic` refuses a localhost
/// client, created IDs carry a sequence suffix, and mainnet's genesis has
/// no clients, so it cannot exist.
fn ibc_begin_block(ctx: &mut Context<'_>) -> Result<()> {
    if let Ok(Some(plan)) = x_upgrade::get_upgrade_plan(ctx) {
        let key = x_upgrade::upgraded_client_key(plan.height);
        if ctx.get(x_upgrade::STORE_KEY, &key)?.is_some() {
            return Err(not_ported("ibc-go v6 upgraded consensus state"));
        }
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
        "evidence" => super::evidence::begin_block(ctx),
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

/// One v1 module's EndBlock; staking returns the validator updates.
fn end_module(ctx: &mut Context<'_>, module: &str) -> Result<Vec<ValidatorUpdate>> {
    match module {
        "staking" => return x_staking::end_block(ctx),
        "gov" => super::gov::end_block(ctx)?,
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

/// Whether `module` is in the block's module set: blobstream (`qgb`) only
/// at app version 1. The modules app version 2 adds (signal, minfee,
/// interchain accounts, packet forward middleware) have no begin or end
/// blocker.
fn runs_at(ctx: &Context<'_>, module: &str) -> bool {
    let version = ctx.consensus_params.version.as_ref().map_or(1, |v| v.app);
    module != "qgb" || version == 1
}

/// celestia-app v1 `BeginBlocker`: the module manager's BeginBlock.
pub fn begin_block(ctx: &mut Context<'_>, header: &TmHeader) -> Result<()> {
    for &module in V1_BEGIN_BLOCKERS {
        if !runs_at(ctx, module) {
            continue;
        }
        begin_module(ctx, module, header)?;
    }
    Ok(())
}

/// celestia-app v1 `EndBlocker`: the module manager's EndBlock (at most
/// one module may return validator updates). The app version change
/// that follows is the driver's ([`super::upgrade`]).
pub fn end_block(ctx: &mut Context<'_>) -> Result<Vec<ValidatorUpdate>> {
    let mut updates = Vec::new();
    for &module in V1_END_BLOCKERS {
        if !runs_at(ctx, module) {
            continue;
        }
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
    Ok(updates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message as _;
    use sdk::{ExecMode, Header, MultiStore};

    /// The v4 plan alone is a no-op for ibc-go v6's `BeginBlocker`; only
    /// an upgraded client for the plan's height (which v1 to v3 cannot
    /// write) would make it act.
    #[test]
    fn ibc_begin_block_ignores_a_plan_without_an_upgraded_client() -> Result<()> {
        let _v046 = sdk::version::SdkV046Scope::enter(3);
        let mut ms = MultiStore::new();
        let v1 = sdk::version::Release::for_app_version(1).unwrap();
        crate::stores::mount_release(&mut ms, v1)?;
        let mut ctx = Context::new(
            &mut ms,
            Header {
                height: 9,
                ..Header::default()
            },
            ExecMode::Finalize,
        );
        let plan = celestia_proto::cosmos::upgrade::v1beta1::Plan {
            name: "v4".into(),
            height: 10,
            ..Default::default()
        };
        ctx.set(x_upgrade::STORE_KEY, vec![0x00], plan.encode_to_vec())?;
        ibc_begin_block(&mut ctx)?;
        ctx.set(
            x_upgrade::STORE_KEY,
            x_upgrade::upgraded_client_key(10),
            vec![1],
        )?;
        assert!(ibc_begin_block(&mut ctx).is_err());
        Ok(())
    }
}
