//! The Go JSON behaviors PFM's memo handling depends on: `encoding/json`
//! decoding of the memo into a map and into `PacketMetadata` (the whole
//! memo validated first, at most 10 000 nested levels; every number a
//! `float64` in the map; struct fields matched exactly or by `EqualFold`;
//! type errors; the custom `Duration` decoder), the
//! `iancoleman/orderedmap`-backed `JSONObject` that re-encodes `next` (key
//! order preserved, duplicates moved to the end, numbers as `float64`), and
//! `json.Marshal`'s compaction of a marshaler's output. Values are decoded
//! with `gojson` (no recursion); the recursive re-encoding runs on a large
//! stack for long memos.

use gojson::{Kind, Value};
use x_ibc::gojson::{escape_string, format_float};

/// The Go form of a decoded (valid UTF-8) JSON string.
fn go(s: &str) -> String {
    gostr::from_go_bytes(s.as_bytes()).into_owned()
}

/// `orderedmap` normalization: last value wins, a repeated key moves to the
/// end.
fn dedup(members: &[(String, Value)]) -> Vec<(&str, &Value)> {
    let mut out: Vec<(&str, &Value)> = Vec::new();
    for (k, v) in members {
        if let Some(pos) = out.iter().position(|(ek, _)| ek == k) {
            out.remove(pos);
        }
        out.push((k, v));
    }
    out
}

/// A number literal as Go's `interface{}` decoding reads it: `None` when it
/// does not fit a `float64` (`strconv.ParseFloat` range error).
fn float64(lit: &str) -> Option<f64> {
    lit.parse::<f64>().ok().filter(|f| f.is_finite())
}

/// `json.Marshal` of the `interface{}` / `OrderedMap` tree (a Go form);
/// `None` when a number does not fit a `float64` (Go's decode fails).
fn marshal_ordered(v: &Value) -> Option<String> {
    Some(match &v.kind {
        Kind::Null => "null".into(),
        Kind::Bool(b) => b.to_string(),
        Kind::Number(lit) => format_float(float64(lit)?),
        Kind::String(s) => escape_string(&go(s)),
        Kind::Array(items) => {
            let parts: Option<Vec<String>> = items.iter().map(marshal_ordered).collect();
            format!("[{}]", parts?.join(","))
        }
        Kind::Object(members) => {
            let parts: Option<Vec<String>> = dedup(members)
                .into_iter()
                .map(|(k, v)| marshal_ordered(v).map(|s| format!("{}:{s}", escape_string(&go(k)))))
                .collect();
            format!("{{{}}}", parts?.join(","))
        }
    })
}

/// `json.Marshal`'s `appendCompact` (HTML escaping on) of a marshaler's
/// output `raw`: the scanner's error for invalid JSON (10 000 levels at
/// most), else `raw` without insignificant whitespace (a Go form).
pub fn compact(raw: &[u8]) -> Result<String, String> {
    gojson::check_valid(raw)?;
    let mut out = Vec::with_capacity(raw.len());
    let mut in_str = false;
    let mut esc = false;
    let mut i = 0;
    while i < raw.len() {
        let c = raw[i];
        i += 1;
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
        } else if matches!(c, b' ' | b'\t' | b'\n' | b'\r') {
            continue;
        } else if c == b'"' {
            in_str = true;
        }
        match c {
            b'<' => out.extend_from_slice(b"\\u003c"),
            b'>' => out.extend_from_slice(b"\\u003e"),
            b'&' => out.extend_from_slice(b"\\u0026"),
            // U+2028 / U+2029.
            0xe2 if raw.get(i) == Some(&0x80) && raw.get(i + 1).is_some_and(|b| b & !1 == 0xa8) => {
                out.extend_from_slice(if raw[i + 1] == 0xa8 {
                    b"\\u2028"
                } else {
                    b"\\u2029"
                });
                i += 2;
            }
            c => out.push(c),
        }
    }
    Ok(gostr::from_go_vec(out))
}

/// `json.Marshal(metadata.Next)` for a `JSONObject` decoded from `v` (a
/// value of `src`).
fn marshal_next(v: &Value, src: &[u8]) -> Result<String, String> {
    let primitive: Vec<u8> = match &v.kind {
        Kind::Object(_) => match marshal_ordered(v) {
            Some(s) => return Ok(s),
            None => src[v.span.clone()].to_vec(),
        },
        Kind::String(s) => s.as_bytes().to_vec(),
        // `JSONObject.UnmarshalJSON("null")` leaves an empty primitive.
        Kind::Null => Vec::new(),
        _ => src[v.span.clone()].to_vec(),
    };
    compact(&primitive)
        .map_err(|e| format!("json: error calling MarshalJSON for type *types.JSONObject: {e}"))
}

