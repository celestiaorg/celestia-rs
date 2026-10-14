//! Query services backed by celestia-core RPC (`clientCtx.Client` in Go):
//! `cosmos.base.tendermint.v1beta1.Service` (cmtservice), the tx-service
//! methods that read the block store and tx index or broadcast
//! (`GetTx`, `GetTxsEvent`, `GetBlockWithTxs`, `BroadcastTx`),
//! `celestia.core.v1.tx.Tx` (tx status) and
//! `celestia.core.v1.gas_estimation.GasEstimator`.
//!
//! They are registered on the same query router as every other service,
//! but run outside the application lock: an RPC call (e.g.
//! `broadcast_tx_sync`) can make celestia-core call back into the app
//! (`CheckTx`) before it returns. Only the state reads (query context,
//! simulation, nested ABCI queries) take the lock, briefly.

use celestia_proto::Any;
use celestia_proto::celestia::core::v1::{gas_estimation as gepb, tx as ctxpb};
use celestia_proto::celestia_core_proto::abci::{RequestQuery, ResponseQuery};
use celestia_proto::celestia_core_proto::tendermint::abci as abcipb;
use celestia_proto::celestia_core_proto::tendermint::p2p as p2ppb;
use celestia_proto::celestia_core_proto::tendermint::types as tpb;
use celestia_proto::cosmos::base::abci::v1beta1::{
    AbciMessageLog, Attribute, StringEvent, TxResponse,
};
use celestia_proto::cosmos::base::query::v1beta1::PageResponse;
use celestia_proto::cosmos::base::tendermint::v1beta1 as cmt;
use celestia_proto::cosmos::tx::v1beta1 as txpb;
use grpc_query::{Code, QResult, QueryError};
use prost::Message;
use sdk::error::sdk as errs;
use serde_json::{Map, Value};

use crate::comet::{self, Client};
use crate::query::GrpcQueryError;
use crate::{AbciApp, App};

/// The service methods answered here.
const METHODS: &[&str] = &[
    "/cosmos.base.tendermint.v1beta1.Service/GetNodeInfo",
    "/cosmos.base.tendermint.v1beta1.Service/GetSyncing",
    "/cosmos.base.tendermint.v1beta1.Service/GetLatestBlock",
    "/cosmos.base.tendermint.v1beta1.Service/GetBlockByHeight",
    "/cosmos.base.tendermint.v1beta1.Service/GetLatestValidatorSet",
    "/cosmos.base.tendermint.v1beta1.Service/GetValidatorSetByHeight",
    "/cosmos.base.tendermint.v1beta1.Service/ABCIQuery",
    "/cosmos.tx.v1beta1.Service/GetTx",
    "/cosmos.tx.v1beta1.Service/GetTxsEvent",
    "/cosmos.tx.v1beta1.Service/GetBlockWithTxs",
    "/cosmos.tx.v1beta1.Service/BroadcastTx",
    "/celestia.core.v1.tx.Tx/TxStatus",
    "/celestia.core.v1.tx.Tx/TxStatusBatch",
    "/celestia.core.v1.gas_estimation.GasEstimator/EstimateGasPrice",
    "/celestia.core.v1.gas_estimation.GasEstimator/EstimateGasPriceAndUsage",
];

/// Whether `path` is one of these services' methods.
#[must_use]
pub fn handles(path: &str) -> bool {
    METHODS.contains(&path)
}

/// What a handler sees: the app (locked on demand) and the height of the
/// query context.
struct Env<'a> {
    app: &'a AbciApp,
    height: i64,
}

impl Env<'_> {
    fn with_app<R>(&self, f: impl FnOnce(&mut App) -> R) -> QResult<R> {
        self.app
            .with_app(f)
            .ok_or_else(|| QueryError::Status(Code::Internal, "app lock poisoned".into()))
    }
}

fn decode<M: Message + Default>(data: &[u8]) -> QResult<M> {
    M::decode(data).map_err(|e| grpc_query::decode_error(&e))
}

