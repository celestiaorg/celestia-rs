//! Go-generated vectors (`conformance/mintvectors`, using celestia-app
//! v10.4.0 `x/mint/types`) for the inflation schedule and block provisions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use math::LegacyDec;
use pbjson_types::Timestamp;
use serde::Deserialize;
use x_mint::{Minter, calculate_inflation_rate};

#[derive(Deserialize)]
struct InflationCase {
    genesis_unix_nanos: i64,
    block_unix_nanos: i64,
    inflation_rate: String,
}

#[derive(Deserialize)]
struct ProvisionCase {
    annual_provisions: String,
    previous_unix_nanos: i64,
    current_unix_nanos: i64,
    amount: String,
    error: bool,
}

#[derive(Deserialize)]
struct Vectors {
    inflation: Vec<InflationCase>,
    provision: Vec<ProvisionCase>,
}

fn ts(n: i64) -> Timestamp {
    sdk::time::from_nanos(i128::from(n))
}

#[test]
fn go_vectors() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../../conformance/fixtures/mint/vectors.json"
    );
    let v: Vectors = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(v.inflation.len() > 50);
    for c in &v.inflation {
        let got =
            calculate_inflation_rate(&ts(c.genesis_unix_nanos), &ts(c.block_unix_nanos)).unwrap();
        assert_eq!(
            got.to_string(),
            c.inflation_rate,
            "block {}",
            c.block_unix_nanos
        );
    }
    assert!(v.provision.len() > 100);
    for c in &v.provision {
        let m = Minter {
            inflation_rate: LegacyDec::zero(),
            annual_provisions: LegacyDec::from_str_go(&c.annual_provisions).unwrap(),
            previous_block_time: None,
            bond_denom: "utia".into(),
        };
        let got =
            m.calculate_block_provision(&ts(c.current_unix_nanos), &ts(c.previous_unix_nanos));
        if c.error {
            assert!(got.is_err());
        } else {
            assert_eq!(got.unwrap().amount.to_string(), c.amount);
        }
    }
}
