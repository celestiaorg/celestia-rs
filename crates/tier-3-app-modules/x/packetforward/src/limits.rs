//! celestia-app's limits on packet-forward memos (`app/pfm_memo_limits.go`
//! and `app/pfm_timeout.go`, PROTOCO-3780 / PROTOCO-3781), applied from app
//! version 11 by its `pfmIBCMiddleware` wrapper around PFM v8.2.0.

use std::collections::HashSet;

use gojson::{Kind, Value};

use crate::memo;

/// `maxPFMForwardMemoLength`: `transfertypes.MaximumMemoLength`.
pub const MAX_FORWARD_MEMO_LENGTH: usize = x_transfer::relay::MAXIMUM_MEMO_LENGTH;

/// `maxPFMForwardMemoDepth`: the deepest JSON nesting (objects and arrays
/// together) accepted in a forward memo, a string `forward.next`, or a
/// retried memo.
pub const MAX_FORWARD_MEMO_DEPTH: usize = 16;

/// `maxPFMRetryMemoLength`: the longest forwarded memo that may be retried.
pub const MAX_RETRY_MEMO_LENGTH: usize = 6 * MAX_FORWARD_MEMO_LENGTH;

/// `pfmTimeoutLimit`: 2^63. Go converts a `float64` timeout to
/// `time.Duration` with a result the Go specification defines only in
/// `[-2^63, 2^63)`.
const TIMEOUT_LIMIT: f64 = 9_223_372_036_854_775_808.0;

/// `rejectPFMForwardMemo` on the memo of transfer packet data that
/// `ModuleCdc.UnmarshalJSON` decoded: whether the wrapper answers with the
/// fixed error acknowledgement before PFM runs. It rejects a forward memo
/// (PFM's map decode succeeds and `forward` is not null) that is outside
/// the length, depth and repeated-key limits or has a `forward.next` of an
/// unaccepted form, that decodes to no forward metadata, or whose numeric
/// timeout is outside the `int64` range. Other memos are left to PFM.
#[must_use]
pub fn reject_forward_memo(memo: &str) -> bool {
    let src = gostr::to_go_bytes(memo);
    // `json.Unmarshal(memo, &map[string]any)`, then `d["forward"] == nil`.
    let Ok(root) = gojson::parse(&src) else {
        return false;
    };
    if !memo::is_forward_memo(&root) {
        return false;
    }
    // `pfmForwardMemoWithinLimits`, before any other decoding.
    if src.len() > MAX_FORWARD_MEMO_LENGTH || !within_limits(&root, true) {
        return true;
    }
    // `json.Unmarshal` into `struct{ Forward *struct{ Timeout pfmTimeout } }`:
    // fields match exactly or by Go's fold; a later `forward` decodes into
    // the same struct, `null` resets it; any other value is a type error,
    // which Go reports after decoding the rest (not rejected: PFM fails on
    // its own).
    let Kind::Object(members) = &root.kind else {
        return false;
    };
    let mut forward: Option<bool> = None;
    let mut type_error = false;
    for (k, v) in members {
        if !memo::field_is(k, "forward") {
            continue;
        }
        match &v.kind {
            Kind::Null => forward = None,
            Kind::Object(fields) => {
                let mut out_of_range = forward.unwrap_or(false);
                for (fk, fv) in fields {
                    if memo::field_is(fk, "timeout") {
                        out_of_range = timeout_out_of_range(fv);
                    }
                }
                forward = Some(out_of_range);
            }
            _ => {
                forward.get_or_insert(false);
                type_error = true;
            }
        }
    }
    if type_error {
        return false;
    }
    // A nil `Forward` (a later key that folds to "forward", set to null).
    forward.unwrap_or(true)
}

/// `pfmTimeout.UnmarshalJSON`: a number (that fits a `float64`) outside
/// `[-2^63, 2^63)`. Every other value, `null` included, is in range.
fn timeout_out_of_range(v: &Value) -> bool {
    match &v.kind {
        Kind::Number(lit) => {
            memo::float64(lit).is_some_and(|f| !(-TIMEOUT_LIMIT..TIMEOUT_LIMIT).contains(&f))
        }
        _ => false,
    }
}

/// `pfmRetryMemoWithinLimits`: whether a forwarded packet with this memo
/// may be retried after a timeout. The empty memo, or a valid JSON object
/// of at most [`MAX_RETRY_MEMO_LENGTH`] bytes within the depth and exact
/// repeated-key limits.
#[must_use]
pub fn retry_memo_within_limits(memo: &str) -> bool {
    if memo.is_empty() {
        return true;
    }
    let bz = gostr::to_go_bytes(memo);
    bz.len() <= MAX_RETRY_MEMO_LENGTH
        && gojson::check_valid(&bz).is_ok()
        && next_memo_within_limits(&bz)
}

