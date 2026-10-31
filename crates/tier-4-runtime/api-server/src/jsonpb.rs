//! Go `jsonpb` (gogoproto, `OrigName` + `EmitDefaults`) rendering on top of
//! the generated proto3-JSON (pbjson) form.
//!
//! The two agree on field names, 64-bit integers as strings, bytes as
//! base64, enums by name, `Any` with `@type`, timestamps and durations. They
//! differ where gogoproto options change the Go type:
//!
//! - unset message fields are `null` in Go (pbjson omits them);
//! - `customtype` fields use the custom type's `MarshalJSON`:
//!   `math.LegacyDec` is a decimal string (the wire form is the scaled
//!   integer), `math.Int` the integer string, and a nullable custom field
//!   that is unset is `null`;
//! - `casttype` `HexBytes` fields marshal as upper-case hex;
//! - `cosmos.ics23.v1` messages come from the `ics23` crate (camelCase,
//!   defaults omitted) and are normalized to the gogoproto form.

use celestia_proto::services;
use celestia_proto::wire_table::{self, ty};
use serde_json::Value;

const DEC: &str = "cosmossdk.io/math.LegacyDec";
const INT: &str = "cosmossdk.io/math.Int";

fn decode_b64(s: &str) -> Option<Vec<u8>> {
    use std::collections::BTreeMap;
    // Standard alphabet with padding (pbjson's encoding).
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let table: BTreeMap<u8, u32> = alphabet
        .iter()
        .enumerate()
        .map(|(i, c)| (*c, u32::try_from(i).unwrap_or_default()))
        .collect();
    let mut out = Vec::new();
    let mut acc = 0u32;
    let mut bits = 0u32;
    for c in s.bytes().filter(|c| *c != b'=') {
        acc = (acc << 6) | table.get(&c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xff).unwrap_or_default());
        }
    }
    Some(out)
}

/// Converts the wire text of a customtype value to Go's JSON form.
fn custom_json(custom: &str, wire: &str, nullable: bool) -> Value {
    if wire.is_empty() && nullable {
        return Value::Null;
    }
    match custom {
        DEC => math::LegacyDec::unmarshal(wire.as_bytes()).map_or_else(
            |_| Value::String(wire.to_owned()),
            |d| Value::String(d.to_string()),
        ),
        INT if wire.is_empty() => Value::String("0".into()),
        _ => Value::String(wire.to_owned()),
    }
}

fn convert_custom(v: &mut Value, field: &services::Field, info: &wire_table::FieldInfo) {
    let one = |x: &Value| -> Value {
        let Value::String(s) = x else {
            return x.clone();
        };
        let wire = if field.ty == ty::BYTES {
            decode_b64(s).map_or_else(|| s.clone(), |b| String::from_utf8_lossy(&b).into_owned())
        } else {
            s.clone()
        };
        custom_json(info.custom, &wire, info.nullable && !field.repeated)
    };
    match v {
        Value::Array(items) => {
            for x in items {
                *x = one(x);
            }
        }
        other => *other = one(other),
    }
}

/// Go formats UTC timestamps with `Z` (pbjson-types uses `+00:00`).
fn go_timestamps(v: &mut Value) {
    match v {
        Value::String(s) => {
            if let Some(t) = s.strip_suffix("+00:00") {
                *s = format!("{t}Z");
            }
        }
        Value::Array(items) => items.iter_mut().for_each(go_timestamps),
        _ => {}
    }
}

/// `time.Time.MarshalJSON` (RFC 3339 with trailing fraction zeros removed)
/// from a jsonpb timestamp.
fn rfc3339_nano(v: &mut Value) {
    let Value::String(s) = v else {
        return;
    };
    if let Some((head, frac)) = s.strip_suffix('Z').and_then(|t| t.split_once('.')) {
        let frac = frac.trim_end_matches('0');
        *s = if frac.is_empty() {
            format!("{head}Z")
        } else {
            format!("{head}.{frac}Z")
        };
    }
}

/// Removes `key` when it holds a zero/empty value (`json:",omitempty"`),
/// otherwise turns a 64-bit integer string into a JSON number (Go
/// `encoding/json` renders int64/uint64 as numbers).
fn omitempty_number(obj: &mut serde_json::Map<String, Value>, key: &str) {
    match obj.get(key) {
        Some(Value::String(s)) if s == "0" || s.is_empty() => {
            obj.remove(key);
        }
        Some(Value::String(s)) => {
            if let Ok(n) = s.parse::<i64>() {
                obj.insert(key.to_owned(), Value::from(n));
            } else if let Ok(n) = s.parse::<u64>() {
                obj.insert(key.to_owned(), Value::from(n));
            }
        }
        _ => {}
    }
}

fn omitempty_string(obj: &mut serde_json::Map<String, Value>, keys: &[&str]) {
    for k in keys {
        if obj.get(*k).and_then(Value::as_str) == Some("") {
            obj.remove(*k);
        }
    }
}

