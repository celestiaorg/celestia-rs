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
//!
//! From app version 12 celestia-app runs the Celestia fork of PFM
//! ([`Pfm::Fork`], celestiaorg/ibc-apps#3): it keeps `next` and retried
//! memos as the sender's bytes, rejects numeric timeouts outside the
//! `int64` range, a forward that decodes to null and forward memos longer
//! than [`MAX_FORWARD_MEMO_LENGTH`].

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

pub use sdk::time::parse_duration;

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

/// The packet-forward-middleware release that handles a memo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pfm {
    /// Upstream v8.2.0 (app versions before 12).
    Upstream,
    /// The Celestia fork of v8.2.0 (celestiaorg/ibc-apps#3), from app
    /// version 12.
    Fork,
}

/// The fork's limit on a forward memo at receive:
/// `transfertypes.MaximumMemoLength`, in Go bytes.
pub const MAX_FORWARD_MEMO_LENGTH: usize = x_transfer::relay::MAXIMUM_MEMO_LENGTH;

/// 2^63: the fork accepts a numeric timeout `v` (a `float64`) only when
/// `-2^63 <= v < 2^63` (`maxDurationFloat`).
const MAX_DURATION_FLOAT: f64 = 9_223_372_036_854_775_808.0;

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
    /// `next`: the next hop's memo. Upstream, `json.Marshal(metadata.Next)`
    /// (or the marshaling error Go returns when it forwards); in the fork,
    /// `metadata.Next.Memo()`, never an error.
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
    pfm: Pfm,
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
                    if pfm == Pfm::Fork && !(-MAX_DURATION_FLOAT..MAX_DURATION_FLOAT).contains(&f) {
                        return Err("invalid duration: number out of range".into());
                    }
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
            m.next = match (&v.kind, pfm) {
                (Kind::Null, _) => None,
                (_, Pfm::Upstream) => Some(marshal_next(v, src)),
                // `JSONObject.UnmarshalJSON`: a string's unquoted content,
                // else the value's bytes.
                (Kind::String(s), Pfm::Fork) => Some(Ok(go(s))),
                (_, Pfm::Fork) => Some(Ok(gostr::from_go_bytes(&src[v.span.clone()]).into_owned())),
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

/// Classifies a transfer memo as upstream PFM's `OnRecvPacket` does.
#[must_use]
pub fn classify(memo: &str) -> Memo {
    classify_as(memo, Pfm::Upstream)
}

/// Classifies a transfer memo as `pfm`'s `OnRecvPacket` decodes it.
#[must_use]
pub fn classify_as(memo: &str, pfm: Pfm) -> Memo {
    let src = gostr::to_go_bytes(memo);
    gojson::on_large_stack(src.len(), || match forward_root(&src) {
        None => Memo::NotForward,
        Some(root) => decode_metadata(&root, &src, pfm),
    })
}

/// What `OnRecvPacket` does with a memo before it receives any funds.
#[derive(Debug, Clone, PartialEq)]
pub enum Recv {
    /// Not a forward: the packet goes to the transfer app.
    NotForward,
    /// An error acknowledgement with this error (upstream puts it in the
    /// acknowledgement; the fork logs it).
    Error(String),
    /// Upstream only: `forward` decoded to nil and Go panics.
    NilForwardPanic,
    /// Valid forward metadata.
    Forward(ForwardMetadata),
}

/// `OnRecvPacket` from the map decode to `metadata.Validate()`, for `pfm`.
#[must_use]
pub fn on_recv(memo: &str, pfm: Pfm) -> Recv {
    let src = gostr::to_go_bytes(memo);
    gojson::on_large_stack(src.len(), || {
        let Some(root) = forward_root(&src) else {
            return Recv::NotForward;
        };
        // The fork checks `len(data.Memo)` before it decodes the metadata.
        if pfm == Pfm::Fork && src.len() > MAX_FORWARD_MEMO_LENGTH {
            return Recv::Error(format!(
                "forward memo length {} exceeds {MAX_FORWARD_MEMO_LENGTH} bytes",
                src.len()
            ));
        }
        match decode_metadata(&root, &src, pfm) {
            Memo::NotForward => Recv::NotForward,
            Memo::ParseError(e) => Recv::Error(format!("error parsing forward metadata: {e}")),
            Memo::NilForward => match pfm {
                Pfm::Upstream => Recv::NilForwardPanic,
                Pfm::Fork => Recv::Error("forward metadata is null".into()),
            },
            Memo::Forward(m) => match validate_as(&m, pfm) {
                Ok(()) => Recv::Forward(m),
                Err(e) => Recv::Error(e),
            },
        }
    })
}

/// `RetryTimeout`'s `metadata.Next` for the memo of the timed-out packet:
/// `None` for the empty memo. Upstream decodes it (`Err`: the decode
/// error) and re-encodes it when it forwards; the fork keeps it as is.
pub fn retry_next(memo: &str, pfm: Pfm) -> Result<Option<Result<String, String>>, String> {
    if memo.is_empty() {
        return Ok(None);
    }
    match pfm {
        Pfm::Upstream => unmarshal_next(memo).map(Some),
        Pfm::Fork => Ok(Some(Ok(memo.to_owned()))),
    }
}

/// PFM's forward check on a memo that `json.Unmarshal` accepted
/// (`gojson::parse`): the map decode `json.Unmarshal(memo,
/// &map[string]interface{})` succeeds (an object, every number a
/// `float64`) and `d["forward"] != nil` (the last exact `forward` key wins).
fn is_forward_memo(root: &Value) -> bool {
    let Kind::Object(members) = &root.kind else {
        return false;
    };
    if !root.all(|v| !matches!(&v.kind, Kind::Number(n) if float64(n).is_none())) {
        return false;
    }
    members
        .iter()
        .rev()
        .find(|(k, _)| k == "forward")
        .is_some_and(|(_, v)| !matches!(v.kind, Kind::Null))
}

/// `json.Unmarshal(memo, &map[string]interface{})` and the forward check:
/// the decoded memo when PFM treats it as a forward memo. Valid JSON
/// nested at most 10 000 levels, an object (`null` leaves a nil map),
/// every number a `float64`.
fn forward_root(src: &[u8]) -> Option<Value> {
    gojson::parse(src).ok().filter(is_forward_memo)
}

/// `json.Unmarshal(memo, &PacketMetadata{})` of a forward memo.
fn decode_metadata(root: &Value, src: &[u8], pfm: Pfm) -> Memo {
    let Kind::Object(members) = &root.kind else {
        return Memo::NotForward;
    };
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
                if let Err(e) = decode_forward(&mut m, fields, src, &mut saved, pfm) {
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

/// Upstream `ForwardMetadata.Validate`.
pub fn validate(m: &ForwardMetadata) -> Result<(), String> {
    validate_as(m, Pfm::Upstream)
}

/// `pfm`'s `ForwardMetadata.Validate`. The fork also requires the next
/// memo to be JSON (`json.Valid`, at most 10 000 nested levels).
pub fn validate_as(m: &ForwardMetadata, pfm: Pfm) -> Result<(), String> {
    if m.receiver.is_empty() {
        return Err("failed to validate metadata. receiver cannot be empty".into());
    }
    identifier(
        &m.port,
        2,
        x_ibc::host::DEFAULT_MAX_PORT_CHARACTER_LENGTH,
        pfm,
    )?;
    identifier(
        &m.channel,
        8,
        x_ibc::host::DEFAULT_MAX_CHARACTER_LENGTH,
        pfm,
    )?;
    if pfm == Pfm::Fork
        && let Some(Ok(next)) = &m.next
        && gojson::check_valid(&gostr::to_go_bytes(next)).is_err()
    {
        return Err("failed to validate metadata: next is not JSON".into());
    }
    Ok(())
}

/// `fmt.Errorf("failed to validate metadata: %w", err)` of a 24-host
/// identifier error. `%w` formats the `errorsmod` error with `%v`, which
/// appends the source location of its innermost `Wrap` (`[file:line]`, the
/// path cut after `github.com/`). Upstream puts that string in the
/// committed error acknowledgement (celestia-app v10 and v11 build with
/// celestiaorg/ibc-go v8.7.2); the fork puts it in the
/// `packet_forward_error` event only (celestia-app's app version 12 builds
/// with v8.7.3), so the path follows the ibc-go of that build.
fn identifier(id: &str, min: usize, max: usize, pfm: Pfm) -> Result<(), String> {
    let file = match pfm {
        Pfm::Upstream => "celestiaorg/ibc-go/v8@v8.7.2/modules/core/24-host/validate.go",
        Pfm::Fork => "celestiaorg/ibc-go/v8@v8.7.3/modules/core/24-host/validate.go",
    };
    let length = gostr::go_len(id);
    let (msg, line) = if id.trim().is_empty() {
        ("identifier cannot be blank".to_owned(), 41)
    } else if id.contains('/') {
        (format!("identifier {id} cannot contain separator '/'"), 45)
    } else if length < min || length > max {
        (
            format!(
                "identifier {id} has invalid length: {length}, must be between {min}-{max} characters"
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
        "failed to validate metadata: {msg}: invalid identifier [{file}:{line}]"
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

/// The Celestia fork's memo handling (celestiaorg/ibc-apps #3), from its Go
/// tests (`types/duration_test.go`, `memo_handling_test.go`), next to
/// upstream v8.2.0 for the same memos.
#[cfg(test)]
mod fork_tests {
    use super::*;

    const RECEIVER: &str = "cosmos16plylpsgxechajltx9yeseqexzdzut9g8vla4k";

    /// `forwardMemo`: a forward memo to channel-0 with the given extra
    /// fields of the forward object (each starting with a comma).
    fn forward_memo(fields: &str) -> String {
        format!(
            r#"{{"forward":{{"receiver":"{RECEIVER}","port":"transfer","channel":"channel-0"{fields}}}}}"#
        )
    }

    fn forwarded(memo: &str, pfm: Pfm) -> ForwardMetadata {
        match on_recv(memo, pfm) {
            Recv::Forward(m) => m,
            other => panic!("{memo}: expected a forward, got {other:?}"),
        }
    }

    fn rejected(memo: &str, pfm: Pfm) -> bool {
        matches!(on_recv(memo, pfm), Recv::Error(_))
    }

    /// The Go bytes of the forwarded packet's memo (empty: no memo).
    fn next_memo(memo: &str, pfm: Pfm) -> Vec<u8> {
        match forwarded(memo, pfm).next {
            None => Vec::new(),
            Some(Ok(s)) => gostr::to_go_bytes(&s).into_owned(),
            Some(Err(e)) => panic!("{memo}: {e}"),
        }
    }

    /// `TestDurationUnmarshalNumberRange`,
    /// `TestPacketMetadataOutOfRangeTimeout`: a numeric timeout is accepted
    /// only in `[-2^63, 2^63)` as a `float64`. Upstream converts any
    /// `float64` (amd64 gives `MinInt64` out of range).
    #[test]
    fn timeout_range() {
        let cases: &[(&str, Option<i64>)] = &[
            ("0", Some(0)),
            ("-1", Some(-1)),
            ("1.9", Some(1)),
            ("9223372036854774784", Some(9_223_372_036_854_774_784)),
            // Rounds down to 2^63 - 1024 as a float64.
            ("9223372036854775295", Some(9_223_372_036_854_774_784)),
            // Rounds to 2^63.
            ("9223372036854775807", None),
            ("9223372036854775808", None),
            ("-9223372036854775808", Some(i64::MIN)),
            // Rounds to -2^63.
            ("-9223372036854775809", Some(i64::MIN)),
            ("-9223372036854777856", None),
            ("1e19", None),
            ("-1e19", None),
            ("1e308", None),
            (r#""1s""#, Some(1_000_000_000)),
            (r#""1e19""#, None),
            ("null", None),
            ("true", None),
        ];
        for (timeout, want) in cases {
            let memo = forward_memo(&format!(r#","timeout":{timeout}"#));
            match want {
                Some(ns) => assert_eq!(forwarded(&memo, Pfm::Fork).timeout, *ns, "{memo}"),
                None => assert!(rejected(&memo, Pfm::Fork), "{memo}"),
            }
        }
        // A number that does not fit a float64 fails the map decode: not a
        // forward memo at all.
        let memo = forward_memo(r#","timeout":1e400"#);
        assert_eq!(on_recv(&memo, Pfm::Fork), Recv::NotForward);
        assert_eq!(on_recv(&memo, Pfm::Upstream), Recv::NotForward);
        // Upstream forwards an out-of-range timeout as `MinInt64`.
        for timeout in ["1e19", "-1e19", "9223372036854775807"] {
            let memo = forward_memo(&format!(r#","timeout":{timeout}"#));
            assert_eq!(forwarded(&memo, Pfm::Upstream).timeout, i64::MIN, "{memo}");
        }
        // Keys match without case; a decode error stops at the first bad
        // value, wherever a later key would put the field.
        for fields in [
            r#","timeout":"1s","TIMEOUT":1e19"#,
            r#","Timeout":1e19,"timeout":5"#,
        ] {
            assert!(rejected(&forward_memo(fields), Pfm::Fork), "{fields}");
        }
        assert_eq!(
            forwarded(
                &forward_memo(r#","Timeout":1e19,"timeout":5"#),
                Pfm::Upstream
            )
            .timeout,
            5
        );
        assert!(rejected(
            &forward_memo(r#","timeout":1e19,"tImeout":"2ns""#),
            Pfm::Fork
        ));
        assert_eq!(
            forwarded(&forward_memo(r#","timeout":"1s","tImeout":2"#), Pfm::Fork).timeout,
            2
        );
    }

    /// `TestOnRecvPacket_ForwardDecodesToNil`: a later key that folds to
    /// `forward` and is null leaves no forward metadata. The fork answers
    /// with an error acknowledgement; upstream dereferences nil.
    #[test]
    fn forward_decodes_to_nil() {
        let fwd = format!(r#"{{"receiver":"{RECEIVER}","port":"transfer","channel":"channel-0"}}"#);
        for memo in [
            format!(r#"{{"forward":{fwd},"Forward":null}}"#),
            format!(r#"{{"forward":{fwd},"FORWARD":null}}"#),
            format!(r#"{{"Forward":null,"forward":{fwd},"fORWARD":null}}"#),
            format!(r#"{{"forward":{fwd},"forward":{fwd},"Forward":null}}"#),
        ] {
            assert!(rejected(&memo, Pfm::Fork), "{memo}");
            assert_eq!(
                on_recv(&memo, Pfm::Upstream),
                Recv::NilForwardPanic,
                "{memo}"
            );
        }
        // The map decode sees the exact key only: not a forward.
        let memo = format!(r#"{{"Forward":{fwd},"forward":null}}"#);
        for pfm in [Pfm::Upstream, Pfm::Fork] {
            assert_eq!(on_recv(&memo, pfm), Recv::NotForward);
        }
        // `forward` of another type is a decode error in both.
        for memo in [
            r#"{"forward":"x"}"#,
            r#"{"forward":5}"#,
            r#"{"forward":[]}"#,
            r#"{"forward":true}"#,
        ] {
            assert!(rejected(memo, Pfm::Fork), "{memo}");
            assert!(rejected(memo, Pfm::Upstream), "{memo}");
        }
    }

    /// `TestForwardedMemoIsNextAsSent`: the fork forwards `next` as the
    /// sender's bytes (a JSON string: its unquoted content, which must be
    /// JSON). Upstream re-encodes it.
    #[test]
    fn forwarded_memo_is_next_as_sent() {
        // (next value, fork memo, upstream memo; None: error ack).
        let q = |s: &str| serde_json::to_string(s).unwrap();
        let cases: Vec<(String, Option<&str>, Option<&str>)> = vec![
            (
                r#"{"forward":{"receiver":"r2"}}"#.into(),
                Some(r#"{"forward":{"receiver":"r2"}}"#),
                Some(r#"{"forward":{"receiver":"r2"}}"#),
            ),
            (
                r#"{ "b" : 1,  "a" : [ 1 , 2 ] }"#.into(),
                Some(r#"{ "b" : 1,  "a" : [ 1 , 2 ] }"#),
                Some(r#"{"b":1,"a":[1,2]}"#),
            ),
            (
                "  \t{\"a\":1}\n ".into(),
                Some(r#"{"a":1}"#),
                Some(r#"{"a":1}"#),
            ),
            (
                r#"{"a":"<&>"}"#.into(),
                Some(r#"{"a":"<&>"}"#),
                Some(r#"{"a":"\u003c\u0026\u003e"}"#),
            ),
            (
                r#"{"a":"\u003c\ud800"}"#.into(),
                Some(r#"{"a":"\u003c\ud800"}"#),
                Some("{\"a\":\"\\u003c\u{fffd}\"}"),
            ),
            (
                "{\"a\":\"\u{2028}\"}".into(),
                Some("{\"a\":\"\u{2028}\"}"),
                Some(r#"{"a":"\u2028"}"#),
            ),
            (
                r#"{"a":1,"a":2}"#.into(),
                Some(r#"{"a":1,"a":2}"#),
                Some(r#"{"a":2}"#),
            ),
            (
                r#"{"a":1e20,"b":1.0}"#.into(),
                Some(r#"{"a":1e20,"b":1.0}"#),
                Some(r#"{"a":100000000000000000000,"b":1}"#),
            ),
            ("1e20".into(), Some("1e20"), Some("1e20")),
            ("-0".into(), Some("-0"), Some("-0")),
            ("[ 1 ]".into(), Some("[ 1 ]"), Some("[1]")),
            ("true".into(), Some("true"), Some("true")),
            (
                q(r#"{ "a" : "<" }"#),
                Some(r#"{ "a" : "<" }"#),
                Some(r#"{"a":"\u003c"}"#),
            ),
            (q("null"), Some("null"), Some("null")),
            (q(r#""x""#), Some(r#""x""#), Some(r#""x""#)),
            (q(" [1] "), Some(" [1] "), Some("[1]")),
            (q("hello"), None, Some("")),
            (q(""), None, Some("")),
            (
                r#"{"a":1},"next":{"b":2}"#.into(),
                Some(r#"{"b":2}"#),
                Some(r#"{"b":2}"#),
            ),
            (
                r#""hello","NEXT":{"b":2}"#.into(),
                Some(r#"{"b":2}"#),
                Some(r#"{"b":2}"#),
            ),
            (r#"{"b":2},"Next":"hello""#.into(), None, Some("")),
            ("null".into(), Some(""), Some("")),
            (r#"{"a":1},"next":null"#.into(), Some(""), Some("")),
        ];
        for (next, fork, upstream) in cases {
            let memo = forward_memo(&format!(r#","next":{next}"#));
            match fork {
                Some(want) => assert_eq!(
                    String::from_utf8(next_memo(&memo, Pfm::Fork)).unwrap(),
                    want,
                    "fork: {memo}"
                ),
                None => assert!(rejected(&memo, Pfm::Fork), "fork: {memo}"),
            }
            // Upstream accepts every form here and fails only when it
            // forwards (`Some("")` marks a marshal error or no memo).
            let m = forwarded(&memo, Pfm::Upstream);
            let got = match m.next {
                None | Some(Err(_)) => String::new(),
                Some(Ok(s)) => s,
            };
            assert_eq!(Some(got.as_str()), upstream, "upstream: {memo}");
        }
    }

    /// Bytes that are not UTF-8 stay as sent in an object `next`; a string
    /// `next` is unquoted as Go does (invalid UTF-8 becomes U+FFFD).
    #[test]
    fn forwarded_memo_keeps_go_bytes() {
        let memo = |next: &[u8]| {
            let mut b = forward_memo(",\"next\":").into_bytes();
            let tail = b.split_off(b.len() - 2);
            b.extend_from_slice(next);
            b.extend_from_slice(&tail);
            gostr::from_go_vec(b)
        };
        assert_eq!(
            next_memo(&memo(b"{\"a\":\"\xff\"}"), Pfm::Fork),
            b"{\"a\":\"\xff\"}"
        );
        assert_eq!(
            next_memo(&memo(b"\"\\\"\xff\\\"\""), Pfm::Fork),
            "\"\u{fffd}\"".as_bytes()
        );
        assert!(rejected(&memo(b"\"\xff\""), Pfm::Fork));
    }

    /// `Validate` uses `json.Valid`, with its limit of 10 000 nested
    /// levels, on a string `next`.
    #[test]
    fn string_next_nesting_limit() {
        let nested = |d: usize| format!("{}{}", "[".repeat(d), "]".repeat(d));
        let memo = |d: usize| forward_memo(&format!(r#","next":"{}""#, nested(d)));
        assert_eq!(
            next_memo(&memo(10_000), Pfm::Fork),
            nested(10_000).into_bytes()
        );
        assert!(rejected(&memo(10_001), Pfm::Fork));
    }

    /// A forward memo of [`MAX_FORWARD_MEMO_LENGTH`] Go bytes is decoded;
    /// a longer one gets the error acknowledgement before it is decoded.
    /// Other memos are not limited.
    #[test]
    fn forward_memo_length() {
        let padded = |len: usize, c: char| {
            let base = forward_memo("");
            let fixed = base.len() + r#","pad":"""#.len();
            let n = (len - fixed) / c.len_utf8();
            let memo = format!(
                r#"{},"pad":"{}{}"}}"#,
                &base[..base.len() - 1],
                c.to_string().repeat(n),
                "x".repeat((len - fixed) % c.len_utf8())
            );
            assert_eq!(gostr::go_len(&memo), len);
            memo
        };
        for c in ['x', '\u{e9}'] {
            let at = padded(MAX_FORWARD_MEMO_LENGTH, c);
            assert!(matches!(on_recv(&at, Pfm::Fork), Recv::Forward(_)));
            let over = padded(MAX_FORWARD_MEMO_LENGTH + 2, c);
            assert!(rejected(&over, Pfm::Fork));
            assert!(matches!(on_recv(&over, Pfm::Upstream), Recv::Forward(_)));
        }
        let over = padded(MAX_FORWARD_MEMO_LENGTH + 1, 'x');
        assert!(rejected(&over, Pfm::Fork));
        // Any error ack, even for a memo that would fail to decode anyway.
        let bad = format!(r#"{{"forward":5,"pad":"{}"}}"#, "x".repeat(40 << 10));
        assert!(rejected(&bad, Pfm::Fork));

        let big = "x".repeat(40 << 10);
        for memo in [
            format!(r#"{{"pad":"{big}"}}"#),
            format!(r#"{{"forward":null,"pad":"{big}"}}"#),
            format!(r#"{{"Forward":{{}},"pad":"{big}"}}"#),
            big.clone(),
        ] {
            for pfm in [Pfm::Upstream, Pfm::Fork] {
                assert_eq!(on_recv(&memo, pfm), Recv::NotForward);
            }
        }
    }

    /// `TestRetryResendsMemoAsIs`: the fork sends the memo of a timed-out
    /// packet again as is, without decoding it. Upstream decodes and
    /// re-encodes it, and fails on a memo that does not decode.
    #[test]
    fn retry_resends_memo_as_is() {
        let deep = |d: usize| format!("{}1{}", r#"{"a":"#.repeat(d), "}".repeat(d));
        let lists = |d: usize| format!("{}{}", "[".repeat(d), "]".repeat(d));
        let big = forward_memo(&format!(r#","pad":"{}""#, "x".repeat(40 << 10)));
        let memos = [
            forward_memo(""),
            " { \"a\" : 1 } ".into(),
            r#"{"a":"<&>"}"#.into(),
            r#""x""#.into(),
            "null".into(),
            "5".into(),
            "hello".into(),
            r#"{"a":1,"a":2}"#.into(),
            deep(3000),
            lists(10_001),
            r#"{"a":"\x"}"#.into(),
            big,
            gostr::from_go_vec(b"{\"a\":\"\xff\"}".to_vec()),
        ];
        for memo in &memos {
            assert_eq!(
                retry_next(memo, Pfm::Fork),
                Ok(Some(Ok(memo.clone()))),
                "{memo:.80}"
            );
        }
        assert_eq!(retry_next("", Pfm::Fork), Ok(None));
        assert_eq!(retry_next("", Pfm::Upstream), Ok(None));
        // Upstream.
        assert_eq!(
            retry_next(r#"{"a":"<&>"}"#, Pfm::Upstream),
            Ok(Some(Ok(r#"{"a":"\u003c\u0026\u003e"}"#.into())))
        );
        assert!(retry_next("hello", Pfm::Upstream).is_err());
        assert!(retry_next(&lists(10_001), Pfm::Upstream).is_err());
        assert!(matches!(
            retry_next("null", Pfm::Upstream),
            Ok(Some(Err(_)))
        ));
    }
}
