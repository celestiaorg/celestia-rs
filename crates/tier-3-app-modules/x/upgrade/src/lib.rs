//! `x/upgrade` (celestiaorg/cosmos-sdk/x/upgrade v0.2.0).
//!
//! Store layout: `0x00` plan, `0x01 || u64(height) || name` done markers,
//! `0x02 || module` consensus versions (big-endian u64), and the IBC
//! upgrade keys `upgradedIBCState/<h>/upgradedClient|upgradedConsState`.
//!
//! Upgrade handlers are supplied by the app ([`UpgradeHandlers`]); a due plan
//! without a handler fails the block exactly like Go's `PreBlocker` (the node
//! halts).

use std::collections::BTreeMap;

use celestia_proto::cosmos::upgrade::v1beta1::{
    MsgCancelUpgrade, MsgCancelUpgradeResponse, MsgSoftwareUpgrade, MsgSoftwareUpgradeResponse,
    Plan,
};
use prost::Message;
use sdk::address::undefined;
use sdk::error::sdk as sdkerr;
use sdk::{Context, Result, SdkError};

/// Store key.
pub const STORE_KEY: &str = "upgrade";
/// Module name.
pub const MODULE_NAME: &str = "upgrade";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 3;
/// Key of the scheduled plan.
pub const PLAN_BYTE: u8 = 0x00;
/// Prefix of completed upgrades.
pub const DONE_BYTE: u8 = 0x01;
/// Prefix of the module version map.
pub const VERSION_MAP_BYTE: u8 = 0x02;

/// Module name → consensus version.
pub type VersionMap = BTreeMap<String, u64>;

/// `ErrInvalidSigner` (codespace `upgrade`, code 6).
#[must_use]
pub fn err_invalid_signer() -> SdkError {
    SdkError::new(
        MODULE_NAME,
        6,
        "expected authority account as only signer for proposal message",
    )
}