fn empty_array_to_null(obj: &mut serde_json::Map<String, Value>, key: &str) {
    if obj
        .get(key)
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
    {
        obj.insert(key.to_owned(), Value::Null);
    }
}

/// Staking's `DelegationResponse` and `RedelegationResponse` implement
/// `json.Marshaler` with `encoding/json` over the gogoproto structs
/// (`omitempty` tags, numeric int64s, `time.Time` formatting).
fn staking_encoding_json(v: &mut Value, msg: &str) {
    let Value::Object(obj) = v else {
        return;
    };
    match msg {
        "cosmos.staking.v1beta1.DelegationResponse" => {
            if let Some(Value::Object(d)) = obj.get_mut("delegation") {
                omitempty_string(d, &["delegator_address", "validator_address"]);
            }
            if let Some(Value::Object(c)) = obj.get_mut("balance") {
                omitempty_string(c, &["denom"]);
            }
        }
        "cosmos.staking.v1beta1.RedelegationResponse" => {
            if let Some(Value::Object(r)) = obj.get_mut("redelegation") {
                omitempty_string(
                    r,
                    &[
                        "delegator_address",
                        "validator_src_address",
                        "validator_dst_address",
                    ],
                );
                empty_array_to_null(r, "entries");
            }
            if let Some(Value::Array(entries)) = obj.get_mut("entries") {
                for e in entries {
                    if let Some(Value::Object(re)) = e.get_mut("redelegation_entry") {
                        omitempty_number(re, "creation_height");
                        omitempty_number(re, "unbonding_id");
                        omitempty_number(re, "unbonding_on_hold_ref_count");
                        if let Some(t) = re.get_mut("completion_time") {
                            rfc3339_nano(t);
                        }
                    }
                }
            }
            empty_array_to_null(obj, "entries");
        }
        _ => {}
    }
}

/// Rewrites `v` (pbjson output for message `msg`) into Go's jsonpb form.
pub fn go_style(v: &mut Value, msg: &str) {
    go_style_fields(v, msg);
    staking_encoding_json(v, msg);
}

/// Serializes jsonpb output in protobuf declaration order, as Go's
/// reflection-based jsonpb marshaler does.
#[must_use]
pub fn descriptor_json(v: &Value, msg: &str) -> String {
    let mut out = String::new();
    write_descriptor_json(v, Some(msg), &mut out);
    out
}

fn write_descriptor_json(v: &Value, msg: Option<&str>, out: &mut String) {
    fn write_field(
        out: &mut String,
        first: &mut bool,
        written: &mut std::collections::BTreeSet<String>,
        name: &str,
        value: &Value,
        nested: Option<&str>,
    ) {
        if !*first {
            out.push(',');
        }
        *first = false;
        out.push_str(&serde_json::to_string(name).unwrap_or_else(|_| "\"\"".into()));
        out.push(':');
        write_descriptor_json(value, nested, out);
        written.insert(name.to_owned());
    }

    let Value::Object(obj) = v else {
        if let Value::Array(values) = v {
            out.push('[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    out.push(',');
                }
                write_descriptor_json(value, msg, out);
            }
            out.push(']');
        } else {
            out.push_str(&serde_json::to_string(v).unwrap_or_else(|_| "null".into()));
        }
        return;
    };

    let any_type = (msg == Some("google.protobuf.Any"))
        .then(|| obj.get("@type").and_then(Value::as_str))
        .flatten()
        .map(|name| name.trim_start_matches('/'));
    let schema_name = any_type.or(msg);
    let schema = schema_name.and_then(services::message);
    let mut written = std::collections::BTreeSet::new();
    let mut first = true;
    out.push('{');
    if schema.is_some_and(|message| message.map_entry) {
        let value_type = schema
            .and_then(|message| message.fields.iter().find(|field| field.number == 2))
            .and_then(|field| (field.ty == ty::MESSAGE).then_some(field.type_name));
        for (name, value) in obj {
            write_field(out, &mut first, &mut written, name, value, value_type);
        }
        out.push('}');
        return;
    }
    if let Some(value) = obj.get("@type") {
        write_field(out, &mut first, &mut written, "@type", value, None);
    }
    if let Some(schema) = schema {
        for descriptor in schema.fields {
            let Some(value) = obj.get(descriptor.name) else {
                continue;
            };
            let nested = (descriptor.ty == ty::MESSAGE).then_some(descriptor.type_name);
            write_field(
                out,
                &mut first,
                &mut written,
                descriptor.name,
                value,
                nested,
            );
        }
    }
    for (name, value) in obj {
        if !written.contains(name) {
            write_field(out, &mut first, &mut written, name, value, None);
        }
    }
    out.push('}');
}

