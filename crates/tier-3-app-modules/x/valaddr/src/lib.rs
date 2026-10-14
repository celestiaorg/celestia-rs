//! celestia-app v10 `x/valaddr`.
//!
//! Store layout: `0x01 || consensus address` → `FibreProviderInfo`.
//! EndBlocker removes entries of validators that no longer exist or that are
//! jailed, unbonded and past `unbonding_time + 7 days`. The staking lookup is
//! the [`StakingKeeper`] trait (implemented in `app`). The Msg server
//! (`MsgSetFibreProviderInfo`) is not ported yet.

use celestia_proto::celestia::valaddr::v1::{FibreProviderInfo, GenesisState};
use pbjson_types::Timestamp;
use prost::Message;
use sdk::address::{cons_from_bech32, undefined};
use sdk::error::sdk as sdkerr;
use sdk::{Context, Result};

/// Store key.
pub const STORE_KEY: &str = "valaddr";
/// Module name.
pub const MODULE_NAME: &str = "valaddr";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 1;
/// `FibreProviderInfoPrefix`.
pub const FIBRE_PROVIDER_INFO_PREFIX: u8 = 0x01;
/// `JailedGracePeriod` (7 days) in nanoseconds.
pub const JAILED_GRACE_PERIOD_NANOS: i128 = 7 * 24 * 3600 * 1_000_000_000;

/// The validator fields `RemoveStaleFibreProviders` reads.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatorInfo {
    /// `IsJailed()`.
    pub jailed: bool,
    /// `IsBonded()`.
    pub bonded: bool,
    /// `UnbondingTime`.
    pub unbonding_time: Timestamp,
}

/// Expected staking keeper.
pub trait StakingKeeper {
    /// `GetValidatorByConsAddr`; `Ok(None)` is `ErrNoValidatorFound`.
    fn validator_by_cons_addr(
        &mut self,
        ctx: &mut Context<'_>,
        cons_addr: &[u8],
    ) -> Result<Option<ValidatorInfo>>;
}

fn key(cons_addr: &[u8]) -> Vec<u8> {
    let mut k = vec![FIBRE_PROVIDER_INFO_PREFIX];
    k.extend_from_slice(cons_addr);
    k
}

/// `SetFibreProviderInfo`.
pub fn set_fibre_provider_info(
    ctx: &mut Context<'_>,
    cons_addr: &[u8],
    info: &FibreProviderInfo,
) -> Result<()> {
    ctx.set(STORE_KEY, key(cons_addr), info.encode_to_vec())
}

/// `GetFibreProviderInfo` (decode failures read as absent, as in Go).
pub fn get_fibre_provider_info(
    ctx: &mut Context<'_>,
    cons_addr: &[u8],
) -> Result<Option<FibreProviderInfo>> {
    Ok(ctx
        .get(STORE_KEY, &key(cons_addr))?
        .and_then(|b| FibreProviderInfo::decode(b.as_slice()).ok()))
}

/// `DeleteFibreProviderInfo`.
pub fn delete_fibre_provider_info(ctx: &mut Context<'_>, cons_addr: &[u8]) -> Result<()> {
    ctx.delete(STORE_KEY, &key(cons_addr))
}

/// `ValidateGenesis` + `InitGenesis`.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &GenesisState) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    let mut entries = Vec::with_capacity(gs.fibre_providers.len());
    for p in &gs.fibre_providers {
        let addr = cons_from_bech32(&p.validator_consensus_address).map_err(|e| {
            undefined(format!(
                "invalid validator consensus address {:?}: {}",
                p.validator_consensus_address, e.log
            ))
        })?;
        if !seen.insert(addr.clone()) {
            return Err(undefined(format!(
                "duplicate validator consensus address {:?}",
                p.validator_consensus_address
            )));
        }
        entries.push((addr, p.info.clone().unwrap_or_default()));
    }
    for (addr, info) in entries {
        set_fibre_provider_info(ctx, &addr, &info)?;
    }
    Ok(())
}

