//! `cosmos.slashing.v1beta1.Query` (`x/slashing/keeper/grpc_query.go`).

use celestia_proto::cosmos::slashing::v1beta1 as pb;
use sdk::Context;

use crate::Router;
use crate::error::{QResult, invalid_arg, not_found};
use crate::pagination::paginate;
use crate::util::{VALCONS, codec_to_bytes, decode, get, page};

const STORE: &str = "slashing";

pub(crate) fn register(r: &mut Router) {
    r.add("/cosmos.slashing.v1beta1.Query/Params", params);
    r.add("/cosmos.slashing.v1beta1.Query/SigningInfo", signing_info);
    r.add("/cosmos.slashing.v1beta1.Query/SigningInfos", signing_infos);
}

fn params(ctx: &mut Context<'_>, _: pb::QueryParamsRequest) -> QResult<pb::QueryParamsResponse> {
    Ok(pb::QueryParamsResponse {
        params: Some(get(ctx, STORE, &[0x00])?.unwrap_or_default()),
    })
}

fn signing_info(
    ctx: &mut Context<'_>,
    req: pb::QuerySigningInfoRequest,
) -> QResult<pb::QuerySigningInfoResponse> {
    if req.cons_address.is_empty() {
        return Err(invalid_arg("invalid request"));
    }
    let cons = codec_to_bytes(&req.cons_address, VALCONS)?;
    let Some(info) = get(ctx, STORE, &x_slashing::signing_info_key(&cons))? else {
        return Err(not_found(format!(
            "SigningInfo not found for validator {}",
            req.cons_address
        )));
    };
    Ok(pb::QuerySigningInfoResponse {
        val_signing_info: Some(info),
    })
}

fn signing_infos(
    ctx: &mut Context<'_>,
    req: pb::QuerySigningInfosRequest,
) -> QResult<pb::QuerySigningInfosResponse> {
    let mut info = Vec::new();
    let pr = paginate(ctx, STORE, &[0x01], req.pagination.as_ref(), |_, _, v| {
        info.push(decode(v)?);
        Ok(())
    })?;
    Ok(pb::QuerySigningInfosResponse {
        info,
        pagination: page(pr),
    })
}
