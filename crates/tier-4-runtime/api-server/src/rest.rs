//! REST API: grpc-gateway (v1, gogo JSONPb) routes derived from the
//! `google.api.http` bindings of the generated method table.
//!
//! A request is matched against the HTTP bindings, turned into the method's
//! protobuf request (path parameters, query parameters and the JSON body,
//! as `runtime.PopulateQueryParameters` does), answered by the gRPC query
//! path and rendered as Go's `jsonpb` with `OrigName` and `EmitDefaults`
//! ([`crate::jsonpb`]). Unmatched routes get grpc-gateway's 501.

use celestia_proto::services::{self, HttpRule, Method};
use celestia_proto::wire_table::ty;
use serde_json::{Map, Value, json};

use crate::jsonpb;

/// Services celestia-appd does not register on the gateway
/// (`RegisterGRPCGatewayRoutes` is not called for them).
const NOT_ON_GATEWAY: &[&str] = &[
    "/celestia.minfee.v1.Query/",
    "/celestia.fibre.v1.Fibre/",
    "/celestia.core.v1.gas_estimation.",
    "/cosmos.base.reflection.",
    "/cosmos.mint.v1beta1.",
    "/cosmos.store.streaming.",
    "/ibc.applications.fee.",
    "/celestia.forwarding.v1.",
    "/ibc.lightclients.wasm.",
];

/// An HTTP request.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    /// HTTP method.
    pub method: &'a str,
    /// URL path (still percent-encoded).
    pub path: &'a str,
    /// Raw query string.
    pub query: &'a str,
    /// `x-cosmos-block-height` header.
    pub height: Option<&'a str>,
    /// Request body.
    pub body: &'a [u8],
}

/// An HTTP response.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    /// HTTP status.
    pub status: u16,
    /// JSON body.
    pub body: Value,
}

/// Stable response metadata emitted by the Go REST gateway.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResponseMetadata {
    /// The gRPC codec content type is present after a handler is entered.
    pub content_type: bool,
    /// The committed height used by the query handler.
    pub height: Option<i64>,
}

/// A response plus the transport details needed to match grpc-gateway.
#[derive(Debug, Clone, PartialEq)]
pub struct DetailedReply {
    /// Status and decoded JSON body returned by [`handle`].
    pub reply: Reply,
    /// Stable gRPC metadata forwarded by grpc-gateway.
    pub metadata: ResponseMetadata,
    /// Response descriptor used by jsonpb to preserve declaration order.
    pub(crate) response_type: Option<&'static str>,
    /// Whether grpc-gateway's error JSON formatting applies.
    pub(crate) error: bool,
}

impl From<Reply> for DetailedReply {
    fn from(reply: Reply) -> Self {
        Self {
            reply,
            metadata: ResponseMetadata::default(),
            response_type: None,
            error: true,
        }
    }
}

/// A gRPC call: `(path, request bytes, height)` → response bytes or a gRPC
/// status (code, message).
pub type Call<'a> =
    &'a mut dyn FnMut(&str, &[u8], i64) -> Result<(Vec<u8>, i64), (grpc_query::Code, String)>;

/// A gRPC call that retains response metadata on handler errors.
pub type CallWithMetadata<'a> =
    &'a mut dyn FnMut(
        &str,
        &[u8],
        i64,
    ) -> Result<(Vec<u8>, i64), (grpc_query::Code, String, Option<i64>)>;

/// `runtime.HTTPStatusFromCode` (grpc-gateway v1).
#[must_use]
pub fn http_status(code: grpc_query::Code) -> u16 {
    use grpc_query::Code as C;
    match code {
        C::Ok => 200,
        C::Canceled => 408,
        C::InvalidArgument | C::FailedPrecondition | C::OutOfRange => 400,
        C::DeadlineExceeded => 504,
        C::NotFound => 404,
        C::AlreadyExists | C::Aborted => 409,
        C::PermissionDenied => 403,
        C::Unauthenticated => 401,
        C::ResourceExhausted => 429,
        C::Unimplemented => 501,
        C::Unavailable => 503,
        C::Unknown | C::Internal | C::DataLoss => 500,
    }
}

fn error_reply(code: grpc_query::Code, message: &str) -> Reply {
    Reply {
        status: http_status(code),
        body: json!({"code": code as i32, "message": message, "details": []}),
    }
}

