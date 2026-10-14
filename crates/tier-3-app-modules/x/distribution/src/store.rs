//! Store layout and accessors (`types/keys.go`, `keeper/store.go`,
//! collections `Params`/`FeePool`/`UserOutstandingRewards`).

use celestia_proto::cosmos::distribution::v1beta1 as pb;
use math::LegacyDec;
use prost::Message;
use sdk::coins::Coins;
use sdk::dec_coins::DecCoins;
use sdk::error::sdk as errs;
use sdk::{Context, Result};

use crate::STORE_KEY;

/// Fee pool (`collections.Item`, prefix 0).
pub const FEE_POOL: &[u8] = &[0x00];
/// Previous proposer.
pub const PROPOSER: &[u8] = &[0x01];
/// Validator outstanding rewards.
pub const OUTSTANDING_REWARDS: u8 = 0x02;
/// Delegator withdraw address.
pub const WITHDRAW_ADDR: u8 = 0x03;
/// Delegator starting info.
pub const STARTING_INFO: u8 = 0x04;
/// Validator historical rewards.
pub const HISTORICAL_REWARDS: u8 = 0x05;
/// Validator current rewards.
pub const CURRENT_REWARDS: u8 = 0x06;
/// Validator accumulated commission.
pub const ACCUMULATED_COMMISSION: u8 = 0x07;
/// Validator slash events.
pub const SLASH_EVENT: u8 = 0x08;
/// Params (`collections.Item`, prefix 9).
pub const PARAMS: &[u8] = &[0x09];
/// User outstanding rewards (`collections.Map`, prefix 10).
pub const USER_OUTSTANDING_REWARDS: u8 = 0x0a;

use sdk::codec::length_prefixed as lp;

fn k1(prefix: u8, a: &[u8]) -> Vec<u8> {
    let mut k = vec![prefix];
    k.extend_from_slice(&lp(a));
    k
}

/// `GetValidatorOutstandingRewardsKey`.
#[must_use]
pub fn outstanding_key(val: &[u8]) -> Vec<u8> {
    k1(OUTSTANDING_REWARDS, val)
}

/// `GetDelegatorWithdrawAddrKey`.
#[must_use]
pub fn withdraw_addr_key(del: &[u8]) -> Vec<u8> {
    k1(WITHDRAW_ADDR, del)
}

/// `GetDelegatorStartingInfoKey`.
#[must_use]
pub fn starting_info_key(val: &[u8], del: &[u8]) -> Vec<u8> {
    let mut k = k1(STARTING_INFO, val);
    k.extend_from_slice(&lp(del));
    k
}

/// `GetValidatorHistoricalRewardsPrefix`.
#[must_use]
pub fn historical_prefix(val: &[u8]) -> Vec<u8> {
    k1(HISTORICAL_REWARDS, val)
}

/// `GetValidatorHistoricalRewardsKey` (period little-endian).
#[must_use]
pub fn historical_key(val: &[u8], period: u64) -> Vec<u8> {
    let mut k = historical_prefix(val);
    k.extend_from_slice(&period.to_le_bytes());
    k
}

/// `GetValidatorCurrentRewardsKey`.
#[must_use]
pub fn current_key(val: &[u8]) -> Vec<u8> {
    k1(CURRENT_REWARDS, val)
}

/// `GetValidatorAccumulatedCommissionKey`.
#[must_use]
pub fn commission_key(val: &[u8]) -> Vec<u8> {
    k1(ACCUMULATED_COMMISSION, val)
}

/// `GetValidatorSlashEventPrefix`.
#[must_use]
pub fn slash_event_prefix(val: &[u8]) -> Vec<u8> {
    k1(SLASH_EVENT, val)
}

/// `GetValidatorSlashEventKeyPrefix`.
#[must_use]
pub fn slash_event_height_prefix(val: &[u8], height: u64) -> Vec<u8> {
    let mut k = slash_event_prefix(val);
    k.extend_from_slice(&height.to_be_bytes());
    k
}

/// `GetValidatorSlashEventKey`.
#[must_use]
pub fn slash_event_key(val: &[u8], height: u64, period: u64) -> Vec<u8> {
    let mut k = slash_event_height_prefix(val, height);
    k.extend_from_slice(&period.to_be_bytes());
    k
}

/// `UserOutstandingRewards` key: `Pair[AccAddress, ValAddress]`.
#[must_use]
pub fn user_outstanding_key(del: &[u8], val: &[u8]) -> Vec<u8> {
    let mut k = k1(USER_OUTSTANDING_REWARDS, del);
    k.extend_from_slice(val);
    k
}

fn decode<M: Message + Default>(b: &[u8]) -> Result<M> {
    M::decode(b).map_err(|e| errs::json_unmarshal(e.to_string()))
}

