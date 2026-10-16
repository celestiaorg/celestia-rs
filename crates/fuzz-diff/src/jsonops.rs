//! The `json_*` ops: every JSON decode reachable from consensus execution,
//! called on the raw input as the state machine calls it (the Go side is
//! `conformance/fuzzoracle/*/jsondecode.go`, whose doc lists the ops and
//! the output format), plus seeds and a JSON-text mutator aimed at what
//! differs between JSON decoders: nesting depth, numbers out of `float64`
//! range, key folding, duplicate keys, invalid UTF-8.

use crate::Out;
use crate::mutate::Rng;

/// The JSON ops.
pub const OPS: &[&str] = &[
    "json_count",
    "json_gov",
    "json_pfm",
    "json_pfm_next",
    "json_ica_tx",
    "json_ica_pd",
    "json_ica_md",
    "json_ics20",
    "json_ack",
];

fn h(s: &str) -> String {
    hex::encode(gostr::to_go_bytes(s))
}

fn ok(out: String) -> Out {
    Out {
        ok: true,
        out,
        ..Out::default()
    }
}

fn fail(err: impl Into<String>) -> Out {
    Out {
        err: err.into(),
        ..Out::default()
    }
}

/// The Rust outcome of a `json_*` op.
#[must_use]
pub fn rust_json(op: &str, input: &[u8]) -> Out {
    let text = gostr::from_go_bytes(input);
    match op {
        "json_count" => match app::msg_count::count_json_msgs(input) {
            Some(n) => ok(n.to_string()),
            None => fail("count failed"),
        },
        "json_gov" => match x_gov::msgs::metadata_title_summary(input) {
            Some((t, s)) => ok(format!("{} {}", h(&t), h(&s))),
            None => fail("unmarshal failed"),
        },
        "json_pfm" => {
            use x_packetforward::memo::{self, Memo};
            match memo::classify(&text) {
                Memo::NotForward => ok("not_forward".into()),
                Memo::NilForward => ok("nil_forward".into()),
                Memo::ParseError(e) => fail(e),
                Memo::Forward(f) => {
                    let retries = f.retries.map_or_else(|| "-".into(), |r| r.to_string());
                    let next = match &f.next {
                        None => "-".into(),
                        Some(Ok(s)) => h(s),
                        Some(Err(_)) => "err".into(),
                    };
                    ok(format!(
                        "forward {} {} {} {} {retries} {next}",
                        h(&f.receiver),
                        h(&f.port),
                        h(&f.channel),
                        f.timeout
                    ))
                }
            }
        }
        "json_pfm_next" => match x_packetforward::memo::unmarshal_next(&text) {
            Ok(Ok(s)) => ok(h(&s)),
            Ok(Err(e)) | Err(e) => fail(e),
        },
        "json_ica_tx" => match x_icahost::host::deserialize_cosmos_tx(
            input,
            x_icahost::host::ENCODING_PROTO3_JSON,
            app::ibc::ica_registry(),
        ) {
            Ok(msgs) => Out {
                ok: true,
                msgs: msgs
                    .iter()
                    .map(|m| format!("{}:{}", h(&m.type_url), hex::encode(&m.value)))
                    .collect(),
                ..Out::default()
            },
            Err(e) => fail(e.log),
        },
        "json_ica_pd" => match x_icahost::host::packet_data(input) {
            Some((ty, data)) => ok(format!("{ty} {}", hex::encode(data))),
            None => fail("unmarshal failed"),
        },
        "json_ica_md" => match x_icahost::host::metadata_from_version(&text) {
            Ok(m) => ok([
                &m.version,
                &m.controller_connection_id,
                &m.host_connection_id,
                &m.address,
                &m.encoding,
                &m.tx_type,
            ]
            .map(|s| h(s))
            .join(" ")),
            Err(e) => fail(e.log),
        },
        "json_ics20" => match x_transfer::relay::PacketData::decode(input) {
            Ok(d) => ok([&d.denom, &d.amount, &d.sender, &d.receiver, &d.memo]
                .map(|s| h(s))
                .join(" ")),
            Err(e) => fail(e),
        },
        "json_ack" => match x_ibc::channel::decode_acknowledgement(input) {
            Ok(x_ibc::channel::AckResponse::Empty) => ok("none".into()),
            Ok(x_ibc::channel::AckResponse::Result(r)) => ok(format!("result {}", hex::encode(r))),
            Ok(x_ibc::channel::AckResponse::Error(e)) => ok(format!("error {}", h(&e))),
            Err(e) => fail(e),
        },
        _ => fail(format!("unknown op {op}")),
    }
}

/// `None` when Go and Rust agree on a `json_*` op (accept / reject and the
/// decoded summary; error texts are not compared).
#[must_use]
pub fn compare(op: &str, go: &Out, rust: &Out) -> Option<String> {
    (go.ok != rust.ok || go.out != rust.out || go.msgs != rust.msgs || !go.panic.is_empty())
        .then(|| format!("{op}: go {go:?}\n  rust {rust:?}"))
}

