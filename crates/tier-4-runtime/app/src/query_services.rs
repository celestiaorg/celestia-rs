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
    sdk::version::set_app_version(app.execution_version());
    let next = app.pending_next_sequence_reader();
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
    let app_version =
        sdk::version::executing(ctx.consensus_params.version.as_ref().map_or(0, |v| v.app));
    let (mut p, res) = tx::run_ante(
        &mut ctx,
        tx_bytes,
        txpipe::decoder(app_version),
        |ctx, tx, _| {
            x_ante::ante_simulate(ctx, tx, &txpipe::ante_deps(None), next.as_deref())
                .map(|out| out.gas_wanted)
                .map_err(|err| (err.error, err.gas_wanted))
        },
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
    let decode = if sdk::version::release(0).v9_registry() {
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use celestia_proto::Any;
    use celestia_proto::cosmos::auth::v1beta1::{BaseAccount, Params as AuthParams};
    use celestia_proto::cosmos::bank::v1beta1::{MsgSend, Params as BankParams};
    use celestia_proto::cosmos::base::v1beta1::Coin;
    use celestia_proto::cosmos::tx::v1beta1::{
        AuthInfo, Fee, ModeInfo, SignerInfo, TxBody, mode_info,
    };

    use super::*;

    const SIGNER: [u8; 20] = [1; 20];

    fn simulate_send(sequence: u64) -> Vec<u8> {
        TxRaw {
            body_bytes: TxBody {
                messages: vec![Any::pack(&MsgSend {
                    from_address: x_auth::bech32(&SIGNER).unwrap(),
                    to_address: x_auth::bech32(&[2; 20]).unwrap(),
                    amount: vec![Coin {
                        denom: "utia".into(),
                        amount: "1".into(),
                    }],
                })],
                ..Default::default()
            }
            .encode_to_vec(),
            auth_info_bytes: AuthInfo {
                signer_infos: vec![SignerInfo {
                    mode_info: Some(ModeInfo {
                        sum: Some(mode_info::Sum::Single(mode_info::Single { mode: 1 })),
                    }),
                    sequence,
                    ..Default::default()
                }],
                fee: Some(Fee::default()),
                ..Default::default()
            }
            .encode_to_vec(),
            signatures: vec![vec![0; 64]],
        }
        .encode_to_vec()
    }

    #[test]
    fn tx_decode_query_keeps_the_legacy_decoder_at_v12() {
        let mut raw = TxRaw::decode(simulate_send(0).as_slice()).unwrap();
        for memo in ["first", "second"] {
            TxBody {
                memo: memo.into(),
                ..Default::default()
            }
            .encode(&mut raw.body_bytes)
            .unwrap();
        }
        let tx_bytes = raw.encode_to_vec();
        assert!(txdecode::decode_for(12, &tx_bytes).is_err());

        let mut app = App::new();
        app.last_height = app.ms.commit().unwrap().version;
        app.execution_version = Some(12);
        let (data, _) = app
            .grpc_query(
                "/cosmos.tx.v1beta1.Service/TxDecode",
                &TxDecodeRequest { tx_bytes }.encode_to_vec(),
                0,
            )
            .unwrap();
        let decoded = TxDecodeResponse::decode(data.as_slice()).unwrap();
        assert_eq!(decoded.tx.unwrap().body.unwrap().memo, "second");
    }

    #[test]
    fn simulate_uses_pending_sequence_without_mutating_check_state() {
        let mut app = App::new();
        app.execution_version = Some(12);
        app.set_pending_sequencer(|signer| (signer == SIGNER).then_some(5));
        {
            let mut ctx = Context::new(&mut app.ms, sdk::Header::default(), ExecMode::Check);
            x_auth::set_params(
                &mut ctx,
                &AuthParams {
                    max_memo_characters: 256,
                    tx_sig_limit: 7,
                    tx_size_cost_per_byte: 10,
                    sig_verify_cost_secp256k1: 1000,
                    ..Default::default()
                },
            )
            .unwrap();
            let account_number = x_auth::next_account_number(&mut ctx).unwrap();
            x_auth::set_account(
                &mut ctx,
                &x_auth::Account::Base(BaseAccount {
                    address: x_auth::bech32(&SIGNER).unwrap(),
                    account_number,
                    sequence: 3,
                    ..Default::default()
                }),
            )
            .unwrap();
            x_bank::set_params(
                &mut ctx,
                BankParams {
                    default_send_enabled: true,
                    ..Default::default()
                },
            )
            .unwrap();
            x_blob::set_params(&mut ctx, &x_blob::default_params()).unwrap();
            x_bank::set_balance(&mut ctx, &SIGNER, &sdk::coins::Coin::new("utia", 10)).unwrap();
        }

        for sequence in [5, 7] {
            let (_, result) = simulate(&mut app, &simulate_send(sequence));
            let error = result.unwrap_err();
            assert_eq!((error.codespace, error.code), ("sdk", 32));
            assert!(
                error
                    .log
                    .contains(&format!("sequence mismatch, expected 6, got {sequence}")),
                "expected a sequence mismatch for sequence {sequence}"
            );
        }
        let (_, result) = simulate(&mut app, &simulate_send(6));
        assert!(result.is_ok(), "{result:?}");
        let mut ctx = Context::new(&mut app.ms, sdk::Header::default(), ExecMode::Check);
        let account = x_auth::get_account(&mut ctx, &SIGNER).unwrap().unwrap();
        assert_eq!(account.sequence(), 3);
        assert!(account.pub_key().is_none());
        assert!(x_auth::get_account(&mut ctx, &[2; 20]).unwrap().is_none());
    }
}
