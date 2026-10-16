//! `cosmos.auth.v1beta1.Query` (`x/auth/keeper/grpc_query.go`).

use celestia_proto::Any;
use celestia_proto::cosmos::auth::v1beta1::{
    AddressBytesToStringRequest, AddressBytesToStringResponse, AddressStringToBytesRequest,
    AddressStringToBytesResponse, BaseAccount, Bech32PrefixRequest, Bech32PrefixResponse,
    QueryAccountAddressByIdRequest, QueryAccountAddressByIdResponse, QueryAccountInfoRequest,
    QueryAccountInfoResponse, QueryAccountRequest, QueryAccountResponse, QueryAccountsRequest,
    QueryAccountsResponse, QueryModuleAccountByNameRequest, QueryModuleAccountByNameResponse,
    QueryModuleAccountsRequest, QueryModuleAccountsResponse, QueryParamsRequest,
    QueryParamsResponse,
};
use sdk::Context;

use crate::Router;
use crate::error::{QResult, QueryError, invalid_arg, not_found};
use crate::pagination::{Collection, collection_paginate};
use crate::util::{ACC, bech32, codec_to_bytes, decode};

const STORE: &str = "acc";
const ACCOUNTS: &[u8] = &[0x01];
const BY_NUMBER: &[u8] = b"accountNumber";

pub(crate) fn register(r: &mut Router) {
    r.add(
        "/cosmos.auth.v1beta1.Query/AccountAddressByID",
        account_address_by_id,
    );
    r.add("/cosmos.auth.v1beta1.Query/Accounts", accounts);
    r.add("/cosmos.auth.v1beta1.Query/Account", account);
    r.add("/cosmos.auth.v1beta1.Query/Params", params);
    r.add("/cosmos.auth.v1beta1.Query/ModuleAccounts", module_accounts);
    r.add(
        "/cosmos.auth.v1beta1.Query/ModuleAccountByName",
        module_account_by_name,
    );
    r.add("/cosmos.auth.v1beta1.Query/Bech32Prefix", bech32_prefix);
    r.add(
        "/cosmos.auth.v1beta1.Query/AddressBytesToString",
        address_bytes_to_string,
    );
    r.add(
        "/cosmos.auth.v1beta1.Query/AddressStringToBytes",
        address_string_to_bytes,
    );
    r.add("/cosmos.auth.v1beta1.Query/AccountInfo", account_info);
}

/// The stored `Any` of the account at `addr`.
fn stored_account(ctx: &mut Context<'_>, addr: &[u8]) -> QResult<Option<Any>> {
    let key = [ACCOUNTS, addr].concat();
    ctx.get(STORE, &key)?.map(|b| decode(&b)).transpose()
}

#[allow(deprecated)]
fn account_address_by_id(
    ctx: &mut Context<'_>,
    req: QueryAccountAddressByIdRequest,
) -> QResult<QueryAccountAddressByIdResponse> {
    if req.id != 0 {
        return Err(invalid_arg(
            "requesting with id isn't supported, try to request using account-id",
        ));
    }
    let key = [BY_NUMBER, &req.account_id.to_be_bytes()].concat();
    let Some(addr) = ctx.get(STORE, &key)? else {
        return Err(not_found(format!(
            "account address not found with account number {}",
            req.account_id
        )));
    };
    Ok(QueryAccountAddressByIdResponse {
        account_address: bech32(ACC, &addr),
    })
}

fn accounts(ctx: &mut Context<'_>, req: QueryAccountsRequest) -> QResult<QueryAccountsResponse> {
    let (accounts, pr) = collection_paginate(
        ctx,
        Collection {
            store: STORE,
            prefix: ACCOUNTS,
        },
        req.pagination.as_ref(),
        None,
        |_, _, v| decode::<Any>(v),
    )?;
    Ok(QueryAccountsResponse {
        accounts,
        pagination: Some(pr),
    })
}

