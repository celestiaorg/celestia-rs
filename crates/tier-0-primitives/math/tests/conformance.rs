//! Replays the Go-generated golden vectors in
//! `conformance/fixtures/math/vectors.json` (generator: `conformance/mathvectors`).

#![allow(clippy::expect_used, clippy::panic, clippy::too_many_lines)]

use std::collections::BTreeMap;

use math::{Int, LegacyDec, Uint, int_json_to_proto_bytes, legacy_dec_json_to_proto_bytes};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    source: String,
    vectors: Vec<Vector>,
}

#[derive(Deserialize)]
struct Vector {
    op: String,
    args: Vec<String>,
    out: String,
}

fn load() -> Fixture {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../conformance/fixtures/math/vectors.json"
    );
    let bz = std::fs::read(path).expect("read vectors.json");
    serde_json::from_slice(&bz).expect("parse vectors.json")
}

fn int(s: &str) -> Int {
    Int::parse(s).expect("fixture int arg")
}

fn uint(s: &str) -> Uint {
    Uint::parse(s).expect("fixture uint arg")
}

fn dec(s: &str) -> LegacyDec {
    LegacyDec::from_str_go(s).expect("fixture dec arg")
}

fn i64_arg(s: &str) -> i64 {
    s.parse().expect("fixture i64 arg")
}

fn u64_arg(s: &str) -> u64 {
    s.parse().expect("fixture u64 arg")
}

fn s<T: ToString, E>(r: Result<T, E>) -> Option<String> {
    r.ok().map(|v| v.to_string())
}

fn bytes<E>(r: Result<Vec<u8>, E>) -> Option<String> {
    r.ok().map(|v| String::from_utf8(v).expect("ascii"))
}

