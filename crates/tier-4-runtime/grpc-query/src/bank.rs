//! `cosmos.bank.v1beta1.Query` (`x/bank/keeper/grpc_query.go`).

use celestia_proto::cosmos::bank::v1beta1::{
    DenomOwner, Metadata, QueryAllBalancesRequest, QueryAllBalancesResponse, QueryBalanceRequest,
    QueryBalanceResponse, QueryDenomMetadataByQueryStringRequest,
    QueryDenomMetadataByQueryStringResponse, QueryDenomMetadataRequest, QueryDenomMetadataResponse,
    QueryDenomOwnersByQueryRequest, QueryDenomOwnersByQueryResponse, QueryDenomOwnersRequest,
    QueryDenomOwnersResponse, QueryDenomsMetadataRequest, QueryDenomsMetadataResponse,
    QueryParamsRequest, QueryParamsResponse, QuerySendEnabledRequest, QuerySendEnabledResponse,
    QuerySpendableBalanceByDenomRequest, QuerySpendableBalanceByDenomResponse,
    QuerySpendableBalancesRequest, QuerySpendableBalancesResponse, QuerySupplyOfRequest,
    QuerySupplyOfResponse, QueryTotalSupplyRequest, QueryTotalSupplyResponse, SendEnabled,
};
use celestia_proto::cosmos::base::v1beta1::Coin;
use sdk::Context;
use sdk::coins::Amount;

use crate::Router;
use crate::error::{QResult, QueryError, internal, invalid_arg, not_found};
use crate::pagination::{Collection, collection_paginate, paginate};
use crate::util::{ACC, bech32, codec_to_bytes, decode, get, page, validate_denom};

const STORE: &str = "bank";
const SUPPLY: &[u8] = &[0x00];
const DENOM_METADATA: &[u8] = &[0x01];
const BALANCES: &[u8] = &[0x02];
const DENOM_ADDRESS: &[u8] = &[0x03];
const SEND_ENABLED: &[u8] = &[0x04];

pub(crate) fn register(r: &mut Router) {
    r.add("/cosmos.bank.v1beta1.Query/Balance", balance);
    r.add("/cosmos.bank.v1beta1.Query/AllBalances", all_balances);
    r.add(
        "/cosmos.bank.v1beta1.Query/SpendableBalances",
        spendable_balances,
    );
    r.add(
        "/cosmos.bank.v1beta1.Query/SpendableBalanceByDenom",
        spendable_balance_by_denom,
    );
    r.add("/cosmos.bank.v1beta1.Query/TotalSupply", total_supply);
    r.add("/cosmos.bank.v1beta1.Query/SupplyOf", supply_of);
    r.add("/cosmos.bank.v1beta1.Query/Params", params);
    r.add("/cosmos.bank.v1beta1.Query/DenomMetadata", denom_metadata);
    r.add(
        "/cosmos.bank.v1beta1.Query/DenomMetadataByQueryString",
        denom_metadata_by_query_string,
    );
    r.add("/cosmos.bank.v1beta1.Query/DenomsMetadata", denoms_metadata);
    r.add("/cosmos.bank.v1beta1.Query/DenomOwners", denom_owners);
    r.add(
        "/cosmos.bank.v1beta1.Query/DenomOwnersByQuery",
        denom_owners_by_query,
    );
    r.add("/cosmos.bank.v1beta1.Query/SendEnabled", send_enabled);
}

fn coin(denom: &str, amount: &Amount) -> Coin {
    Coin {
        denom: denom.to_owned(),
        amount: amount.to_string(),
    }
}

fn parse_amount(b: &[u8]) -> QResult<Amount> {
    let s = std::str::from_utf8(b).map_err(|e| QueryError::Plain(e.to_string()))?;
    Amount::parse(s).map_err(|e| QueryError::Plain(e.to_string()))
}

fn address(s: &str) -> QResult<Vec<u8>> {
    codec_to_bytes(s, ACC)
        .map_err(|e| invalid_arg(format!("invalid address: {}", e.go_error_string())))
}

fn balance_amount(ctx: &mut Context<'_>, addr: &[u8], denom: &str) -> QResult<Amount> {
    let key = [
        BALANCES,
        &sdk::codec::length_prefixed(addr),
        denom.as_bytes(),
    ]
    .concat();
    ctx.get(STORE, &key)?
        .map_or_else(|| Ok(Amount::zero()), |b| parse_amount(&b))
}

fn get_metadata(ctx: &mut Context<'_>, denom: &str) -> QResult<Option<Metadata>> {
    get(ctx, STORE, &[DENOM_METADATA, denom.as_bytes()].concat())
}

