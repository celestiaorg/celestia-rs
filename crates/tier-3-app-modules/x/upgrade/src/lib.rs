//! `x/upgrade` (celestiaorg/cosmos-sdk/x/upgrade v0.2.0).
//!
//! Store layout: `0x00` plan, `0x01 || name` done heights,
//! `0x02 || module` consensus versions (big-endian u64).

use celestia_proto::cosmos::upgrade::v1beta1::Plan;
use prost::Message;
use sdk::{Context, Result};

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

/// Returns the scheduled upgrade plan, if any.
pub fn get_upgrade_plan(ctx: &mut Context<'_>) -> Result<Option<Plan>> {
    ctx.get(STORE_KEY, &[PLAN_BYTE])?
        .map(|b| {
            Plan::decode(b.as_slice()).map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))
        })
        .transpose()
}

/// `PreBlocker`. Executing a scheduled plan is not supported yet; this
/// returns an error if one is due so divergence is loud, never silent.
pub fn pre_block(ctx: &mut Context<'_>) -> Result<()> {
    if let Some(plan) = get_upgrade_plan(ctx)? {
        if plan.height > 0 && ctx.height() >= plan.height {
            return Err(sdk::error::sdk::not_supported(format!(
                "upgrade plan {} at height {} is due but upgrade execution is not implemented",
                plan.name, plan.height
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::{ExecMode, Header, Kind, MultiStore};

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
        pre_block(&mut ctx)?;
        Ok(())
    }
}
