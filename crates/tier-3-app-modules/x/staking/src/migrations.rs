//! The v4 to v5 store migrations (`x/staking/migrations/v5` and
//! `keeper/validator_index.go` of celestiaorg cosmos-sdk
//! v1.29.4-sdk-v0.50.14), shared by the upgrade handler (from the start)
//! and the `BeginBlocker` (from a cursor).

use sdk::{Context, Result};

use crate::{STORE_KEY, keys};

fn internal(e: impl std::fmt::Display) -> sdk::SdkError {
    sdk::error::sdk::logic(e.to_string())
}

/// The keys and values under `prefix` from `prefix || start` on.
fn from(
    ctx: &mut Context<'_>,
    prefix: u8,
    start: Option<&[u8]>,
) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let mut out = Vec::new();
    let mut it = ctx.prefix_iter(STORE_KEY, &[prefix])?;
    while let Some((k, v)) = it.next(ctx)? {
        if start.is_none_or(|s| &k[1..] >= s) {
            out.push((k, v));
        }
    }
    Ok(out)
}

/// `ParseDelegationKey`: `0x31 || len || delegator || len || validator`.
fn parse_delegation_key(key: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let bad = || internal(format!("invalid delegation key {key:x?}"));
    let rest = key.get(1..).ok_or_else(bad)?;
    let (&dl, rest) = rest.split_first().ok_or_else(bad)?;
    let del = rest.get(..usize::from(dl)).ok_or_else(bad)?;
    let rest = &rest[usize::from(dl)..];
    let (&vl, rest) = rest.split_first().ok_or_else(bad)?;
    let val = rest.get(..usize::from(vl)).ok_or_else(bad)?;
    Ok((del.to_vec(), val.to_vec()))
}

/// The delegations-by-validator index (an empty value at
/// `GetDelegationsByValKey(validator, delegator)` per delegation) from the
/// delegation `start` (its key without the 0x31 prefix; `None`: the
/// first). After `limit` delegations the last one's key (without prefix)
/// goes to the cursor 0x81.
///
/// From the start this is v5 `migrateDelegationsByValidatorIndex` (limit
/// 1,000), which leaves the cursor alone when it ends first; from a cursor
/// it is `Keeper.MigrateDelegationsByValidatorIndex` (10,000 per block),
/// which deletes the cursor when it ends first.
pub fn index_delegations_by_validator(
    ctx: &mut Context<'_>,
    start: Option<&[u8]>,
    limit: usize,
) -> Result<()> {
    for (i, (key, _)) in from(ctx, keys::DELEGATION, start)?.into_iter().enumerate() {
        let (del, val) = parse_delegation_key(&key)?;
        ctx.set(STORE_KEY, keys::delegation_by_val(&val, &del), Vec::new())?;
        if i + 1 >= limit {
            return ctx.set(
                STORE_KEY,
                keys::NEXT_MIGRATE_DELEGATIONS_BY_VALIDATOR_INDEX.to_vec(),
                key[1..].to_vec(),
            );
        }
    }
    if start.is_some() {
        ctx.delete(STORE_KEY, keys::NEXT_MIGRATE_DELEGATIONS_BY_VALIDATOR_INDEX)?;
    }
    Ok(())
}

/// v5 `MigrateHistoricalInfoKeys(index, limit)`: each `0x50 || decimal
/// height` key from `start` on (in byte order) becomes `0x50 ||
/// big-endian height`; after `limit` the next old key (without prefix)
/// goes to the cursor 0x82, and when the walk ends first the cursor is
/// deleted.
pub fn migrate_historical_info_keys(
    ctx: &mut Context<'_>,
    start: Option<&[u8]>,
    limit: usize,
) -> Result<()> {
    for (i, (key, value)) in from(ctx, keys::HISTORICAL_INFO, start)?
        .into_iter()
        .enumerate()
    {
        let decimal = &key[1..];
        if i >= limit {
            return ctx.set(
                STORE_KEY,
                keys::NEXT_MIGRATE_HISTORICAL_INFO.to_vec(),
                decimal.to_vec(),
            );
        }
        let height: i64 = std::str::from_utf8(decimal)
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| {
                internal(format!(
                    "can't parse height from key {:?} to int64",
                    String::from_utf8_lossy(decimal)
                ))
            })?;
        ctx.set(STORE_KEY, keys::historical_info(height), value)?;
        ctx.delete(STORE_KEY, &key)?;
    }
    ctx.delete(STORE_KEY, keys::NEXT_MIGRATE_HISTORICAL_INFO)
}