/// The keeper's retry path: `json.Unmarshal(memo, metadata.Next)` (`Err`:
/// its error) and then `json.Marshal(metadata.Next)` (the inner result).
pub fn unmarshal_next(memo: &str) -> Result<Result<String, String>, String> {
    let src = gostr::to_go_bytes(memo);
    gojson::on_large_stack(src.len(), || {
        let v = gojson::parse(&src)?;
        Ok(marshal_next(&v, &src))
    })
}

/// Go's `time.quote`.
fn time_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        if (c as u32) >= 0x80 || c < ' ' {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                let _ = core::fmt::Write::write_fmt(&mut out, format_args!("\\x{b:02x}"));
            }
        } else {
            if c == '"' || c == '\\' {
                out.push('\\');
            }
            out.push(c);
        }
    }
    out.push('"');
    out
}

/// Go's `time.ParseDuration` (nanoseconds).
#[allow(clippy::too_many_lines, clippy::many_single_char_names)]
pub fn parse_duration(orig: &str) -> Result<i64, String> {
    const LIMIT: u64 = 1 << 63;
    let invalid = || format!("time: invalid duration {}", time_quote(orig));
    let mut s = orig;
    let mut neg = false;
    if let Some(c) = s.chars().next()
        && (c == '-' || c == '+')
    {
        neg = c == '-';
        s = &s[1..];
    }
    if s == "0" {
        return Ok(0);
    }
    if s.is_empty() {
        return Err(invalid());
    }
    let mut d: u64 = 0;
    while !s.is_empty() {
        let b0 = s.as_bytes()[0];
        if !(b0 == b'.' || b0.is_ascii_digit()) {
            return Err(invalid());
        }
        // leadingInt
        let pl = s.len();
        let mut v: u64 = 0;
        let mut i = 0;
        let bytes = s.as_bytes();
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            if v > LIMIT / 10 {
                return Err(invalid());
            }
            v = v * 10 + u64::from(bytes[i] - b'0');
            if v > LIMIT {
                return Err(invalid());
            }
            i += 1;
        }
        s = &s[i..];
        let pre = pl != s.len();
        let mut post = false;
        let mut f: u64 = 0;
        let mut scale = 1f64;
        if s.as_bytes().first() == Some(&b'.') {
            s = &s[1..];
            let pl = s.len();
            let bytes = s.as_bytes();
            let mut i = 0;
            let mut overflow = false;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                if !overflow {
                    if f > (LIMIT - 1) / 10 {
                        overflow = true;
                    } else {
                        let y = f * 10 + u64::from(bytes[i] - b'0');
                        if y > LIMIT {
                            overflow = true;
                        } else {
                            f = y;
                            scale *= 10.0;
                        }
                    }
                }
                i += 1;
            }
            s = &s[i..];
            post = pl != s.len();
        }
        if !pre && !post {
            return Err(invalid());
        }
        let i = s
            .bytes()
            .position(|c| c == b'.' || c.is_ascii_digit())
            .unwrap_or(s.len());
        if i == 0 {
            return Err(format!(
                "time: missing unit in duration {}",
                time_quote(orig)
            ));
        }
        let u = &s[..i];
        s = &s[i..];
        let unit: u64 = match u {
            "ns" => 1,
            "us" | "\u{b5}s" | "\u{3bc}s" => 1_000,
            "ms" => 1_000_000,
            "s" => 1_000_000_000,
            "m" => 60_000_000_000,
            "h" => 3_600_000_000_000,
            _ => {
                return Err(format!(
                    "time: unknown unit {} in duration {}",
                    time_quote(u),
                    time_quote(orig)
                ));
            }
        };
        if v > LIMIT / unit {
            return Err(invalid());
        }
        v *= unit;
        if f > 0 {
            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss
            )]
            let add = (f as f64 * (unit as f64 / scale)) as u64;
            v = v.wrapping_add(add);
            if v > LIMIT {
                return Err(invalid());
            }
        }
        d = d.wrapping_add(v);
        if d > LIMIT {
            return Err(invalid());
        }
    }
    if neg {
        #[allow(clippy::cast_possible_wrap)]
        return Ok((d as i64).wrapping_neg());
    }
    if d > LIMIT - 1 {
        return Err(invalid());
    }
    #[allow(clippy::cast_possible_wrap)]
    Ok(d as i64)
}