fn go_style_fields(v: &mut Value, msg: &str) {
    if msg.starts_with("google.protobuf.") && msg != "google.protobuf.Any" {
        return;
    }
    let Value::Object(obj) = v else {
        return;
    };
    if msg == "google.protobuf.Any" {
        let Some(inner) = obj
            .get("@type")
            .and_then(Value::as_str)
            .map(|t| t.trim_start_matches('/').to_owned())
        else {
            return;
        };
        go_style(v, &inner);
        return;
    }
    let Some(schema) = services::message(msg) else {
        return;
    };
    let ics23 = msg.starts_with("cosmos.ics23.");
    let wire = wire_table::message(msg);
    for field in schema.fields {
        if field.json_name != field.name
            && let Some(v) = obj.remove(field.json_name)
        {
            obj.insert(field.name.to_owned(), v);
        }
        if ics23
            && !obj.contains_key(field.name)
            && let Some(d) = scalar_default(field)
        {
            obj.insert(field.name.to_owned(), d);
        }
        if CAST_HEX.contains(&(msg, field.name)) {
            if let Some(v) = obj.get_mut(field.name)
                && let Some(b) = v.as_str().and_then(decode_b64)
            {
                *v = Value::String(upper_hex(&b));
            }
            continue;
        }
        let info = wire.and_then(|w| w.field(field.number));
        let Some(value) = obj.get_mut(field.name) else {
            let is_message = field.ty == ty::MESSAGE && !field.repeated;
            let nullable_custom = info.is_some_and(|i| !i.custom.is_empty() && i.nullable);
            if field.oneof == 0 && (is_message || nullable_custom) {
                obj.insert(field.name.to_owned(), Value::Null);
            }
            continue;
        };
        if let Some(info) = info.filter(|i| !i.custom.is_empty()) {
            convert_custom(value, field, info);
            continue;
        }
        // Proto3 never encodes empty bytes, so Go's (gateway-decoded)
        // value is a nil slice, which jsonpb renders as null.
        if field.ty == ty::BYTES && !field.repeated && value.as_str() == Some("") {
            *value = Value::Null;
            continue;
        }
        if field.ty != ty::MESSAGE {
            continue;
        }
        if field.type_name == "google.protobuf.Timestamp" {
            go_timestamps(value);
            continue;
        }
        let entry = services::message(field.type_name);
        match value {
            Value::Object(map) if entry.is_some_and(|e| e.map_entry) => {
                let value_ty = entry
                    .and_then(|e| e.fields.iter().find(|f| f.number == 2))
                    .filter(|f| f.ty == ty::MESSAGE)
                    .map(|f| f.type_name);
                if let Some(t) = value_ty {
                    for x in map.values_mut() {
                        go_style(x, t);
                    }
                }
            }
            Value::Array(items) => {
                for x in items {
                    go_style(x, field.type_name);
                }
            }
            other => go_style(other, field.type_name),
        }
    }
}

/// `bytes` fields with `(gogoproto.casttype) = "…/libs/bytes.HexBytes"`.
const CAST_HEX: &[(&str, &str)] = &[(
    "ibc.lightclients.tendermint.v1.ConsensusState",
    "next_validators_hash",
)];

fn upper_hex(b: &[u8]) -> String {
    use std::fmt::Write;
    b.iter().fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02X}");
        s
    })
}

/// Go `EmitDefaults` value of an unset non-message field.
fn scalar_default(field: &services::Field) -> Option<Value> {
    if field.repeated {
        return Some(Value::Array(Vec::new()));
    }
    Some(match field.ty {
        ty::BOOL => Value::Bool(false),
        ty::STRING => Value::String(String::new()),
        ty::BYTES => Value::Null,
        ty::INT64 | ty::UINT64 | ty::SINT64 | ty::FIXED64 | ty::SFIXED64 => {
            Value::String("0".into())
        }
        ty::ENUM => Value::String(
            match field.type_name {
                "cosmos.ics23.v1.HashOp" => "NO_HASH",
                "cosmos.ics23.v1.LengthOp" => "NO_PREFIX",
                _ => return None,
            }
            .into(),
        ),
        ty::MESSAGE => return None,
        _ => Value::from(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nulls_and_custom_types() {
        // DelegationResponse.delegation.shares is a LegacyDec (wire form).
        let mut v = json!({
            "delegation_response": {
                "delegation": {
                    "delegator_address": "a",
                    "validator_address": "b",
                    "shares": "1500000000000000000"
                }
            }
        });
        go_style(&mut v, "cosmos.staking.v1beta1.QueryDelegationResponse");
        assert_eq!(
            v["delegation_response"]["delegation"]["shares"],
            "1.500000000000000000"
        );
        assert_eq!(v["delegation_response"]["balance"], Value::Null);
    }

    #[test]
    fn base64() {
        assert_eq!(decode_b64("MTIz").unwrap(), b"123");
        assert_eq!(decode_b64("MQ==").unwrap(), b"1");
    }
}
