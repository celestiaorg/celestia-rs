//! The query corpus: ABCI `Query` and REST requests recorded against Go
//! `celestia-appd` after it replayed a devnet scenario
//! (`examples/record_queries.rs`), stored under
//! `conformance/querycorpus/<scenario>.json`. The Rust app replays the same
//! scenario and must answer every request identically.

#![allow(
    clippy::cast_possible_truncation,
    clippy::single_match_else,
    clippy::format_push_string,
    clippy::match_same_arms
)]

use std::path::PathBuf;

use celestia_proto::celestia_core_proto::abci::{RequestQuery, ResponseQuery};
use celestia_proto::celestia_core_proto::tendermint::crypto::{ProofOp, ProofOps};
use celestia_proto::cosmos::base::query::v1beta1::{PageRequest, PageResponse};
use prost::Message;
use serde::{Deserialize, Serialize};

pub mod net;
pub mod requests;

/// Directory of the recorded corpora.
#[must_use]
pub fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance/querycorpus")
}

/// A recorded corpus for one scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Corpus {
    /// Scenario name (fixture under `conformance/fixtures/devnet`).
    pub scenario: String,
    /// Last committed height of the scenario.
    pub last_height: i64,
    /// ABCI `Query` exchanges (gRPC routes return the gRPC response bytes).
    pub abci: Vec<AbciCase>,
    /// REST (grpc-gateway) exchanges.
    pub rest: Vec<RestCase>,
}

/// One ABCI query and Go's response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbciCase {
    /// Query path.
    pub path: String,
    /// Request data (hex).
    pub data: String,
    /// Request height.
    pub height: i64,
    /// Request `prove`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub prove: bool,
    /// Go's response.
    pub response: AbciResponse,
}

/// A recorded `ResponseQuery`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AbciResponse {
    /// Code.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub code: u32,
    /// Codespace.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub codespace: String,
    /// Log.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub log: String,
    /// Info.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub info: String,
    /// Index.
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub index: i64,
    /// Key (hex).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key: String,
    /// Value (hex).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub value: String,
    /// Proof ops: (type, key hex, data hex).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof_ops: Option<Vec<(String, String, String)>>,
    /// Height.
    #[serde(default)]
    pub height: i64,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero_i64(v: &i64) -> bool {
    *v == 0
}

