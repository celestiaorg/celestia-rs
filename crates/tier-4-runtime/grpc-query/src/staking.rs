//! `cosmos.staking.v1beta1.Query` (`x/staking/keeper/grpc_query.go`).

use celestia_proto::cosmos::base::v1beta1::Coin;
use celestia_proto::cosmos::staking::v1beta1 as pb;
use math::LegacyDec;
use sdk::Context;
use x_staking::keys;
use x_staking::types::{Repr, Validator};

use crate::Router;
use crate::error::{QResult, QueryError, internal, invalid_arg, not_found};
use crate::pagination::{generic_filtered_paginate, paginate};
use crate::util::{ACC, VALOPER, codec_to_bytes, decode, get, page};

const STORE: &str = "staking";

pub(crate) fn register(r: &mut Router) {
    r.add("/cosmos.staking.v1beta1.Query/Validators", validators);
    r.add("/cosmos.staking.v1beta1.Query/Validator", validator);
    r.add(
        "/cosmos.staking.v1beta1.Query/ValidatorDelegations",
        validator_delegations,
    );
    r.add(
        "/cosmos.staking.v1beta1.Query/ValidatorUnbondingDelegations",
        validator_unbonding_delegations,
    );
    r.add("/cosmos.staking.v1beta1.Query/Delegation", delegation);
    r.add(
        "/cosmos.staking.v1beta1.Query/UnbondingDelegation",
        unbonding_delegation,
    );
    r.add(
        "/cosmos.staking.v1beta1.Query/DelegatorDelegations",
        delegator_delegations,
    );
    r.add(
        "/cosmos.staking.v1beta1.Query/DelegatorUnbondingDelegations",
        delegator_unbonding_delegations,
    );
    r.add("/cosmos.staking.v1beta1.Query/Redelegations", redelegations);
    r.add(
        "/cosmos.staking.v1beta1.Query/DelegatorValidators",
        delegator_validators,
    );
    r.add(
        "/cosmos.staking.v1beta1.Query/DelegatorValidator",
        delegator_validator,
    );
    r.add(
        "/cosmos.staking.v1beta1.Query/HistoricalInfo",
        historical_info,
    );
    r.add("/cosmos.staking.v1beta1.Query/Pool", pool);
    r.add("/cosmos.staking.v1beta1.Query/Params", params);
}

fn val_addr(s: &str) -> QResult<Vec<u8>> {
    codec_to_bytes(s, VALOPER)
}

fn acc_addr(s: &str) -> QResult<Vec<u8>> {
    codec_to_bytes(s, ACC)
}

fn internal_err(e: &QueryError) -> QueryError {
    internal(e.go_error_string())
}

/// `GetValidator` as the stored proto (`ErrNoValidatorFound` if missing).
fn get_validator(ctx: &mut Context<'_>, op: &[u8]) -> QResult<pb::Validator> {
    get(ctx, STORE, &keys::validator(op))?
        .ok_or_else(|| QueryError::Sdk(x_staking::errors::no_validator_found()))
}

fn wire_dec(s: &str) -> QResult<LegacyDec> {
    LegacyDec::unmarshal(s.as_bytes()).map_err(|e| QueryError::Plain(e.to_string()))
}

fn bond_denom(ctx: &mut Context<'_>) -> QResult<String> {
    Ok(x_staking::store::bond_denom(ctx)?)
}

fn tokens_from_shares(v: &pb::Validator, shares: &str) -> QResult<String> {
    let val = Validator::from_proto(v, Repr::Wire)?;
    let t = val.tokens_from_shares(&wire_dec(shares)?)?;
    Ok(t.truncate_int().to_string())
}

/// `delegationToDelegationResponse`.
fn delegation_response(
    ctx: &mut Context<'_>,
    del: pb::Delegation,
) -> QResult<pb::DelegationResponse> {
    let val = val_addr(&del.validator_address)?;
    let v = get_validator(ctx, &val)?;
    acc_addr(&del.delegator_address)?;
    let denom = bond_denom(ctx)?;
    let amount = tokens_from_shares(&v, &del.shares)?;
    Ok(pb::DelegationResponse {
        delegation: Some(del),
        balance: Some(Coin { denom, amount }),
    })
}

