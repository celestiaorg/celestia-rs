//! Go vectors (`conformance/aminoschema/vectors.json`, recorded by
//! `vectors_test.go` through celestia-appd's `TxDecodeAmino` /
//! `TxEncodeAmino` handlers): every output, error and panic must match.

use crate::{Error, Schema, tx_decode_amino, tx_encode_amino};

const VECTORS: &str = include_str!("../../../../conformance/aminoschema/vectors.json");

#[test]
fn go_vectors() {
    let vs: serde_json::Value = serde_json::from_str(VECTORS).unwrap_or_default();
    let vs = vs.as_array().cloned().unwrap_or_default();
    assert!(vs.len() > 70, "{} vectors", vs.len());
    let mut failures = Vec::new();
    for v in &vs {
        let name = v["name"].as_str().unwrap_or_default();
        let op = v["op"].as_str().unwrap_or_default();
        let input = v["input"].as_str().unwrap_or_default();
        let got: Result<String, Error> = if op == "decode" {
            tx_decode_amino(&hex::decode(input).unwrap_or_default())
        } else {
            tx_encode_amino(input.as_bytes()).map(hex::encode)
        };
        // Registered errors render like any Go error.
        let got = got.map_err(|e| match e {
            Error::Sdk(_, s) => Error::Go(s),
            e => e,
        });
        let want = if let Some(p) = v["panic"].as_str() {
            Err(Error::Panic(p.to_owned()))
        } else if let Some(e) = v["err"].as_str() {
            Err(Error::Go(e.to_owned()))
        } else {
            Ok(v["out"].as_str().unwrap_or_default().to_owned())
        };
        if got != want {
            failures.push(format!("{op} {name}:\n  got  {got:?}\n  want {want:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn schema_kinds() {
    let s = Schema::get();
    assert!(s.types.len() > 200);
    assert_eq!(s.t(s.root).go, "legacytx.StdTx");
}