/// Go's `int64(f)` on amd64 (out of range and NaN give `MinInt64`).
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
pub fn go_f64_to_i64(f: f64) -> i64 {
    if f.is_nan() || f >= 9_223_372_036_854_775_808.0 || f < -9_223_372_036_854_775_808.0 {
        i64::MIN
    } else {
        f.trunc() as i64
    }
}

/// `types.ForwardMetadata`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ForwardMetadata {
    /// `receiver`.
    pub receiver: String,
    /// `port`.
    pub port: String,
    /// `channel`.
    pub channel: String,
    /// `timeout` (nanoseconds).
    pub timeout: i64,
    /// `retries`.
    pub retries: Option<u8>,
    /// `next`: `json.Marshal(metadata.Next)` (the next hop's memo, or the
    /// marshaling error Go returns when it forwards).
    pub next: Option<Result<String, String>>,
}

fn type_error(value: &str, strukt: &str, field: &str, ty: &str) -> String {
    format!("json: cannot unmarshal {value} into Go struct field {strukt}.{field} of type {ty}")
}

/// Go's struct field matching: the exact name, else `foldName` (Unicode
/// simple folding: `ſ` matches `s`, `K` (Kelvin) matches `k`).
fn field_is(key: &str, name: &str) -> bool {
    key == name || gostr::equal_fold(&go(key), name)
}

fn decode_forward(
    m: &mut ForwardMetadata,
    members: &[(String, Value)],
    src: &[u8],
    saved: &mut Option<String>,
) -> Result<(), String> {
    let mut save = |e: String| {
        if saved.is_none() {
            *saved = Some(e);
        }
    };
    for (k, v) in members {
        let string_field =
            |dst: &mut String, name: &str, save: &mut dyn FnMut(String)| match &v.kind {
                Kind::String(s) => *dst = go(s),
                Kind::Null => {}
                other => save(type_error(
                    other.type_name(),
                    "ForwardMetadata",
                    &format!("forward.{name}"),
                    "string",
                )),
            };
        if field_is(k, "receiver") {
            string_field(&mut m.receiver, "receiver", &mut save);
        } else if field_is(k, "port") {
            string_field(&mut m.port, "port", &mut save);
        } else if field_is(k, "channel") {
            string_field(&mut m.channel, "channel", &mut save);
        } else if field_is(k, "timeout") {
            m.timeout = match &v.kind {
                Kind::Number(lit) => {
                    let Some(f) = float64(lit) else {
                        return Err(format!(
                            "json: cannot unmarshal number {lit} into Go value of type float64"
                        ));
                    };
                    go_f64_to_i64(f)
                }
                Kind::String(s) => parse_duration(s)?,
                _ => return Err("invalid duration".into()),
            };
        } else if field_is(k, "retries") {
            match &v.kind {
                Kind::Null => m.retries = None,
                Kind::Number(lit) => match lit.parse::<u64>() {
                    Ok(n) if n <= 255 => m.retries = Some(u8::try_from(n).unwrap_or_default()),
                    _ => {
                        m.retries.get_or_insert(0);
                        save(type_error(
                            &format!("number {lit}"),
                            "ForwardMetadata",
                            "forward.retries",
                            "uint8",
                        ));
                    }
                },
                other => {
                    m.retries.get_or_insert(0);
                    save(type_error(
                        other.type_name(),
                        "ForwardMetadata",
                        "forward.retries",
                        "uint8",
                    ));
                }
            }
        } else if field_is(k, "next") {
            m.next = match &v.kind {
                Kind::Null => None,
                _ => Some(marshal_next(v, src)),
            };
        }
    }
    Ok(())
}

/// The memo's routing decision.
#[derive(Debug, Clone, PartialEq)]
pub enum Memo {
    /// Not a forward (`json.Unmarshal` into a map failed or `forward` is
    /// missing / null): the packet goes to the transfer app.
    NotForward,
    /// `json.Unmarshal(memo, &PacketMetadata{})` failed.
    ParseError(String),
    /// `forward` decoded to a nil pointer (Go then panics in `Validate`).
    NilForward,
    /// The forward metadata.
    Forward(ForwardMetadata),
}

/// Classifies a transfer memo as PFM's `OnRecvPacket` does.
#[must_use]
pub fn classify(memo: &str) -> Memo {
    let src = gostr::to_go_bytes(memo);
    gojson::on_large_stack(src.len(), || classify_bytes(&src))
}

