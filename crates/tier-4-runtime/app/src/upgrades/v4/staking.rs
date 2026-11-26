//! staking 3 to 5 (cosmos-sdk `x/staking/migrations/v4` and `v5`, celestiaorg
//! v1.29.4-sdk-v0.50.14).

use celestia_proto::cosmos::staking::v1beta1::{UnbondingDelegation, UnbondingDelegationEntry};
use prost::Message as _;
use sdk::{Context, Result};
use x_staking::keys;

/// `migrateDelegationsByValidatorIndex`'s limit.
const DELEGATION_INDEX_LIMIT: usize = 1000;
/// `MigrateHistoricalInfoKeys(ctx, store, nil, 1000)`.
const HISTORICAL_INFO_LIMIT: usize = 1000;

fn internal(e: impl std::fmt::Display) -> sdk::SdkError {
    sdk::error::internal(e.to_string())
}

/// staking 3 to 4: the params out of the subspace `staking` (0x51), then
/// each unbonding delegation with its entries merged per creation height.
/// 4 to 5: the delegations-by-validator index and the historical info keys
/// in binary, each for at most 1000 entries, with a cursor for the rest.
pub fn migrate_staking(ctx: &mut Context<'_>) -> Result<()> {
    let params = super::legacy(ctx, x_staking::store::get_params)?;
    x_staking::store::set_params(ctx, &params)?;
    merge_unbonding_entries(ctx)?;
    x_staking::migrations::index_delegations_by_validator(ctx, None, DELEGATION_INDEX_LIMIT)?;
    x_staking::migrations::migrate_historical_info_keys(ctx, None, HISTORICAL_INFO_LIMIT)
}

fn prefixed(ctx: &mut Context<'_>, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let mut out = Vec::new();
    let mut it = ctx.prefix_iter(x_staking::STORE_KEY, prefix)?;
    while let Some(kv) = it.next(ctx)? {
        out.push(kv);
    }
    Ok(out)
}

/// `migrateUBDEntries`: entries with the same creation height become one,
/// with the balances summed and the completion time of the last; entries
/// are sorted by creation height and keep only those four fields. Every
/// unbonding delegation is written back at `GetUBDKey(delegator, validator)`.
fn merge_unbonding_entries(ctx: &mut Context<'_>) -> Result<()> {
    for (_, value) in prefixed(ctx, &[keys::UNBONDING_DELEGATION])? {
        let mut ubd = UnbondingDelegation::decode(value.as_slice()).map_err(internal)?;
        let mut by_height: std::collections::BTreeMap<i64, UnbondingDelegationEntry> =
            std::collections::BTreeMap::new();
        for entry in &ubd.entries {
            let merged = by_height.entry(entry.creation_height).or_insert_with(|| {
                UnbondingDelegationEntry {
                    balance: "0".into(),
                    initial_balance: "0".into(),
                    ..UnbondingDelegationEntry::default()
                }
            });
            let sum = |a: &str, b: &str| -> Result<String> {
                Ok(math::Int::parse(a)
                    .map_err(internal)?
                    .checked_add(&math::Int::parse(b).map_err(internal)?)
                    .map_err(internal)?
                    .to_string())
            };
            merged.balance = sum(&merged.balance, &entry.balance)?;
            merged.initial_balance = sum(&merged.initial_balance, &entry.initial_balance)?;
            merged.creation_height = entry.creation_height;
            merged.completion_time.clone_from(&entry.completion_time);
        }
        ubd.entries = by_height.into_values().collect();
        let delegator = sdk::address::from_bech32(&ubd.delegator_address, sdk::address::ACC_PREFIX)
            .map_err(|e| internal(e.log))?;
        let validator =
            sdk::address::from_bech32(&ubd.validator_address, sdk::address::VALOPER_PREFIX)
                .map_err(|e| internal(e.log))?;
        ctx.set(
            x_staking::STORE_KEY,
            keys::ubd(&delegator, &validator),
            ubd.encode_to_vec(),
        )?;
    }
    Ok(())
}
