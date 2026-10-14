//! `cosmos.authz.v1beta1.Query` (`x/authz/keeper/grpc_query.go`).

use celestia_proto::cosmos::authz::v1beta1 as pb;
use sdk::Context;

use crate::Router;
use crate::error::{QResult, QueryError};
use crate::pagination::generic_filtered_paginate;
use crate::util::{ACC, bech32, codec_to_bytes, decode, get, page};

const STORE: &str = "authz";
const GRANT: &[u8] = &[x_authz::GRANT_PREFIX];

pub(crate) fn register(r: &mut Router) {
    r.add("/cosmos.authz.v1beta1.Query/Grants", grants);
    r.add("/cosmos.authz.v1beta1.Query/GranterGrants", granter_grants);
    r.add("/cosmos.authz.v1beta1.Query/GranteeGrants", grantee_grants);
}

/// `address.MustLengthPrefix` (empty input stays empty).
fn lp(b: &[u8]) -> Vec<u8> {
    if b.is_empty() {
        Vec::new()
    } else {
        sdk::codec::length_prefixed(b)
    }
}

fn grants(ctx: &mut Context<'_>, req: pb::QueryGrantsRequest) -> QResult<pb::QueryGrantsResponse> {
    let granter = codec_to_bytes(&req.granter, ACC)?;
    let grantee = codec_to_bytes(&req.grantee, ACC)?;
    if !req.msg_type_url.is_empty() {
        let key = x_authz::grant_key(&grantee, &granter, &req.msg_type_url);
        let Some(g) = get::<pb::Grant>(ctx, STORE, &key)? else {
            return Err(QueryError::Sdk(
                x_authz::errors::no_authorization_found().wrap(format!(
                    "authorization not found for {} type",
                    req.msg_type_url
                )),
            ));
        };
        return Ok(pb::QueryGrantsResponse {
            grants: vec![pb::Grant {
                authorization: g.authorization,
                expiration: g.expiration,
            }],
            pagination: None,
        });
    }
    let prefix = [GRANT, &lp(&granter), &lp(&grantee)].concat();
    let (grants, pr) =
        generic_filtered_paginate(ctx, STORE, &prefix, req.pagination.as_ref(), |_, _, v| {
            let g: pb::Grant = decode(v)?;
            Ok(Some(g))
        })?;
    Ok(pb::QueryGrantsResponse {
        grants,
        pagination: page(pr),
    })
}

fn granter_grants(
    ctx: &mut Context<'_>,
    req: pb::QueryGranterGrantsRequest,
) -> QResult<pb::QueryGranterGrantsResponse> {
    let granter = codec_to_bytes(&req.granter, ACC)?;
    let prefix = [GRANT, &lp(&granter)].concat();
    let (grants, pr) =
        generic_filtered_paginate(ctx, STORE, &prefix, req.pagination.as_ref(), |_, k, v| {
            let g: pb::Grant = decode(v)?;
            let n = usize::from(k.first().copied().unwrap_or_default());
            let grantee = k.get(1..=n).unwrap_or_default();
            Ok(Some(pb::GrantAuthorization {
                granter: req.granter.clone(),
                grantee: bech32(ACC, grantee),
                authorization: g.authorization,
                expiration: g.expiration,
            }))
        })?;
    Ok(pb::QueryGranterGrantsResponse {
        grants,
        pagination: page(pr),
    })
}

fn grantee_grants(
    ctx: &mut Context<'_>,
    req: pb::QueryGranteeGrantsRequest,
) -> QResult<pb::QueryGranteeGrantsResponse> {
    let grantee = codec_to_bytes(&req.grantee, ACC)?;
    let (grants, pr) =
        generic_filtered_paginate(ctx, STORE, GRANT, req.pagination.as_ref(), |_, k, v| {
            let g: pb::Grant = decode(v)?;
            let (granter, g_grantee, _) = x_authz::parse_grant_key(&[GRANT, k].concat());
            if g_grantee != grantee {
                return Ok(None);
            }
            Ok(Some(pb::GrantAuthorization {
                granter: bech32(ACC, &granter),
                grantee: req.grantee.clone(),
                authorization: g.authorization,
                expiration: g.expiration,
            }))
        })?;
    Ok(pb::QueryGranteeGrantsResponse {
        grants,
        pagination: page(pr),
    })
}
