//! celestia-app module queries: `celestia.mint.v1`, `celestia.blob.v1`,
//! `celestia.minfee.v1`, `celestia.signal.v1`, `celestia.fibre.v1`
//! (except `ValidatePaymentPromise`) and `celestia.valaddr.v1`.

use celestia_proto::celestia::{blob, fibre, minfee, mint, signal, valaddr};
use celestia_proto::cosmos::staking::v1beta1 as spb;
use math::Int;
use sdk::Context;
use sdk::SdkError;

use crate::Router;
use crate::error::{QResult, QueryError, invalid_arg};
use crate::pagination::range;
use crate::util::{VALCONS, bech32, decode, from_bech32_legacy, get};

pub(crate) fn register(r: &mut Router) {
    r.add("/celestia.mint.v1.Query/InflationRate", inflation_rate);
    r.add(
        "/celestia.mint.v1.Query/AnnualProvisions",
        annual_provisions,
    );
    r.add("/celestia.mint.v1.Query/GenesisTime", genesis_time);
    r.add("/celestia.blob.v1.Query/Params", blob_params);
    r.add(
        "/celestia.minfee.v1.Query/NetworkMinGasPrice",
        network_min_gas_price,
    );
    r.add("/celestia.minfee.v1.Query/Params", minfee_params);
    r.add("/celestia.signal.v1.Query/VersionTally", version_tally);
    r.add("/celestia.signal.v1.Query/GetUpgrade", get_upgrade);
    r.add(
        "/celestia.signal.v1.Query/GetMissingValidators",
        get_missing_validators,
    );
    r.add("/celestia.fibre.v1.Query/Params", fibre_params);
    r.add("/celestia.fibre.v1.Query/EscrowAccount", escrow_account);
    r.add("/celestia.fibre.v1.Query/Withdrawals", withdrawals);
    r.add(
        "/celestia.fibre.v1.Query/IsPaymentProcessed",
        is_payment_processed,
    );
    r.add(
        "/celestia.valaddr.v1.Query/FibreProviderInfo",
        fibre_provider_info,
    );
    r.add(
        "/celestia.valaddr.v1.Query/AllBondedFibreProviders",
        all_bonded_fibre_providers,
    );
}

fn minter(ctx: &mut Context<'_>) -> QResult<mint::v1::Minter> {
    get(ctx, x_mint::STORE_KEY, x_mint::KEY_MINTER)?
        .ok_or_else(|| QueryError::Plain("stored minter should not have been nil".into()))
}

fn inflation_rate(
    ctx: &mut Context<'_>,
    _: mint::v1::QueryInflationRateRequest,
) -> QResult<mint::v1::QueryInflationRateResponse> {
    Ok(mint::v1::QueryInflationRateResponse {
        inflation_rate: minter(ctx)?.inflation_rate.into_bytes(),
    })
}

fn annual_provisions(
    ctx: &mut Context<'_>,
    _: mint::v1::QueryAnnualProvisionsRequest,
) -> QResult<mint::v1::QueryAnnualProvisionsResponse> {
    Ok(mint::v1::QueryAnnualProvisionsResponse {
        annual_provisions: minter(ctx)?.annual_provisions.into_bytes(),
    })
}

fn genesis_time(
    ctx: &mut Context<'_>,
    _: mint::v1::QueryGenesisTimeRequest,
) -> QResult<mint::v1::QueryGenesisTimeResponse> {
    let gt: mint::v1::GenesisTime = get(ctx, x_mint::STORE_KEY, x_mint::KEY_GENESIS_TIME)?
        .ok_or_else(|| QueryError::Plain("stored genesis time should not have been nil".into()))?;
    Ok(mint::v1::QueryGenesisTimeResponse {
        genesis_time: gt.genesis_time,
    })
}

fn blob_params(
    ctx: &mut Context<'_>,
    _: blob::v1::QueryParamsRequest,
) -> QResult<blob::v1::QueryParamsResponse> {
    Ok(blob::v1::QueryParamsResponse {
        params: Some(x_blob::get_params(ctx)?),
    })
}

