//! Query services that need the application rather than a store view:
//! the SDK tx service (`Simulate`, `TxDecode`, `TxEncode`), the node
//! service, celestia's gas estimation, and the `/custom` proof routes.
//! Registered on the same `GRPCQueryRouter` as the module services in Go.

use celestia_proto::celestia_core_proto::abci::RequestQuery;
use celestia_proto::cosmos::base::abci::v1beta1::{GasInfo, Result as SdkResult};
use celestia_proto::cosmos::base::node::v1beta1::{
    ConfigRequest, ConfigResponse, StatusRequest, StatusResponse,
};
use celestia_proto::cosmos::tx::v1beta1::{
    SimulateRequest, SimulateResponse, Tx, TxDecodeAminoRequest, TxDecodeAminoResponse,
    TxDecodeRequest, TxDecodeResponse, TxEncodeAminoRequest, TxEncodeAminoResponse,
    TxEncodeRequest, TxEncodeResponse, TxRaw,
};
use grpc_query::{Code, QResult, QueryError};
use prost::Message;
use sdk::error::sdk as errs;
use sdk::{Context, ExecMode, SdkError};

use crate::query::QueryState;
use crate::{App, tx, txpipe};

/// An app-level query handler.
pub type AppHandler = fn(&mut App, &mut QueryState, &[u8]) -> QResult<Vec<u8>>;

macro_rules! typed {
    ($f:path, $req:ty) => {{
        fn h(app: &mut App, state: &mut QueryState, data: &[u8]) -> QResult<Vec<u8>> {
            let req = <$req>::decode(data).map_err(|e| grpc_query::decode_error(&e))?;
            Ok($f(app, state, req)?.encode_to_vec())
        }
        h as AppHandler
    }};
}

/// The handler of an app-level route.
#[must_use]
pub fn route(path: &str) -> Option<AppHandler> {
    Some(match path {
        "/cosmos.tx.v1beta1.Service/Simulate" => typed!(simulate_grpc, SimulateRequest),
        "/cosmos.tx.v1beta1.Service/TxDecode" => typed!(tx_decode, TxDecodeRequest),
        "/cosmos.tx.v1beta1.Service/TxEncode" => typed!(tx_encode, TxEncodeRequest),
        "/cosmos.tx.v1beta1.Service/TxDecodeAmino" => tx_decode_amino,
        "/cosmos.tx.v1beta1.Service/TxEncodeAmino" => tx_encode_amino,
        "/cosmos.base.node.v1beta1.Service/Config" => typed!(node_config, ConfigRequest),
        "/cosmos.base.node.v1beta1.Service/Status" => typed!(node_status, StatusRequest),
        _ => return None,
    })
}

/// `BaseApp.Simulate`: `runTx(execModeSimulate)` on a branch of the check
/// state that is discarded afterwards.
pub fn simulate(app: &mut App, tx_bytes: &[u8]) -> (GasInfo, Result<SdkResult, SdkError>) {
    let header = app.check_header.clone();
    let mut ctx = Context::new(&mut app.ms, header, ExecMode::Check);
    ctx.mode = ExecMode::Simulate;
    ctx.consensus_params = x_consensus::get_params(&mut ctx)
        .ok()
        .flatten()
        .unwrap_or_default();
    let cp = ctx.branch();
    // The tx decoder of the current app version (v9.0.8's registry at
    // app version 9).
    let app_version = ctx.consensus_params.version.as_ref().map_or(0, |v| v.app);
    let (mut p, res) = tx::run_ante(
        &mut ctx,
        tx_bytes,
        txpipe::decoder(app_version),
        txpipe::ante,
        true,
    );
    let result = match res {
        Ok(()) => tx::run_msgs(&mut ctx, &mut p, false).map(|r| {
            let msg_responses =
                celestia_proto::cosmos::base::abci::v1beta1::TxMsgData::decode(r.data.as_slice())
                    .map(|d| d.msg_responses)
                    .unwrap_or_default();
            // Go marks events for indexing only in CheckTx / FinalizeBlock
            // (`MarkEventsToIndex`); a simulation returns them unmarked.
            let mut events = r.events;
            for a in events.iter_mut().flat_map(|e| e.attributes.iter_mut()) {
                a.index = false;
            }
            #[allow(deprecated)]
            SdkResult {
                data: r.data,
                log: String::new(),
                events,
                msg_responses,
            }
        }),
        Err(e) => Err(e),
    };
    let gas = GasInfo {
        gas_wanted: p.gas_wanted,
        gas_used: p.gas.consumed(),
    };
    ctx.discard(cp);
    (gas, result)
}