/// Dispatches `path`.
fn dispatch(env: &Env<'_>, path: &str, data: &[u8]) -> QResult<Vec<u8>> {
    let method = path.rsplit('/').next().unwrap_or_default();
    Ok(match (path.split('/').nth(1).unwrap_or_default(), method) {
        ("cosmos.base.tendermint.v1beta1.Service", "GetNodeInfo") => {
            get_node_info()?.encode_to_vec()
        }
        ("cosmos.base.tendermint.v1beta1.Service", "GetSyncing") => get_syncing()?.encode_to_vec(),
        ("cosmos.base.tendermint.v1beta1.Service", "GetLatestBlock") => {
            get_latest_block()?.encode_to_vec()
        }
        ("cosmos.base.tendermint.v1beta1.Service", "GetBlockByHeight") => {
            get_block_by_height(decode(data)?)?.encode_to_vec()
        }
        ("cosmos.base.tendermint.v1beta1.Service", "GetLatestValidatorSet") => {
            get_latest_validator_set(decode(data)?)?.encode_to_vec()
        }
        ("cosmos.base.tendermint.v1beta1.Service", "GetValidatorSetByHeight") => {
            get_validator_set_by_height(decode(data)?)?.encode_to_vec()
        }
        ("cosmos.base.tendermint.v1beta1.Service", "ABCIQuery") => {
            abci_query_service(env, decode(data)?)?.encode_to_vec()
        }
        ("cosmos.tx.v1beta1.Service", "GetTx") => get_tx(decode(data)?)?.encode_to_vec(),
        ("cosmos.tx.v1beta1.Service", "GetTxsEvent") => {
            get_txs_event(decode(data)?)?.encode_to_vec()
        }
        ("cosmos.tx.v1beta1.Service", "GetBlockWithTxs") => {
            get_block_with_txs(env, decode(data)?)?.encode_to_vec()
        }
        ("cosmos.tx.v1beta1.Service", "BroadcastTx") => {
            broadcast_tx(decode(data)?)?.encode_to_vec()
        }
        ("celestia.core.v1.tx.Tx", "TxStatus") => tx_status(decode(data)?)?.encode_to_vec(),
        ("celestia.core.v1.tx.Tx", "TxStatusBatch") => {
            tx_status_batch(decode(data)?)?.encode_to_vec()
        }
        ("celestia.core.v1.gas_estimation.GasEstimator", "EstimateGasPrice") => {
            estimate_gas_price(env, decode(data)?)?.encode_to_vec()
        }
        ("celestia.core.v1.gas_estimation.GasEstimator", "EstimateGasPriceAndUsage") => {
            estimate_gas_price_and_usage(env, decode(data)?)?.encode_to_vec()
        }
        _ => return Err(QueryError::Status(Code::Unimplemented, path.to_owned())),
    })
}

/// The gRPC path: `CreateQueryContext(height)` under the lock, then the
/// handler without it. `None` for other methods.
pub fn grpc_query(
    app: &AbciApp,
    path: &str,
    data: &[u8],
    height: i64,
) -> Option<Result<(Vec<u8>, i64), GrpcQueryError>> {
    if !handles(path) {
        return None;
    }
    let h = match app.with_app(|a| {
        a.create_query_context(height, false)
            .map(|s| s.header.height)
    }) {
        None => {
            return Some(Err(GrpcQueryError::Handler(QueryError::Status(
                Code::Internal,
                "app lock poisoned".into(),
            ))));
        }
        Some(Err(e)) => return Some(Err(GrpcQueryError::Context(e))),
        Some(Ok(h)) => h,
    };
    let env = Env { app, height: h };
    Some(
        dispatch(&env, path, data)
            .map(|v| (v, h))
            .map_err(GrpcQueryError::Handler),
    )
}

/// The ABCI `Query` path (`handleQueryGRPC`). `None` for other paths.
pub fn abci_query(app: &AbciApp, req: &RequestQuery) -> Option<ResponseQuery> {
    // `BaseApp.Query` refuses BroadcastTx before routing.
    if !handles(&req.path) || req.path == "/cosmos.tx.v1beta1.Service/BroadcastTx" {
        return None;
    }
    let (height, ctx) = app.with_app(|a| {
        let h = if req.height == 0 {
            a.last_block_height()
        } else {
            req.height
        };
        (h, a.create_query_context(h, req.prove).map(|_| ()))
    })?;
    if let Err(e) = ctx {
        return Some(crate::query::query_result(&e));
    }
    let env = Env { app, height };
    Some(match dispatch(&env, &req.path, &req.data) {
        Ok(value) => ResponseQuery {
            height,
            value,
            ..ResponseQuery::default()
        },
        Err(e) if e.is_panic() => crate::query::query_result(&e.to_sdk()),
        Err(e) => ResponseQuery {
            height,
            ..crate::query::query_result(&e.to_sdk())
        },
    })
}

// ---------------------------------------------------------------------------
// RPC helpers.

fn node() -> QResult<Client> {
    comet::client().map_err(QueryError::Plain)
}

fn call(method: &str, params: Map<String, Value>) -> QResult<Value> {
    node()?.call(method, params).map_err(QueryError::Plain)
}