/// Registered upgrade handlers (`Keeper.SetUpgradeHandler`).
pub trait UpgradeHandlers {
    /// `HasHandler(name)`.
    fn has_handler(&self, name: &str) -> bool;
    /// Runs the handler for `plan` and returns the new version map.
    fn apply(&self, ctx: &mut Context<'_>, plan: &Plan, from: VersionMap) -> Result<VersionMap>;
}

/// Node-local upgrade state (`Keeper.downgradeVerified`, skip heights).
#[derive(Debug, Default, Clone)]
pub struct NodeState {
    /// Set after the first `PreBlocker` checked the last applied upgrade.
    pub downgrade_verified: bool,
    /// `--unsafe-skip-upgrades` heights.
    pub skip_heights: Vec<i64>,
}

/// Writes the module version map in sorted name order (`SetModuleVersionMap`).
pub fn set_module_version_map<'n>(
    ctx: &mut Context<'_>,
    versions: impl IntoIterator<Item = (&'n str, u64)>,
) -> Result<()> {
    let mut sorted: Vec<(&str, u64)> = versions.into_iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    for (name, version) in sorted {
        let mut key = vec![VERSION_MAP_BYTE];
        key.extend_from_slice(name.as_bytes());
        ctx.set(STORE_KEY, key, version.to_be_bytes().to_vec())?;
    }
    Ok(())
}

/// `GetModuleVersionMap`.
pub fn get_module_version_map(ctx: &mut Context<'_>) -> Result<VersionMap> {
    let mut it = ctx.prefix_iter(STORE_KEY, &[VERSION_MAP_BYTE])?;
    let mut vm = VersionMap::new();
    while let Some((k, v)) = it.next(ctx)? {
        let name = String::from_utf8_lossy(&k[1..]).into_owned();
        let ver = <[u8; 8]>::try_from(v.as_slice()).map_or(0, u64::from_be_bytes);
        vm.insert(name, ver);
    }
    Ok(vm)
}

/// Returns the scheduled upgrade plan, if any.
pub fn get_upgrade_plan(ctx: &mut Context<'_>) -> Result<Option<Plan>> {
    ctx.get(STORE_KEY, &[PLAN_BYTE])?
        .map(|b| Plan::decode(b.as_slice()).map_err(|e| undefined(e.to_string())))
        .transpose()
}

/// `UpgradedClientKey(height)`.
#[must_use]
pub fn upgraded_client_key(height: i64) -> Vec<u8> {
    format!("upgradedIBCState/{height}/upgradedClient").into_bytes()
}

/// `UpgradedConsStateKey(height)`.
#[must_use]
pub fn upgraded_cons_state_key(height: i64) -> Vec<u8> {
    format!("upgradedIBCState/{height}/upgradedConsState").into_bytes()
}

/// `ClearIBCState`.
pub fn clear_ibc_state(ctx: &mut Context<'_>, last_height: i64) -> Result<()> {
    ctx.delete(STORE_KEY, &upgraded_client_key(last_height))?;
    ctx.delete(STORE_KEY, &upgraded_cons_state_key(last_height))
}

/// `ClearUpgradePlan`.
pub fn clear_upgrade_plan(ctx: &mut Context<'_>) -> Result<()> {
    let Some(old) = get_upgrade_plan(ctx)? else {
        return Ok(());
    };
    clear_ibc_state(ctx, old.height)?;
    ctx.delete(STORE_KEY, &[PLAN_BYTE])
}

fn done_key(name: &str, height: i64) -> Vec<u8> {
    let mut k = vec![DONE_BYTE];
    #[allow(clippy::cast_sign_loss)]
    k.extend_from_slice(&(height as u64).to_be_bytes());
    k.extend_from_slice(name.as_bytes());
    k
}

fn parse_done_key(k: &[u8]) -> Result<(String, i64)> {
    if k.len() < 10 {
        return Err(sdk::error::go_panic("key length should be at least 10"));
    }
    let h = <[u8; 8]>::try_from(&k[1..9]).map_or(0, u64::from_be_bytes);
    #[allow(clippy::cast_possible_wrap)]
    Ok((String::from_utf8_lossy(&k[9..]).into_owned(), h as i64))
}

/// `GetDoneHeight` (0 when the upgrade never completed).
pub fn get_done_height(ctx: &mut Context<'_>, name: &str) -> Result<i64> {
    let mut it = ctx.prefix_iter(STORE_KEY, &[DONE_BYTE])?;
    while let Some((k, _)) = it.next(ctx)? {
        let (n, h) = parse_done_key(&k)?;
        if n == name {
            return Ok(h);
        }
    }
    Ok(0)
}

/// `GetLastCompletedUpgrade`: the done entry with the highest key.
pub fn get_last_completed_upgrade(ctx: &mut Context<'_>) -> Result<(String, i64)> {
    let prefix = [DONE_BYTE];
    let end = sdk::context::prefix_end(&prefix);
    let mut it = ctx.reverse_iterator(STORE_KEY, Some(&prefix), end.as_deref())?;
    match it.next(ctx)? {
        Some((k, _)) => parse_done_key(&k),
        None => Ok((String::new(), 0)),
    }
}

/// `Plan.ValidateBasic`.
pub fn validate_plan(p: &Plan) -> Result<()> {
    #[allow(deprecated)]
    if p.time.as_ref().is_some_and(|t| !sdk::time::is_zero(t)) {
        return Err(sdkerr::invalid_request(
            "time-based upgrades have been deprecated in the SDK",
        ));
    }
    #[allow(deprecated)]
    if p.upgraded_client_state.is_some() {
        return Err(sdkerr::invalid_request(
            "upgrade logic for IBC has been moved to the IBC module",
        ));
    }
    if p.name.is_empty() {
        return Err(sdkerr::invalid_request("name cannot be empty"));
    }
    if p.height <= 0 {
        return Err(sdkerr::invalid_request("height must be greater than 0"));
    }
    Ok(())
}

/// The plan as gogoproto marshals it: the non-nullable `stdtime` `time`
/// field is always present (Go's zero time when unset).
#[must_use]
pub fn canonical_plan(p: &Plan) -> Plan {
    let mut p = p.clone();
    #[allow(deprecated)]
    if p.time.is_none() {
        p.time = Some(sdk::time::go_zero());
    }
    p
}

/// `ScheduleUpgrade`.
pub fn schedule_upgrade(ctx: &mut Context<'_>, plan: &Plan) -> Result<()> {
    validate_plan(plan)?;
    if plan.height < ctx.height() {
        return Err(sdkerr::invalid_request(
            "upgrade cannot be scheduled in the past",
        ));
    }
    if get_done_height(ctx, &plan.name)? != 0 {
        return Err(sdkerr::invalid_request(format!(
            "upgrade with name {} has already been completed",
            plan.name
        )));
    }
    if let Some(old) = get_upgrade_plan(ctx)? {
        clear_ibc_state(ctx, old.height)?;
    }
    ctx.set(
        STORE_KEY,
        vec![PLAN_BYTE],
        canonical_plan(plan).encode_to_vec(),
    )
}

fn set_done(ctx: &mut Context<'_>, name: &str) -> Result<()> {
    let h = ctx.height();
    ctx.set(STORE_KEY, done_key(name, h), vec![1])
}

/// `ApplyUpgrade`.
pub fn apply_upgrade(
    ctx: &mut Context<'_>,
    handlers: &dyn UpgradeHandlers,
    plan: &Plan,
) -> Result<()> {
    let vm = get_module_version_map(ctx)?;
    let updated = handlers.apply(ctx, plan, vm)?;
    set_module_version_map(ctx, updated.iter().map(|(k, v)| (k.as_str(), *v)))?;
    clear_ibc_state(ctx, plan.height)?;
    clear_upgrade_plan(ctx)?;
    set_done(ctx, &plan.name)
}

fn should_execute(p: &Plan, h: i64) -> bool {
    p.height > 0 && p.height <= h
}

/// `PreBlocker`. Returns whether consensus params may have changed. An
/// `Err` halts the chain (Go panics on a `PreBlock` error).
pub fn pre_block(
    ctx: &mut Context<'_>,
    handlers: &dyn UpgradeHandlers,
    node: &mut NodeState,
) -> Result<bool> {
    let h = ctx.height();
    let plan = get_upgrade_plan(ctx)?;
    let skip = node.skip_heights.contains(&h);
    if !node.downgrade_verified {
        node.downgrade_verified = true;
        let due = plan.as_ref().is_some_and(|p| should_execute(p, h));
        if plan.is_none() || !due || skip {
            let (last, _) = get_last_completed_upgrade(ctx)?;
            if !last.is_empty() && !handlers.has_handler(&last) {
                let app = ctx.consensus_params.version.as_ref().map_or(0, |v| v.app);
                return Err(undefined(format!(
                    "wrong app version {app}, upgrade handler is missing for {last} upgrade plan"
                )));
            }
        }
    }
    let Some(plan) = plan else {
        return Ok(false);
    };
    if should_execute(&plan, h) {
        if skip {
            clear_upgrade_plan(ctx)?;
            return Ok(false);
        }
        if !handlers.has_handler(&plan.name) {
            return Err(undefined(format!(
                "UPGRADE \"{}\" NEEDED at height: {}: {}",
                plan.name, plan.height, plan.info
            )));
        }
        apply_upgrade(ctx, handlers, &plan)?;
        return Ok(true);
    }
    if handlers.has_handler(&plan.name) {
        return Err(undefined(format!(
            "BINARY UPDATED BEFORE TRIGGER! UPGRADE \"{}\" - in binary but not executed on chain. Downgrade your binary",
            plan.name
        )));
    }
    Ok(false)
}

/// The gov module account (the upgrade authority).
fn authority() -> Result<String> {
    Ok(sdk::address::to_bech32(
        sdk::address::ACC_PREFIX,
        &sdk::address::module_address("gov"),
    ))
}

/// `MsgSoftwareUpgrade`.
pub fn handle_software_upgrade(
    ctx: &mut Context<'_>,
    msg: &MsgSoftwareUpgrade,
) -> Result<MsgSoftwareUpgradeResponse> {
    let auth = authority()?;
    if auth != msg.authority {
        return Err(err_invalid_signer().wrap(format!("expected {auth} got {}", msg.authority)));
    }
    let plan = canonical_plan(&msg.plan.clone().unwrap_or_default());
    schedule_upgrade(ctx, &plan)?;
    Ok(MsgSoftwareUpgradeResponse {})
}

/// `MsgCancelUpgrade`.
pub fn handle_cancel_upgrade(
    ctx: &mut Context<'_>,
    msg: &MsgCancelUpgrade,
) -> Result<MsgCancelUpgradeResponse> {
    let auth = authority()?;
    if auth != msg.authority {
        return Err(err_invalid_signer().wrap(format!("expected {auth} got {}", msg.authority)));
    }
    clear_upgrade_plan(ctx)?;
    Ok(MsgCancelUpgradeResponse {})
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::{ExecMode, Header, Kind, MultiStore};

    struct Handlers(Vec<&'static str>);

    impl UpgradeHandlers for Handlers {
        fn has_handler(&self, name: &str) -> bool {
            self.0.contains(&name)
        }
        fn apply(&self, _: &mut Context<'_>, _: &Plan, from: VersionMap) -> Result<VersionMap> {
            Ok(from)
        }
    }

    #[test]
    fn version_map_keys() -> Result<()> {
        let mut ms = MultiStore::new();
        ms.mount(STORE_KEY, Kind::Iavl)?;
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        set_module_version_map(&mut ctx, [("bank", 4), ("auth", 5)])?;
        assert_eq!(
            ctx.get(STORE_KEY, b"\x02auth")?,
            Some(5u64.to_be_bytes().to_vec())
        );
        assert_eq!(get_upgrade_plan(&mut ctx)?, None);
        assert_eq!(get_module_version_map(&mut ctx)?.get("bank"), Some(&4));
        Ok(())
    }

    #[test]
    fn schedule_apply_and_halt() -> Result<()> {
        let mut ms = MultiStore::new();
        ms.mount(STORE_KEY, Kind::Iavl)?;
        let header = Header {
            height: 5,
            ..Header::default()
        };
        let mut ctx = Context::new(&mut ms, header, ExecMode::Finalize);
        let mut node = NodeState::default();
        let h = Handlers(vec!["v10"]);
        let plan = Plan {
            name: "v10".into(),
            height: 6,
            ..Plan::default()
        };
        assert!(
            schedule_upgrade(
                &mut ctx,
                &Plan {
                    height: 4,
                    ..plan.clone()
                }
            )
            .is_err()
        );
        schedule_upgrade(&mut ctx, &plan)?;
        // Handler present before the plan is due: BINARY UPDATED BEFORE TRIGGER.
        assert!(pre_block(&mut ctx, &h, &mut node).is_err());
        ctx.header.height = 6;
        assert!(pre_block(&mut ctx, &h, &mut node)?);
        assert_eq!(get_upgrade_plan(&mut ctx)?, None);
        assert_eq!(get_done_height(&mut ctx, "v10")?, 6);
        assert_eq!(get_last_completed_upgrade(&mut ctx)?, ("v10".into(), 6));
        // Already completed.
        assert!(
            schedule_upgrade(
                &mut ctx,
                &Plan {
                    height: 9,
                    ..plan.clone()
                }
            )
            .is_err()
        );
        // A plan without a handler halts once due.
        schedule_upgrade(
            &mut ctx,
            &Plan {
                name: "v11".into(),
                height: 7,
                ..Plan::default()
            },
        )?;
        assert!(!pre_block(&mut ctx, &h, &mut node)?);
        ctx.header.height = 7;
        let e = pre_block(&mut ctx, &h, &mut node).unwrap_err();
        assert!(
            e.log.starts_with("UPGRADE \"v11\" NEEDED at height: 7"),
            "{}",
            e.log
        );
        // A restarted node whose last applied upgrade has no handler.
        let mut fresh = NodeState::default();
        clear_upgrade_plan(&mut ctx)?;
        assert!(pre_block(&mut ctx, &Handlers(vec![]), &mut fresh).is_err());
        Ok(())
    }
}