fn validators(
    ctx: &mut Context<'_>,
    req: pb::QueryValidatorsRequest,
) -> QResult<pb::QueryValidatorsResponse> {
    let statuses = [
        "BOND_STATUS_UNSPECIFIED",
        "BOND_STATUS_UNBONDED",
        "BOND_STATUS_UNBONDING",
        "BOND_STATUS_BONDED",
    ];
    if !req.status.is_empty() && !statuses[1..].contains(&req.status.as_str()) {
        return Err(invalid_arg(format!(
            "invalid validator status {}",
            req.status
        )));
    }
    let (validators, pr) = generic_filtered_paginate(
        ctx,
        STORE,
        &[keys::VALIDATORS],
        req.pagination.as_ref(),
        |_, _, v| {
            let val: pb::Validator = decode(v)?;
            let status = usize::try_from(val.status)
                .ok()
                .and_then(|i| statuses.get(i))
                .copied()
                .unwrap_or_default();
            if !req.status.is_empty() && !status.eq_ignore_ascii_case(&req.status) {
                return Ok(None);
            }
            Ok(Some(val))
        },
    )
    .map_err(|e| internal_err(&e))?;
    Ok(pb::QueryValidatorsResponse {
        validators,
        pagination: page(pr),
    })
}

fn validator(
    ctx: &mut Context<'_>,
    req: pb::QueryValidatorRequest,
) -> QResult<pb::QueryValidatorResponse> {
    if req.validator_addr.is_empty() {
        return Err(invalid_arg("validator address cannot be empty"));
    }
    let val = val_addr(&req.validator_addr)?;
    let Some(v) = get::<pb::Validator>(ctx, STORE, &keys::validator(&val))? else {
        return Err(not_found(format!(
            "validator {} not found",
            req.validator_addr
        )));
    };
    Ok(pb::QueryValidatorResponse { validator: Some(v) })
}

fn validator_delegations(
    ctx: &mut Context<'_>,
    req: pb::QueryValidatorDelegationsRequest,
) -> QResult<pb::QueryValidatorDelegationsResponse> {
    if req.validator_addr.is_empty() {
        return Err(invalid_arg("validator address cannot be empty"));
    }
    let val = val_addr(&req.validator_addr)?;
    let mut dels = Vec::new();
    let pr = paginate(
        ctx,
        STORE,
        &keys::delegations_by_val(&val),
        req.pagination.as_ref(),
        |ctx, del, _| {
            if let Some(d) = get::<pb::Delegation>(ctx, STORE, &keys::delegation(del, &val))? {
                dels.push(d);
            } else {
                dels.push(pb::Delegation::default());
            }
            Ok(())
        },
    )
    .map_err(|e| internal_err(&e))?;
    let mut delegation_responses = Vec::with_capacity(dels.len());
    for d in dels {
        delegation_responses.push(delegation_response(ctx, d).map_err(|e| internal_err(&e))?);
    }
    Ok(pb::QueryValidatorDelegationsResponse {
        delegation_responses,
        pagination: page(pr),
    })
}

fn validator_unbonding_delegations(
    ctx: &mut Context<'_>,
    req: pb::QueryValidatorUnbondingDelegationsRequest,
) -> QResult<pb::QueryValidatorUnbondingDelegationsResponse> {
    if req.validator_addr.is_empty() {
        return Err(invalid_arg("validator address cannot be empty"));
    }
    let val = val_addr(&req.validator_addr)?;
    let prefix = keys::ubds_by_val(&val);
    let mut unbonding_responses = Vec::new();
    let pr = paginate(ctx, STORE, &prefix, req.pagination.as_ref(), |ctx, k, _| {
        let full = [prefix.as_slice(), k].concat();
        let key = keys::ubd_from_val_index(&full).unwrap_or_default();
        let ubd = ctx.get(STORE, &key)?.unwrap_or_default();
        unbonding_responses.push(decode(&ubd)?);
        Ok(())
    })
    .map_err(|e| internal_err(&e))?;
    Ok(pb::QueryValidatorUnbondingDelegationsResponse {
        unbonding_responses,
        pagination: page(pr),
    })
}

