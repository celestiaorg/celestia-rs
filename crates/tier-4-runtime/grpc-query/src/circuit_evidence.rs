//! `cosmos.circuit.v1.Query` (`cosmossdk.io/x/circuit` v0.1.1) and
//! `cosmos.evidence.v1beta1.Query` (celestia's `x/evidence` fork).

use celestia_proto::Any;
use celestia_proto::cosmos::circuit::v1 as cpb;
use celestia_proto::cosmos::evidence::v1beta1 as epb;
use sdk::Context;

use crate::Router;
use crate::error::{QResult, QueryError, invalid_arg, not_found};
use crate::pagination::{Collection, collection_paginate};
use crate::util::{ACC, bech32, codec_to_bytes, decode, get, go_hex_decode, page};

pub(crate) fn register(r: &mut Router) {
    r.add("/cosmos.circuit.v1.Query/Account", circuit_account);
    r.add("/cosmos.circuit.v1.Query/Accounts", circuit_accounts);
    r.add("/cosmos.circuit.v1.Query/DisabledList", disabled_list);
    r.add("/cosmos.evidence.v1beta1.Query/Evidence", evidence);
    r.add("/cosmos.evidence.v1beta1.Query/AllEvidence", all_evidence);
}

const CIRCUIT: &str = "circuit";

fn circuit_account(
    ctx: &mut Context<'_>,
    req: cpb::QueryAccountRequest,
) -> QResult<cpb::AccountResponse> {
    let addr = codec_to_bytes(&req.address, ACC)?;
    let key = [&[x_circuit::ACCOUNT_PERMISSION_PREFIX][..], &addr].concat();
    let Some(p) = get::<cpb::Permissions>(ctx, CIRCUIT, &key)? else {
        let hex: String = addr.iter().fold(String::new(), |mut s, b| {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02x}");
            s
        });
        return Err(QueryError::Plain(format!(
            "collections: not found: key 'hexBytes:{hex}' of type github.com/cosmos/gogoproto/cosmos.circuit.v1.Permissions"
        )));
    };
    Ok(cpb::AccountResponse {
        permission: Some(p),
    })
}

fn circuit_accounts(
    ctx: &mut Context<'_>,
    req: cpb::QueryAccountsRequest,
) -> QResult<cpb::AccountsResponse> {
    let (accounts, pr) = collection_paginate(
        ctx,
        Collection {
            store: CIRCUIT,
            prefix: &[x_circuit::ACCOUNT_PERMISSION_PREFIX],
        },
        req.pagination.as_ref(),
        None,
        |_, k, v| {
            Ok(cpb::GenesisAccountPermissions {
                address: bech32(ACC, k),
                permissions: Some(decode(v)?),
            })
        },
    )?;
    Ok(cpb::AccountsResponse {
        accounts,
        pagination: page(pr),
    })
}

fn disabled_list(
    ctx: &mut Context<'_>,
    _: cpb::QueryDisabledListRequest,
) -> QResult<cpb::DisabledListResponse> {
    let items = crate::pagination::range(
        ctx,
        CIRCUIT,
        &[x_circuit::DISABLE_LIST_PREFIX],
        None,
        None,
        false,
    )?;
    Ok(cpb::DisabledListResponse {
        disabled_list: items
            .into_iter()
            .map(|(k, _)| String::from_utf8_lossy(&k).into_owned())
            .collect(),
    })
}

const EVIDENCE: &str = "evidence";

fn evidence(
    ctx: &mut Context<'_>,
    req: epb::QueryEvidenceRequest,
) -> QResult<epb::QueryEvidenceResponse> {
    if req.hash.is_empty() {
        return Err(invalid_arg("invalid request; hash is empty"));
    }
    let hash = go_hex_decode(&req.hash)
        .map_err(|e| QueryError::Plain(format!("invalid evidence hash: {e}")))?;
    let key = [&[0x00u8][..], &hash].concat();
    let Some(any) = get::<Any>(ctx, EVIDENCE, &key)? else {
        return Err(not_found(format!("evidence {} not found", req.hash)));
    };
    Ok(epb::QueryEvidenceResponse {
        evidence: Some(any),
    })
}

fn all_evidence(
    ctx: &mut Context<'_>,
    req: epb::QueryAllEvidenceRequest,
) -> QResult<epb::QueryAllEvidenceResponse> {
    let (evidence, pr) = collection_paginate(
        ctx,
        Collection {
            store: EVIDENCE,
            prefix: &[0x00],
        },
        req.pagination.as_ref(),
        None,
        |_, _, v| decode::<Any>(v),
    )?;
    Ok(epb::QueryAllEvidenceResponse {
        evidence,
        pagination: page(pr),
    })
}