fn nest(n: usize, inner: &str) -> String {
    format!("{}{inner}{}", "[".repeat(n), "]".repeat(n))
}

fn onest(n: usize, inner: &str) -> String {
    format!("{}{inner}{}", "{\"a\":".repeat(n), "}".repeat(n))
}

/// Total nesting depths around the limits of serde_json (128) and Go (10 000).
pub const DEPTHS: &[usize] = &[127, 128, 129, 1000, 9_999, 10_000, 10_001];

const FWD: &str = r#""forward":{"receiver":"r","port":"transfer","channel":"channel-0""#;

/// A well-formed input of `op` whose total nesting depth is `d`
/// (`variant` picks where the nesting sits).
#[must_use]
pub fn deep_input(op: &str, d: usize, variant: usize) -> String {
    let x = |n: usize| {
        if variant.is_multiple_of(2) {
            nest(n, "1")
        } else {
            onest(n, "1")
        }
    };
    match op {
        "json_count" if variant < 2 => format!(r#"{{"messages":[{{}}],"x":{}}}"#, x(d - 1)),
        "json_count" => format!(r#"{{"messages":[{{"a":{}}}]}}"#, x(d - 3)),
        "json_gov" => format!(r#"{{"title":"T","summary":"S","x":{}}}"#, x(d - 1)),
        "json_pfm" if variant < 2 => format!("{{{FWD}}},\"x\":{}}}", x(d - 1)),
        "json_pfm" => format!("{{{FWD},\"next\":{}}}}}", x(d - 2)),
        "json_pfm_next" => x(d),
        "json_ica_tx" => format!(
            r#"{{"messages":[{{"@type":"/cosmos.bank.v1beta1.MsgSend","from_address":"a","to_address":"b","x":{}}}]}}"#,
            x(d - 3)
        ),
        "json_ica_pd" => format!(r#"{{"type":"TYPE_EXECUTE_TX","data":"","x":{}}}"#, x(d - 1)),
        "json_ica_md" => format!(r#"{{"version":"ics27-1","x":{}}}"#, x(d - 1)),
        "json_ics20" => format!(
            r#"{{"denom":"d","amount":"1","sender":"s","receiver":"r","memo":"m","x":{}}}"#,
            x(d - 1)
        ),
        _ => format!(r#"{{"result":"AQ==","x":{}}}"#, x(d - 1)),
    }
}

/// Seeds of `op`: valid documents of its shape and the depth cases.
#[must_use]
pub fn seeds(op: &str) -> Vec<Vec<u8>> {
    let base: &[&str] = match op {
        "json_count" => &[
            r#"{"messages":[{"@type":"/cosmos.bank.v1beta1.MsgSend","amount":[{"denom":"utia","amount":"1"}]},{}]}"#,
            r#"{"Messages":null}"#,
        ],
        "json_gov" => &[
            r#"{"title":"T","authors":["a",null],"summary":"S","details":"d","proposal_forum_url":"u","vote_option_context":"v"}"#,
        ],
        "json_pfm" | "json_pfm_next" => &[
            r#"{"forward":{"receiver":"r","port":"transfer","channel":"channel-0","timeout":"1m","retries":2,"next":{"b":1,"a":[1.50,"<x>",{"c":null}],"b":2}}}"#,
            r#"{"forward":{"receiver":"r","port":"transfer","channel":"channel-0","next":"{\"x\":1}"}}"#,
        ],
        "json_ica_tx" => &[
            r#"{"messages":[{"@type":"/cosmos.authz.v1beta1.MsgExec","grantee":"g","msgs":[{"@type":"/cosmos.bank.v1beta1.MsgSend","from_address":"a","to_address":"b","amount":[{"denom":"utia","amount":"1"}]}]}]}"#,
        ],
        "json_ica_pd" => &[r#"{"type":"TYPE_EXECUTE_TX","data":"AQI=","memo":"m"}"#],
        "json_ica_md" => &[
            r#"{"version":"ics27-1","controller_connection_id":"connection-0","host_connection_id":"connection-1","address":"","encoding":"proto3json","tx_type":"sdk_multi_msg"}"#,
        ],
        "json_ics20" => &[
            r#"{"denom":"utia","amount":"1","sender":"s","receiver":"r","memo":"{\"forward\":{}}"}"#,
        ],
        _ => &[r#"{"result":"AQ=="}"#, r#"{"error":"e"}"#],
    };
    let mut out: Vec<Vec<u8>> = base.iter().map(|s| s.as_bytes().to_vec()).collect();
    if op == "json_ica_tx" {
        // `MsgExec` nested around the interface registry's limit of 10
        // `Any` levels.
        for k in [1, 9, 10, 11] {
            let mut inner = r#"{"@type":"/cosmos.bank.v1beta1.MsgSend","from_address":"a","to_address":"b","amount":[{"denom":"utia","amount":"1"}]}"#.to_owned();
            for _ in 0..k {
                inner = format!(
                    r#"{{"@type":"/cosmos.authz.v1beta1.MsgExec","grantee":"g","msgs":[{inner}]}}"#
                );
            }
            out.push(format!(r#"{{"messages":[{inner}]}}"#).into_bytes());
        }
    }
    for &d in DEPTHS {
        for v in 0..4 {
            out.push(deep_input(op, d, v).into_bytes());
        }
    }
    out
}

/// Byte positions of the values of `s` (cheap: after `:`, `[` or `,`).
fn value_starts(s: &[u8]) -> Vec<usize> {
    let mut v = Vec::new();
    let mut in_str = false;
    let mut esc = false;
    for (i, &c) in s.iter().enumerate() {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b':' | b'[' | b',' => v.push(i + 1),
            _ => {}
        }
    }
    v
}

/// A JSON-text mutation of `input`.
#[must_use]
pub fn mutate(rng: &mut Rng, input: &[u8]) -> Vec<u8> {
    let starts = value_starts(input);
    let mut s = input.to_vec();
    if starts.is_empty() {
        return s;
    }
    let at = starts[rng.below(starts.len())];
    let insert = |s: &mut Vec<u8>, at: usize, b: &[u8]| {
        s.splice(at..at, b.iter().copied());
    };
    match rng.below(9) {
        // A deep value before an existing one (`[[[...]]],` keeps arrays
        // valid; in objects it is a syntax error, also worth comparing).
        0 => {
            let d = [100, 127, 128, 129, 5_000, 9_990, 9_999, 10_000, 10_001][rng.below(9)];
            insert(&mut s, at, format!("{},", nest(d, "0")).as_bytes());
        }
        // A deep member in front of the first key.
        1 => {
            if let Some(p) = s.iter().position(|&c| c == b'{') {
                let d = [126, 127, 128, 9_998, 9_999, 10_000][rng.below(6)];
                insert(
                    &mut s,
                    p + 1,
                    format!("\"zz\":{},", onest(d, "{}")).as_bytes(),
                );
            }
        }
        // Numbers float64 cannot hold, and exotic literals.
        2 => {
            let n = [
                "1e400",
                "-1e400",
                "1e-400",
                "0.0",
                "-0",
                "123456789012345678901234567890",
            ][rng.below(6)];
            insert(&mut s, at, format!("{n},").as_bytes());
        }
        // A key folded with long s / Kelvin sign, or upper case.
        3 => {
            let i = rng.below(s.len());
            match s[i] {
                b's' => {
                    s.splice(i..=i, "\u{17f}".bytes());
                }
                b'k' => {
                    s.splice(i..=i, "\u{212a}".bytes());
                }
                c if c.is_ascii_lowercase() => s[i] = c.to_ascii_uppercase(),
                _ => {}
            }
        }
        // Duplicate a member.
        4 => {
            if let Some(p) = s.iter().position(|&c| c == b'{') {
                let end = s[p..]
                    .iter()
                    .position(|&c| c == b',' || c == b'}')
                    .map_or(s.len(), |e| p + e);
                let member: Vec<u8> = s[p + 1..end].to_vec();
                if !member.is_empty() {
                    let mut m = member;
                    m.push(b',');
                    insert(&mut s, p + 1, &m);
                }
            }
        }
        // `null` or a wrong-typed scalar.
        5 => {
            let v = ["null,", "true,", "\"x\",", "{},", "[],"][rng.below(5)];
            insert(&mut s, at, v.as_bytes());
        }
        // Invalid UTF-8, escapes, control characters.
        6 => {
            let i = rng.below(s.len());
            let b: &[u8] = [
                &b"\xff"[..],
                b"\xc3",
                b"\\ud800",
                b"\\u0000",
                b"\n",
                b"\x01",
            ][rng.below(6)];
            insert(&mut s, i, b);
        }
        // Truncate or append garbage.
        7 => {
            if rng.one_in(2) {
                s.truncate(rng.below(s.len()));
            } else {
                s.extend_from_slice([&b" x"[..], b"{}", b" ", b"]"][rng.below(4)]);
            }
        }
        // Delete a byte.
        _ => {
            let i = rng.below(s.len());
            s.remove(i);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_inputs_have_the_depth() {
        for op in OPS {
            for v in 0..4 {
                for d in [127usize, 10_000] {
                    let s = deep_input(op, d, v);
                    assert_eq!(
                        gojson::check_valid(s.as_bytes()).map(|x| x.depth),
                        Ok(d),
                        "{op} {v} {d}"
                    );
                }
            }
        }
    }
}