fn delegation(
    ctx: &mut Context<'_>,
    req: pb::QueryDelegationRequest,
) -> QResult<pb::QueryDelegationResponse> {
    if req.delegator_addr.is_empty() {
        return Err(invalid_arg("delegator address cannot be empty"));
    }
    if req.validator_addr.is_empty() {
        return Err(invalid_arg("validator address cannot be empty"));
    }
    let del = acc_addr(&req.delegator_addr)?;
    let val = val_addr(&req.validator_addr)?;
    let Some(d) = get::<pb::Delegation>(ctx, STORE, &keys::delegation(&del, &val))? else {
        return Err(not_found(format!(
            "delegation with delegator {} not found for validator {}",
            req.delegator_addr, req.validator_addr
        )));
    };
    let resp = delegation_response(ctx, d).map_err(|e| internal_err(&e))?;
    Ok(pb::QueryDelegationResponse {
        delegation_response: Some(resp),
    })
}

fn unbonding_delegation(
    ctx: &mut Context<'_>,
    req: pb::QueryUnbondingDelegationRequest,
) -> QResult<pb::QueryUnbondingDelegationResponse> {
    if req.delegator_addr.is_empty() {
        return Err(invalid_arg("delegator address cannot be empty"));
    }
    if req.validator_addr.is_empty() {
        return Err(invalid_arg("validator address cannot be empty"));
    }
    let del = acc_addr(&req.delegator_addr)?;
    let val = val_addr(&req.validator_addr)?;
    let Some(u) = get::<pb::UnbondingDelegation>(ctx, STORE, &keys::ubd(&del, &val))? else {
        return Err(not_found(format!(
            "unbonding delegation with delegator {} not found for validator {}",
            req.delegator_addr, req.validator_addr
        )));
    };
    Ok(pb::QueryUnbondingDelegationResponse { unbond: Some(u) })
}

fn delegator_delegations(
    ctx: &mut Context<'_>,
    req: pb::QueryDelegatorDelegationsRequest,
) -> QResult<pb::QueryDelegatorDelegationsResponse> {
    if req.delegator_addr.is_empty() {
        return Err(invalid_arg("delegator address cannot be empty"));
    }
    let del = acc_addr(&req.delegator_addr)?;
    let mut dels = Vec::new();
    let pr = paginate(
        ctx,
        STORE,
        &keys::delegations(&del),
        req.pagination.as_ref(),
        |_, _, v| {
            dels.push(decode::<pb::Delegation>(v)?);
            Ok(())
        },
    )
    .map_err(|e| internal_err(&e))?;
    let mut delegation_responses = Vec::with_capacity(dels.len());
    for d in dels {
        delegation_responses.push(delegation_response(ctx, d).map_err(|e| internal_err(&e))?);
    }
    Ok(pb::QueryDelegatorDelegationsResponse {
        delegation_responses,
        pagination: page(pr),
    })
}

fn delegator_validator(
    ctx: &mut Context<'_>,
    req: pb::QueryDelegatorValidatorRequest,
) -> QResult<pb::QueryDelegatorValidatorResponse> {
    if req.delegator_addr.is_empty() {
        return Err(invalid_arg("delegator address cannot be empty"));
    }
    if req.validator_addr.is_empty() {
        return Err(invalid_arg("validator address cannot be empty"));
    }
    let del = acc_addr(&req.delegator_addr)?;
    let val = val_addr(&req.validator_addr)?;
    let Some(d) = get::<pb::Delegation>(ctx, STORE, &keys::delegation(&del, &val))? else {
        return Err(internal_err(&QueryError::Sdk(
            x_staking::errors::no_delegation(),
        )));
    };
    let val = val_addr(&d.validator_address).map_err(|e| internal_err(&e))?;
    let v = get_validator(ctx, &val).map_err(|e| internal_err(&e))?;
    Ok(pb::QueryDelegatorValidatorResponse { validator: Some(v) })
}