fn network_min_gas_price(
    ctx: &mut Context<'_>,
    _: minfee::v1::QueryNetworkMinGasPrice,
) -> QResult<minfee::v1::QueryNetworkMinGasPriceResponse> {
    Ok(minfee::v1::QueryNetworkMinGasPriceResponse {
        network_min_gas_price: x_minfee::get_params(ctx)?.to_proto().network_min_gas_price,
    })
}

fn minfee_params(
    ctx: &mut Context<'_>,
    _: minfee::v1::QueryParamsRequest,
) -> QResult<minfee::v1::QueryParamsResponse> {
    Ok(minfee::v1::QueryParamsResponse {
        params: Some(x_minfee::get_params(ctx)?.to_proto()),
    })
}

fn last_total_power(ctx: &mut Context<'_>) -> QResult<Int> {
    Ok(x_staking::store::get_last_total_power(ctx)?)
}

fn version_tally(
    ctx: &mut Context<'_>,
    req: signal::v1::QueryVersionTallyRequest,
) -> QResult<signal::v1::QueryVersionTallyResponse> {
    let total = last_total_power(ctx)?;
    let mut current = Int::zero();
    for (k, v) in range(
        ctx,
        x_signal::STORE_KEY,
        &[],
        Some(x_signal::FIRST_SIGNAL_KEY),
        None,
        false,
    )? {
        if k == x_signal::UPGRADE_KEY {
            continue;
        }
        let power = x_staking::store::get_last_validator_power(ctx, &k)?;
        if x_signal::version_from_bytes(&v) == req.version {
            current = current
                .checked_add_raw(power)
                .map_err(|e| QueryError::Plain(e.to_string()))?;
        }
    }
    let threshold = x_signal::threshold_fraction()
        .checked_mul_int(&total)
        .and_then(|d| d.ceil())
        .map_err(|e| QueryError::Plain(e.to_string()))?
        .truncate_int();
    let u = |i: &Int| i.to_u64().unwrap_or_default();
    Ok(signal::v1::QueryVersionTallyResponse {
        voting_power: u(&current),
        threshold_power: u(&threshold),
        total_voting_power: u(&total),
    })
}

fn get_upgrade(
    ctx: &mut Context<'_>,
    _: signal::v1::QueryGetUpgradeRequest,
) -> QResult<signal::v1::QueryGetUpgradeResponse> {
    Ok(signal::v1::QueryGetUpgradeResponse {
        upgrade: x_signal::get_upgrade(ctx)?,
    })
}

fn get_missing_validators(
    ctx: &mut Context<'_>,
    req: signal::v1::QueryGetMissingValidatorsRequest,
) -> QResult<signal::v1::QueryGetMissingValidatorsResponse> {
    let mut missing = Vec::new();
    for (_, v) in range(
        ctx,
        "staking",
        &[x_staking::keys::VALIDATORS],
        None,
        None,
        false,
    )? {
        let val: spb::Validator = decode(&v)?;
        let Ok(op) = crate::util::from_bech32_legacy(&val.operator_address, crate::util::VALOPER)
        else {
            continue;
        };
        let power = x_staking::store::get_last_validator_power(ctx, &op)?;
        if power <= 0 {
            continue;
        }
        let moniker = val.description.map(|d| d.moniker).unwrap_or_default();
        match ctx.get(x_signal::STORE_KEY, &op)? {
            None => missing.push(moniker),
            Some(b) if x_signal::version_from_bytes(&b) != req.version => missing.push(moniker),
            Some(_) => {}
        }
    }
    Ok(signal::v1::QueryGetMissingValidatorsResponse {
        missing_validators: missing,
    })
}

fn fibre_params(
    ctx: &mut Context<'_>,
    _: fibre::v1::QueryParamsRequest,
) -> QResult<fibre::v1::QueryParamsResponse> {
    Ok(fibre::v1::QueryParamsResponse {
        params: Some(x_fibre::get_params(ctx)?),
    })
}