fn params(kv: &[(&str, Value)]) -> Map<String, Value> {
    kv.iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

fn conv<T>(r: Result<T, String>) -> QResult<T> {
    r.map_err(QueryError::Plain)
}

fn status() -> QResult<Value> {
    call("status", Map::new())
}

fn latest_height(status: &Value) -> QResult<i64> {
    conv(comet::int(&status["sync_info"]["latest_block_height"]))
}

fn block_at(height: Option<i64>) -> QResult<(tpb::BlockId, tpb::Block)> {
    let p = height.map_or_else(Map::new, |h| params(&[("height", comet::int_param(h))]));
    conv(comet::result_block(&call("block", p)?))
}

// ---------------------------------------------------------------------------
// cmtservice.

fn sdk_block(b: &tpb::Block) -> cmt::Block {
    let h = b.header.clone().unwrap_or_default();
    cmt::Block {
        header: Some(cmt::Header {
            version: h.version,
            chain_id: h.chain_id,
            height: h.height,
            time: h.time,
            last_block_id: h.last_block_id,
            last_commit_hash: h.last_commit_hash,
            data_hash: h.data_hash,
            validators_hash: h.validators_hash,
            next_validators_hash: h.next_validators_hash,
            consensus_hash: h.consensus_hash,
            app_hash: h.app_hash,
            last_results_hash: h.last_results_hash,
            evidence_hash: h.evidence_hash,
            proposer_address: grpc_query::util::bech32(
                grpc_query::util::VALCONS,
                &h.proposer_address,
            ),
        }),
        data: b.data.clone(),
        evidence: b.evidence.clone(),
        last_commit: b.last_commit.clone(),
    }
}

fn get_syncing() -> QResult<cmt::GetSyncingResponse> {
    let s = status()?;
    Ok(cmt::GetSyncingResponse {
        syncing: s["sync_info"]["catching_up"].as_bool().unwrap_or(false),
    })
}

fn get_node_info() -> QResult<cmt::GetNodeInfoResponse> {
    let s = status()?;
    let ni = &s["node_info"];
    let pv = &ni["protocol_version"];
    Ok(cmt::GetNodeInfoResponse {
        default_node_info: Some(p2ppb::DefaultNodeInfo {
            protocol_version: Some(p2ppb::ProtocolVersion {
                p2p: conv(comet::uint(&pv["p2p"]))?,
                block: conv(comet::uint(&pv["block"]))?,
                app: conv(comet::uint(&pv["app"]))?,
            }),
            default_node_id: ni["id"].as_str().unwrap_or_default().to_owned(),
            listen_addr: ni["listen_addr"].as_str().unwrap_or_default().to_owned(),
            network: ni["network"].as_str().unwrap_or_default().to_owned(),
            version: ni["version"].as_str().unwrap_or_default().to_owned(),
            channels: conv(comet::hex_bytes(&ni["channels"]))?,
            moniker: ni["moniker"].as_str().unwrap_or_default().to_owned(),
            other: Some(p2ppb::DefaultNodeInfoOther {
                tx_index: ni["other"]["tx_index"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                rpc_address: ni["other"]["rpc_address"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            }),
        }),
        // Go reports its own build (`version.NewInfo()`: Go toolchain and
        // module list); this is the Rust node's.
        application_version: Some(cmt::VersionInfo {
            name: "celestia-app".into(),
            app_name: "celestia-rs".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            ..cmt::VersionInfo::default()
        }),
    })
}

fn get_latest_block() -> QResult<cmt::GetLatestBlockResponse> {
    let (id, block) = block_at(None)?;
    Ok(cmt::GetLatestBlockResponse {
        sdk_block: Some(sdk_block(&block)),
        block_id: Some(id),
        block: Some(block),
    })
}

fn too_high() -> QueryError {
    QueryError::Status(
        Code::InvalidArgument,
        "requested block height is bigger then the chain length".into(),
    )
}

fn get_block_by_height(
    req: cmt::GetBlockByHeightRequest,
) -> QResult<cmt::GetBlockByHeightResponse> {
    let latest = latest_height(&status()?)?;
    if req.height > latest {
        return Err(too_high());
    }
    let (id, block) = block_at(Some(req.height))?;
    Ok(cmt::GetBlockByHeightResponse {
        sdk_block: Some(sdk_block(&block)),
        block_id: Some(id),
        block: Some(block),
    })
}

/// `query.ParsePagination`: (page, limit).
fn parse_pagination(
    p: Option<&celestia_proto::cosmos::base::query::v1beta1::PageRequest>,
) -> QResult<(i64, i64)> {
    let (offset, mut limit) = p.map_or((0i64, 100i64), |p| {
        (
            i64::try_from(p.offset).unwrap_or(-1),
            i64::try_from(p.limit).unwrap_or(-1),
        )
    });
    if offset < 0 {
        return Err(QueryError::Status(
            Code::InvalidArgument,
            "offset must greater than 0".into(),
        ));
    }
    if limit < 0 {
        return Err(QueryError::Status(
            Code::InvalidArgument,
            "limit must greater than 0".into(),
        ));
    }
    if limit == 0 {
        limit = 100;
    }
    Ok((offset / limit + 1, limit))
}

/// `cryptocodec.FromCmtPubKeyInterface` packed in an `Any`.
fn pub_key_any(v: &Value) -> QResult<Any> {
    let key = conv(comet::b64(&v["value"]))?;
    let type_url = match v["type"].as_str() {
        Some("tendermint/PubKeyEd25519") => "/cosmos.crypto.ed25519.PubKey",
        Some("tendermint/PubKeySecp256k1") => "/cosmos.crypto.secp256k1.PubKey",
        other => {
            return Err(QueryError::Plain(format!(
                "cryptotypes.PubKey: cannot convert {} to cryptotypes.PubKey",
                other.unwrap_or_default()
            )));
        }
    };
    // Both are `message PubKey { bytes key = 1; }`.
    let mut value = Vec::new();
    prost::encoding::bytes::encode(1, &key, &mut value);
    Ok(Any {
        type_url: type_url.into(),
        value,
    })
}

fn validators_output(
    height: Option<i64>,
    page: i64,
    limit: i64,
) -> QResult<cmt::GetLatestValidatorSetResponse> {
    let mut p = params(&[
        ("page", comet::int_param(page)),
        ("per_page", comet::int_param(limit)),
    ]);
    if let Some(h) = height {
        p.insert("height".into(), comet::int_param(h));
    }
    let r = call("validators", p)?;
    let validators = r["validators"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|v| {
            Ok(cmt::Validator {
                address: grpc_query::util::bech32(
                    grpc_query::util::VALCONS,
                    &conv(comet::hex_bytes(&v["address"]))?,
                ),
                pub_key: Some(pub_key_any(&v["pub_key"])?),
                voting_power: conv(comet::int(&v["voting_power"]))?,
                proposer_priority: conv(comet::int(&v["proposer_priority"]))?,
            })
        })
        .collect::<QResult<_>>()?;
    Ok(cmt::GetLatestValidatorSetResponse {
        block_height: conv(comet::int(&r["block_height"]))?,
        validators,
        pagination: Some(PageResponse {
            next_key: Vec::new(),
            total: u64::try_from(conv(comet::int(&r["total"]))?).unwrap_or_default(),
        }),
    })
}

fn get_latest_validator_set(
    req: cmt::GetLatestValidatorSetRequest,
) -> QResult<cmt::GetLatestValidatorSetResponse> {
    let (page, limit) = parse_pagination(req.pagination.as_ref())?;
    validators_output(None, page, limit)
}

fn get_validator_set_by_height(
    req: cmt::GetValidatorSetByHeightRequest,
) -> QResult<cmt::GetValidatorSetByHeightResponse> {
    let (page, limit) = parse_pagination(req.pagination.as_ref())?;
    let latest = status()
        .and_then(|s| latest_height(&s))
        .map_err(|_| QueryError::Status(Code::Internal, "failed to parse chain height".into()))?;
    if req.height > latest {
        return Err(too_high());
    }
    let r = validators_output(Some(req.height), page, limit)?;
    Ok(cmt::GetValidatorSetByHeightResponse {
        block_height: r.block_height,
        validators: r.validators,
        pagination: r.pagination,
    })
}

fn abci_query_service(
    env: &Env<'_>,
    req: cmt::AbciQueryRequest,
) -> QResult<cmt::AbciQueryResponse> {
    if req.path.is_empty() {
        return Err(QueryError::Status(
            Code::InvalidArgument,
            "empty query path".into(),
        ));
    }
    let first = req
        .path
        .split('/')
        .find(|s| !s.is_empty())
        .unwrap_or_default();
    let split_nonempty = req.path.split('/').any(|s| !s.is_empty()) || req.path.starts_with('/');
    if split_nonempty && !matches!(first, "app" | "store" | "p2p" | "custom") {
        return Err(QueryError::Status(
            Code::InvalidArgument,
            format!("unsupported ABCI query path: {}", req.path),
        ));
    }
    let q = RequestQuery {
        data: req.data,
        path: req.path,
        height: req.height,
        prove: req.prove,
    };
    let res = env.with_app(|a| a.query(&q))?;
    Ok(cmt::AbciQueryResponse {
        code: res.code,
        log: res.log,
        info: res.info,
        index: res.index,
        key: res.key,
        value: res.value,
        proof_ops: res.proof_ops.map(|p| cmt::ProofOps {
            ops: p
                .ops
                .into_iter()
                .map(|o| cmt::ProofOp {
                    r#type: o.r#type,
                    key: o.key,
                    data: o.data,
                })
                .collect(),
        }),
        height: res.height,
        codespace: res.codespace,
    })
}

// ---------------------------------------------------------------------------
// Tx service (RPC-backed methods).

fn upper_hex(b: &[u8]) -> String {
    use std::fmt::Write;
    b.iter().fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02X}");
        s
    })
}

