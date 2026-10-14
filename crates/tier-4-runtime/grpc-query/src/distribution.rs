//! `cosmos.distribution.v1beta1.Query` (`x/distribution/keeper/grpc_query.go`,
//! celestia fork with `UserOutstandingRewards`).

use celestia_proto::cosmos::distribution::v1beta1 as pb;
use sdk::Context;
use sdk::dec_coins::DecCoins;
use x_distribution::store as dstore;

use crate::Router;
use crate::error::{QResult, QueryError, invalid_arg};
use crate::pagination::generic_filtered_paginate;
use crate::util::{ACC, VALOPER, bech32, codec_to_bytes, decode, get, page};

const STORE: &str = "distribution";

pub(crate) fn register(r: &mut Router) {
    r.add("/cosmos.distribution.v1beta1.Query/Params", params);
    r.add(
        "/cosmos.distribution.v1beta1.Query/ValidatorDistributionInfo",
        validator_distribution_info,
    );
    r.add(
        "/cosmos.distribution.v1beta1.Query/ValidatorOutstandingRewards",
        validator_outstanding_rewards,
    );
    r.add(
        "/cosmos.distribution.v1beta1.Query/ValidatorCommission",
        validator_commission,
    );
    r.add(
        "/cosmos.distribution.v1beta1.Query/ValidatorSlashes",
        validator_slashes,
    );
    r.add(
        "/cosmos.distribution.v1beta1.Query/DelegationRewards",
        delegation_rewards,
    );
    r.add(
        "/cosmos.distribution.v1beta1.Query/DelegationTotalRewards",
        delegation_total_rewards,
    );
    r.add(
        "/cosmos.distribution.v1beta1.Query/DelegatorValidators",
        delegator_validators,
    );
    r.add(
        "/cosmos.distribution.v1beta1.Query/DelegatorWithdrawAddress",
        delegator_withdraw_address,
    );
    r.add(
        "/cosmos.distribution.v1beta1.Query/CommunityPool",
        community_pool,
    );
}

fn params(ctx: &mut Context<'_>, _: pb::QueryParamsRequest) -> QResult<pb::QueryParamsResponse> {
    Ok(pb::QueryParamsResponse {
        params: Some(get(ctx, STORE, dstore::PARAMS)?.unwrap_or_default()),
    })
}

/// `stakingKeeper.Validator` (staking `ErrNoValidatorFound` if missing).
fn validator(ctx: &mut Context<'_>, val: &[u8]) -> QResult<x_staking::types::Validator> {
    Ok(x_staking::store::validator(ctx, val)?)
}

fn val_addr(s: &str) -> QResult<Vec<u8>> {
    codec_to_bytes(s, VALOPER)
}

fn validator_distribution_info(
    ctx: &mut Context<'_>,
    req: pb::QueryValidatorDistributionInfoRequest,
) -> QResult<pb::QueryValidatorDistributionInfoResponse> {
    if req.validator_address.is_empty() {
        return Err(invalid_arg("empty validator address"));
    }
    let val = val_addr(&req.validator_address)?;
    let v = validator(ctx, &val)?;
    let Some(del) = x_staking::store::get_delegation(ctx, &val, &val)? else {
        return Err(QueryError::Sdk(x_staking::errors::no_delegation()));
    };
    let ending = x_distribution::keeper::increment_validator_period(ctx, &v)?;
    let rewards = x_distribution::keeper::calculate_delegation_rewards(ctx, &v, &del, ending)?;
    let commission = dstore::get_commission(ctx, &val)?;
    Ok(pb::QueryValidatorDistributionInfoResponse {
        operator_address: bech32(ACC, &val),
        self_bond_rewards: rewards.to_proto(),
        commission: commission.to_proto(),
    })
}