fn not_implemented() -> Reply {
    error_reply(grpc_query::Code::Unimplemented, "Not Implemented")
}

/// Percent-decodes a URL component (`+` is kept in paths, a space in
/// queries).
#[must_use]
pub fn percent_decode(s: &str, plus_space: bool) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz");
                if let Ok(v) = u8::from_str_radix(hex, 16) {
                    out.push(v);
                    i += 3;
                } else {
                    out.push(b'%');
                    i += 1;
                }
            }
            b'+' if plus_space => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Matches `segments` against a binding pattern; returns the bound
/// variables.
fn match_pattern(pattern: &str, segments: &[&str]) -> Option<Vec<(String, String)>> {
    let pat: Vec<&str> = pattern.trim_start_matches('/').split('/').collect();
    let mut vars = Vec::new();
    let mut i = 0;
    for (pi, p) in pat.iter().enumerate() {
        if let Some(var) = p.strip_prefix('{').and_then(|v| v.strip_suffix('}')) {
            match var.split_once('=') {
                Some((name, "**")) => {
                    // Deep wildcard: one or more segments, leaving enough
                    // for the (literal) rest of the pattern.
                    let rest = pat.len() - pi - 1;
                    if segments.len() < i + 1 + rest {
                        return None;
                    }
                    let end = segments.len() - rest;
                    vars.push((name.to_owned(), segments[i..end].join("/")));
                    i = end;
                }
                Some(_) => return None,
                None => {
                    let seg = segments.get(i)?;
                    if seg.is_empty() {
                        return None;
                    }
                    vars.push((var.to_owned(), (*seg).to_owned()));
                    i += 1;
                }
            }
        } else {
            if segments.get(i) != Some(p) {
                return None;
            }
            i += 1;
        }
    }
    (i == segments.len()).then_some(vars)
}

/// Bound path variables.
type Vars = Vec<(String, String)>;

fn on_gateway(m: &Method) -> bool {
    !NOT_ON_GATEWAY.iter().any(|p| m.path.starts_with(p))
}

/// The route for `method` + `path`: grpc-gateway v1 tries routes in
/// registration order (declaration order) and takes the first match.
fn route(method: &str, path: &str) -> Option<(&'static Method, &'static HttpRule, Vars)> {
    let decoded = percent_decode(path, false);
    let segments: Vec<&str> = decoded.trim_start_matches('/').split('/').collect();
    let mut best: Option<(usize, &'static Method, &'static HttpRule, Vars)> = None;
    for m in services::METHODS.iter().filter(|m| on_gateway(m)) {
        for rule in m.http.iter().filter(|r| r.method == method) {
            if let Some(vars) = match_pattern(rule.pattern, &segments) {
                let order = m.order as usize;
                if best.as_ref().is_none_or(|(o, ..)| order < *o) {
                    best = Some((order, m, rule, vars));
                }
            }
        }
    }
    best.map(|(_, m, r, v)| (m, r, v))
}

/// Go `base64.Encoding.DecodeString` (padded, non-strict) with its
/// `CorruptInputError` offset.
fn go_base64(s: &str, alphabet: &[u8; 64]) -> Result<Vec<u8>, usize> {
    let src = s.as_bytes();
    let val = |c: u8| alphabet.iter().position(|a| *a == c);
    let mut out = Vec::new();
    let mut si = 0;
    loop {
        let mut buf = [0u8; 4];
        let mut j = 0;
        let mut dlen = 4;
        while j < 4 {
            if si == src.len() {
                if j == 0 {
                    return Ok(out);
                }
                return Err(si - j);
            }
            let c = src[si];
            si += 1;
            if let Some(v) = val(c) {
                buf[j] = u8::try_from(v).unwrap_or_default();
                j += 1;
                continue;
            }
            if c != b'=' || j < 2 {
                return Err(si - 1);
            }
            if j == 2 {
                if si == src.len() {
                    return Err(src.len());
                }
                if src[si] != b'=' {
                    return Err(si - 1);
                }
                si += 1;
            }
            if si < src.len() {
                return Err(si);
            }
            dlen = j;
            break;
        }
        let n = (u32::from(buf[0]) << 18)
            | (u32::from(buf[1]) << 12)
            | (u32::from(buf[2]) << 6)
            | u32::from(buf[3]);
        let bytes = n.to_be_bytes();
        out.extend_from_slice(&bytes[1..dlen]);
        if dlen < 4 {
            return Ok(out);
        }
    }
}