fn delegator_unbonding_delegations(
    ctx: &mut Context<'_>,
    req: pb::QueryDelegatorUnbondingDelegationsRequest,
) -> QResult<pb::QueryDelegatorUnbondingDelegationsResponse> {
    if req.delegator_addr.is_empty() {
        return Err(invalid_arg("delegator address cannot be empty"));
    }
    let del = acc_addr(&req.delegator_addr)?;
    let mut unbonding_responses = Vec::new();
    let pr = paginate(
        ctx,
        STORE,
        &keys::ubds(&del),
        req.pagination.as_ref(),
        |_, _, v| {
            unbonding_responses.push(decode(v)?);
            Ok(())
        },
    )
    .map_err(|e| internal_err(&e))?;
    Ok(pb::QueryDelegatorUnbondingDelegationsResponse {
        unbonding_responses,
        pagination: page(pr),
    })
}

fn historical_info(
    ctx: &mut Context<'_>,
    req: pb::QueryHistoricalInfoRequest,
) -> QResult<pb::QueryHistoricalInfoResponse> {
    if req.height < 0 {
        return Err(invalid_arg("height cannot be negative"));
    }
    if ctx.has(STORE, keys::NEXT_MIGRATE_HISTORICAL_INFO)? {
        return Err(internal(
            "store migration has not finished, try again later",
        ));
    }
    let Some(hist) = get::<pb::HistoricalInfo>(ctx, STORE, &keys::historical_info(req.height))?
    else {
        return Err(not_found(format!(
            "historical info for height {} not found",
            req.height
        )));
    };
    Ok(pb::QueryHistoricalInfoResponse { hist: Some(hist) })
}

fn redelegations(
    ctx: &mut Context<'_>,
    req: pb::QueryRedelegationsRequest,
) -> QResult<pb::QueryRedelegationsResponse> {
    let (reds, pagination) = if !req.delegator_addr.is_empty()
        && !req.src_validator_addr.is_empty()
        && !req.dst_validator_addr.is_empty()
    {
        let res = (|| {
            let del = acc_addr(&req.delegator_addr)?;
            let src = val_addr(&req.src_validator_addr)?;
            let dst = val_addr(&req.dst_validator_addr)?;
            match get::<pb::Redelegation>(ctx, STORE, &keys::red(&del, &src, &dst))? {
                Some(r) => Ok(vec![r]),
                None => Err(not_found(format!(
                    "redelegation not found for delegator address {} from validator address {}",
                    req.delegator_addr, req.src_validator_addr
                ))),
            }
        })();
        (res.map_err(|e| internal_err(&e))?, None)
    } else if req.delegator_addr.is_empty()
        && !req.src_validator_addr.is_empty()
        && req.dst_validator_addr.is_empty()
    {
        let src = val_addr(&req.src_validator_addr).map_err(|e| internal_err(&e))?;
        let prefix = keys::reds_from_val_src(&src);
        let mut reds = Vec::new();
        let pr = paginate(ctx, STORE, &prefix, req.pagination.as_ref(), |ctx, k, _| {
            let full = [prefix.as_slice(), k].concat();
            let key = keys::red_from_val_src_index(&full).unwrap_or_default();
            let v = ctx.get(STORE, &key)?.unwrap_or_default();
            reds.push(decode::<pb::Redelegation>(&v)?);
            Ok(())
        })
        .map_err(|e| internal_err(&e))?;
        (reds, page(pr))
    } else {
        let del = acc_addr(&req.delegator_addr).map_err(|e| internal_err(&e))?;
        let mut reds = Vec::new();
        let pr = paginate(
            ctx,
            STORE,
            &keys::reds(&del),
            req.pagination.as_ref(),
            |_, _, v| {
                reds.push(decode::<pb::Redelegation>(v)?);
                Ok(())
            },
        )
        .map_err(|e| internal_err(&e))?;
        (reds, page(pr))
    };
    let mut redelegation_responses = Vec::with_capacity(reds.len());
    for red in reds {
        redelegation_responses.push(redelegation_response(ctx, red).map_err(|e| internal_err(&e))?);
    }
    Ok(pb::QueryRedelegationsResponse {
        redelegation_responses,
        pagination,
    })
}