fn dec_str(d: &LegacyDec) -> String {
    String::from_utf8_lossy(&d.marshal()).into_owned()
}

fn wire_dec(s: &str) -> Result<LegacyDec> {
    LegacyDec::unmarshal(s.as_bytes()).map_err(|e| sdk::error::go_panic(e.to_string()))
}

/// Distribution params.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Params {
    /// Community tax.
    pub community_tax: LegacyDec,
    /// Deprecated base proposer reward (`None` when nil).
    pub base_proposer_reward: Option<LegacyDec>,
    /// Deprecated bonus proposer reward (`None` when nil).
    pub bonus_proposer_reward: Option<LegacyDec>,
    /// Whether withdraw addresses may be set.
    pub withdraw_addr_enabled: bool,
}

#[allow(deprecated)]
impl Params {
    /// From a stored/wire proto.
    pub fn from_wire(p: &pb::Params) -> Result<Self> {
        let opt = |s: &str| -> Result<Option<LegacyDec>> {
            if s.is_empty() {
                Ok(None)
            } else {
                wire_dec(s).map(Some)
            }
        };
        Ok(Self {
            community_tax: wire_dec(&p.community_tax)?,
            base_proposer_reward: opt(&p.base_proposer_reward)?,
            bonus_proposer_reward: opt(&p.bonus_proposer_reward)?,
            withdraw_addr_enabled: p.withdraw_addr_enabled,
        })
    }

    /// From proto3 JSON.
    pub fn from_json(p: &pb::Params) -> Result<Self> {
        let parse =
            |s: &str| LegacyDec::from_str_go(s).map_err(|e| errs::json_unmarshal(e.to_string()));
        Ok(Self {
            community_tax: parse(&p.community_tax)?,
            base_proposer_reward: Some(parse(&p.base_proposer_reward)?),
            bonus_proposer_reward: Some(parse(&p.bonus_proposer_reward)?),
            withdraw_addr_enabled: p.withdraw_addr_enabled,
        })
    }

    /// Store encoding.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        pb::Params {
            community_tax: dec_str(&self.community_tax),
            base_proposer_reward: self
                .base_proposer_reward
                .as_ref()
                .map(dec_str)
                .unwrap_or_default(),
            bonus_proposer_reward: self
                .bonus_proposer_reward
                .as_ref()
                .map(dec_str)
                .unwrap_or_default(),
            withdraw_addr_enabled: self.withdraw_addr_enabled,
        }
        .encode_to_vec()
    }
}

/// `Params.Get` (collections: missing is an error).
pub fn get_params(ctx: &mut Context<'_>) -> Result<Params> {
    let b = ctx
        .get(STORE_KEY, PARAMS)?
        .ok_or_else(|| sdk::error::internal("collections: not found: params"))?;
    Params::from_wire(&decode(&b)?)
}

/// `Params.Set`.
pub fn set_params(ctx: &mut Context<'_>, p: &Params) -> Result<()> {
    ctx.set(STORE_KEY, PARAMS.to_vec(), p.encode())
}

/// `FeePool.Get`.
pub fn get_fee_pool(ctx: &mut Context<'_>) -> Result<DecCoins> {
    let b = ctx
        .get(STORE_KEY, FEE_POOL)?
        .ok_or_else(|| sdk::error::internal("collections: not found: fee_pool"))?;
    DecCoins::from_proto(&decode::<pb::FeePool>(&b)?.community_pool)
}

/// `FeePool.Set`.
pub fn set_fee_pool(ctx: &mut Context<'_>, community_pool: &DecCoins) -> Result<()> {
    let v = pb::FeePool {
        community_pool: community_pool.to_proto(),
    }
    .encode_to_vec();
    ctx.set(STORE_KEY, FEE_POOL.to_vec(), v)
}

/// `GetDelegatorWithdrawAddr`.
pub fn get_withdraw_addr(ctx: &mut Context<'_>, del: &[u8]) -> Result<Vec<u8>> {
    Ok(ctx
        .get(STORE_KEY, &withdraw_addr_key(del))?
        .unwrap_or_else(|| del.to_vec()))
}

/// `SetDelegatorWithdrawAddr`.
pub fn set_withdraw_addr(ctx: &mut Context<'_>, del: &[u8], to: &[u8]) -> Result<()> {
    ctx.set(STORE_KEY, withdraw_addr_key(del), to.to_vec())
}

/// `SetPreviousProposerConsAddr` (`gogotypes.BytesValue`).
pub fn set_previous_proposer(ctx: &mut Context<'_>, cons: &[u8]) -> Result<()> {
    let mut v = Vec::new();
    if !cons.is_empty() {
        prost::encoding::bytes::encode(1, &cons.to_vec(), &mut v);
    }
    ctx.set(STORE_KEY, PROPOSER.to_vec(), v)
}