fn balance(ctx: &mut Context<'_>, req: QueryBalanceRequest) -> QResult<QueryBalanceResponse> {
    validate_denom(&req.denom)?;
    let addr = address(&req.address)?;
    let amount = balance_amount(ctx, &addr, &req.denom)?;
    Ok(QueryBalanceResponse {
        balance: Some(coin(&req.denom, &amount)),
    })
}

fn paginate_err(e: QueryError) -> QueryError {
    invalid_arg(format!("paginate: {}", e.go_error_string()))
}

fn all_balances(
    ctx: &mut Context<'_>,
    req: QueryAllBalancesRequest,
) -> QResult<QueryAllBalancesResponse> {
    let addr = address(&req.address)?;
    let prefix = sdk::codec::length_prefixed(&addr);
    let (balances, pr) = collection_paginate(
        ctx,
        Collection {
            store: STORE,
            prefix: BALANCES,
        },
        req.pagination.as_ref(),
        Some(&prefix),
        |ctx, k, v| {
            let denom = String::from_utf8_lossy(&k[prefix.len()..]).into_owned();
            let amount = parse_amount(v)?;
            if req.resolve_denom {
                if let Some(m) = get_metadata(ctx, &denom)? {
                    return Ok(coin(&m.display, &amount));
                }
            }
            Ok(coin(&denom, &amount))
        },
    )
    .map_err(paginate_err)?;
    Ok(QueryAllBalancesResponse {
        balances,
        pagination: page(pr),
    })
}

fn spendable_balances(
    ctx: &mut Context<'_>,
    req: QuerySpendableBalancesRequest,
) -> QResult<QuerySpendableBalancesResponse> {
    let addr = address(&req.address)?;
    let prefix = sdk::codec::length_prefixed(&addr);
    let (denoms, pr) = collection_paginate(
        ctx,
        Collection {
            store: STORE,
            prefix: BALANCES,
        },
        req.pagination.as_ref(),
        Some(&prefix),
        |_, k, _| Ok(String::from_utf8_lossy(&k[prefix.len()..]).into_owned()),
    )
    .map_err(paginate_err)?;
    let spendable = x_bank::spendable_coins(ctx, &addr)?;
    Ok(QuerySpendableBalancesResponse {
        balances: denoms
            .iter()
            .map(|d| coin(d, &spendable.amount_of(d)))
            .collect(),
        pagination: page(pr),
    })
}

fn spendable_balance_by_denom(
    ctx: &mut Context<'_>,
    req: QuerySpendableBalanceByDenomRequest,
) -> QResult<QuerySpendableBalanceByDenomResponse> {
    let addr = address(&req.address)?;
    validate_denom(&req.denom)?;
    let balance = balance_amount(ctx, &addr, &req.denom)?;
    let locked = x_bank::locked_coins(ctx, &addr)?.amount_of(&req.denom);
    let amount = balance
        .checked_sub(&locked)
        .ok()
        .filter(|a| !a.is_negative())
        .unwrap_or_else(Amount::zero);
    Ok(QuerySpendableBalanceByDenomResponse {
        balance: Some(coin(&req.denom, &amount)),
    })
}

fn total_supply(
    ctx: &mut Context<'_>,
    req: QueryTotalSupplyRequest,
) -> QResult<QueryTotalSupplyResponse> {
    let (supply, pr) = collection_paginate(
        ctx,
        Collection {
            store: STORE,
            prefix: SUPPLY,
        },
        req.pagination.as_ref(),
        None,
        |_, k, v| Ok((String::from_utf8_lossy(k).into_owned(), parse_amount(v)?)),
    )
    .map_err(|e| internal(e.go_error_string()))?;
    // sdk.NewCoins: drop zero amounts (already sorted by denom).
    let supply = supply
        .into_iter()
        .filter(|(_, a)| !a.is_zero())
        .map(|(d, a)| coin(&d, &a))
        .collect();
    Ok(QueryTotalSupplyResponse {
        supply,
        pagination: page(pr),
    })
}

fn supply_of(ctx: &mut Context<'_>, req: QuerySupplyOfRequest) -> QResult<QuerySupplyOfResponse> {
    validate_denom(&req.denom)?;
    let amount = ctx
        .get(STORE, &[SUPPLY, req.denom.as_bytes()].concat())?
        .map_or_else(|| Ok(Amount::zero()), |b| parse_amount(&b))?;
    Ok(QuerySupplyOfResponse {
        amount: Some(coin(&req.denom, &amount)),
    })
}