impl From<&ResponseQuery> for AbciResponse {
    fn from(r: &ResponseQuery) -> Self {
        Self {
            code: r.code,
            codespace: r.codespace.clone(),
            log: r.log.clone(),
            info: r.info.clone(),
            index: r.index,
            key: hex::encode(&r.key),
            value: hex::encode(&r.value),
            proof_ops: r.proof_ops.as_ref().map(|p| {
                p.ops
                    .iter()
                    .map(|o| (o.r#type.clone(), hex::encode(&o.key), hex::encode(&o.data)))
                    .collect()
            }),
            height: r.height,
        }
    }
}

impl AbciResponse {
    /// Back to a `ResponseQuery`.
    #[must_use]
    pub fn to_response(&self) -> ResponseQuery {
        ResponseQuery {
            code: self.code,
            codespace: self.codespace.clone(),
            log: self.log.clone(),
            info: self.info.clone(),
            index: self.index,
            key: hex::decode(&self.key).unwrap_or_default(),
            value: hex::decode(&self.value).unwrap_or_default(),
            proof_ops: self.proof_ops.as_ref().map(|ops| ProofOps {
                ops: ops
                    .iter()
                    .map(|(t, k, d)| ProofOp {
                        r#type: t.clone(),
                        key: hex::decode(k).unwrap_or_default(),
                        data: hex::decode(d).unwrap_or_default(),
                    })
                    .collect(),
            }),
            height: self.height,
        }
    }
}

impl AbciCase {
    /// The request.
    #[must_use]
    pub fn request(&self) -> RequestQuery {
        RequestQuery {
            path: self.path.clone(),
            data: hex::decode(&self.data).unwrap_or_default(),
            height: self.height,
            prove: self.prove,
        }
    }
}

/// One REST request and Go's response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestCase {
    /// gRPC method the URL maps to.
    pub method: String,
    /// Path and query string.
    pub url: String,
    /// `x-cosmos-block-height` request header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<i64>,
    /// HTTP status.
    pub status: u16,
    /// Response body (JSON when parseable, else a string).
    pub body: serde_json::Value,
}

/// Serializes a corpus with one case per line (reviewable diffs).
pub fn to_lines(c: &Corpus) -> anyhow::Result<String> {
    let mut out = format!(
        "{{\"scenario\":{},\"last_height\":{},\"abci\":[\n",
        serde_json::to_string(&c.scenario)?,
        c.last_height
    );
    for (i, a) in c.abci.iter().enumerate() {
        out.push_str(&serde_json::to_string(a)?);
        out.push_str(if i + 1 < c.abci.len() { ",\n" } else { "\n" });
    }
    out.push_str("],\"rest\":[\n");
    for (i, r) in c.rest.iter().enumerate() {
        out.push_str(&serde_json::to_string(r)?);
        out.push_str(if i + 1 < c.rest.len() { ",\n" } else { "\n" });
    }
    out.push_str("]}\n");
    Ok(out)
}

/// Loads a recorded corpus.
pub fn load(scenario: &str) -> anyhow::Result<Corpus> {
    use std::io::Read;
    let gz = corpus_dir().join(format!("{scenario}.json.gz"));
    let bytes = if gz.exists() {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(std::fs::File::open(gz)?).read_to_end(&mut out)?;
        out
    } else {
        std::fs::read(corpus_dir().join(format!("{scenario}.json")))?
    };
    Ok(serde_json::from_slice(&bytes)?)
}

/// Methods answered by celestia-core RPC (block store, tx indexer,
/// mempool) rather than the application: the recorder's Go node runs
/// without consensus, so its answers are connection errors. They are
/// recorded but not compared.
pub const NEEDS_CONSENSUS: &[&str] = &[
    "/cosmos.base.tendermint.v1beta1.Service/",
    "/celestia.core.v1.gas_estimation.GasEstimator/",
    "/celestia.core.v1.tx.Tx/",
    "/cosmos.tx.v1beta1.Service/GetTx",
    "/cosmos.tx.v1beta1.Service/GetTxsEvent",
    "/cosmos.tx.v1beta1.Service/GetBlockWithTxs",
];

/// Whether `path` is answered by celestia-core (see [`NEEDS_CONSENSUS`]).
#[must_use]
pub fn needs_consensus(path: &str) -> bool {
    NEEDS_CONSENSUS.iter().any(|p| path.starts_with(p))
}

/// Removes Go source annotations (` [path/file.go:123]`) that `%v` of an
/// `errorsmod` error appends; they name Go source lines and have no Rust
/// counterpart.
#[must_use]
pub fn strip_go_frames(log: &str) -> String {
    let mut out = String::with_capacity(log.len());
    let mut rest = log;
    while let Some(i) = rest.find(" [") {
        let tail = &rest[i + 2..];
        let frame = tail.find(']').map(|j| &tail[..j]).filter(|f| {
            f.rsplit_once(".go:").is_some_and(|(_, line)| {
                !line.is_empty() && line.bytes().all(|b| b.is_ascii_digit())
            })
        });
        match frame {
            Some(f) => {
                out.push_str(&rest[..i]);
                rest = &tail[f.len() + 1..];
            }
            None => {
                out.push_str(&rest[..i + 2]);
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Go answers some queries in Go map iteration order (random per process);
/// both sides are brought into a canonical order before comparing.
#[must_use]
pub fn canonical_value(path: &str, value_hex: &str) -> String {
    use celestia_proto::cosmos::base::reflection::v1beta1 as refl;
    use celestia_proto::cosmos::distribution::v1beta1 as distr;
    use celestia_proto::cosmos::params::v1beta1 as params;
    let Ok(bytes) = hex::decode(value_hex) else {
        return value_hex.to_owned();
    };
    let out = match path {
        "/cosmos.distribution.v1beta1.Query/DelegationTotalRewards" => {
            distr::QueryDelegationTotalRewardsResponse::decode(bytes.as_slice())
                .ok()
                .map(|mut r| {
                    r.rewards
                        .sort_by(|a, b| a.validator_address.cmp(&b.validator_address));
                    r.encode_to_vec()
                })
        }
        "/cosmos.params.v1beta1.Query/Subspaces" => {
            params::QuerySubspacesResponse::decode(bytes.as_slice())
                .ok()
                .map(|mut r| {
                    r.subspaces.sort_by(|a, b| a.subspace.cmp(&b.subspace));
                    r.encode_to_vec()
                })
        }
        "/cosmos.base.reflection.v1beta1.ReflectionService/ListAllInterfaces" => {
            refl::ListAllInterfacesResponse::decode(bytes.as_slice())
                .ok()
                .map(|mut r| {
                    r.interface_names.sort();
                    r.encode_to_vec()
                })
        }
        "/cosmos.base.reflection.v1beta1.ReflectionService/ListImplementations" => {
            refl::ListImplementationsResponse::decode(bytes.as_slice())
                .ok()
                .map(|mut r| {
                    r.implementation_message_names.sort();
                    r.encode_to_vec()
                })
        }
        // Go reports its build (toolchain, module list); only the
        // celestia-core node info is compared.
        "/cosmos.base.tendermint.v1beta1.Service/GetNodeInfo" => {
            celestia_proto::cosmos::base::tendermint::v1beta1::GetNodeInfoResponse::decode(
                bytes.as_slice(),
            )
            .ok()
            .map(|mut r| {
                r.application_version = None;
                r.encode_to_vec()
            })
        }
        _ => None,
    };
    out.map_or_else(|| value_hex.to_owned(), hex::encode)
}

/// The response as compared: canonical value order, Go frames stripped.
#[must_use]
pub fn comparable(path: &str, r: &AbciResponse) -> AbciResponse {
    AbciResponse {
        value: canonical_value(path, &r.value),
        log: strip_go_frames(&r.log),
        ..r.clone()
    }
}

/// Result of replaying a corpus against the Rust app.
#[derive(Debug, Default)]
pub struct Report {
    /// ABCI cases compared.
    pub compared: usize,
    /// ABCI cases skipped ([`NEEDS_CONSENSUS`]).
    pub skipped: usize,
    /// Mismatches: (case, Rust response).
    pub mismatches: Vec<(AbciCase, AbciResponse)>,
}

/// Replays `scenario` into a fresh in-memory app that keeps every height
/// and answers every ABCI case of its corpus.
pub fn check_abci(scenario: &str) -> anyhow::Result<Report> {
    use abci_server::Application as _;
    use celestia_proto::celestia_core_proto::abci::{request, response};
    let corpus = load(scenario)?;
    let (app, rpc) = replayed(scenario)?;
    let mut report = Report::default();
    for case in &corpus.abci {
        if needs_consensus(&case.path) && !rpc {
            report.skipped += 1;
            continue;
        }
        report.compared += 1;
        let ours = match app.handle(request::Value::Query(case.request())) {
            response::Value::Query(q) => AbciResponse::from(&q),
            other => anyhow::bail!("unexpected response {other:?}"),
        };
        if comparable(&case.path, &ours) != comparable(&case.path, &case.response) {
            report.mismatches.push((case.clone(), ours));
        }
    }
    Ok(report)
}

/// The recorded celestia-core RPC exchanges of a scenario
/// (`rpc.json.gz` next to its ABCI recording), if any.
pub fn load_rpc(scenario: &str) -> anyhow::Result<Option<Vec<net::RpcExchange>>> {
    use std::io::Read;
    let path = crate::scenario_dir(scenario).join("rpc.json.gz");
    if !path.exists() {
        return Ok(None);
    }
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(std::fs::File::open(path)?).read_to_end(&mut out)?;
    Ok(Some(serde_json::from_slice(&out)?))
}

/// The app after replaying `scenario` (pruning nothing), with the
/// celestia-core RPC served from the scenario's recording when it has one
/// (then `true`). The RPC address is process-wide: one recorded scenario
/// is checked at a time.
fn replayed(scenario: &str) -> anyhow::Result<(app::AbciApp, bool)> {
    static RPC_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let mut app = app::App::new();
    app.set_pruning(multistore_nothing());
    let r = crate::replay::replay_into(scenario, &mut app, 0, None)?;
    anyhow::ensure!(
        r.errors.is_empty() && r.first_mismatch.is_none(),
        "{scenario}: replay failed: {:?} first mismatch {:?}",
        r.errors,
        r.first_mismatch
    );
    let rpc = match load_rpc(scenario)? {
        Some(ex) => {
            let _guard = RPC_LOCK.lock();
            app::comet::set_rpc_address(Some(net::spawn_replaying_mock(&ex)?));
            true
        }
        None => false,
    };
    Ok((app::AbciApp::new(app), rpc))
}

/// Result of replaying the REST part of a corpus.
#[derive(Debug, Default)]
pub struct RestReport {
    /// Cases compared.
    pub compared: usize,
    /// Cases skipped ([`NEEDS_CONSENSUS`]).
    pub skipped: usize,
    /// Mismatches: (case, Rust status, Rust body).
    pub mismatches: Vec<(RestCase, u16, serde_json::Value)>,
}

/// Replays `scenario` and answers every REST case of its corpus through
/// the Rust REST gateway; bodies are compared as JSON values (field order
/// is irrelevant), messages with Go frames stripped.
pub fn check_rest(scenario: &str) -> anyhow::Result<RestReport> {
    let corpus = load(scenario)?;
    let (app, rpc) = replayed(scenario)?;
    let mut report = RestReport::default();
    for case in &corpus.rest {
        if needs_consensus(&case.method) && !rpc {
            report.skipped += 1;
            continue;
        }
        report.compared += 1;
        let (path, query) = case.url.split_once('?').unwrap_or((&case.url, ""));
        let height = case.height.map(|h| h.to_string());
        let req = api_server::rest::Request {
            method: "GET",
            path,
            query,
            height: height.as_deref(),
            body: &[],
        };
        let ours = api_server::rest::handle(&req, &mut |p, d, h| {
            app.grpc_query(p, d, h)
                .map_err(|e| api_server::rest_error_status(p, e))
        });
        let norm = |v: &serde_json::Value| -> serde_json::Value {
            let mut v = v.clone();
            if let Some(serde_json::Value::String(m)) = v.get_mut("message") {
                *m = strip_go_frames(m);
            }
            if let Some(o) = v.as_object_mut() {
                o.remove("application_version");
            }
            // Go map-order responses (see `canonical_value`).
            for (list, key) in [("subspaces", "subspace"), ("rewards", "validator_address")] {
                if let Some(serde_json::Value::Array(a)) = v.get_mut(list) {
                    a.sort_by(|x, y| x[key].as_str().cmp(&y[key].as_str()));
                }
            }
            v
        };
        // What the server sends (Go strings' invalid bytes as `\ufffd`).
        let served: serde_json::Value =
            serde_json::from_slice(&api_server::body_bytes(&ours.body)).unwrap_or_default();
        if ours.status != case.status || norm(&served) != norm(&case.body) {
            report
                .mismatches
                .push((case.clone(), ours.status, ours.body));
        }
    }
    Ok(report)
}

fn multistore_nothing() -> app::PruningOptions {
    app::PruningOptions::nothing()
}

/// Raw protobuf top-level fields: (number, wire type, payload bytes for
/// length-delimited fields, varint value otherwise).
#[must_use]
pub fn raw_fields(mut b: &[u8]) -> Vec<(u32, u8, Vec<u8>, u64)> {
    fn varint(b: &mut &[u8]) -> Option<u64> {
        let mut v = 0u64;
        for i in 0..10 {
            let (&c, rest) = b.split_first()?;
            *b = rest;
            v |= u64::from(c & 0x7f) << (7 * i);
            if c & 0x80 == 0 {
                return Some(v);
            }
        }
        None
    }
    let mut out = Vec::new();
    while !b.is_empty() {
        let Some(tag) = varint(&mut b) else { break };
        let (num, wt) = ((tag >> 3) as u32, (tag & 7) as u8);
        match wt {
            0 => {
                let Some(v) = varint(&mut b) else { break };
                out.push((num, wt, Vec::new(), v));
            }
            2 => {
                let Some(len) = varint(&mut b) else { break };
                let len = usize::try_from(len).unwrap_or(usize::MAX);
                if len > b.len() {
                    break;
                }
                out.push((num, wt, b[..len].to_vec(), 0));
                b = &b[len..];
            }
            1 => {
                if b.len() < 8 {
                    break;
                }
                b = &b[8..];
            }
            5 => {
                if b.len() < 4 {
                    break;
                }
                b = &b[4..];
            }
            _ => break,
        }
    }
    out
}

/// Percent-encodes a URL component (RFC 3986 unreserved kept).
#[must_use]
pub fn url_escape(s: &str, keep_slash: bool) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) || (keep_slash && b == b'/') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn flatten_query(prefix: &str, v: &serde_json::Value, out: &mut Vec<(String, String)>) {
    match v {
        serde_json::Value::Object(m) => {
            for (k, x) in m {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten_query(&key, x, out);
            }
        }
        serde_json::Value::Array(a) => {
            for x in a {
                flatten_query(prefix, x, out);
            }
        }
        serde_json::Value::Null => {}
        serde_json::Value::String(s) if s.is_empty() || s == "0" => {}
        serde_json::Value::Bool(false) => {}
        serde_json::Value::Number(n) if n.as_f64() == Some(0.0) => {}
        serde_json::Value::String(s) => out.push((prefix.to_owned(), s.clone())),
        other => out.push((prefix.to_owned(), other.to_string())),
    }
}

/// The REST URL (path + query) of a gRPC query with a GET binding, built
/// from the request like a grpc-gateway client would. `None` when the method
/// has no GET binding, the request does not decode, or a path parameter is
/// empty.
#[must_use]
pub fn rest_url(path: &str, data: &[u8]) -> Option<String> {
    let m = celestia_proto::services::method(path)?;
    let rule = m.http.iter().find(|r| r.method == "GET")?;
    let mut json =
        celestia_proto::registry::any_value_to_json(&format!("/{}", m.input), data)?.ok()?;
    let obj = json.as_object_mut()?;
    let mut url = String::new();
    let mut rest = rule.pattern;
    while let Some(start) = rest.find('{') {
        url.push_str(&rest[..start]);
        let end = start + rest[start..].find('}')?;
        let var = &rest[start + 1..end];
        let (name, wildcard) = match var.split_once('=') {
            Some((n, _)) => (n, true),
            None => (var, false),
        };
        let value = match obj.remove(name)? {
            serde_json::Value::String(s) => s,
            serde_json::Value::Null => String::new(),
            other => other.to_string(),
        };
        if value.is_empty() {
            return None;
        }
        url.push_str(&url_escape(&value, wildcard));
        rest = &rest[end + 1..];
    }
    url.push_str(rest);
    let mut params = Vec::new();
    flatten_query("", &json, &mut params);
    if !params.is_empty() {
        url.push('?');
        let q: Vec<String> = params
            .iter()
            .map(|(k, v)| format!("{}={}", url_escape(k, false), url_escape(v, false)))
            .collect();
        url.push_str(&q.join("&"));
    }
    Some(url)
}

/// The `PageResponse` of a list response (`output` = response full name).
#[must_use]
pub fn page_response(output: &str, bytes: &[u8]) -> Option<PageResponse> {
    let msg = celestia_proto::services::message(output)?;
    let f = msg
        .fields
        .iter()
        .find(|f| f.type_name == "cosmos.base.query.v1beta1.PageResponse")?;
    raw_fields(bytes)
        .into_iter()
        .find(|(n, wt, _, _)| *n == f.number && *wt == 2)
        .and_then(|(_, _, b, _)| PageResponse::decode(b.as_slice()).ok())
}

/// Appends `page` to an encoded request of type `input` (its
/// `PageRequest` field); `None` when the request has no pagination.
#[must_use]
pub fn with_page(input: &str, req: &[u8], page: &PageRequest) -> Option<Vec<u8>> {
    let msg = celestia_proto::services::message(input)?;
    let f = msg
        .fields
        .iter()
        .find(|f| f.type_name == "cosmos.base.query.v1beta1.PageRequest")?;
    let mut out = req.to_vec();
    let body = page.encode_to_vec();
    prost::encoding::encode_key(
        f.number,
        prost::encoding::WireType::LengthDelimited,
        &mut out,
    );
    prost::encoding::encode_varint(body.len() as u64, &mut out);
    out.extend_from_slice(&body);
    Some(out)
}