/// `/app/simulate`: the JSON `SimulationResponse`.
pub fn simulate_json(app: &mut App, tx_bytes: &[u8]) -> Result<Vec<u8>, SdkError> {
    let (gas_info, result) = simulate(app, tx_bytes);
    let resp = celestia_proto::cosmos::base::abci::v1beta1::SimulationResponse {
        gas_info: Some(gas_info),
        result: Some(result?),
    };
    serde_json::to_vec(&resp).map_err(|e| errs::json_marshal(e.to_string()))
}

fn simulate_grpc(
    app: &mut App,
    _: &mut QueryState,
    req: SimulateRequest,
) -> QResult<SimulateResponse> {
    #[allow(deprecated)]
    let legacy_tx = req.tx;
    let tx_bytes = if req.tx_bytes.is_empty() {
        match &legacy_tx {
            Some(tx) => tx.encode_to_vec(),
            None => {
                return Err(QueryError::Status(
                    Code::InvalidArgument,
                    "empty txBytes is not allowed".into(),
                ));
            }
        }
    } else {
        req.tx_bytes
    };
    let (gas_info, result) = simulate(app, &tx_bytes);
    match result {
        Ok(result) => Ok(SimulateResponse {
            gas_info: Some(gas_info),
            result: Some(result),
        }),
        Err(e) => Err(QueryError::Status(
            Code::Unknown,
            format!(
                // Go formats the error with `%v`, which appends the Go
                // source location of the wrap (`[…/sigverify.go:290]`);
                // that diagnostic build identity is the documented Simulate
                // deviation in conformance/mainnet-rpc/simulation/README.md.
                "{} with gas used: '{}'",
                e.log, gas_info.gas_used
            ),
        )),
    }
}

fn tx_decode(_: &mut App, _: &mut QueryState, req: TxDecodeRequest) -> QResult<TxDecodeResponse> {
    let decode = if sdk::version::is_v9(sdk::version::app_version()) {
        txdecode::default_tx_decode_v9
    } else {
        txdecode::default_tx_decode
    };
    if req.tx_bytes.is_empty() {
        return Err(QueryError::Status(
            Code::InvalidArgument,
            "invalid empty tx bytes".into(),
        ));
    }
    let tx = decode(&req.tx_bytes)?;
    Ok(TxDecodeResponse {
        tx: Some(Tx {
            body: Some(tx.body),
            auth_info: Some(tx.auth_info),
            signatures: tx.signatures,
        }),
    })
}

fn amino_error(e: legacy_amino::Error) -> QueryError {
    match e {
        legacy_amino::Error::Go(s) => QueryError::Plain(s),
        legacy_amino::Error::Panic(s) => QueryError::Panic(s),
        legacy_amino::Error::Sdk(code, s) => QueryError::Sdk(SdkError::new("sdk", code, s)),
    }
}

/// Whether field 1 occurs in `data` (gogoproto sets a present but empty
/// bytes field to `[]byte{}`, which Go's `== nil` check lets through).
fn has_field_1(mut data: &[u8]) -> bool {
    use prost::encoding::{DecodeContext, decode_key, skip_field};
    while !data.is_empty() {
        let Ok((tag, wt)) = decode_key(&mut data) else {
            return false;
        };
        if tag == 1 {
            return true;
        }
        if skip_field(wt, tag, &mut data, DecodeContext::default()).is_err() {
            return false;
        }
    }
    false
}