/// `InitGenesis` from the module's genesis JSON.
pub fn init_genesis_json(ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    let gs: GenesisState = serde_json::from_value(genesis.clone())
        .map_err(|e| sdkerr::json_unmarshal(e.to_string()))?;
    init_genesis(ctx, &gs)
}

/// `EndBlock` (`RemoveStaleFibreProviders`).
pub fn end_block(ctx: &mut Context<'_>, staking: &mut dyn StakingKeeper) -> Result<()> {
    let now = sdk::time::to_nanos(&ctx.header.time);
    let mut stale = Vec::new();
    let prefix = [FIBRE_PROVIDER_INFO_PREFIX];
    let end = sdk::context::prefix_end(&prefix);
    let mut it = ctx.iterator(STORE_KEY, Some(&prefix), end.as_deref())?;
    while let Some((k, v)) = it.next(ctx)? {
        FibreProviderInfo::decode(v.as_slice()).map_err(|e| undefined(e.to_string()))?;
        let addr = k[1..].to_vec();
        match staking.validator_by_cons_addr(ctx, &addr)? {
            None => stale.push(addr),
            Some(v) => {
                let grace_end = sdk::time::to_nanos(&v.unbonding_time) + JAILED_GRACE_PERIOD_NANOS;
                if v.jailed && !v.bonded && now > grace_end {
                    stale.push(addr);
                }
            }
        }
    }
    for addr in stale {
        delete_fibre_provider_info(ctx, &addr)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use celestia_proto::celestia::valaddr::v1::FibreProvider;
    use sdk::{ExecMode, Header, Kind, MultiStore};

    struct Staking(Vec<(Vec<u8>, ValidatorInfo)>);

    impl StakingKeeper for Staking {
        fn validator_by_cons_addr(
            &mut self,
            _ctx: &mut Context<'_>,
            cons_addr: &[u8],
        ) -> Result<Option<ValidatorInfo>> {
            Ok(self
                .0
                .iter()
                .find(|(a, _)| a == cons_addr)
                .map(|(_, v)| v.clone()))
        }
    }

    #[test]
    fn genesis_and_gc() -> Result<()> {
        let mut ms = MultiStore::new();
        ms.mount(STORE_KEY, Kind::Iavl)?;
        let header = Header {
            time: Timestamp {
                seconds: 30 * 86_400,
                nanos: 0,
            },
            ..Header::default()
        };
        let mut ctx = Context::new(&mut ms, header, ExecMode::Finalize);
        let addrs: Vec<Vec<u8>> = (1u8..=3).map(|i| vec![i; 20]).collect();
        let gs = GenesisState {
            fibre_providers: addrs
                .iter()
                .map(|a| FibreProvider {
                    validator_consensus_address: sdk::address::to_bech32("celestiavalcons", a),
                    info: Some(FibreProviderInfo { host: "h:1".into() }),
                })
                .collect(),
        };
        init_genesis(&mut ctx, &gs)?;
        let mut dup = gs.clone();
        dup.fibre_providers.push(gs.fibre_providers[0].clone());
        assert!(init_genesis(&mut ctx, &dup).is_err());

        let jailed_long_ago = ValidatorInfo {
            jailed: true,
            bonded: false,
            unbonding_time: Timestamp::default(),
        };
        let active = ValidatorInfo {
            jailed: false,
            bonded: true,
            unbonding_time: Timestamp::default(),
        };
        // addrs[0] unknown, addrs[1] jailed past grace, addrs[2] active.
        let mut staking = Staking(vec![
            (addrs[1].clone(), jailed_long_ago),
            (addrs[2].clone(), active),
        ]);
        end_block(&mut ctx, &mut staking)?;
        assert_eq!(get_fibre_provider_info(&mut ctx, &addrs[0])?, None);
        assert_eq!(get_fibre_provider_info(&mut ctx, &addrs[1])?, None);
        assert!(get_fibre_provider_info(&mut ctx, &addrs[2])?.is_some());
        Ok(())
    }
}