/// Evaluates one vector in Rust. `None` means the operation failed (Go
/// returned an error or panicked).
fn eval(v: &Vector) -> Option<String> {
    let a = &v.args;
    match v.op.as_str() {
        // ---- Int
        "int_parse" => s(Int::parse(&a[0])),
        "int_unmarshal" => s(Int::unmarshal(a[0].as_bytes())),
        "int_json_to_proto_bytes" => bytes(int_json_to_proto_bytes(&a[0])),
        "int_unmarshal_json" => s(Int::from_json(a[0].as_bytes())),
        "int_add" => s(int(&a[0]).checked_add(&int(&a[1]))),
        "int_sub" => s(int(&a[0]).checked_sub(&int(&a[1]))),
        "int_mul" => s(int(&a[0]).checked_mul(&int(&a[1]))),
        "int_quo" => s(int(&a[0]).checked_quo(&int(&a[1]))),
        "int_mod" => s(int(&a[0]).checked_mod(&int(&a[1]))),
        "int_cmp" => Some((int(&a[0]).cmp(&int(&a[1])) as i8).to_string()),
        "int_neg" => Some(int(&a[0]).neg().to_string()),
        "int_abs" => Some(int(&a[0]).abs().to_string()),
        "int_marshal" => bytes::<()>(Ok(int(&a[0]).marshal())),
        "int_marshal_json" => Some(int(&a[0]).to_json()),
        "int_to_dec" => Some(int(&a[0]).to_legacy_dec().to_string()),
        "int_int64" => int(&a[0]).to_i64().map(|v| v.to_string()),
        "int_uint64" => int(&a[0]).to_u64().map(|v| v.to_string()),
        // ---- Uint
        "uint_parse" => s(Uint::parse(&a[0])),
        "uint_unmarshal" => s(Uint::unmarshal(a[0].as_bytes())),
        "uint_add" => s(uint(&a[0]).checked_add(&uint(&a[1]))),
        "uint_sub" => s(uint(&a[0]).checked_sub(&uint(&a[1]))),
        "uint_mul" => s(uint(&a[0]).checked_mul(&uint(&a[1]))),
        "uint_quo" => s(uint(&a[0]).checked_quo(&uint(&a[1]))),
        "uint_mod" => s(uint(&a[0]).checked_mod(&uint(&a[1]))),
        "uint_relative_pow" => s(Uint::relative_pow(&uint(&a[0]), &uint(&a[1]), &uint(&a[2]))),
        // ---- LegacyDec
        "dec_parse" => s(LegacyDec::from_str_go(&a[0])),
        "dec_json_to_proto_bytes" => bytes(legacy_dec_json_to_proto_bytes(&a[0])),
        "dec_unmarshal" => s(LegacyDec::unmarshal(a[0].as_bytes())),
        "dec_unmarshal_json" => s(LegacyDec::from_json(a[0].as_bytes())),
        "dec_new_with_prec" => s(LegacyDec::new_with_prec(i64_arg(&a[0]), i64_arg(&a[1]))),
        "dec_new" => Some(LegacyDec::new(i64_arg(&a[0])).to_string()),
        "dec_from_int_with_prec" => s(LegacyDec::from_int_with_prec(&int(&a[0]), i64_arg(&a[1]))),
        "dec_add" => s(dec(&a[0]).checked_add(&dec(&a[1]))),
        "dec_sub" => s(dec(&a[0]).checked_sub(&dec(&a[1]))),
        "dec_mul" => s(dec(&a[0]).checked_mul(&dec(&a[1]))),
        "dec_mul_truncate" => s(dec(&a[0]).checked_mul_truncate(&dec(&a[1]))),
        "dec_mul_round_up" => s(dec(&a[0]).checked_mul_round_up(&dec(&a[1]))),
        "dec_quo" => s(dec(&a[0]).checked_quo(&dec(&a[1]))),
        "dec_quo_truncate" => s(dec(&a[0]).checked_quo_truncate(&dec(&a[1]))),
        "dec_quo_round_up" => s(dec(&a[0]).checked_quo_round_up(&dec(&a[1]))),
        "dec_cmp" => Some((dec(&a[0]).cmp(&dec(&a[1])) as i8).to_string()),
        "dec_mul_int" => s(dec(&a[0]).checked_mul_int(&int(&a[1]))),
        "dec_quo_int" => s(dec(&a[0]).checked_quo_int(&int(&a[1]))),
        "dec_mul_int64" => s(dec(&a[0]).checked_mul_int64(i64_arg(&a[1]))),
        "dec_quo_int64" => s(dec(&a[0]).checked_quo_int64(i64_arg(&a[1]))),
        "dec_neg" => Some(dec(&a[0]).neg().to_string()),
        "dec_abs" => Some(dec(&a[0]).abs().to_string()),
        "dec_ceil" => s(dec(&a[0]).ceil()),
        "dec_truncate_dec" => Some(dec(&a[0]).truncate_dec().to_string()),
        "dec_truncate_int" => Some(dec(&a[0]).truncate_int().to_string()),
        "dec_truncate_int64" => s(dec(&a[0]).truncate_int64()),
        "dec_round_int" => s(dec(&a[0]).round_int()),
        "dec_round_int64" => s(dec(&a[0]).round_int64()),
        "dec_is_integer" => Some(dec(&a[0]).is_integer().to_string()),
        "dec_marshal" => bytes::<()>(Ok(dec(&a[0]).marshal())),
        "dec_marshal_json" => Some(dec(&a[0]).to_json()),
        "dec_approx_sqrt" => s(dec(&a[0]).approx_sqrt()),
        "dec_power" => s(dec(&a[0]).checked_power(u64_arg(&a[1]))),
        "dec_approx_root" => s(dec(&a[0]).approx_root(u64_arg(&a[1]))),
        other => panic!("unknown op {other}"),
    }
}

#[test]
fn go_vectors() {
    let fixture = load();
    assert_eq!(fixture.source, "cosmossdk.io/math v1.5.3");
    assert!(fixture.vectors.len() > 5000, "too few vectors");

    let mut per_op: BTreeMap<&str, usize> = BTreeMap::new();
    let mut failures = Vec::new();
    for v in &fixture.vectors {
        *per_op.entry(v.op.as_str()).or_default() += 1;
        let got = eval(v);
        let ok = match v.out.as_str() {
            "error" | "panic" => got.is_none(),
            // Go leaves the value nil; this port yields zero (same marshalled bytes).
            "nil" => matches!(got.as_deref(), Some("0" | "0.000000000000000000")),
            want => got.as_deref() == Some(want),
        };
        if !ok {
            failures.push(format!(
                "{}({:?}): want {:?}, got {:?}",
                v.op, v.args, v.out, got
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} vectors failed:\n{}",
        failures.len(),
        fixture.vectors.len(),
        failures
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    // Every op family must be exercised.
    for op in [
        "int_parse",
        "int_mod",
        "uint_relative_pow",
        "dec_parse",
        "dec_mul",
        "dec_quo_round_up",
        "dec_power",
        "dec_approx_root",
        "dec_json_to_proto_bytes",
    ] {
        assert!(per_op.contains_key(op), "missing op {op}");
    }
}
