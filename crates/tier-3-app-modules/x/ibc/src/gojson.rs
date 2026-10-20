//! Go `encoding/json` / gogoproto `jsonpb` behaviors that end up in
//! consensus state (packet commitments, acknowledgements): Go's string
//! escaping (HTML-safe), `float64` number formatting, `sdk.MustSortJSON`,
//! and `jsonpb`'s "first value only" decoding.

use serde_json::Value;

/// Go `encoding/json` string encoding (Go 1.26, HTML-safe: `json.Marshal`'s
/// default), including the surrounding quotes, of the Go string `s` (Go form, `gostr`): each invalid UTF-8 byte is written as
/// `\ufffd`. Returns a Go form (bytes via `gostr::to_go_bytes`).
#[must_use]
pub fn escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for r in gostr::runes(s) {
        let Some(c) = r else {
            out.push_str("\\ufffd");
            continue;
        };
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Short escapes since Go 1.22.
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => {
                let _ = core::fmt::Write::write_fmt(&mut out, format_args!("\\u{:04x}", c as u32));
            }
            c => gostr::push_char(&mut out, c),
        }
    }
    out.push('"');
    out
}

/// Go's `encoding/json` `float64` formatting.
#[must_use]
pub fn format_float(f: f64) -> String {
    let abs = f.abs();
    if abs == 0.0 || (1e-6..1e21).contains(&abs) {
        let s = format!("{f}");
        return s;
    }
    let s = format!("{f:e}");
    // Rust: "1e21" / "1.5e-7"; Go: "1e+21" / "1.5e-07" cleaned to "1.5e-7".
    match s.split_once('e') {
        Some((m, e)) if !e.starts_with('-') => format!("{m}e+{e}"),
        _ => s,
    }
}

/// `json.Marshal` of a decoded `interface{}` value: object keys sorted (Go
/// maps), numbers as `float64`.
#[must_use]
pub fn marshal_sorted(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => format_float(n.as_f64().unwrap_or_default()),
        Value::String(s) => escape_string(s),
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(marshal_sorted).collect::<Vec<_>>().join(",")
        ),
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .into_iter()
                .map(|k| format!("{}:{}", escape_string(k), marshal_sorted(&m[k])))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
    }
}

/// The text Go's JSON decoder reads from `bz`, as a Go form: invalid UTF-8
/// inside a string decodes to U+FFFD (`encoding/json` `unquote`), and
/// anywhere else stays an invalid character (a syntax error either way).
#[must_use]
pub fn go_json_input(bz: &[u8]) -> String {
    gostr::lossy(&gostr::from_go_bytes(bz)).into_owned()
}

/// `sdk.MustSortJSON`: re-marshals JSON with sorted keys.
pub fn sort_json(bz: &[u8]) -> Result<String, String> {
    let v = to_serde(&gojson::parse(bz)?)?;
    Ok(marshal_sorted(&v))
}

/// Values nested deeper than this are not converted to `serde_json`
/// (whose recursive code they would strain): no message the `jsonpb`
/// decoders of this crate's callers read nests that deep (Go fails such
/// an input on an unknown field or a type mismatch, or at the interface
/// registry's 10 nested `Any`s), so the call fails either way.
pub const MAX_SERDE_DEPTH: usize = 128;

/// A `gojson` tree as a `serde_json` value (Go's unquoted strings; a
/// number out of `float64` range is an error, as for every `jsonpb`
/// numeric field).
fn to_serde(v: &gojson::Value) -> Result<Value, String> {
    fn conv(v: &gojson::Value, depth: usize) -> Result<Value, String> {
        use gojson::Kind;
        if depth > MAX_SERDE_DEPTH {
            return Err("exceeded max depth".into());
        }
        Ok(match &v.kind {
            Kind::Null => Value::Null,
            Kind::Bool(b) => Value::Bool(*b),
            Kind::Number(n) => serde_json::from_str::<serde_json::Number>(n)
                .map(Value::Number)
                .map_err(|e| e.to_string())?,
            Kind::String(s) => Value::String(s.clone()),
            Kind::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|x| conv(x, depth + 1))
                    .collect::<Result<_, _>>()?,
            ),
            Kind::Object(members) => {
                let mut m = serde_json::Map::new();
                for (k, x) in members {
                    m.insert(k.clone(), conv(x, depth + 1)?);
                }
                Value::Object(m)
            }
        })
    }
    conv(v, 0)
}