fn classify_bytes(src: &[u8]) -> Memo {
    // `json.Unmarshal(memo, &map[string]interface{})`: valid JSON nested at
    // most 10 000 levels, an object (`null` leaves a nil map), every number
    // a `float64`.
    let Ok(root) = gojson::parse(src) else {
        return Memo::NotForward;
    };
    let Kind::Object(members) = &root.kind else {
        return Memo::NotForward;
    };
    if !root.all(|v| !matches!(&v.kind, Kind::Number(n) if float64(n).is_none())) {
        return Memo::NotForward;
    }
    // `d["forward"] == nil` on the map decode (last duplicate wins).
    match members.iter().rev().find(|(k, _)| k == "forward") {
        None => return Memo::NotForward,
        Some((_, v)) if matches!(v.kind, Kind::Null) => return Memo::NotForward,
        Some(_) => {}
    }
    let mut fwd: Option<ForwardMetadata> = None;
    let mut saved: Option<String> = None;
    for (k, v) in members {
        if !field_is(k, "forward") {
            continue;
        }
        match &v.kind {
            Kind::Null => fwd = None,
            Kind::Object(fields) => {
                let mut m = fwd.take().unwrap_or_default();
                if let Err(e) = decode_forward(&mut m, fields, src, &mut saved) {
                    return Memo::ParseError(e);
                }
                fwd = Some(m);
            }
            other => {
                fwd.get_or_insert_with(ForwardMetadata::default);
                if saved.is_none() {
                    saved = Some(type_error(
                        other.type_name(),
                        "PacketMetadata",
                        "forward",
                        "types.ForwardMetadata",
                    ));
                }
            }
        }
    }
    if let Some(e) = saved {
        return Memo::ParseError(e);
    }
    fwd.map_or(Memo::NilForward, Memo::Forward)
}

/// `ForwardMetadata.Validate`.
pub fn validate(m: &ForwardMetadata) -> Result<(), String> {
    if m.receiver.is_empty() {
        return Err("failed to validate metadata. receiver cannot be empty".into());
    }
    identifier(&m.port, 2, x_ibc::host::DEFAULT_MAX_PORT_CHARACTER_LENGTH)?;
    identifier(&m.channel, 8, x_ibc::host::DEFAULT_MAX_CHARACTER_LENGTH)?;
    Ok(())
}