fn escrow_account(
    ctx: &mut Context<'_>,
    req: fibre::v1::QueryEscrowAccountRequest,
) -> QResult<fibre::v1::QueryEscrowAccountResponse> {
    if req.signer.is_empty() {
        return Err(invalid_arg("signer address cannot be empty"));
    }
    let account = x_fibre::get_escrow_account(ctx, &req.signer)?;
    let found = account.is_some();
    // Go's zero `EscrowAccount` has non-nullable coins whose nil
    // `math.Int` amounts marshal as "0".
    let zero_coin = || celestia_proto::cosmos::base::v1beta1::Coin {
        denom: String::new(),
        amount: "0".into(),
    };
    let account = account.unwrap_or_else(|| fibre::v1::EscrowAccount {
        balance: Some(zero_coin()),
        available_balance: Some(zero_coin()),
        ..Default::default()
    });
    Ok(fibre::v1::QueryEscrowAccountResponse {
        found,
        escrow_account: Some(account),
    })
}

fn withdrawals(
    ctx: &mut Context<'_>,
    req: fibre::v1::QueryWithdrawalsRequest,
) -> QResult<fibre::v1::QueryWithdrawalsResponse> {
    if req.signer.is_empty() {
        return Err(invalid_arg("signer address cannot be empty"));
    }
    let prefix = [
        &[x_fibre::WITHDRAWALS_BY_SIGNER_PREFIX][..],
        req.signer.as_bytes(),
    ]
    .concat();
    let withdrawals = range(ctx, x_fibre::STORE_KEY, &prefix, None, None, false)?
        .into_iter()
        .map(|(_, v)| decode(&v))
        .collect::<QResult<_>>()?;
    Ok(fibre::v1::QueryWithdrawalsResponse {
        withdrawals,
        pagination: None,
    })
}

fn is_payment_processed(
    ctx: &mut Context<'_>,
    req: fibre::v1::QueryIsPaymentProcessedRequest,
) -> QResult<fibre::v1::QueryIsPaymentProcessedResponse> {
    if req.promise_hash.is_empty() {
        return Err(invalid_arg("payment promise hash cannot be empty"));
    }
    let key = [&[x_fibre::PROCESSED_BY_HASH_PREFIX][..], &req.promise_hash].concat();
    let processed: Option<fibre::v1::ProcessedPayment> = get(ctx, x_fibre::STORE_KEY, &key)?;
    Ok(fibre::v1::QueryIsPaymentProcessedResponse {
        found: processed.is_some(),
        processed_at: processed.map(|p| p.processed_at.unwrap_or_else(sdk::time::go_zero)),
    })
}

fn invalid_validator(msg: String) -> QueryError {
    QueryError::Sdk(SdkError::new("valaddr", 2, "invalid validator").wrap(msg))
}

fn fibre_provider_info(
    ctx: &mut Context<'_>,
    req: valaddr::v1::QueryFibreProviderInfoRequest,
) -> QResult<valaddr::v1::QueryFibreProviderInfoResponse> {
    let cons = from_bech32_legacy(&req.validator_consensus_address, VALCONS).map_err(|e| {
        invalid_validator(format!(
            "invalid consensus address: {}",
            e.go_error_string()
        ))
    })?;
    let info = x_valaddr::get_fibre_provider_info(ctx, &cons)?;
    Ok(valaddr::v1::QueryFibreProviderInfoResponse {
        found: info.is_some(),
        info: Some(info.unwrap_or_default()),
    })
}

fn all_bonded_fibre_providers(
    ctx: &mut Context<'_>,
    _: valaddr::v1::QueryAllBondedFibreProvidersRequest,
) -> QResult<valaddr::v1::QueryAllBondedFibreProvidersResponse> {
    let mut providers = Vec::new();
    let items = range(
        ctx,
        x_valaddr::STORE_KEY,
        &[x_valaddr::FIBRE_PROVIDER_INFO_PREFIX],
        None,
        None,
        false,
    )?;
    for (cons, v) in items {
        let Ok(info) = decode::<valaddr::v1::FibreProviderInfo>(&v) else {
            continue;
        };
        match x_staking::store::get_validator_by_cons_addr(ctx, &cons)? {
            Some(val) if val.is_bonded() => {}
            _ => continue,
        }
        providers.push(valaddr::v1::FibreProvider {
            validator_consensus_address: bech32(VALCONS, &cons),
            info: Some(info),
        });
    }
    Ok(valaddr::v1::QueryAllBondedFibreProvidersResponse { providers })
}
