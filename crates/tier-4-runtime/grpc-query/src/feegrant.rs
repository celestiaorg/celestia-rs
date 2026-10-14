//! `cosmos.feegrant.v1beta1.Query` (`cosmossdk.io/x/feegrant` v0.1.1
//! `keeper/grpc_query.go`).

use celestia_proto::cosmos::feegrant::v1beta1 as pb;
use sdk::Context;

use crate::Router;
use crate::error::{QResult, internal};
use crate::pagination::{generic_filtered_paginate, paginate};
use crate::util::{ACC, codec_to_bytes, decode, get, page};

const STORE: &str = "feegrant";
const PREFIX: &[u8] = &[0x00];

pub(crate) fn register(r: &mut Router) {
    r.add("/cosmos.feegrant.v1beta1.Query/Allowance", allowance);
    r.add("/cosmos.feegrant.v1beta1.Query/Allowances", allowances);
    r.add(
        "/cosmos.feegrant.v1beta1.Query/AllowancesByGranter",
        allowances_by_granter,
    );
}

fn allowance(
    ctx: &mut Context<'_>,
    req: pb::QueryAllowanceRequest,
) -> QResult<pb::QueryAllowanceResponse> {
    let granter = codec_to_bytes(&req.granter, ACC)?;
    let grantee = codec_to_bytes(&req.grantee, ACC)?;
    let key = x_feegrant::allowance_key(&granter, &grantee);
    let Some(g) = get::<pb::Grant>(ctx, STORE, &key)? else {
        return Err(internal("fee-grant not found: not found"));
    };
    Ok(pb::QueryAllowanceResponse {
        allowance: Some(pb::Grant {
            granter: req.granter,
            grantee: req.grantee,
            allowance: g.allowance,
        }),
    })
}

fn allowances(
    ctx: &mut Context<'_>,
    req: pb::QueryAllowancesRequest,
) -> QResult<pb::QueryAllowancesResponse> {
    let grantee = codec_to_bytes(&req.grantee, ACC)?;
    let prefix = [PREFIX, &sdk::codec::length_prefixed(&grantee)].concat();
    let mut allowances = Vec::new();
    let pr = paginate(ctx, STORE, &prefix, req.pagination.as_ref(), |_, _, v| {
        allowances.push(decode(v)?);
        Ok(())
    })
    .map_err(|e| internal(e.go_error_string()))?;
    Ok(pb::QueryAllowancesResponse {
        allowances,
        pagination: page(pr),
    })
}

fn allowances_by_granter(
    ctx: &mut Context<'_>,
    req: pb::QueryAllowancesByGranterRequest,
) -> QResult<pb::QueryAllowancesByGranterResponse> {
    let granter = codec_to_bytes(&req.granter, ACC)?;
    let (allowances, pr) =
        generic_filtered_paginate(ctx, STORE, PREFIX, req.pagination.as_ref(), |_, k, v| {
            // key: lp(grantee) || lp(granter)
            let n = usize::from(k.first().copied().unwrap_or_default());
            let rest = k.get(1 + n..).unwrap_or_default();
            let m = usize::from(rest.first().copied().unwrap_or_default());
            let g = rest.get(1..=m).unwrap_or_default();
            if g != granter.as_slice() {
                return Ok(None);
            }
            Ok(Some(decode::<pb::Grant>(v)?))
        })
        .map_err(|e| internal(e.go_error_string()))?;
    Ok(pb::QueryAllowancesByGranterResponse {
        allowances,
        pagination: page(pr),
    })
}