fn account(ctx: &mut Context<'_>, req: QueryAccountRequest) -> QResult<QueryAccountResponse> {
    if req.address.is_empty() {
        return Err(invalid_arg("Address cannot be empty"));
    }
    let addr = codec_to_bytes(&req.address, ACC)?;
    let Some(account) = stored_account(ctx, &addr)? else {
        return Err(not_found(format!("account {} not found", req.address)));
    };
    Ok(QueryAccountResponse {
        account: Some(account),
    })
}

fn params(ctx: &mut Context<'_>, _: QueryParamsRequest) -> QResult<QueryParamsResponse> {
    Ok(QueryParamsResponse {
        params: Some(x_auth::get_params(ctx)?),
    })
}

fn module_account_any(ctx: &mut Context<'_>, name: &str) -> QResult<Any> {
    match x_auth::get_module_account(ctx, name)? {
        Some(m) => Ok(Any::pack(&m)),
        None => Err(not_found(format!("account {name} not found"))),
    }
}

fn module_accounts(
    ctx: &mut Context<'_>,
    _: QueryModuleAccountsRequest,
) -> QResult<QueryModuleAccountsResponse> {
    let mut names: Vec<&str> = x_auth::module_account_perms().map(|(n, _)| *n).collect();
    names.sort_unstable();
    let mut accounts = Vec::with_capacity(names.len());
    for name in names {
        accounts.push(module_account_any(ctx, name)?);
    }
    Ok(QueryModuleAccountsResponse { accounts })
}

fn module_account_by_name(
    ctx: &mut Context<'_>,
    req: QueryModuleAccountByNameRequest,
) -> QResult<QueryModuleAccountByNameResponse> {
    if req.name.is_empty() {
        return Err(invalid_arg("module name is empty"));
    }
    Ok(QueryModuleAccountByNameResponse {
        account: Some(module_account_any(ctx, &req.name)?),
    })
}

fn bech32_prefix(_: &mut Context<'_>, _: Bech32PrefixRequest) -> QResult<Bech32PrefixResponse> {
    Ok(Bech32PrefixResponse {
        bech32_prefix: ACC.into(),
    })
}

fn address_bytes_to_string(
    _: &mut Context<'_>,
    req: AddressBytesToStringRequest,
) -> QResult<AddressBytesToStringResponse> {
    if req.address_bytes.is_empty() {
        return Err(QueryError::Plain(
            "empty address bytes is not allowed".into(),
        ));
    }
    if req.address_bytes.len() > 255 {
        return Err(QueryError::Sdk(sdk::error::sdk::unknown_address(format!(
            "address max length is 255, got {}",
            req.address_bytes.len()
        ))));
    }
    Ok(AddressBytesToStringResponse {
        address_string: bech32(ACC, &req.address_bytes),
    })
}

fn address_string_to_bytes(
    _: &mut Context<'_>,
    req: AddressStringToBytesRequest,
) -> QResult<AddressStringToBytesResponse> {
    if req.address_string.trim().is_empty() {
        return Err(QueryError::Plain(
            "empty address string is not allowed".into(),
        ));
    }
    Ok(AddressStringToBytesResponse {
        address_bytes: codec_to_bytes(&req.address_string, ACC)?,
    })
}

fn account_info(
    ctx: &mut Context<'_>,
    req: QueryAccountInfoRequest,
) -> QResult<QueryAccountInfoResponse> {
    if req.address.is_empty() {
        return Err(invalid_arg("address cannot be empty"));
    }
    let addr = codec_to_bytes(&req.address, ACC)?;
    let Some(any) = stored_account(ctx, &addr)? else {
        return Err(not_found(format!("account {} not found", req.address)));
    };
    let acc = x_auth::Account::from_any(&any)?;
    Ok(QueryAccountInfoResponse {
        info: Some(BaseAccount {
            address: req.address,
            pub_key: acc.pub_key().cloned(),
            account_number: acc.account_number(),
            sequence: acc.sequence(),
        }),
    })
}
