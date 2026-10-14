//! Go `encoding/json` / gogoproto `jsonpb` behaviors that end up in
//! consensus state (packet commitments, acknowledgements): Go's string
//! escaping (HTML-safe), `float64` number formatting, `sdk.MustSortJSON`,
//! and `jsonpb`'s "first value only" decoding.

use serde_json::Value;

/// Go `encoding/json` string encoding (with `SetEscapeHTML(true)`, the
/// `json.Marshal` default), including the surrounding quotes.
#[must_use]
pub fn escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => {
                let _ = core::fmt::Write::write_fmt(&mut out, format_args!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
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

/// `sdk.MustSortJSON`: re-marshals JSON with sorted keys.
pub fn sort_json(bz: &[u8]) -> Result<String, String> {
    let v: Value = serde_json::from_slice(bz).map_err(|e| e.to_string())?;
    Ok(marshal_sorted(&v))
}

/// Decodes the first JSON value of `bz` (a `json.Decoder` reads one value
/// and ignores whatever follows).
pub fn first_value(bz: &[u8]) -> Result<Value, String> {
    let mut it = serde_json::Deserializer::from_slice(bz).into_iter::<Value>();
    match it.next() {
        Some(Ok(v)) => Ok(v),
        Some(Err(e)) => Err(e.to_string()),
        None => Err("EOF".into()),
    }
}

/// `base64.StdEncoding.DecodeString` as `encoding/json` uses it for `[]byte`
/// (padding required; `\r` and `\n` are ignored).
#[must_use]
pub fn decode_base64_std(s: &str) -> Option<Vec<u8>> {
    let clean: Vec<u8> = s.bytes().filter(|b| *b != b'\r' && *b != b'\n').collect();
    if !clean.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(clean.len() / 4 * 3);
    for chunk in clean.chunks(4) {
        let mut n = 0u32;
        let mut pad = 0;
        for (i, &c) in chunk.iter().enumerate() {
            let v = match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' if i >= 2 => {
                    pad += 1;
                    0
                }
                _ => return None,
            };
            if pad > 0 && c != b'=' {
                return None;
            }
            n = (n << 6) | u32::from(v);
        }
        let bytes = n.to_be_bytes();
        out.extend_from_slice(&bytes[1..4 - pad]);
    }
    Some(out)
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
        assert!(decode_string_fields(br#"{"denom":1}"#, &["denom"]).is_err());
    }
}