/// `runtime.Bytes`: standard, then URL base64 (the URL error is reported).
fn gateway_bytes(v: &str) -> Result<Vec<u8>, String> {
    const STD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    const URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    go_base64(v, STD)
        .or_else(|_| go_base64(v, URL))
        .map_err(|i| format!("illegal base64 data at input byte {i}"))
}

/// Gogo-only enums the gateway cannot resolve by name: it falls back to
/// `strconv.ParseInt`.
const NUMERIC_ENUM_PACKAGES: &[&str] = &["cosmos.gov.v1beta1."];

/// Protobuf enums for which grpc-gateway accepts both the symbolic JSON name
/// and its decimal value in a query parameter.
const NUMERIC_OR_NAMED_ENUM_PACKAGES: &[&str] = &["cosmos.gov.v1."];

/// Go's `strconv` error for `value` (`ParseInt`/`ParseUint`, bit size
/// `bits`), or `None` if it parses.
fn go_int_error(value: &str, signed: bool, bits: u32) -> Option<String> {
    let func = if signed { "ParseInt" } else { "ParseUint" };
    let digits = if signed {
        value.strip_prefix(['+', '-']).unwrap_or(value)
    } else {
        value
    };
    let reason = if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        "invalid syntax"
    } else {
        let ok = if signed {
            value
                .parse::<i128>()
                .is_ok_and(|n| n >= -(1i128 << (bits - 1)) && n < (1i128 << (bits - 1)))
        } else {
            value.parse::<u128>().is_ok_and(|n| n < (1u128 << bits))
        };
        if ok {
            return None;
        }
        "value out of range"
    };
    Some(format!(
        "strconv.{func}: parsing {}: {reason}",
        serde_json::to_string(value).unwrap_or_default()
    ))
}

