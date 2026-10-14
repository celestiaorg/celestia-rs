//! `cosmos.upgrade.v1beta1.Query` (celestia fork `x/upgrade` v0.2.0),
//! `cosmos.consensus.v1.Query` and `cosmos.params.v1beta1.Query`.

use celestia_proto::celestia_core_proto::tendermint::types::ConsensusParams;
use celestia_proto::cosmos::consensus::v1 as cpb;
use celestia_proto::cosmos::params::v1beta1 as ppb;
use celestia_proto::cosmos::upgrade::v1beta1 as upb;
use sdk::Context;
use sdk::error::SdkError;

use crate::Router;
use crate::error::{QResult, QueryError, internal, invalid_arg};
use crate::pagination::range;
use crate::util::{ACC, bech32, get};

pub(crate) fn register(r: &mut Router) {
    r.add("/cosmos.upgrade.v1beta1.Query/CurrentPlan", current_plan);
    r.add("/cosmos.upgrade.v1beta1.Query/AppliedPlan", applied_plan);
    r.add(
        "/cosmos.upgrade.v1beta1.Query/UpgradedConsensusState",
        upgraded_consensus_state,
    );
    r.add(
        "/cosmos.upgrade.v1beta1.Query/ModuleVersions",
        module_versions,
    );
    r.add("/cosmos.upgrade.v1beta1.Query/Authority", authority);
    r.add("/cosmos.consensus.v1.Query/Params", consensus_params);
    r.add("/cosmos.params.v1beta1.Query/Params", params_params);
    r.add("/cosmos.params.v1beta1.Query/Subspaces", subspaces);
}

const UPGRADE: &str = "upgrade";

fn current_plan(
    ctx: &mut Context<'_>,
    _: upb::QueryCurrentPlanRequest,
) -> QResult<upb::QueryCurrentPlanResponse> {
    Ok(upb::QueryCurrentPlanResponse {
        plan: get(ctx, UPGRADE, &[x_upgrade::PLAN_BYTE])?,
    })
}

fn applied_plan(
    ctx: &mut Context<'_>,
    req: upb::QueryAppliedPlanRequest,
) -> QResult<upb::QueryAppliedPlanResponse> {
    let mut height = 0;
    for (k, _) in range(ctx, UPGRADE, &[x_upgrade::DONE_BYTE], None, None, false)? {
        if k.len() < 9 {
            continue;
        }
        let name = String::from_utf8_lossy(&k[8..]);
        if name == req.name {
            let mut h = [0u8; 8];
            h.copy_from_slice(&k[..8]);
            height = i64::from_be_bytes(h);
            break;
        }
    }
    Ok(upb::QueryAppliedPlanResponse { height })
}

#[allow(deprecated)]
fn upgraded_consensus_state(
    ctx: &mut Context<'_>,
    req: upb::QueryUpgradedConsensusStateRequest,
) -> QResult<upb::QueryUpgradedConsensusStateResponse> {
    let state = ctx
        .get(
            UPGRADE,
            &x_upgrade::upgraded_cons_state_key(req.last_height),
        )?
        .unwrap_or_default();
    Ok(upb::QueryUpgradedConsensusStateResponse {
        upgraded_consensus_state: state,
    })
}

fn module_versions(
    ctx: &mut Context<'_>,
    req: upb::QueryModuleVersionsRequest,
) -> QResult<upb::QueryModuleVersionsResponse> {
    let all = range(
        ctx,
        UPGRADE,
        &[x_upgrade::VERSION_MAP_BYTE],
        None,
        None,
        false,
    )?;
    let versions = all.into_iter().map(|(k, v)| upb::ModuleVersion {
        name: String::from_utf8_lossy(&k).into_owned(),
        version: v
            .as_slice()
            .try_into()
            .map(u64::from_be_bytes)
            .unwrap_or_default(),
    });
    if !req.module_name.is_empty() {
        let Some(mv) = versions.into_iter().find(|m| m.name == req.module_name) else {
            return Err(QueryError::Sdk(
                SdkError::new("upgrade", 2, "module version not found").wrap(format!(
                    "x/upgrade: QueryModuleVersions module {} not found",
                    req.module_name
                )),
            ));
        };
        return Ok(upb::QueryModuleVersionsResponse {
            module_versions: vec![mv],
        });
    }
    Ok(upb::QueryModuleVersionsResponse {
        module_versions: versions.collect(),
    })
}

fn authority(
    _: &mut Context<'_>,
    _: upb::QueryAuthorityRequest,
) -> QResult<upb::QueryAuthorityResponse> {
    Ok(upb::QueryAuthorityResponse {
        address: bech32(ACC, &sdk::address::module_address("gov")),
    })
}

fn consensus_params(
    ctx: &mut Context<'_>,
    _: cpb::QueryParamsRequest,
) -> QResult<cpb::QueryParamsResponse> {
    let Some(p) = get::<ConsensusParams>(ctx, "consensus", x_consensus::PARAMS_KEY)? else {
        return Err(internal(
            "collections: not found: key 'no_key' of type tendermint.types.ConsensusParams",
        ));
    };
    Ok(cpb::QueryParamsResponse { params: Some(p) })
}

/// Subspaces registered by celestia-app's `initParamsKeeper`.
const SUBSPACES: &[&str] = &[
    "auth",
    "bank",
    "staking",
    "mint",
    "distribution",
    "slashing",
    "gov",
    "transfer",
    "ibc",
    "icahost",
    "blob",
    "minfee",
    "packetfowardmiddleware",
];

fn params_params(
    ctx: &mut Context<'_>,
    req: ppb::QueryParamsRequest,
) -> QResult<ppb::QueryParamsResponse> {
    if req.subspace.is_empty() || req.key.is_empty() {
        return Err(invalid_arg("invalid request"));
    }
    if !SUBSPACES.contains(&req.subspace.as_str()) {
        return Err(QueryError::Sdk(
            SdkError::new("params", 2, "unknown subspace").wrap(&req.subspace),
        ));
    }
    let key = format!("{}/{}", req.subspace, req.key);
    let raw = ctx.get("params", key.as_bytes())?.unwrap_or_default();
    Ok(ppb::QueryParamsResponse {
        param: Some(ppb::ParamChange {
            subspace: req.subspace,
            key: req.key,
            value: String::from_utf8_lossy(&raw).into_owned(),
        }),
    })
}

fn subspaces(
    ctx: &mut Context<'_>,
    _: ppb::QuerySubspacesRequest,
) -> QResult<ppb::QuerySubspacesResponse> {
    let mut out = Vec::with_capacity(SUBSPACES.len());
    for name in SUBSPACES {
        let prefix = format!("{name}/");
        let keys = range(ctx, "params", prefix.as_bytes(), None, None, false)?
            .into_iter()
            .map(|(k, _)| String::from_utf8_lossy(&k).into_owned())
            .collect();
        out.push(ppb::Subspace {
            subspace: (*name).to_owned(),
            keys,
        });
    }
    Ok(ppb::QuerySubspacesResponse { subspaces: out })
}