fn params(ctx: &mut Context<'_>, _: QueryParamsRequest) -> QResult<QueryParamsResponse> {
    Ok(QueryParamsResponse {
        params: Some(x_bank::get_params(ctx)?),
    })
}

fn denom_metadata(
    ctx: &mut Context<'_>,
    req: QueryDenomMetadataRequest,
) -> QResult<QueryDenomMetadataResponse> {
    validate_denom(&req.denom)?;
    let Some(metadata) = get_metadata(ctx, &req.denom)? else {
        return Err(not_found(format!(
            "client metadata for denom {}",
            req.denom
        )));
    };
    Ok(QueryDenomMetadataResponse {
        metadata: Some(metadata),
    })
}

fn denom_metadata_by_query_string(
    ctx: &mut Context<'_>,
    req: QueryDenomMetadataByQueryStringRequest,
) -> QResult<QueryDenomMetadataByQueryStringResponse> {
    let r = denom_metadata(ctx, QueryDenomMetadataRequest { denom: req.denom })?;
    Ok(QueryDenomMetadataByQueryStringResponse {
        metadata: r.metadata,
    })
}

fn denoms_metadata(
    ctx: &mut Context<'_>,
    req: QueryDenomsMetadataRequest,
) -> QResult<QueryDenomsMetadataResponse> {
    let mut metadatas = Vec::new();
    let pr = paginate(
        ctx,
        STORE,
        DENOM_METADATA,
        req.pagination.as_ref(),
        |_, _, v| {
            metadatas.push(decode(v)?);
            Ok(())
        },
    )
    .map_err(|e| internal(e.go_error_string()))?;
    Ok(QueryDenomsMetadataResponse {
        metadatas,
        pagination: page(pr),
    })
}

fn denom_owners(
    ctx: &mut Context<'_>,
    req: QueryDenomOwnersRequest,
) -> QResult<QueryDenomOwnersResponse> {
    validate_denom(&req.denom)?;
    let prefix = sdk::codec::string_non_terminal(&req.denom);
    let (denom_owners, pr) = collection_paginate(
        ctx,
        Collection {
            store: STORE,
            prefix: DENOM_ADDRESS,
        },
        req.pagination.as_ref(),
        Some(&prefix),
        |ctx, k, _| {
            let rest = &k[prefix.len()..];
            let addr = rest.get(1..).unwrap_or_default();
            let amount = balance_amount(ctx, addr, &req.denom)?;
            Ok(DenomOwner {
                address: bech32(ACC, addr),
                balance: Some(coin(&req.denom, &amount)),
            })
        },
    )?;
    Ok(QueryDenomOwnersResponse {
        denom_owners,
        pagination: page(pr),
    })
}

fn denom_owners_by_query(
    ctx: &mut Context<'_>,
    req: QueryDenomOwnersByQueryRequest,
) -> QResult<QueryDenomOwnersByQueryResponse> {
    let r = denom_owners(
        ctx,
        QueryDenomOwnersRequest {
            denom: req.denom,
            pagination: req.pagination,
        },
    )?;
    Ok(QueryDenomOwnersByQueryResponse {
        denom_owners: r.denom_owners,
        pagination: r.pagination,
    })
}

fn bool_value(b: &[u8]) -> bool {
    // google.protobuf.BoolValue
    b == [0x08, 0x01]
}

fn send_enabled(
    ctx: &mut Context<'_>,
    req: QuerySendEnabledRequest,
) -> QResult<QuerySendEnabledResponse> {
    if !req.denoms.is_empty() {
        let mut send_enabled = Vec::new();
        for denom in &req.denoms {
            if let Some(v) = ctx.get(STORE, &[SEND_ENABLED, denom.as_bytes()].concat())? {
                send_enabled.push(SendEnabled {
                    denom: denom.clone(),
                    enabled: bool_value(&v),
                });
            }
        }
        return Ok(QuerySendEnabledResponse {
            send_enabled,
            pagination: None,
        });
    }
    let (send_enabled, pr) = collection_paginate(
        ctx,
        Collection {
            store: STORE,
            prefix: SEND_ENABLED,
        },
        req.pagination.as_ref(),
        None,
        |_, k, v| {
            Ok(SendEnabled {
                denom: String::from_utf8_lossy(k).into_owned(),
                enabled: bool_value(v),
            })
        },
    )
    .map_err(|e| internal(e.go_error_string()))?;
    Ok(QuerySendEnabledResponse {
        send_enabled,
        pagination: page(pr),
    })
}