/// The gateway's typed conversion of a path parameter (`runtime.Int64`,
/// `runtime.Uint64Slice`, ...): Go errors for malformed integers.
fn check_path_param(field: &services::Field, value: &str) -> Result<(), String> {
    let (signed, bits) = match field.ty {
        ty::INT32 | ty::SINT32 | ty::SFIXED32 => (true, 32),
        ty::INT64 | ty::SINT64 | ty::SFIXED64 => (true, 64),
        ty::UINT32 | ty::FIXED32 => (false, 32),
        ty::UINT64 | ty::FIXED64 => (false, 64),
        _ => return Ok(()),
    };
    match go_int_error(value, signed, bits) {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Converts a query/path parameter string to the JSON value of a field.
fn param_value(msg: &str, field: &services::Field, value: &str) -> Result<Value, String> {
    Ok(match field.ty {
        ty::BOOL => match value {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            other => Value::String(other.to_owned()),
        },
        ty::INT32 | ty::UINT32 | ty::SINT32 | ty::FIXED32 | ty::SFIXED32 => value
            .parse::<i64>()
            .map_or_else(|_| Value::String(value.to_owned()), |n| json!(n)),
        ty::DOUBLE | ty::FLOAT => value
            .parse::<f64>()
            .map_or_else(|_| Value::String(value.to_owned()), |n| json!(n)),
        ty::BYTES => {
            gateway_bytes(value)?;
            Value::String(value.to_owned())
        }
        ty::ENUM
            if NUMERIC_OR_NAMED_ENUM_PACKAGES
                .iter()
                .any(|p| msg.starts_with(p)) =>
        {
            value
                .parse::<i64>()
                .map_or_else(|_| Value::String(value.to_owned()), |number| json!(number))
        }
        ty::ENUM if NUMERIC_ENUM_PACKAGES.iter().any(|p| msg.starts_with(p)) => {
            let n: i64 = value
                .parse()
                .map_err(|_| format!("strconv.ParseInt: parsing {value:?}: invalid syntax"))?;
            json!(n)
        }
        _ => Value::String(value.to_owned()),
    })
}

/// Sets `path` (dotted field names) in `obj` per the schema of `msg`.
fn set_field(
    obj: &mut Map<String, Value>,
    msg: &str,
    path: &[&str],
    value: &str,
) -> Result<(), String> {
    set_field_as(obj, msg, path, value, false)
}

/// [`set_field`]; `path_param` applies the gateway's path-parameter
/// conversion (repeated values are comma separated, integers parsed
/// strictly).
fn set_field_as(
    obj: &mut Map<String, Value>,
    msg: &str,
    path: &[&str],
    value: &str,
    path_param: bool,
) -> Result<(), String> {
    let Some(schema) = services::message(msg) else {
        return Ok(());
    };
    let Some((first, rest)) = path.split_first() else {
        return Ok(());
    };
    let Some(field) = schema.field(first) else {
        return Ok(());
    };
    let key = field.name.to_owned();
    if rest.is_empty() && path_param {
        let parts: Vec<&str> = if field.repeated {
            value.split(',').collect()
        } else {
            vec![value]
        };
        for part in &parts {
            check_path_param(field, part)?;
        }
        if field.repeated {
            let mut vals = Vec::new();
            for part in parts {
                vals.push(param_value(msg, field, part)?);
            }
            obj.insert(key, Value::Array(vals));
            return Ok(());
        }
    }
    if rest.is_empty() {
        let v = param_value(msg, field, value)?;
        if field.repeated {
            match obj.entry(key).or_insert_with(|| Value::Array(Vec::new())) {
                Value::Array(a) => a.push(v),
                other => *other = Value::Array(vec![v]),
            }
        } else {
            obj.insert(key, v);
        }
        return Ok(());
    }
    if field.ty != ty::MESSAGE || field.repeated {
        return Ok(());
    }
    let entry = obj.entry(key).or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    if let Value::Object(inner) = entry {
        set_field_as(inner, field.type_name, rest, value, path_param)?;
    }
    Ok(())
}

/// Validates JSON byte fields with grpc-gateway's Go base64 decoder before
/// pbjson turns the request into protobuf bytes. This preserves Go's exact
/// corrupt-input offset and message for request bodies as well as URL/query
/// parameters.
fn validate_body_bytes(msg: &str, value: &Value) -> Result<(), String> {
    let (Some(schema), Value::Object(obj)) = (services::message(msg), value) else {
        return Ok(());
    };
    for (name, value) in obj {
        let Some(field) = schema.field(name) else {
            continue;
        };
        match field.ty {
            ty::BYTES if field.repeated => {
                if let Value::Array(values) = value {
                    for value in values {
                        if let Value::String(value) = value {
                            gateway_bytes(value)?;
                        }
                    }
                }
            }
            ty::BYTES => {
                if let Value::String(value) = value {
                    gateway_bytes(value)?;
                }
            }
            ty::MESSAGE if field.repeated => {
                if let Value::Array(values) = value {
                    for value in values {
                        validate_body_bytes(field.type_name, value)?;
                    }
                }
            }
            ty::MESSAGE => validate_body_bytes(field.type_name, value)?,
            _ => {}
        }
    }
    Ok(())
}

/// Handles one REST request.
pub fn handle(req: &Request<'_>, call: Call<'_>) -> Reply {
    handle_with_metadata(req, &mut |path, data, height| {
        call(path, data, height).map_err(|(code, message)| (code, message, None))
    })
    .reply
}

/// Handles one REST request while retaining metadata from failed handlers.
pub fn handle_with_metadata(req: &Request<'_>, call: CallWithMetadata<'_>) -> DetailedReply {
    let Some((method, rule, vars)) = route(req.method, req.path) else {
        return not_implemented().into();
    };
    let mut obj = Map::new();
    if rule.body == "*" {
        match serde_json::from_slice::<Value>(req.body) {
            Ok(Value::Object(m)) => obj = m,
            Ok(_) | Err(_) if req.body.is_empty() => {}
            Ok(_) => {
                return error_reply(grpc_query::Code::InvalidArgument, "unexpected request body")
                    .into();
            }
            Err(e) => {
                return error_reply(grpc_query::Code::InvalidArgument, &e.to_string()).into();
            }
        }
    }
    for (name, value) in &vars {
        let path: Vec<&str> = name.split('.').collect();
        if let Err(e) = set_field_as(&mut obj, method.input, &path, value, true) {
            return error_reply(
                grpc_query::Code::InvalidArgument,
                &format!("type mismatch, parameter: {name}, error: {e}"),
            )
            .into();
        }
    }
    if rule.body != "*" {
        for pair in req.query.split('&').filter(|p| !p.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            let k = percent_decode(k, true);
            if vars.iter().any(|(n, _)| *n == k) {
                continue;
            }
            let v = percent_decode(v, true);
            let path: Vec<&str> = k.split('.').collect();
            if let Err(e) = set_field(&mut obj, method.input, &path, &v) {
                return error_reply(grpc_query::Code::InvalidArgument, &e).into();
            }
        }
    }
    if let Err(e) = validate_body_bytes(method.input, &Value::Object(obj.clone())) {
        return error_reply(grpc_query::Code::InvalidArgument, &e).into();
    }
    let data = match celestia_proto::registry::json_to_any_value(
        &format!("/{}", method.input),
        Value::Object(obj),
    ) {
        Some(Ok(d)) => d,
        Some(Err(e)) => return error_reply(grpc_query::Code::InvalidArgument, &e).into(),
        None => return not_implemented().into(),
    };
    let height = match crate::parse_height(req.height) {
        Ok(h) => h,
        Err((c, m)) => return error_reply(c, &m).into(),
    };
    match call(method.path, &data, height) {
        Ok((bytes, height)) => {
            match celestia_proto::registry::any_value_to_json(
                &format!("/{}", method.output),
                &bytes,
            ) {
                Some(Ok(mut v)) => {
                    jsonpb::go_style(&mut v, method.output);
                    DetailedReply {
                        reply: Reply {
                            status: 200,
                            body: v,
                        },
                        metadata: ResponseMetadata {
                            content_type: height >= 0,
                            height: (height >= 0).then_some(height),
                        },
                        response_type: Some(method.output),
                        error: false,
                    }
                }
                Some(Err(e)) => error_reply(grpc_query::Code::Internal, &e).into(),
                None => error_reply(grpc_query::Code::Internal, "unknown response type").into(),
            }
        }
        Err((code, message, height)) => {
            let mut reply = DetailedReply::from(error_reply(code, &message));
            reply.metadata = ResponseMetadata {
                content_type: height.is_some(),
                height,
            };
            reply
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message as _;

    #[test]
    fn routes_from_bindings() {
        let (m, _, vars) =
            route("GET", "/cosmos/bank/v1beta1/balances/celestia1abc/by_denom").unwrap();
        assert_eq!(m.path, "/cosmos.bank.v1beta1.Query/Balance");
        assert_eq!(
            vars,
            vec![("address".to_owned(), "celestia1abc".to_owned())]
        );
        // An encoded slash is decoded before matching (Go's mux sees the
        // decoded path), so a single-segment variable does not match.
        assert!(route("GET", "/cosmos/bank/v1beta1/denoms_metadata/ibc%2FABC").is_none());
        let (m, _, vars) = route("GET", "/ibc/apps/transfer/v1/denom_traces/a/b").unwrap();
        assert_eq!(m.path, "/ibc.applications.transfer.v1.Query/DenomTrace");
        assert_eq!(vars[0].1, "a/b");
        assert!(route("GET", "/cosmos/mint/v1beta1/params").is_none());
    }

    #[test]
    fn go_base64_errors() {
        assert_eq!(gateway_bytes("AAEC").unwrap(), vec![0, 1, 2]);
        assert_eq!(gateway_bytes("AQ==").unwrap(), vec![1]);
        assert_eq!(
            gateway_bytes("celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7").unwrap_err(),
            "illegal base64 data at input byte 44"
        );
    }

    #[test]
    fn request_body_bytes_use_go_base64_errors() {
        let reply = handle(
            &Request {
                method: "POST",
                path: "/cosmos/tx/v1beta1/txs",
                query: "",
                height: None,
                body: br#"{"tx_bytes":"%%%","mode":"BROADCAST_MODE_SYNC"}"#,
            },
            &mut |_, _, _| panic!("invalid bytes must fail before dispatch"),
        );
        assert_eq!(
            reply,
            Reply {
                status: 400,
                body: json!({
                    "code": 3,
                    "message": "illegal base64 data at input byte 0",
                    "details": []
                }),
            }
        );
    }

    #[test]
    fn query_params_follow_the_schema() {
        let mut obj = Map::new();
        let msg = "cosmos.bank.v1beta1.QueryAllBalancesRequest";
        set_field(&mut obj, msg, &["pagination", "limit"], "2").unwrap();
        set_field(&mut obj, msg, &["pagination", "count_total"], "true").unwrap();
        set_field(&mut obj, msg, &["resolveDenom"], "true").unwrap();
        set_field(&mut obj, msg, &["nope"], "x").unwrap();
        assert_eq!(
            Value::Object(obj),
            json!({"pagination": {"limit": "2", "count_total": true}, "resolve_denom": true})
        );
    }

    #[test]
    fn governance_v1_query_enum_accepts_number_and_name() {
        let msg = "cosmos.gov.v1.QueryProposalsRequest";
        let field = services::message(msg)
            .unwrap()
            .field("proposal_status")
            .unwrap();
        assert_eq!(param_value(msg, field, "2").unwrap(), json!(2));
        assert_eq!(
            param_value(msg, field, "PROPOSAL_STATUS_VOTING_PERIOD").unwrap(),
            json!("PROPOSAL_STATUS_VOTING_PERIOD")
        );
    }

    #[test]
    fn go_rest_height_boundaries() {
        let fixture: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../conformance/mainnet-rpc/rest-boundaries/v10.5.0/rest-heights.json"
        )))
        .unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 42);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let heights = case["heights"].as_array().unwrap();
            // net/http trims optional whitespace and grpc-gateway ignores a
            // repeated metadata value rather than choosing one of them.
            let height =
                (heights.len() == 1).then(|| heights[0].as_str().unwrap().trim().to_owned());
            let expected_body = case["body"].as_str().unwrap();
            let expected_json: Value = serde_json::from_str(expected_body).unwrap();
            let expected_message = expected_json["message"].as_str().unwrap_or_default();
            let req = Request {
                method: "GET",
                path: case["path"].as_str().unwrap(),
                query: "",
                height: height.as_deref(),
                body: &[],
            };
            let mut calls = 0;
            let reply = handle_with_metadata(&req, &mut |path, _, requested_height| {
                calls += 1;
                let resolved_height = if requested_height == 0 {
                    20
                } else {
                    requested_height
                };
                if requested_height > 20 {
                    return Err((grpc_query::Code::Unknown, expected_message.to_owned(), None));
                }
                if path == "/cosmos.bank.v1beta1.Query/Params" {
                    let response = celestia_proto::cosmos::bank::v1beta1::QueryParamsResponse {
                        params: Some(celestia_proto::cosmos::bank::v1beta1::Params {
                            default_send_enabled: true,
                            ..Default::default()
                        }),
                    };
                    Ok((response.encode_to_vec(), resolved_height))
                } else {
                    Err((
                        grpc_query::Code::InvalidArgument,
                        expected_message.to_owned(),
                        Some(resolved_height),
                    ))
                }
            });
            assert_eq!(
                reply.reply.status,
                u16::try_from(case["status"].as_u64().unwrap()).unwrap(),
                "{name}"
            );
            assert_eq!(
                crate::reply_body_bytes(&reply),
                expected_body.as_bytes(),
                "{name}"
            );
            let grpc_headers: Vec<_> = case["headers"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|pair| {
                    let pair = pair.as_array().unwrap();
                    pair[0]
                        .as_str()
                        .unwrap()
                        .to_ascii_lowercase()
                        .starts_with("grpc-metadata-")
                        .then(|| {
                            (
                                pair[0].as_str().unwrap().to_ascii_lowercase(),
                                pair[1].as_str().unwrap().to_owned(),
                            )
                        })
                })
                .collect();
            assert_eq!(
                reply.metadata.content_type,
                grpc_headers
                    .iter()
                    .any(|(name, _)| name == "grpc-metadata-content-type"),
                "{name}"
            );
            assert_eq!(
                reply.metadata.height.map(|height| height.to_string()),
                grpc_headers
                    .iter()
                    .find(|(name, _)| name == "grpc-metadata-x-cosmos-block-height")
                    .map(|(_, value)| value.clone()),
                "{name}"
            );
            if name.starts_with("unknown:") {
                assert_eq!(calls, 0, "unknown routes must precede height parsing");
            }
        }
    }
}
