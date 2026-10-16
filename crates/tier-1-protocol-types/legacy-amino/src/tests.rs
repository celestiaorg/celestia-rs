//! Go vectors (`conformance/aminoschema/vectors.json` from celestia-appd
//! v10.4.0, `vectors_v9.json` from v9.0.8, recorded by `vectors_test.go`
//! through the `TxDecodeAmino` / `TxEncodeAmino` handlers): every output,
//! error and panic must match.

use crate::{Error, Schema, tx_decode_amino_in, tx_encode_amino_in};

const VECTORS: &str = include_str!("../../../../conformance/aminoschema/vectors.json");
const VECTORS_V9: &str = include_str!("../../../../conformance/aminoschema/vectors_v9.json");

#[test]
fn go_vectors() {
    check_vectors(Schema::get(), VECTORS);
}

#[test]
fn go_vectors_v9() {
    check_vectors(Schema::v9(), VECTORS_V9);
    assert!(std::ptr::eq(Schema::for_app_version(9), Schema::v9()));
    assert!(std::ptr::eq(Schema::for_app_version(10), Schema::get()));
    assert!(std::ptr::eq(Schema::for_app_version(0), Schema::get()));
}

fn check_vectors(s: &Schema, src: &str) {
    let vs: serde_json::Value = serde_json::from_str(src).unwrap_or_default();
    let vs = vs.as_array().cloned().unwrap_or_default();
    assert!(vs.len() > 70, "{} vectors", vs.len());
    let mut failures = Vec::new();
    for v in &vs {
        let name = v["name"].as_str().unwrap_or_default();
        let op = v["op"].as_str().unwrap_or_default();
        let input = v["input"].as_str().unwrap_or_default();
        let got: Result<String, Error> = if op == "decode" {
            tx_decode_amino_in(s, &hex::decode(input).unwrap_or_default())
        } else {
            tx_encode_amino_in(s, input.as_bytes()).map(hex::encode)
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
    for s in [Schema::get(), Schema::v9()] {
        assert!(s.types.len() > 200);
        assert_eq!(s.t(s.root).go, "legacytx.StdTx");
    }
}