/// Decodes the first JSON value of `bz` as `json.Decoder` does (`jsonpb`
/// reads one value and ignores whatever follows): Go's scanner (10 000
/// nested levels) and string unquoting (a lone surrogate escape or invalid
/// UTF-8 inside a string is U+FFFD); values deeper than
/// [`MAX_SERDE_DEPTH`] are an error.
pub fn first_value(bz: &[u8]) -> Result<Value, String> {
    to_serde(&gojson::parse_first(bz)?)
}

/// `base64.StdEncoding.DecodeString` as `encoding/json` uses it for `[]byte`
/// (padding required; `\r` and `\n` are ignored).
#[must_use]
pub fn decode_base64_std(s: &str) -> Option<Vec<u8>> {
    gojson::decode_base64_std(s.as_bytes())
}

/// A string-only protobuf message decoded with `jsonpb` (unknown fields
/// rejected, `null` leaves a field unset, non-string values are errors).
pub fn decode_string_fields(bz: &[u8], fields: &[&str]) -> Result<Vec<String>, String> {
    let v = first_value(bz)?;
    let mut out = vec![String::new(); fields.len()];
    match v {
        Value::Null => return Ok(out),
        Value::Object(map) => {
            for (k, val) in map {
                let Some(i) = fields.iter().position(|f| *f == k) else {
                    return Err(format!("unknown field {k:?}"));
                };
                match val {
                    Value::Null => {}
                    Value::String(s) => out[i] = s,
                    _ => return Err(format!("cannot unmarshal into string field {k}")),
                }
            }
        }
        _ => return Err("cannot unmarshal into message".into()),
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_compatible_output() {
        assert_eq!(
            escape_string("a<b>&\"\n\u{1}"),
            "\"a\\u003cb\\u003e\\u0026\\\"\\n\\u0001\""
        );
        // Go 1.26 `json.Marshal("\b\f\x01<\u2028")`.
        assert_eq!(
            escape_string("\u{8}\u{c}\u{1}<\u{2028}"),
            r#""\b\f\u0001\u003c\u2028""#
        );
        assert_eq!(format_float(1.0), "1");
        assert_eq!(format_float(1e21), "1e+21");
        assert_eq!(format_float(1.5e-7), "1.5e-7");
        assert_eq!(format_float(0.5), "0.5");
        assert_eq!(
            sort_json(br#"{"b":1,"a":{"d":"x","c":[true,null]}}"#).unwrap(),
            r#"{"a":{"c":[true,null],"d":"x"},"b":1}"#
        );
        assert_eq!(decode_base64_std("AQ=="), Some(vec![1]));
        assert_eq!(decode_base64_std("AQ"), None);
        assert_eq!(decode_base64_std("YWJj"), Some(b"abc".to_vec()));
        assert_eq!(first_value(b"{\"a\":1} trailing").unwrap()["a"], 1);
        assert_eq!(
            decode_string_fields(br#"{"denom":"x","memo":null}"#, &["denom", "memo"]).unwrap(),
            vec!["x".to_owned(), String::new()]
        );
        assert!(decode_string_fields(br#"{"zz":"x"}"#, &["denom"]).is_err());
        // Go's unquoting (v10.4.0 oracle, `json_ics20`): a lone surrogate
        // escape is U+FFFD; serde_json rejected it.
        assert_eq!(
            decode_string_fields(br#"{"denom":"a\ud800b"}"#, &["denom"]).unwrap(),
            vec!["a\u{fffd}b".to_owned()]
        );
        let deep = format!("{}1{}", "[".repeat(129), "]".repeat(129));
        assert!(first_value(deep.as_bytes()).is_err());
        assert!(first_value(b"1e400").is_err());
        assert!(decode_string_fields(br#"{"denom":1}"#, &["denom"]).is_err());
    }
}