fn validator_outstanding_rewards(
    ctx: &mut Context<'_>,
    req: pb::QueryValidatorOutstandingRewardsRequest,
) -> QResult<pb::QueryValidatorOutstandingRewardsResponse> {
    if req.validator_address.is_empty() {
        return Err(invalid_arg("empty validator address"));
    }
    let val = val_addr(&req.validator_address)?;
    validator(ctx, &val)?;
    let rewards = dstore::get_outstanding(ctx, &val)?;
    Ok(pb::QueryValidatorOutstandingRewardsResponse {
        rewards: Some(pb::ValidatorOutstandingRewards {
            rewards: rewards.to_proto(),
        }),
    })
}

fn validator_commission(
    ctx: &mut Context<'_>,
    req: pb::QueryValidatorCommissionRequest,
) -> QResult<pb::QueryValidatorCommissionResponse> {
    if req.validator_address.is_empty() {
        return Err(invalid_arg("empty validator address"));
    }
    let val = val_addr(&req.validator_address)?;
    validator(ctx, &val)?;
    let commission = dstore::get_commission(ctx, &val)?;
    Ok(pb::QueryValidatorCommissionResponse {
        commission: Some(pb::ValidatorAccumulatedCommission {
            commission: commission.to_proto(),
        }),
    })
}

fn validator_slashes(
    ctx: &mut Context<'_>,
    req: pb::QueryValidatorSlashesRequest,
) -> QResult<pb::QueryValidatorSlashesResponse> {
    if req.validator_address.is_empty() {
        return Err(invalid_arg("empty validator address"));
    }
    if req.ending_height < req.starting_height {
        return Err(invalid_arg(format!(
            "starting height greater than ending height ({} > {})",
            req.starting_height, req.ending_height
        )));
    }
    let val =
        val_addr(&req.validator_address).map_err(|_| invalid_arg("invalid validator address"))?;
    let (slashes, pr) = generic_filtered_paginate(
        ctx,
        STORE,
        &dstore::slash_event_prefix(&val),
        req.pagination.as_ref(),
        |_, _, v| {
            let e: pb::ValidatorSlashEvent = decode(v)?;
            if e.validator_period < req.starting_height || e.validator_period > req.ending_height {
                return Ok(None);
            }
            Ok(Some(e))
        },
    )?;
    Ok(pb::QueryValidatorSlashesResponse {
        slashes,
        pagination: page(pr),
    })
}

fn delegation_rewards(
    ctx: &mut Context<'_>,
    req: pb::QueryDelegationRewardsRequest,
) -> QResult<pb::QueryDelegationRewardsResponse> {
    if req.delegator_address.is_empty() {
        return Err(invalid_arg("empty delegator address"));
    }
    if req.validator_address.is_empty() {
        return Err(invalid_arg("empty validator address"));
    }
    let val = val_addr(&req.validator_address)?;
    let v = validator(ctx, &val)?;
    let del = codec_to_bytes(&req.delegator_address, ACC)?;
    let d = x_staking::store::get_delegation(ctx, &del, &val)?;
    let outstanding = dstore::get_user_outstanding(ctx, &del, &val)?;
    let Some(d) = d else {
        return match outstanding {
            None => Err(QueryError::Sdk(
                x_distribution::errors::no_delegation_exists(),
            )),
            Some(o) => Ok(pb::QueryDelegationRewardsResponse {
                rewards: DecCoins::from_coins(&o)?.to_proto(),
            }),
        };
    };
    let ending = x_distribution::keeper::increment_validator_period(ctx, &v)?;
    let mut rewards = x_distribution::keeper::calculate_delegation_rewards(ctx, &v, &d, ending)?;
    if let Some(o) = outstanding {
        if !o.is_empty() && o.iter().any(|c| !c.is_zero()) {
            rewards = rewards.add(&DecCoins::from_coins(&o)?)?;
        }
    }
    Ok(pb::QueryDelegationRewardsResponse {
        rewards: rewards.to_proto(),
    })
}