/// `DelegatorStartingInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartingInfo {
    /// Period before the delegation started.
    pub previous_period: u64,
    /// Stake at the start.
    pub stake: LegacyDec,
    /// Creation height.
    pub height: u64,
}

impl StartingInfo {
    fn encode(&self) -> Vec<u8> {
        pb::DelegatorStartingInfo {
            previous_period: self.previous_period,
            stake: dec_str(&self.stake),
            height: self.height,
        }
        .encode_to_vec()
    }

    fn from_proto(p: &pb::DelegatorStartingInfo, json: bool) -> Result<Self> {
        Ok(Self {
            previous_period: p.previous_period,
            stake: if json {
                LegacyDec::from_str_go(&p.stake).map_err(|e| errs::json_unmarshal(e.to_string()))?
            } else {
                wire_dec(&p.stake)?
            },
            height: p.height,
        })
    }
}

/// `GetDelegatorStartingInfo` (zero value when missing, like Go).
pub fn get_starting_info(ctx: &mut Context<'_>, val: &[u8], del: &[u8]) -> Result<StartingInfo> {
    let b = ctx
        .get(STORE_KEY, &starting_info_key(val, del))?
        .unwrap_or_default();
    StartingInfo::from_proto(&decode(&b)?, false)
}

/// `SetDelegatorStartingInfo`.
pub fn set_starting_info(
    ctx: &mut Context<'_>,
    val: &[u8],
    del: &[u8],
    s: &StartingInfo,
) -> Result<()> {
    ctx.set(STORE_KEY, starting_info_key(val, del), s.encode())
}

/// Starting info from a genesis JSON proto.
pub fn starting_info_from_json(p: &pb::DelegatorStartingInfo) -> Result<StartingInfo> {
    StartingInfo::from_proto(p, true)
}

/// `HasDelegatorStartingInfo`.
pub fn has_starting_info(ctx: &mut Context<'_>, val: &[u8], del: &[u8]) -> Result<bool> {
    ctx.has(STORE_KEY, &starting_info_key(val, del))
}

/// `DeleteDelegatorStartingInfo`.
pub fn delete_starting_info(ctx: &mut Context<'_>, val: &[u8], del: &[u8]) -> Result<()> {
    ctx.delete(STORE_KEY, &starting_info_key(val, del))
}

/// `ValidatorHistoricalRewards`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HistoricalRewards {
    /// Cumulative reward ratio.
    pub cumulative_reward_ratio: DecCoins,
    /// Reference count.
    pub reference_count: u32,
}

impl HistoricalRewards {
    /// Store encoding.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        pb::ValidatorHistoricalRewards {
            cumulative_reward_ratio: self.cumulative_reward_ratio.to_proto(),
            reference_count: self.reference_count,
        }
        .encode_to_vec()
    }
}

/// `GetValidatorHistoricalRewards` (zero value when missing).
pub fn get_historical(ctx: &mut Context<'_>, val: &[u8], period: u64) -> Result<HistoricalRewards> {
    let b = ctx
        .get(STORE_KEY, &historical_key(val, period))?
        .unwrap_or_default();
    let p: pb::ValidatorHistoricalRewards = decode(&b)?;
    Ok(HistoricalRewards {
        cumulative_reward_ratio: DecCoins::from_proto(&p.cumulative_reward_ratio)?,
        reference_count: p.reference_count,
    })
}

/// `SetValidatorHistoricalRewards`.
pub fn set_historical(
    ctx: &mut Context<'_>,
    val: &[u8],
    period: u64,
    h: &HistoricalRewards,
) -> Result<()> {
    ctx.set(STORE_KEY, historical_key(val, period), h.encode())
}

/// `ValidatorCurrentRewards`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CurrentRewards {
    /// Rewards accumulated in the current period.
    pub rewards: DecCoins,
    /// Current period.
    pub period: u64,
}

/// `GetValidatorCurrentRewards` (zero value when missing).
pub fn get_current(ctx: &mut Context<'_>, val: &[u8]) -> Result<CurrentRewards> {
    let b = ctx.get(STORE_KEY, &current_key(val))?.unwrap_or_default();
    let p: pb::ValidatorCurrentRewards = decode(&b)?;
    Ok(CurrentRewards {
        rewards: DecCoins::from_proto(&p.rewards)?,
        period: p.period,
    })
}

/// `SetValidatorCurrentRewards`.
pub fn set_current(ctx: &mut Context<'_>, val: &[u8], c: &CurrentRewards) -> Result<()> {
    let v = pb::ValidatorCurrentRewards {
        rewards: c.rewards.to_proto(),
        period: c.period,
    }
    .encode_to_vec();
    ctx.set(STORE_KEY, current_key(val), v)
}