/// `ParseABCILogs` (failure: no logs).
fn parse_logs(log: &str) -> Vec<AbciMessageLog> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(log) else {
        return Vec::new();
    };
    items
        .iter()
        .map(|l| AbciMessageLog {
            msg_index: u32::try_from(l["msg_index"].as_u64().unwrap_or(0)).unwrap_or_default(),
            log: l["log"].as_str().unwrap_or_default().to_owned(),
            events: l["events"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .map(|e| StringEvent {
                    r#type: e["type"].as_str().unwrap_or_default().to_owned(),
                    attributes: e["attributes"]
                        .as_array()
                        .map(Vec::as_slice)
                        .unwrap_or_default()
                        .iter()
                        .map(|a| Attribute {
                            key: a["key"].as_str().unwrap_or_default().to_owned(),
                            value: a["value"].as_str().unwrap_or_default().to_owned(),
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect()
}

fn events(v: &Value) -> Vec<abcipb::Event> {
    v.as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|e| abcipb::Event {
            r#type: e["type"].as_str().unwrap_or_default().to_owned(),
            attributes: e["attributes"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .map(|a| abcipb::EventAttribute {
                    key: a["key"].as_str().unwrap_or_default().to_owned(),
                    value: a["value"].as_str().unwrap_or_default().to_owned(),
                    index: a["index"].as_bool().unwrap_or(false),
                })
                .collect(),
        })
        .collect()
}

/// A Go error returned as is by a handler: unregistered errors are plain.
#[allow(clippy::needless_pass_by_value)]
fn go_err(e: sdk::SdkError) -> QueryError {
    match (e.codespace, e.code) {
        ("undefined", 1) => QueryError::Plain(e.log),
        ("undefined", 111_222) => QueryError::Panic(e.log),
        _ => QueryError::Sdk(e),
    }
}

/// The decoded tx of a `ResultTx` (celestia's `TxDecoder`, which unwraps
/// blob txs, + `AsAny`).
fn decode_tx(raw: &[u8]) -> QResult<txpb::Tx> {
    let tx = txdecode::default_tx_decode(&inner_tx(raw)).map_err(go_err)?;
    Ok(txpb::Tx {
        body: Some(tx.body),
        auth_info: Some(tx.auth_info),
        signatures: tx.signatures,
    })
}

/// `mkTxResult`: the tx response of a `/tx` result, timestamped with its
/// block's time.
fn tx_result(v: &Value, block_time: &str) -> QResult<(txpb::Tx, TxResponse)> {
    let raw = conv(comet::b64(&v["tx"]))?;
    let tx = decode_tx(&raw)?;
    let r = &v["tx_result"];
    let log = r["log"].as_str().unwrap_or_default().to_owned();
    Ok((
        tx.clone(),
        TxResponse {
            height: conv(comet::int(&v["height"]))?,
            txhash: v["hash"].as_str().unwrap_or_default().to_owned(),
            codespace: r["codespace"].as_str().unwrap_or_default().to_owned(),
            code: u32::try_from(conv(comet::uint(&r["code"]))?).unwrap_or_default(),
            data: upper_hex(&conv(comet::b64(&r["data"]))?),
            logs: parse_logs(&log),
            raw_log: log,
            info: r["info"].as_str().unwrap_or_default().to_owned(),
            gas_wanted: conv(comet::int(&r["gas_wanted"]))?,
            gas_used: conv(comet::int(&r["gas_used"]))?,
            tx: Some(Any::pack(&tx)),
            timestamp: block_time.to_owned(),
            events: events(&r["events"]),
            signers: Vec::new(),
        },
    ))
}

/// `time.Format(time.RFC3339)` of a block's time.
fn block_time(height: i64, cache: &mut std::collections::BTreeMap<i64, String>) -> QResult<String> {
    if let Some(t) = cache.get(&height) {
        return Ok(t.clone());
    }
    let (_, b) = block_at(Some(height))?;
    let t = sdk::time::format_rfc3339(&b.header.and_then(|h| h.time).unwrap_or_default());
    cache.insert(height, t.clone());
    Ok(t)
}

fn get_tx(req: txpb::GetTxRequest) -> QResult<txpb::GetTxResponse> {
    if req.hash.is_empty() {
        return Err(QueryError::Status(
            Code::InvalidArgument,
            "tx hash cannot be empty".into(),
        ));
    }
    let hash = grpc_query::util::go_hex_decode(&req.hash).map_err(QueryError::Plain)?;
    let res = node()?.call(
        "tx",
        params(&[
            ("hash", Value::String(comet::to_b64(&hash))),
            ("prove", Value::Bool(false)),
        ]),
    );
    let v = match res {
        Ok(v) => v,
        Err(e) if e.contains("not found") => {
            return Err(QueryError::Status(
                Code::NotFound,
                format!("tx not found: {}", req.hash),
            ));
        }
        Err(e) => return Err(QueryError::Plain(e)),
    };
    let mut cache = std::collections::BTreeMap::new();
    let t = block_time(conv(comet::int(&v["height"]))?, &mut cache)?;
    let (tx, resp) = tx_result(&v, &t)?;
    Ok(txpb::GetTxResponse {
        tx: Some(tx),
        tx_response: Some(resp),
    })
}

#[allow(deprecated)]
fn get_txs_event(req: txpb::GetTxsEventRequest) -> QResult<txpb::GetTxsEventResponse> {
    let internal = |e: QueryError| QueryError::Status(Code::Internal, e.go_error_string());
    let order_by = match req.order_by {
        1 => "asc",
        2 => "desc",
        _ => "",
    };
    if req.query.is_empty() {
        return Err(internal(QueryError::Plain("query cannot be empty".into())));
    }
    let page = i64::try_from(req.page).unwrap_or(0).max(1);
    let limit = match i64::try_from(req.limit).unwrap_or(0) {
        l if l <= 0 => 100,
        l => l,
    };
    let client = node().map_err(internal)?;
    let r = client
        .call(
            "tx_search",
            params(&[
                ("query", Value::String(req.query.clone())),
                ("prove", Value::Bool(false)),
                ("page", comet::int_param(page)),
                ("per_page", comet::int_param(limit)),
                ("order_by", Value::String(order_by.into())),
            ]),
        )
        .map_err(|e| internal(QueryError::Plain(format!("failed to search for txs: {e}"))))?;
    let items = r["txs"].as_array().cloned().unwrap_or_default();
    let mut cache = std::collections::BTreeMap::new();
    for t in &items {
        block_time(
            conv(comet::int(&t["height"])).map_err(internal)?,
            &mut cache,
        )
        .map_err(internal)?;
    }
    let mut txs = Vec::new();
    let mut responses = Vec::new();
    for t in &items {
        let h = conv(comet::int(&t["height"])).map_err(internal)?;
        let (tx, resp) = tx_result(t, &cache[&h]).map_err(internal)?;
        txs.push(tx);
        responses.push(resp);
    }
    Ok(txpb::GetTxsEventResponse {
        txs,
        tx_responses: responses,
        pagination: None,
        total: u64::try_from(conv(comet::int(&r["total_count"])).map_err(internal)?)
            .unwrap_or_default(),
    })
}

fn get_block_with_txs(
    env: &Env<'_>,
    req: txpb::GetBlockWithTxsRequest,
) -> QResult<txpb::GetBlockWithTxsResponse> {
    let current = env.height;
    if req.height < 1 || req.height > current {
        return Err(QueryError::Sdk(errs::invalid_height(format!(
            "requested height {} but height must not be less than 1 or greater than the current height {current}",
            req.height
        ))));
    }
    let (id, block) = block_at(Some(req.height))?;
    let (offset, limit, reverse) = req
        .pagination
        .as_ref()
        .map_or((0, 100, false), |p| (p.offset, p.limit, p.reverse));
    let block_txs = block
        .data
        .as_ref()
        .map(|d| d.txs.clone())
        .unwrap_or_default();
    let n = block_txs.len() as u64;
    if offset >= n && n != 0 {
        return Err(QueryError::Sdk(errs::invalid_request(format!(
            "out of range: cannot paginate {n} txs with offset {offset} and limit {limit}"
        ))));
    }
    let mut txs = Vec::new();
    let mut push = |i: u64| {
        if let Ok(tx) = decode_tx(&block_txs[usize::try_from(i).unwrap_or(usize::MAX)]) {
            txs.push(tx);
        }
    };
    if reverse {
        let (mut i, mut count) = (offset, 0u64);
        while i > 0 && count != limit {
            push(i);
            i -= 1;
            count += 1;
        }
    } else {
        let (mut i, mut count) = (offset, 0u64);
        while i < n && count != limit {
            push(i);
            i += 1;
            count += 1;
        }
    }
    Ok(txpb::GetBlockWithTxsResponse {
        txs,
        block_id: Some(id),
        block: Some(block),
        pagination: Some(PageResponse {
            next_key: Vec::new(),
            total: n,
        }),
    })
}

/// `CheckCometError`.
fn check_comet_error(err: &str, tx: &[u8]) -> Option<TxResponse> {
    let e = err.to_lowercase();
    let hash = upper_hex(&hash::sha256(tx));
    let (code, codespace) = if e.contains("tx already exists in cache") {
        (19, "sdk")
    } else if e.contains("mempool is full") {
        (20, "sdk")
    } else if e.contains("tx too large") {
        (21, "sdk")
    } else {
        return None;
    };
    Some(TxResponse {
        code,
        codespace: codespace.into(),
        txhash: hash,
        ..TxResponse::default()
    })
}

fn broadcast_tx(req: txpb::BroadcastTxRequest) -> QResult<txpb::BroadcastTxResponse> {
    if req.tx_bytes.is_empty() {
        return Err(QueryError::Status(
            Code::InvalidArgument,
            "invalid empty tx".into(),
        ));
    }
    let method = match req.mode {
        3 => "broadcast_tx_async",
        2 => "broadcast_tx_sync",
        _ => {
            return Err(QueryError::Plain(
                "unsupported return type unspecified; supported types: sync, async".into(),
            ));
        }
    };
    let client = node()?;
    let res = client.call(
        method,
        params(&[("tx", Value::String(comet::to_b64(&req.tx_bytes)))]),
    );
    let v = match res {
        Ok(v) => v,
        Err(e) => {
            return match check_comet_error(&e, &req.tx_bytes) {
                Some(r) => Ok(txpb::BroadcastTxResponse {
                    tx_response: Some(r),
                }),
                None => Err(QueryError::Plain(e)),
            };
        }
    };
    let log = v["log"].as_str().unwrap_or_default().to_owned();
    Ok(txpb::BroadcastTxResponse {
        tx_response: Some(TxResponse {
            code: u32::try_from(conv(comet::uint(&v["code"]))?).unwrap_or_default(),
            codespace: v["codespace"].as_str().unwrap_or_default().to_owned(),
            data: upper_hex(&conv(comet::hex_bytes(&v["data"]))?),
            logs: parse_logs(&log),
            raw_log: log,
            txhash: v["hash"].as_str().unwrap_or_default().to_owned(),
            ..TxResponse::default()
        }),
    })
}

// ---------------------------------------------------------------------------
// celestia.core.v1.tx.

fn tx_status_of(r: &Value) -> QResult<ctxpb::TxStatusResponse> {
    Ok(ctxpb::TxStatusResponse {
        height: conv(comet::int(&r["height"]))?,
        index: u32::try_from(conv(comet::uint(&r["index"]))?).unwrap_or_default(),
        execution_code: u32::try_from(conv(comet::uint(&r["execution_code"]))?).unwrap_or_default(),
        error: r["error"].as_str().unwrap_or_default().to_owned(),
        status: r["status"].as_str().unwrap_or_default().to_owned(),
        codespace: r["codespace"].as_str().unwrap_or_default().to_owned(),
        gas_wanted: conv(comet::int(&r["gas_wanted"]))?,
        gas_used: conv(comet::int(&r["gas_used"]))?,
        signers: r["signers"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|s| s.as_str().unwrap_or_default().to_owned())
            .collect(),
    })
}

fn invalid_tx_id(e: &str) -> QueryError {
    QueryError::Status(Code::InvalidArgument, format!("invalid tx id: {e}"))
}

fn tx_status(req: ctxpb::TxStatusRequest) -> QResult<ctxpb::TxStatusResponse> {
    if req.tx_id.is_empty() {
        return Err(QueryError::Status(
            Code::InvalidArgument,
            "tx id cannot be empty".into(),
        ));
    }
    let client = node()?;
    let id = grpc_query::util::go_hex_decode(&req.tx_id).map_err(|e| invalid_tx_id(&e))?;
    let r = client
        .call(
            "tx_status",
            params(&[("hash", Value::String(comet::to_b64(&id)))]),
        )
        .map_err(QueryError::Plain)?;
    tx_status_of(&r)
}

fn tx_status_batch(req: ctxpb::TxStatusBatchRequest) -> QResult<ctxpb::TxStatusBatchResponse> {
    if req.tx_ids.len() > 20 {
        return Err(QueryError::Status(
            Code::InvalidArgument,
            "maximum of 20 tx ids allowed".into(),
        ));
    }
    if req.tx_ids.is_empty() {
        return Err(QueryError::Status(
            Code::InvalidArgument,
            "tx ids cannot be empty".into(),
        ));
    }
    let client = node()?;
    let mut ids = Vec::new();
    for id in &req.tx_ids {
        let b = grpc_query::util::go_hex_decode(id).map_err(|e| invalid_tx_id(&e))?;
        ids.push(Value::String(comet::to_b64(&b)));
    }
    let r = client
        .call("tx_status_batch", params(&[("hashes", Value::Array(ids))]))
        .map_err(QueryError::Plain)?;
    let statuses = r["statuses"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .zip(&req.tx_ids)
        .map(|(s, id)| {
            Ok(ctxpb::TxStatusResult {
                tx_hash: id.clone(),
                status: Some(tx_status_of(&s["result"])?),
            })
        })
        .collect::<QResult<_>>()?;
    Ok(ctxpb::TxStatusBatchResponse { statuses })
}

// ---------------------------------------------------------------------------
// Gas estimation.

const GAS_MULTIPLIER: f64 = 1.1;
const ESTIMATION_THRESHOLD: f64 = 0.70;
const DEFAULT_MIN_GAS_PRICE: f64 = 0.004;
const SHARE_SIZE: u64 = 512;

fn median(p: &[f64]) -> QResult<f64> {
    let n = p.len();
    if n == 0 {
        return Err(QueryError::Plain(
            "cannot compute median of an empty slice".into(),
        ));
    }
    Ok(if n % 2 == 1 {
        p[n / 2]
    } else {
        // Go: `(mid1 + mid2) / 2.0` (not a midpoint that avoids overflow).
        #[allow(clippy::manual_midpoint)]
        let m = (p[n / 2 - 1] + p[n / 2]) / 2.0;
        m
    })
}

#[allow(clippy::cast_precision_loss)]
fn mean(p: &[f64]) -> f64 {
    if p.is_empty() {
        return 0.0;
    }
    p.iter().sum::<f64>() / p.len() as f64
}

#[allow(clippy::cast_precision_loss)]
fn std_dev(m: f64, p: &[f64]) -> f64 {
    if p.len() < 2 {
        return 0.0;
    }
    let v = p.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / p.len() as f64;
    v.sqrt()
}

/// `estimateGasPriceForTransactions`.
fn estimate_for(prices: &[f64], priority: i32) -> QResult<f64> {
    if prices.is_empty() {
        return Err(QueryError::Plain("empty gas prices list".into()));
    }
    let sd = std_dev(mean(prices), prices);
    match priority {
        0 | 2 => {
            let e = median(prices)?;
            Ok(if sd < 0.001 { e * 1.1 } else { e })
        }
        1 => {
            let idx = (prices.len() * 10 / 100).max(1);
            median(&prices[..idx])
        }
        3 => {
            let e = median(&prices[prices.len() * 90 / 100..])?;
            Ok(if sd < 0.001 { e * 1.3 } else { e })
        }
        p => Err(QueryError::Plain(format!("unknown priority: {p}"))),
    }
}

/// The SDK tx of a mempool tx (blob txs unwrapped).
fn inner_tx(raw: &[u8]) -> Vec<u8> {
    match txdecode::unmarshal_blob_tx(raw) {
        Some(Ok(b)) => b.tx,
        _ => raw.to_vec(),
    }
}

/// `SortAndExtractGasPrices`.
#[allow(clippy::cast_precision_loss)]
fn sort_and_extract(txs: &[Vec<u8>], max_bytes: i64) -> QResult<Vec<f64>> {
    let mut items = Vec::new();
    for raw in txs {
        let tx = txdecode::default_tx_decode(&inner_tx(raw))?;
        let fee = tx.auth_info.fee.clone().unwrap_or_default();
        let amount: u64 = fee
            .amount
            .iter()
            .find(|c| c.denom == "utia")
            .and_then(|c| c.amount.parse::<u64>().ok())
            .unwrap_or(0);
        let price = amount as f64 / fee.gas_limit as f64;
        items.push((price, i64::try_from(raw.len()).unwrap_or(i64::MAX)));
    }
    items.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut prices = Vec::new();
    let mut total = 0i64;
    for (price, size) in items {
        if size + total > max_bytes {
            continue;
        }
        prices.push(price);
        total += size;
    }
    prices.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Ok(prices)
}

#[allow(clippy::cast_precision_loss)]
fn estimate_gas_price_inner(env: &Env<'_>, priority: i32) -> QResult<f64> {
    let r = call(
        "unconfirmed_txs",
        params(&[("limit", comet::int_param(-1))]),
    )?;
    let (max_square_bytes, min_gas_price) = env.with_app(gas_inputs)??;
    let total_bytes = conv(comet::int(&r["total_bytes"]))?;
    if (total_bytes as f64) < (max_square_bytes as f64) * ESTIMATION_THRESHOLD {
        return Ok(DEFAULT_MIN_GAS_PRICE.max(min_gas_price));
    }
    let txs: Vec<Vec<u8>> = r["txs"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(comet::b64)
        .collect::<Result<_, _>>()
        .map_err(QueryError::Plain)?;
    let prices = sort_and_extract(&txs, i64::try_from(max_square_bytes).unwrap_or(i64::MAX))?;
    Ok(estimate_for(&prices, priority)?.max(min_gas_price))
}

/// `getGovMaxSquareBytes` and `getMinGasPrice` at the latest height.
fn gas_inputs(app: &mut App) -> QResult<(u64, f64)> {
    let h = app.last_block_height();
    let mut state = app
        .create_query_context(h, false)
        .map_err(QueryError::Sdk)?;
    let local: f64 = app
        .node_min_gas_price()
        .map_or(0.0, |p| p.to_string().parse().unwrap_or(0.0));
    let mut ctx = state.context();
    let size = x_blob::get_params(&mut ctx)?.gov_max_square_size;
    let network: f64 = x_minfee::get_params(&mut ctx)?
        .network_min_gas_price
        .to_string()
        .parse()
        .unwrap_or(0.0);
    Ok((size * size * SHARE_SIZE, network.max(local)))
}

fn estimate_gas_price(
    env: &Env<'_>,
    req: gepb::EstimateGasPriceRequest,
) -> QResult<gepb::EstimateGasPriceResponse> {
    Ok(gepb::EstimateGasPriceResponse {
        estimated_gas_price: estimate_gas_price_inner(env, req.tx_priority)?,
    })
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn estimate_gas_price_and_usage(
    env: &Env<'_>,
    req: gepb::EstimateGasPriceAndUsageRequest,
) -> QResult<gepb::EstimateGasPriceAndUsageResponse> {
    let price = estimate_gas_price_inner(env, req.tx_priority)?;
    let tx = match txdecode::unmarshal_blob_tx(&req.tx_bytes) {
        Some(Ok(b)) => b.tx,
        Some(Err(e)) => return Err(QueryError::Plain(format!("{e:?}"))),
        None => req.tx_bytes,
    };
    let (gas, result) = env.with_app(|a| crate::query_services::simulate(a, &tx))?;
    result.map_err(go_err)?;
    Ok(gepb::EstimateGasPriceAndUsageResponse {
        estimated_gas_price: price,
        estimated_gas_used: (gas.gas_used as f64 * GAS_MULTIPLIER).round() as u64,
    })
}