fn redelegation_response(
    ctx: &mut Context<'_>,
    red: pb::Redelegation,
) -> QResult<pb::RedelegationResponse> {
    val_addr(&red.validator_src_address)?;
    let dst = val_addr(&red.validator_dst_address)?;
    acc_addr(&red.delegator_address)?;
    let v = get_validator(ctx, &dst)?;
    let mut entries = Vec::with_capacity(red.entries.len());
    for e in &red.entries {
        entries.push(pb::RedelegationEntryResponse {
            redelegation_entry: Some(pb::RedelegationEntry {
                creation_height: e.creation_height,
                completion_time: e.completion_time,
                initial_balance: e.initial_balance.clone(),
                shares_dst: e.shares_dst.clone(),
                unbonding_id: e.unbonding_id,
                unbonding_on_hold_ref_count: 0,
            }),
            balance: tokens_from_shares(&v, &e.shares_dst)?,
        });
    }
    Ok(pb::RedelegationResponse {
        redelegation: Some(pb::Redelegation {
            delegator_address: red.delegator_address,
            validator_src_address: red.validator_src_address,
            validator_dst_address: red.validator_dst_address,
            entries: Vec::new(),
        }),
        entries,
    })
}

fn delegator_validators(
    ctx: &mut Context<'_>,
    req: pb::QueryDelegatorValidatorsRequest,
) -> QResult<pb::QueryDelegatorValidatorsResponse> {
    if req.delegator_addr.is_empty() {
        return Err(invalid_arg("delegator address cannot be empty"));
    }
    let del = acc_addr(&req.delegator_addr)?;
    let mut validators = Vec::new();
    let pr = paginate(
        ctx,
        STORE,
        &keys::delegations(&del),
        req.pagination.as_ref(),
        |ctx, _, v| {
            let d: pb::Delegation = decode(v)?;
            let val = val_addr(&d.validator_address)?;
            validators.push(get_validator(ctx, &val)?);
            Ok(())
        },
    )
    .map_err(|e| internal_err(&e))?;
    Ok(pb::QueryDelegatorValidatorsResponse {
        validators,
        pagination: page(pr),
    })
}

fn pool(ctx: &mut Context<'_>, _: pb::QueryPoolRequest) -> QResult<pb::QueryPoolResponse> {
    let denom = bond_denom(ctx)?;
    let bonded = x_auth::module_address("bonded_tokens_pool");
    let not_bonded = x_auth::module_address("not_bonded_tokens_pool");
    // GetBondedPool / GetNotBondedPool go through GetModuleAccount, which
    // creates a missing module account.
    x_auth::get_module_account(ctx, "bonded_tokens_pool")?;
    x_auth::get_module_account(ctx, "not_bonded_tokens_pool")?;
    let nb = x_bank::get_balance(ctx, &not_bonded, &denom)?;
    let b = x_bank::get_balance(ctx, &bonded, &denom)?;
    Ok(pb::QueryPoolResponse {
        pool: Some(pb::Pool {
            not_bonded_tokens: nb.amount.to_string(),
            bonded_tokens: b.amount.to_string(),
        }),
    })
}

fn params(ctx: &mut Context<'_>, _: pb::QueryParamsRequest) -> QResult<pb::QueryParamsResponse> {
    Ok(pb::QueryParamsResponse {
        params: Some(get(ctx, STORE, keys::PARAMS)?.unwrap_or_default()),
    })
}