/// `pfmStringNextWithinLimits` on the Go bytes of a string `forward.next`:
/// accepted when it is not valid JSON (`json.Valid`; PFM then fails to
/// encode it), else when it passes [`next_memo_within_limits`].
fn string_next_within_limits(s: &[u8]) -> bool {
    gojson::check_valid(s).is_err() || next_memo_within_limits(s)
}

/// `pfmNextMemoWithinLimits`: `bz`, valid JSON, is an object within the
/// depth and exact repeated-key limits. `false` for invalid JSON.
fn next_memo_within_limits(bz: &[u8]) -> bool {
    let starts_object = bz
        .iter()
        .find(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        == Some(&b'{');
    starts_object && gojson::parse(bz).is_ok_and(|v| within_limits(&v, false))
}

/// `pfmMemoWithinLimits` on a decoded value: at most
/// [`MAX_FORWARD_MEMO_DEPTH`] nested objects and arrays, and no repeated
/// object key. For a forward memo, the keys of the memo and of its forward
/// object compare folded (`encoding/json` matches them to struct fields),
/// and `forward.next` must be null, an object, or an accepted string.
/// Recursion stops at the depth limit.
fn within_limits(root: &Value, forward_memo: bool) -> bool {
    Walk { forward_memo }.value(root, 0, forward_memo, false)
}

struct Walk {
    forward_memo: bool,
}

impl Walk {
    /// `v` opens at nesting level `depth + 1` when it is a container.
    /// `fold` and `forward` describe `v` when it is an object.
    fn value(&self, v: &Value, depth: usize, fold: bool, forward: bool) -> bool {
        match &v.kind {
            Kind::Array(items) => {
                depth < MAX_FORWARD_MEMO_DEPTH
                    && items.iter().all(|i| self.value(i, depth + 1, false, false))
            }
            Kind::Object(members) => {
                if depth >= MAX_FORWARD_MEMO_DEPTH {
                    return false;
                }
                let mut keys = HashSet::with_capacity(members.len());
                for (k, child) in members {
                    let key = if fold { fold_json_key(k) } else { k.clone() };
                    if forward && key == "NEXT" && !forward_next_within_limits(child) {
                        return false;
                    }
                    let child_forward = self.forward_memo && depth == 0 && key == "FORWARD";
                    if !keys.insert(key) {
                        return false;
                    }
                    if !self.value(child, depth + 1, child_forward, child_forward) {
                        return false;
                    }
                }
                true
            }
            _ => true,
        }
    }
}

/// `pfmForwardNextWithinLimits`: `forward.next` is null, an object, or a
/// string that passes [`string_next_within_limits`].
fn forward_next_within_limits(v: &Value) -> bool {
    match &v.kind {
        Kind::Null | Kind::Object(_) => true,
        // A decoded JSON string: its UTF-8 is Go's bytes.
        Kind::String(s) => string_next_within_limits(s.as_bytes()),
        _ => false,
    }
}

/// `foldJSONKey`, Go's `encoding/json` `foldName`: ASCII letters upper
/// case, any other rune the smallest of its `unicode.SimpleFold` orbit. Two
/// keys match a struct field alike exactly when their folds are equal.
fn fold_json_key(key: &str) -> String {
    key.chars()
        .map(|c| {
            if c.is_ascii() {
                return c.to_ascii_uppercase();
            }
            let mut r = u32::from(c);
            loop {
                let r2 = gostr::simple_fold(r);
                if r2 <= r {
                    r = r2;
                    break;
                }
                r = r2;
            }
            char::from_u32(r).unwrap_or(c)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use x_ibc::gojson::escape_string;

    const MAX: usize = MAX_FORWARD_MEMO_LENGTH;
    const DEPTH: usize = MAX_FORWARD_MEMO_DEPTH;

    /// `nestedJSON`: a JSON object that nests `depth` levels.
    fn nested_json(depth: usize) -> String {
        format!(
            "{}{{}}{}",
            "{\"a\":".repeat(depth - 1),
            "}".repeat(depth - 1)
        )
    }

    /// `pfmForwardMemo`: a forward memo nested `depth` (at least 2) levels,
    /// at least `size` bytes long.
    fn forward_memo(depth: usize, size: usize) -> String {
        let next = if depth > 2 {
            nested_json(depth - 2)
        } else {
            String::new()
        };
        forward_memo_with_next(&next, size)
    }

    /// `pfmForwardMemoWithNext`: `next` is the JSON of `forward.next` (empty
    /// to omit it).
    fn forward_memo_with_next(next: &str, size: usize) -> String {
        let head = r#"{"forward":{"receiver":""#;
        let mut tail = r#"","port":"transfer","channel":"channel-0""#.to_owned();
        if !next.is_empty() {
            tail.push_str(",\"next\":");
            tail.push_str(next);
        }
        tail.push_str("}}");
        let pad = size.saturating_sub(head.len() + tail.len()).max(1);
        format!("{head}{}{tail}", "r".repeat(pad))
    }

    /// `jsonString`: `s` as a JSON string (`json.Marshal`).
    fn json_string(s: &str) -> String {
        escape_string(s)
    }

    fn check(cases: &[(&str, String, bool)]) {
        for (name, memo, want) in cases {
            assert_eq!(reject_forward_memo(memo), *want, "{name}");
        }
    }

    /// `TestRejectPFMForwardMemo`.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn reject_forward_memo_cases() {
        let cases: Vec<(&str, &str, bool)> = vec![
            ("no memo", r"", false),
            (
                "memo without forward",
                r#"{"wasm":{"timeout":1e19}}"#,
                false,
            ),
            ("invalid memo json", r#"{"forward":{"timeout":1e19}"#, false),
            ("forward null", r#"{"forward":null}"#, false),
            ("forward not an object", r#"{"forward":5}"#, false),
            ("no timeout", r#"{"forward":{"receiver":"r"}}"#, false),
            ("string timeout", r#"{"forward":{"timeout":"10m"}}"#, false),
            (
                "invalid string timeout",
                r#"{"forward":{"timeout":"1e19"}}"#,
                false,
            ),
            ("bool timeout", r#"{"forward":{"timeout":true}}"#, false),
            ("null timeout", r#"{"forward":{"timeout":null}}"#, false),
            ("zero", r#"{"forward":{"timeout":0}}"#, false),
            ("negative zero", r#"{"forward":{"timeout":-0}}"#, false),
            ("fraction", r#"{"forward":{"timeout":1.5}}"#, false),
            ("negative in range", r#"{"forward":{"timeout":-5}}"#, false),
            (
                "min int64",
                r#"{"forward":{"timeout":-9223372036854775808}}"#,
                false,
            ),
            (
                "largest float64 below 2^63",
                r#"{"forward":{"timeout":9223372036854774784}}"#,
                false,
            ),
            (
                "max int64 literal rounds to 2^63",
                r#"{"forward":{"timeout":9223372036854775807}}"#,
                true,
            ),
            (
                "2^63",
                r#"{"forward":{"timeout":9223372036854775808}}"#,
                true,
            ),
            ("9.3e18", r#"{"forward":{"timeout":9.3e18}}"#, true),
            ("1e19", r#"{"forward":{"timeout":1e19}}"#, true),
            ("-1e19", r#"{"forward":{"timeout":-1e19}}"#, true),
            (
                "next float64 below -2^63",
                r#"{"forward":{"timeout":-9223372036854777856}}"#,
                true,
            ),
            (
                "does not fit in float64",
                r#"{"forward":{"timeout":1e400}}"#,
                false,
            ),
            (
                "number elsewhere does not fit in float64",
                r#"{"forward":{"timeout":1e19},"x":1e400}"#,
                false,
            ),
            (
                "underflow is zero",
                r#"{"forward":{"timeout":1e-400}}"#,
                false,
            ),
            ("field name case", r#"{"forward":{"TimeOut":1e19}}"#, true),
            (
                "forward key case",
                r#"{"forward":{},"Forward":{"timeout":1e19}}"#,
                true,
            ),
            (
                "duplicate timeout, last in range",
                r#"{"forward":{"timeout":1e19,"timeout":5}}"#,
                true,
            ),
            (
                "duplicate timeout, last out of range",
                r#"{"forward":{"timeout":5,"timeout":1e19}}"#,
                true,
            ),
            (
                "timeout in next only",
                r#"{"forward":{"next":{"forward":{"timeout":1e19}}}}"#,
                false,
            ),
            (
                "timeout in next string only",
                r#"{"forward":{"next":"{\"forward\":{\"timeout\":1e19}}"}}"#,
                false,
            ),
            (
                "upper case exponent",
                r#"{"forward":{"timeout":1E19}}"#,
                true,
            ),
            (
                "positive exponent sign",
                r#"{"forward":{"timeout":1e+19}}"#,
                true,
            ),
            (
                "2^63 as integer digits",
                r#"{"forward":{"timeout":10000000000000000000}}"#,
                true,
            ),
            ("1e308", r#"{"forward":{"timeout":1e308}}"#, true),
            (
                "negative zero fraction",
                r#"{"forward":{"timeout":-0.0}}"#,
                false,
            ),
            (
                "escaped field name",
                r#"{"forward":{"\u0074imeout":1e19}}"#,
                true,
            ),
            (
                "Kelvin sign field name",
                "{\"forward\":{\"\u{212a}\":1,\"timeout\":1e19}}",
                true,
            ),
            ("array timeout", r#"{"forward":{"timeout":[1e19]}}"#, false),
            (
                "object timeout",
                r#"{"forward":{"timeout":{"v":1e19}}}"#,
                false,
            ),
            (
                "stringified forward",
                r#"{"forward":"{\"timeout\":1e19}"}"#,
                false,
            ),
            (
                "duplicate forward merges",
                r#"{"forward":{"timeout":1e19},"forward":{"receiver":"r"}}"#,
                true,
            ),
            (
                "duplicate forward, last timeout in range",
                r#"{"forward":{"timeout":1e19},"forward":{"timeout":5}}"#,
                true,
            ),
            (
                "upper case forward first, lower case second",
                r#"{"Forward":{"timeout":1e19},"forward":{"receiver":"r"}}"#,
                true,
            ),
            (
                "forward reset by null",
                r#"{"forward":{"timeout":1e19},"Forward":null,"forward":{"receiver":"r"}}"#,
                true,
            ),
            (
                "lower case forward key null",
                r#"{"Forward":{"timeout":1e19},"forward":null}"#,
                false,
            ),
            (
                "duplicate timeout, last does not fit in float64",
                r#"{"forward":{"timeout":5,"timeout":1e400}}"#,
                false,
            ),
            (
                "duplicate timeout, last null",
                r#"{"forward":{"timeout":1e19,"timeout":null}}"#,
                true,
            ),
            (
                "duplicate timeout, last string",
                r#"{"forward":{"timeout":1e19,"timeout":"10m"}}"#,
                true,
            ),
            (
                "out of range with another invalid field",
                r#"{"forward":{"timeout":1e19,"retries":300}}"#,
                true,
            ),
            (
                "Forward null after forward",
                r#"{"forward":{"receiver":"r"},"Forward":null}"#,
                true,
            ),
            (
                "FORWARD null after forward",
                r#"{"forward":{"receiver":"r"},"FORWARD":null}"#,
                true,
            ),
            (
                "escaped Forward null after forward",
                r#"{"forward":{"receiver":"r"},"Forw\u0061rd":null}"#,
                true,
            ),
            (
                "Forward null after empty forward",
                r#"{"forward":{},"Forward":null}"#,
                true,
            ),
            (
                "Forward null before forward",
                r#"{"Forward":null,"forward":{"receiver":"r"}}"#,
                true,
            ),
            (
                "forward object then forward null",
                r#"{"forward":{"receiver":"r"},"forward":null}"#,
                false,
            ),
            (
                "escaped forward null after forward",
                r#"{"forward":{"receiver":"r"},"forw\u0061rd":null}"#,
                false,
            ),
            (
                "Forward object then forward null",
                r#"{"Forward":{"receiver":"r"},"forward":null}"#,
                false,
            ),
            (
                "Forward null then invalid Forward",
                r#"{"forward":{"receiver":"r"},"Forward":null,"Forward":5}"#,
                true,
            ),
            (
                "Forward null then Forward object",
                r#"{"forward":{"receiver":"r"},"Forward":null,"Forward":{"receiver":"r"}}"#,
                true,
            ),
            ("top-level null", "null", false),
            (
                "top-level array",
                r#"[{"forward":{"timeout":1e19}}]"#,
                false,
            ),
            ("trailing data", r#"{"forward":{"timeout":1e19}} 1"#, false),
        ];
        let cases: Vec<(&str, String, bool)> = cases
            .into_iter()
            .map(|(n, m, w)| (n, m.to_owned(), w))
            .collect();
        check(&cases);
    }

    /// `TestRejectPFMForwardMemoLimits`.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn reject_forward_memo_limits() {
        assert_eq!(forward_memo(2, MAX).len(), MAX);
        let fwd_array = |key: &str, n: usize| {
            format!(
                r#"{{"forward":{{"receiver":"r","{key}":{}{}}}}}"#,
                "[".repeat(n),
                "]".repeat(n)
            )
        };
        let next_str = |s: &str| forward_memo_with_next(&json_string(s), 0);
        let cases: Vec<(&str, String, bool)> = vec![
            ("depth 2", forward_memo(2, 0), false),
            ("depth at limit", forward_memo(DEPTH, 0), false),
            ("depth over limit", forward_memo(DEPTH + 1, 0), true),
            ("depth 4000", forward_memo(4000, 0), true),
            ("array depth at limit", fwd_array("x", DEPTH - 2), false),
            ("array depth over limit", fwd_array("x", DEPTH - 1), true),
            ("next array depth over limit", fwd_array("next", DEPTH - 1), true),
            (
                "brackets in strings do not count",
                format!(r#"{{"forward":{{"receiver":"{}\"{{"}}}}"#, "{[".repeat(100)),
                false,
            ),
            ("length at limit", forward_memo(2, MAX), false),
            ("length over limit", forward_memo(2, MAX + 1), true),
            ("length and depth at limit", forward_memo(DEPTH, MAX), false),
            (
                "length over limit with invalid forward",
                format!(
                    r#"{{"forward":{{"receiver":"{}","retries":300}}}}"#,
                    "r".repeat(MAX)
                ),
                true,
            ),
            ("string next at depth limit", next_str(&nested_json(DEPTH)), false),
            ("string next over depth limit", next_str(&nested_json(DEPTH + 1)), true),
            ("string next 3000 levels", next_str(&nested_json(3000)), true),
            (
                "string next array over depth limit",
                next_str(&format!("{}{}", "[".repeat(DEPTH + 1), "]".repeat(DEPTH + 1))),
                true,
            ),
            (
                "string next over the Go scanner's depth is not JSON",
                next_str(&format!("[{}{}]", "[".repeat(10_000), "]".repeat(10_000))),
                false,
            ),
            ("string next with repeated key", next_str(r#"{"a":1,"a":2}"#), true),
            (
                "string next with repeated nested key",
                next_str(r#"{"a":{"b":1,"b":2}}"#),
                true,
            ),
            (
                "string next with keys that differ in case",
                next_str(r#"{"a":1,"A":2}"#),
                false,
            ),
            (
                "string next holding a JSON string",
                next_str(&json_string(r#"{"a":1}"#)),
                true,
            ),
            ("string next, not JSON", next_str(r#"{"a":"#), false),
            ("string next, number", next_str("5"), true),
            ("string next, null", next_str("null"), true),
            ("string next, true", next_str("true"), true),
            ("string next, array", next_str(r#"[{"a":1}]"#), true),
            ("string next, object", next_str(" {\"a\":[1]} "), false),
            ("string next, object after tab", next_str("\t\r\n{}"), false),
            ("string next, empty", next_str(""), false),
            ("next null", forward_memo_with_next("null", 0), false),
            ("next number", forward_memo_with_next("5", 0), true),
            ("next true", forward_memo_with_next("true", 0), true),
            ("next array", forward_memo_with_next(r#"[{"a":1}]"#, 0), true),
            (
                "Next key case, next array",
                r#"{"forward":{"receiver":"r","Next":[]}}"#.to_owned(),
                true,
            ),
            (
                "next key in a nested object is not forward.next",
                r#"{"forward":{"receiver":"r","next":{"next":[1]}},"a":{"next":5}}"#.to_owned(),
                false,
            ),
            (
                "Next key case, string next over depth limit",
                format!(
                    r#"{{"forward":{{"receiver":"r","NEXT":{}}}}}"#,
                    json_string(&nested_json(DEPTH + 1))
                ),
                true,
            ),
            (
                "deep memo without forward",
                format!(
                    r#"{{"a":{}1{}"#,
                    "{\"a\":".repeat(4000),
                    "}".repeat(4001)
                ),
                false,
            ),
            (
                "long memo without forward",
                format!(r#"{{"a":"{}"}}"#, "r".repeat(2 * MAX)),
                false,
            ),
            (
                "repeated keys without forward",
                r#"{"a":1,"a":2,"forward":null}"#.to_owned(),
                false,
            ),
            (
                "deep forward null",
                format!(
                    r#"{{"forward":null,"a":{}{}}}"#,
                    "[".repeat(100),
                    "]".repeat(100)
                ),
                false,
            ),
            (
                "1-hop",
                r#"{"forward":{"receiver":"r","port":"transfer","channel":"channel-0","timeout":"10m","retries":2}}"#.to_owned(),
                false,
            ),
            (
                "2-hop",
                r#"{"forward":{"receiver":"r","port":"transfer","channel":"channel-0","next":{"forward":{"receiver":"r2","port":"transfer","channel":"channel-1"}}}}"#.to_owned(),
                false,
            ),
            (
                "2-hop next string",
                r#"{"forward":{"receiver":"r","port":"transfer","channel":"channel-0","next":"{\"forward\":{\"receiver\":\"r2\",\"port\":\"transfer\",\"channel\":\"channel-1\"}}"}}"#.to_owned(),
                false,
            ),
        ];
        check(&cases);
    }

    /// `TestPFMForwardMemoDepthLimitIs16`.
    #[test]
    fn depth_limit_is_16() {
        for (depth, want) in [(16, false), (17, true)] {
            assert_eq!(
                reject_forward_memo(&forward_memo(depth, 0)),
                want,
                "{depth}"
            );
            assert_eq!(
                reject_forward_memo(&forward_memo_with_next(
                    &json_string(&nested_json(depth)),
                    0
                )),
                want,
                "string next {depth}"
            );
            assert_eq!(
                retry_memo_within_limits(&nested_json(depth)),
                !want,
                "retry {depth}"
            );
        }
    }

    /// `TestRejectPFMForwardMemoRepeatedKeys`, with Go's Unicode simple
    /// folding (`ſ` and the Kelvin sign) for the keys `encoding/json`
    /// matches to struct fields.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn reject_forward_memo_repeated_keys() {
        const FWD: &str = r#""receiver":"r","port":"transfer","channel":"channel-0""#;
        let f = |s: &str| s.replace("FWD", FWD);
        let cases: Vec<(&str, String, bool)> = vec![
            (
                "no repeated keys",
                f(r#"{"forward":{FWD,"next":{"a":1,"b":{"a":2}}},"other":{"a":1}}"#),
                false,
            ),
            (
                "case differs inside next",
                f(r#"{"forward":{FWD,"next":{"a":1,"A":2}}}"#),
                false,
            ),
            (
                "fold differs inside next",
                f("{\"forward\":{FWD,\"next\":{\"k\":1,\"\u{212a}\":2,\"s\":3,\"\u{17f}\":4}}}"),
                false,
            ),
            (
                "case differs inside other value",
                f(r#"{"forward":{FWD},"other":{"a":1,"A":2}}"#),
                false,
            ),
            (
                "repeated next",
                f(r#"{"forward":{FWD,"next":{"a":1},"next":{}}}"#),
                true,
            ),
            (
                "repeated next, other case",
                f(r#"{"forward":{FWD,"next":{"a":1},"NEXT":{}}}"#),
                true,
            ),
            (
                "repeated next, escaped",
                f(r#"{"forward":{FWD,"next":{"a":1},"n\u0065xt":{}}}"#),
                true,
            ),
            (
                "repeated receiver",
                f(r#"{"forward":{FWD,"receiver":"r"}}"#),
                true,
            ),
            (
                "repeated retries, Unicode fold",
                f("{\"forward\":{FWD,\"retries\":1,\"retrie\u{17f}\":2}}"),
                true,
            ),
            (
                "repeated key, Kelvin sign fold",
                f("{\"forward\":{FWD},\"k\":1,\"\u{212a}\":2}"),
                true,
            ),
            (
                "repeated key, non-ASCII fold",
                f("{\"forward\":{FWD},\"\u{e9}\":1,\"\u{c9}\":2}"),
                true,
            ),
            (
                "repeated key, escaped Kelvin sign",
                f(r#"{"forward":{FWD},"K":1,"\u212a":2}"#),
                true,
            ),
            (
                "repeated forward, other case",
                f(r#"{"forward":{FWD},"Forward":{"next":{}}}"#),
                true,
            ),
            (
                "repeated other top-level key",
                f(r#"{"forward":{FWD},"a":1,"a":2}"#),
                true,
            ),
            (
                "repeated top-level key, other case",
                f(r#"{"forward":{FWD},"a":1,"A":2}"#),
                true,
            ),
            (
                "repeated key inside next",
                f(r#"{"forward":{FWD,"next":{"a":1,"b":2,"a":3}}}"#),
                true,
            ),
            (
                "repeated key deep inside next",
                f(r#"{"forward":{FWD,"next":{"x":[{"a":1,"a":2}]}}}"#),
                true,
            ),
            (
                "repeated key inside other value",
                f(r#"{"forward":{FWD},"other":{"a":1,"a":2}}"#),
                true,
            ),
            (
                "same key in sibling objects",
                f(r#"{"forward":{FWD,"next":{"x":{"a":1},"y":{"a":1}}}}"#),
                false,
            ),
            (
                "lone surrogates decode to the same key",
                f(r#"{"forward":{FWD,"next":{"\ud800":1,"\udbff":2}}}"#),
                true,
            ),
            (
                "lone surrogate and U+FFFD are the same key",
                f(r#"{"forward":{FWD,"next":{"\ud800":1,"\ufffd":2}}}"#),
                true,
            ),
            (
                "surrogate pair is not U+FFFD",
                f(r#"{"forward":{FWD,"next":{"\ud83d\ude00":1,"\ufffd":2}}}"#),
                false,
            ),
        ];
        check(&cases);
    }

    /// Invalid UTF-8 in the memo: Go decodes each invalid byte of a string
    /// to U+FFFD, so two keys that differ only in invalid bytes repeat.
    #[test]
    fn invalid_utf8_keys() {
        let memo = |key_a: &[u8], key_b: &[u8]| {
            let mut b = br#"{"forward":{"receiver":"r","next":{""#.to_vec();
            b.extend_from_slice(key_a);
            b.extend_from_slice(br#"":1,""#);
            b.extend_from_slice(key_b);
            b.extend_from_slice(br#"":2}}}"#);
            gostr::from_go_vec(b)
        };
        assert!(reject_forward_memo(&memo(b"\xff", b"\xfe")));
        // Go reads a truncated sequence one byte at a time: two U+FFFD.
        assert!(reject_forward_memo(&memo(
            b"\xe2\x82",
            "\u{fffd}\u{fffd}".as_bytes()
        )));
        assert!(!reject_forward_memo(&memo(
            b"\xe2\x82",
            "\u{fffd}".as_bytes()
        )));
        // The length counts Go bytes.
        let mut long = br#"{"forward":{"receiver":""#.to_vec();
        let tail = br#""}}"#;
        long.resize(MAX - tail.len(), 0xff);
        long.extend_from_slice(tail);
        assert!(!reject_forward_memo(&gostr::from_go_vec(long.clone())));
        long.insert(30, 0xff);
        assert!(reject_forward_memo(&gostr::from_go_vec(long)));
    }

    /// The memos of `TestPFMForwardMemoWorkIsBounded`: inputs whose
    /// decoding and encoding work grows faster than their length are
    /// rejected; the worst accepted ones are not.
    #[test]
    fn amplified_memos_are_rejected() {
        // ~2000 distinct next keys, then many repeated "next":{} keys.
        let keys: Vec<String> = (0..2000u32)
            .map(|i| {
                let a = char::from_u32(u32::from(b'A') + i / 64).unwrap();
                let b = char::from_u32(u32::from(b'A') + i % 64).unwrap();
                format!("{}:0", json_string(&format!("{a}{b}")))
            })
            .collect();
        let repeated_next = format!(
            r#"{{"forward":{{"next":{{{}}}{}}}}}"#,
            keys.join(","),
            r#","next":{}"#.repeat(1860)
        );
        let dup_keys: Vec<String> = (0..2)
            .flat_map(|_| (0..1500).map(|i| format!("\"{i}\":0")))
            .collect();
        let amplified = [
            ("nested next", forward_memo(4000, 0)),
            (
                "nested string next",
                forward_memo_with_next(&json_string(&nested_json(3000)), 0),
            ),
            ("repeated next keys", repeated_next),
            (
                "repeated keys in next",
                forward_memo_with_next(&format!("{{{}}}", dup_keys.join(",")), 0),
            ),
        ];
        for (name, memo) in &amplified {
            assert!(memo.len() <= MAX, "{name}: {}", memo.len());
            assert!(reject_forward_memo(memo), "{name}");
        }
        assert!(reject_forward_memo(&forward_memo(2, 1 << 20)));

        // The worst accepted memos, at the length and depth limits.
        let deep_html = |n: usize| {
            format!(
                "{}\"{}\"{}",
                "{\"a\":".repeat(DEPTH - 3),
                "<".repeat(n),
                "}".repeat(DEPTH - 3)
            )
        };
        let html_len = MAX - forward_memo_with_next(&deep_html(0), 0).len();
        let deep_numbers = |n: usize| {
            format!(
                "{}[1{}]{}",
                "{\"a\":".repeat(DEPTH - 3),
                ",1e20".repeat(n),
                "}".repeat(DEPTH - 3)
            )
        };
        let numbers = (MAX - forward_memo_with_next(&deep_numbers(0), 0).len()) / ",1e20".len();
        for (name, memo) in [
            ("object next", forward_memo(DEPTH, MAX)),
            (
                "string next",
                forward_memo_with_next(&json_string(&nested_json(DEPTH)), MAX),
            ),
            (
                "html in next",
                forward_memo_with_next(&deep_html(html_len), 0),
            ),
            (
                "numbers in next",
                forward_memo_with_next(&deep_numbers(numbers), MAX),
            ),
        ] {
            assert_eq!(memo.len(), MAX, "{name}");
            assert!(!reject_forward_memo(&memo), "{name}");
        }
    }

    /// `TestPFMMemoWithinLimitsMalformed`: no panic on malformed input.
    #[test]
    fn malformed_memos_do_not_panic() {
        for bz in [
            &b""[..],
            b" ",
            b"{",
            b"}",
            b"]",
            b"[}",
            b"{]",
            br#"{"a""#,
            br#"{"a":}"#,
            br#"{"a":1,}"#,
            b"\"",
            br#"{"forward":{"next":""#,
            br#"{"forward":{"NEXT":"\""#,
            b"\xff",
            b"{\"\xff\":1,\"\xfe\":2}",
            b"1 2",
            b"{}}",
        ] {
            let s = gostr::from_go_bytes(bz);
            assert!(!reject_forward_memo(&s), "{s:?}");
            assert!(!retry_memo_within_limits(&s) || s.is_empty(), "{s:?}");
            let _ = string_next_within_limits(bz);
            let _ = next_memo_within_limits(bz);
        }
    }

    /// `TestPFMRetryMemoWithinLimits`.
    #[test]
    fn retry_memo_limits() {
        let max_len = MAX_RETRY_MEMO_LENGTH;
        let pad = |n: usize| {
            let head = r#"{"forward":{"receiver":""#;
            format!("{head}{}\"}}}}", "r".repeat(n - head.len() - 3))
        };
        let cases: Vec<(&str, String, bool)> = vec![
            ("no memo", String::new(), true),
            (
                "2-hop",
                r#"{"forward":{"receiver":"r2","port":"transfer","channel":"channel-1"}}"#
                    .to_owned(),
                true,
            ),
            ("depth at limit", nested_json(DEPTH), true),
            ("depth over limit", nested_json(DEPTH + 1), false),
            ("depth 2000", nested_json(2000), false),
            ("depth 10001", nested_json(10_001), false),
            ("repeated key", r#"{"a":1,"a":2}"#.to_owned(), false),
            ("keys differ in case", r#"{"a":1,"A":2}"#.to_owned(), true),
            (
                "forward keys differ in case",
                r#"{"forward":{"receiver":"r"},"Forward":{}}"#.to_owned(),
                true,
            ),
            ("length at limit", pad(max_len), true),
            ("length over limit", pad(max_len + 1), false),
            ("not JSON", "hello".to_owned(), false),
            ("whitespace", " ".to_owned(), false),
            ("array", "[1]".to_owned(), false),
            ("number", "5".to_owned(), false),
            ("null", "null".to_owned(), false),
            ("string", r#""{}""#.to_owned(), false),
            ("object after whitespace", " \n{} ".to_owned(), true),
            (
                "number out of float64 range",
                r#"{"a":1e400}"#.to_owned(),
                true,
            ),
        ];
        for (name, memo, want) in cases {
            assert_eq!(retry_memo_within_limits(&memo), want, "{name}");
        }
    }

    /// Go's `foldName`: the smallest rune of each simple-fold orbit, ASCII
    /// letters upper case.
    #[test]
    fn fold_json_keys() {
        assert_eq!(fold_json_key("forward"), "FORWARD");
        assert_eq!(fold_json_key("\u{212a}"), "K");
        assert_eq!(fold_json_key("retrie\u{17f}"), "RETRIES");
        assert_eq!(fold_json_key("\u{e9}"), "\u{c9}");
        assert_eq!(fold_json_key("\u{c9}"), "\u{c9}");
        assert_eq!(fold_json_key("\u{3c3}"), fold_json_key("\u{3c2}"));
        assert_eq!(fold_json_key("\u{fffd}"), "\u{fffd}");
    }

    proptest::proptest! {
        /// No panic on arbitrary bytes, and a rejection only for a memo
        /// with a "forward" key.
        #[test]
        fn arbitrary_memos(bz in proptest::collection::vec(proptest::num::u8::ANY, 0..512)) {
            let s = gostr::from_go_bytes(&bz);
            if reject_forward_memo(&s) {
                proptest::prop_assert!(s.contains("forward"));
            }
            let _ = retry_memo_within_limits(&s);
            let _ = string_next_within_limits(&bz);
        }

        /// No panic on forward memos built from JSON-ish fragments.
        #[test]
        fn forward_like_memos(parts in proptest::collection::vec(
            proptest::sample::select(vec![
                "{", "}", "[", "]", ",", ":", "\"next\"", "\"NEXT\"", "\"forward\"",
                "\"Forward\"", "\"timeout\"", "1e19", "1e400", "null", "\"a\"",
                "\"{\\\"a\\\":1}\"", "\"\\ud800\"", "\u{212a}", " ",
            ]),
            0..64,
        )) {
            let memo = format!("{{\"forward\":{}", parts.concat());
            let _ = reject_forward_memo(&memo);
            let _ = retry_memo_within_limits(&memo);
        }
    }
}