/// `fmt.Errorf("failed to validate metadata: %w", err)` of a 24-host
/// identifier error. `%w` formats the `errorsmod` error with `%v`, which
/// appends the source location of its innermost `Wrap` (`[file:line]`, the
/// path cut after `github.com/`); that string ends up in the committed
/// error acknowledgement.
fn identifier(id: &str, min: usize, max: usize) -> Result<(), String> {
    const FILE: &str = "celestiaorg/ibc-go/v8@v8.7.2/modules/core/24-host/validate.go";
    let (msg, line) = if id.trim().is_empty() {
        ("identifier cannot be blank".to_owned(), 41)
    } else if id.contains('/') {
        (format!("identifier {id} cannot contain separator '/'"), 45)
    } else if id.len() < min || id.len() > max {
        (
            format!(
                "identifier {id} has invalid length: {}, must be between {min}-{max} characters",
                id.len()
            ),
            49,
        )
    } else if !x_ibc::host::is_valid_id(id) {
        (
            format!(
                "identifier {id} must contain only alphanumeric or the following characters: '.', '_', '+', '-', '#', '[', ']', '<', '>'"
            ),
            53,
        )
    } else {
        return Ok(());
    };
    Err(format!(
        "failed to validate metadata: {msg}: invalid identifier [{FILE}:{line}]"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_memos() {
        assert_eq!(classify(""), Memo::NotForward);
        assert_eq!(classify("hello"), Memo::NotForward);
        assert_eq!(classify("[1]"), Memo::NotForward);
        assert_eq!(classify(r#"{"forward":null}"#), Memo::NotForward);
        assert_eq!(classify(r#"{"Forward":{}}"#), Memo::NotForward);
        let Memo::Forward(m) = classify(
            r#"{"forward":{"receiver":"r","port":"transfer","channel":"channel-0","timeout":"1m","retries":2,"next":{"b":1,"a":"<x>","b":2}}}"#,
        ) else {
            panic!("expected forward");
        };
        assert_eq!(m.timeout, 60_000_000_000);
        assert_eq!(m.retries, Some(2));
        assert_eq!(
            m.next.clone().unwrap().unwrap(),
            "{\"a\":\"\\u003cx\\u003e\",\"b\":2}"
        );
        assert_eq!(
            classify(r#"{"forward":{"receiver":1}}"#),
            Memo::ParseError(
                "json: cannot unmarshal number into Go struct field ForwardMetadata.forward.receiver of type string".into()
            )
        );
        assert_eq!(
            classify(r#"{"forward":{"retries":300}}"#),
            Memo::ParseError(
                "json: cannot unmarshal number 300 into Go struct field ForwardMetadata.forward.retries of type uint8".into()
            )
        );
        assert_eq!(
            classify(r#"{"forward":"x"}"#),
            Memo::ParseError(
                "json: cannot unmarshal string into Go struct field PacketMetadata.forward of type types.ForwardMetadata".into()
            )
        );
        assert_eq!(
            classify(r#"{"forward":{"timeout":null}}"#),
            Memo::ParseError("invalid duration".into())
        );
    }

    #[test]
    fn next_primitives() {
        let next = |s: &str| unmarshal_next(s).unwrap();
        assert_eq!(next(r#""{\"a\": [1, 2]}""#).unwrap(), r#"{"a":[1,2]}"#);
        assert_eq!(next("[1.50, 2]").unwrap(), "[1.50,2]");
        assert_eq!(
            next(r#""abc""#).unwrap_err(),
            "json: error calling MarshalJSON for type *types.JSONObject: invalid character 'a' looking for beginning of value"
        );
        assert_eq!(
            next(r#"{"x":1.0,"y":{"z":1e21}}"#).unwrap(),
            r#"{"x":1,"y":{"z":1e+21}}"#
        );
        // An object with a number out of float64 range stays a primitive.
        assert_eq!(next(r#"{"x": 1e400}"#).unwrap(), r#"{"x":1e400}"#);
        assert_eq!(
            next("null").unwrap_err(),
            "json: error calling MarshalJSON for type *types.JSONObject: unexpected end of JSON input"
        );
        assert!(unmarshal_next("{").is_err());
    }

    /// Go's 10 000-level limit (checked against the v10.4.0 oracle,
    /// `json_pfm`): a deeper memo is no forward; recursion runs on a large
    /// stack.
    #[test]
    fn deep_memos() {
        let fwd = r#""forward":{"receiver":"r","port":"transfer","channel":"channel-0""#;
        let deep = |d: usize| format!("{}1{}", "[".repeat(d), "]".repeat(d));
        assert!(matches!(
            classify(&format!("{{{fwd}}},\"x\":{}}}", deep(9_999))),
            Memo::Forward(_)
        ));
        assert_eq!(
            classify(&format!("{{{fwd}}},\"x\":{}}}", deep(10_000))),
            Memo::NotForward
        );
        let next = format!("{}{{}}{}", "{\"a\":".repeat(9_997), "}".repeat(9_997));
        let Memo::Forward(m) = classify(&format!("{{{fwd},\"next\":{next}}}}}")) else {
            panic!("expected forward");
        };
        assert_eq!(m.next, Some(Ok(next)));
        assert!(unmarshal_next(&deep(10_001)).is_err());
        // Numbers the map decode cannot hold; `EqualFold` field names.
        assert_eq!(
            classify(&format!("{{{fwd}}},\"x\":1e400}}")),
            Memo::NotForward
        );
        let Memo::Forward(m) = classify(&format!("{{{fwd},\"retrie\u{17f}\":3}}}}")) else {
            panic!("expected forward");
        };
        assert_eq!(m.retries, Some(3));
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("1h30m"), Ok(5_400_000_000_000));
        assert_eq!(parse_duration("1.5s"), Ok(1_500_000_000));
        assert_eq!(parse_duration("-2ms"), Ok(-2_000_000));
        assert_eq!(parse_duration("0"), Ok(0));
        assert_eq!(
            parse_duration(""),
            Err("time: invalid duration \"\"".into())
        );
        assert_eq!(
            parse_duration("1"),
            Err("time: missing unit in duration \"1\"".into())
        );
        assert_eq!(
            parse_duration("1x"),
            Err("time: unknown unit \"x\" in duration \"1x\"".into())
        );
        assert_eq!(go_f64_to_i64(1.9), 1);
        assert_eq!(go_f64_to_i64(1e30), i64::MIN);
    }
}