fn delegation_total_rewards(
    ctx: &mut Context<'_>,
    req: pb::QueryDelegationTotalRewardsRequest,
) -> QResult<pb::QueryDelegationTotalRewardsResponse> {
    if req.delegator_address.is_empty() {
        return Err(invalid_arg("empty delegator address"));
    }
    let del = codec_to_bytes(&req.delegator_address, ACC)?;
    let mut total = DecCoins::new();
    // (validator, reward) in iteration order; Go returns map values in
    // random order.
    let mut rewards: Vec<(String, DecCoins)> = Vec::new();
    let prefix = x_staking::keys::delegations(&del);
    let items = crate::pagination::range(ctx, "staking", &prefix, None, None, false)?;
    for (_, v) in items {
        let d: celestia_proto::cosmos::staking::v1beta1::Delegation = decode(&v)?;
        let val = val_addr(&d.validator_address)?;
        let v = validator(ctx, &val)?;
        let d = x_staking::store::get_delegation(ctx, &del, &val)?
            .ok_or_else(|| QueryError::Sdk(x_staking::errors::no_delegation()))?;
        let ending = x_distribution::keeper::increment_validator_period(ctx, &v)?;
        let r = x_distribution::keeper::calculate_delegation_rewards(ctx, &v, &d, ending)?;
        total = total.add(&r)?;
        rewards.push((d.validator_address.clone(), r));
    }
    let prefix = dstore::user_outstanding_key(&del, &[]);
    let items = crate::pagination::range(ctx, STORE, &prefix, None, None, false)?;
    let mut outstanding: Vec<(String, sdk::coins::Coins)> = Vec::new();
    for (k, v) in items {
        let key = bech32(VALOPER, &k);
        let o: pb::UserOutstandingRewards = decode(&v)?;
        let coins = sdk::coins::Coins::from_proto(&o.rewards)?;
        match outstanding.iter_mut().find(|(k, _)| *k == key) {
            Some((_, c)) => *c = c.add(&coins)?,
            None => outstanding.push((key, coins)),
        }
    }
    for (val, coins) in outstanding {
        let dec = DecCoins::from_coins(&coins)?;
        total = total.add(&dec)?;
        match rewards.iter_mut().find(|(k, _)| *k == val) {
            Some((_, r)) => *r = r.add(&dec)?,
            None => rewards.push((val, dec)),
        }
    }
    Ok(pb::QueryDelegationTotalRewardsResponse {
        rewards: rewards
            .into_iter()
            .map(|(validator_address, r)| pb::DelegationDelegatorReward {
                validator_address,
                reward: r.to_proto(),
            })
            .collect(),
        total: total.to_proto(),
    })
}

fn delegator_validators(
    ctx: &mut Context<'_>,
    req: pb::QueryDelegatorValidatorsRequest,
) -> QResult<pb::QueryDelegatorValidatorsResponse> {
    if req.delegator_address.is_empty() {
        return Err(invalid_arg("empty delegator address"));
    }
    let del = codec_to_bytes(&req.delegator_address, ACC)?;
    let prefix = x_staking::keys::delegations(&del);
    let items = crate::pagination::range(ctx, "staking", &prefix, None, None, false)?;
    let mut validators = Vec::new();
    for (_, v) in items {
        let d: celestia_proto::cosmos::staking::v1beta1::Delegation = decode(&v)?;
        validators.push(d.validator_address);
    }
    Ok(pb::QueryDelegatorValidatorsResponse { validators })
}

fn delegator_withdraw_address(
    ctx: &mut Context<'_>,
    req: pb::QueryDelegatorWithdrawAddressRequest,
) -> QResult<pb::QueryDelegatorWithdrawAddressResponse> {
    if req.delegator_address.is_empty() {
        return Err(invalid_arg("empty delegator address"));
    }
    let del = codec_to_bytes(&req.delegator_address, ACC)?;
    let addr = dstore::get_withdraw_addr(ctx, &del)?;
    Ok(pb::QueryDelegatorWithdrawAddressResponse {
        withdraw_address: bech32(ACC, &addr),
    })
}

fn community_pool(
    ctx: &mut Context<'_>,
    _: pb::QueryCommunityPoolRequest,
) -> QResult<pb::QueryCommunityPoolResponse> {
    let pool = get::<pb::FeePool>(ctx, STORE, dstore::FEE_POOL)?.unwrap_or_default();
    Ok(pb::QueryCommunityPoolResponse {
        pool: pool.community_pool,
    })
}