/// `TxDecodeAmino`: the legacy amino codec of `StdTx` (`legacy-amino`).
fn tx_decode_amino(_: &mut App, _: &mut QueryState, data: &[u8]) -> QResult<Vec<u8>> {
    let req = TxDecodeAminoRequest::decode(data).map_err(|e| grpc_query::decode_error(&e))?;
    if req.amino_binary.is_empty() && !has_field_1(data) {
        return Err(QueryError::Status(
            Code::InvalidArgument,
            "invalid empty tx bytes".into(),
        ));
    }
    let schema = legacy_amino::Schema::for_app_version(sdk::version::app_version());
    let json = legacy_amino::tx_decode_amino_in(schema, &req.amino_binary).map_err(amino_error)?;
    Ok(TxDecodeAminoResponse { amino_json: json }.encode_to_vec())
}

/// `TxEncodeAmino`.
fn tx_encode_amino(_: &mut App, _: &mut QueryState, data: &[u8]) -> QResult<Vec<u8>> {
    let req = TxEncodeAminoRequest::decode(data).map_err(|e| grpc_query::decode_error(&e))?;
    if req.amino_json.is_empty() {
        return Err(QueryError::Status(
            Code::InvalidArgument,
            "invalid empty tx json".into(),
        ));
    }
    let schema = legacy_amino::Schema::for_app_version(sdk::version::app_version());
    let bz =
        legacy_amino::tx_encode_amino_in(schema, req.amino_json.as_bytes()).map_err(amino_error)?;
    Ok(TxEncodeAminoResponse { amino_binary: bz }.encode_to_vec())
}

fn tx_encode(_: &mut App, _: &mut QueryState, req: TxEncodeRequest) -> QResult<TxEncodeResponse> {
    let Some(tx) = req.tx else {
        return Err(QueryError::Status(
            Code::InvalidArgument,
            "invalid empty tx".into(),
        ));
    };
    let raw = TxRaw {
        body_bytes: tx.body.map(|b| b.encode_to_vec()).unwrap_or_default(),
        auth_info_bytes: tx.auth_info.map(|a| a.encode_to_vec()).unwrap_or_default(),
        signatures: tx.signatures,
    };
    Ok(TxEncodeResponse {
        tx_bytes: raw.encode_to_vec(),
    })
}

fn node_config(app: &mut App, _: &mut QueryState, _: ConfigRequest) -> QResult<ConfigResponse> {
    let pruning = app.ms.root().pruning();
    Ok(ConfigResponse {
        minimum_gas_price: app
            .node_min_gas_price
            .as_ref()
            .map(|p| format!("{p}utia"))
            .unwrap_or_default(),
        pruning_keep_recent: pruning.keep_recent.to_string(),
        pruning_interval: pruning.interval.to_string(),
        halt_height: 0,
    })
}

fn node_status(_: &mut App, state: &mut QueryState, _: StatusRequest) -> QResult<StatusResponse> {
    let h = &state.header;
    Ok(StatusResponse {
        earliest_store_height: 0,
        height: u64::try_from(h.height).unwrap_or_default(),
        timestamp: Some(h.time),
        app_hash: h.app_hash.clone(),
        validator_hash: h.next_validators_hash.clone(),
    })
}

/// Whether `/custom/<route>` is registered (`CustomQueryRouter`).
#[must_use]
pub fn has_custom_route(route: &str) -> bool {
    route == crate::inclusion_proof::TX_INCLUSION_QUERY_PATH
        || route == crate::inclusion_proof::SHARE_INCLUSION_QUERY_PATH
}

/// Runs a `/custom/<route>/<args...>` query.
pub fn custom(
    _app: &mut App,
    _state: &mut QueryState,
    route: &str,
    args: &[&str],
    req: &RequestQuery,
) -> Result<Vec<u8>, SdkError> {
    match route {
        crate::inclusion_proof::TX_INCLUSION_QUERY_PATH => {
            crate::inclusion_proof::query_tx_inclusion_proof(args, &req.data)
        }
        crate::inclusion_proof::SHARE_INCLUSION_QUERY_PATH => {
            crate::inclusion_proof::query_share_inclusion_proof(args, &req.data)
        }
        _ => Err(errs::unknown_request(format!(
            "no custom querier found for route {route}"
        ))),
    }
}