/// `GetValidatorAccumulatedCommission` (zero when missing).
pub fn get_commission(ctx: &mut Context<'_>, val: &[u8]) -> Result<DecCoins> {
    match ctx.get(STORE_KEY, &commission_key(val))? {
        Some(b) => {
            DecCoins::from_proto(&decode::<pb::ValidatorAccumulatedCommission>(&b)?.commission)
        }
        None => Ok(DecCoins::new()),
    }
}

/// `SetValidatorAccumulatedCommission`.
pub fn set_commission(ctx: &mut Context<'_>, val: &[u8], c: &DecCoins) -> Result<()> {
    let v = if c.is_zero() {
        Vec::new()
    } else {
        pb::ValidatorAccumulatedCommission {
            commission: c.to_proto(),
        }
        .encode_to_vec()
    };
    ctx.set(STORE_KEY, commission_key(val), v)
}

/// `GetValidatorOutstandingRewards` (zero when missing).
pub fn get_outstanding(ctx: &mut Context<'_>, val: &[u8]) -> Result<DecCoins> {
    let b = ctx
        .get(STORE_KEY, &outstanding_key(val))?
        .unwrap_or_default();
    DecCoins::from_proto(&decode::<pb::ValidatorOutstandingRewards>(&b)?.rewards)
}

/// `SetValidatorOutstandingRewards`.
pub fn set_outstanding(ctx: &mut Context<'_>, val: &[u8], r: &DecCoins) -> Result<()> {
    let v = pb::ValidatorOutstandingRewards {
        rewards: r.to_proto(),
    }
    .encode_to_vec();
    ctx.set(STORE_KEY, outstanding_key(val), v)
}

/// `SetValidatorSlashEvent`.
pub fn set_slash_event(
    ctx: &mut Context<'_>,
    val: &[u8],
    height: u64,
    period: u64,
    fraction: &LegacyDec,
) -> Result<()> {
    let v = pb::ValidatorSlashEvent {
        validator_period: period,
        fraction: dec_str(fraction),
    }
    .encode_to_vec();
    ctx.set(STORE_KEY, slash_event_key(val, height, period), v)
}

/// `IterateValidatorSlashEventsBetween`: `(height, period, fraction)`.
pub fn slash_events_between(
    ctx: &mut Context<'_>,
    val: &[u8],
    start: u64,
    end: u64,
) -> Result<Vec<(u64, u64, LegacyDec)>> {
    let lo = slash_event_height_prefix(val, start);
    let hi = slash_event_height_prefix(val, end + 1);
    let mut it = ctx.iterator(STORE_KEY, Some(&lo), Some(&hi))?;
    let mut out = Vec::new();
    let plen = slash_event_prefix(val).len();
    while let Some((k, v)) = it.next(ctx)? {
        let e: pb::ValidatorSlashEvent = decode(&v)?;
        let height = k
            .get(plen..plen + 8)
            .and_then(|b| b.try_into().ok())
            .map_or(0, u64::from_be_bytes);
        out.push((height, e.validator_period, wire_dec(&e.fraction)?));
    }
    Ok(out)
}

/// Deletes every key under `prefix` (`Delete*` iterator loops).
pub fn delete_prefix(ctx: &mut Context<'_>, prefix: &[u8]) -> Result<()> {
    let mut it = ctx.prefix_iter(STORE_KEY, prefix)?;
    while let Some((k, _)) = it.next(ctx)? {
        ctx.delete(STORE_KEY, &k)?;
    }
    Ok(())
}

/// `UserOutstandingRewards.Get` (`None` when not found).
pub fn get_user_outstanding(
    ctx: &mut Context<'_>,
    del: &[u8],
    val: &[u8],
) -> Result<Option<Coins>> {
    ctx.get(STORE_KEY, &user_outstanding_key(del, val))?
        .map(|b| Coins::from_proto(&decode::<pb::UserOutstandingRewards>(&b)?.rewards))
        .transpose()
}

/// `UserOutstandingRewards.Set`.
pub fn set_user_outstanding(
    ctx: &mut Context<'_>,
    del: &[u8],
    val: &[u8],
    c: &Coins,
) -> Result<()> {
    let v = pb::UserOutstandingRewards {
        rewards: c.to_proto(),
    }
    .encode_to_vec();
    ctx.set(STORE_KEY, user_outstanding_key(del, val), v)
}

/// `UserOutstandingRewards.Remove`.
pub fn remove_user_outstanding(ctx: &mut Context<'_>, del: &[u8], val: &[u8]) -> Result<()> {
    ctx.delete(STORE_KEY, &user_outstanding_key(del, val))
}
