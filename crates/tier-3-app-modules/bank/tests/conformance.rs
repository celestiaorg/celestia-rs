//! Conformance tests for bank storage against Go-generated SDK fixtures.

#![allow(clippy::expect_used, clippy::panic)]

use bank::{Keeper, STORE_KEY};
use cosmos_types::Coin;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct Fixture {
    scenario: String,
    store_key: String,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    label: String,
    address_hex: String,
    denom: String,
    amount: String,
    balance_key_hex: String,
    balance_value_hex: String,
    denom_index_key_hex: String,
    denom_index_value_hex: String,
    supply_key_hex: String,
    supply_value_hex: String,
}

#[test]
fn balance_supply_and_index_bytes_match_go() {
    let fixture: Fixture = load_fixture();
    assert_eq!(fixture.scenario, "bank-balances");
    assert_eq!(fixture.store_key, STORE_KEY);

    for case in fixture.cases {
        let address = decode_address(&case.address_hex);
        let coin = Coin::new(&case.denom, &case.amount).expect("valid fixture coin");
        let mut store = kvstore::Store::new();
        {
            let mut keeper = Keeper::new(&mut store);
            keeper
                .set_balance(&address, &coin)
                .unwrap_or_else(|error| panic!("{}: set balance: {error}", case.label));
            keeper
                .set_supply(&coin)
                .unwrap_or_else(|error| panic!("{}: set supply: {error}", case.label));
        }
        assert_raw_value(&store, &case.balance_key_hex, &case.balance_value_hex);
        assert_raw_value(
            &store,
            &case.denom_index_key_hex,
            &case.denom_index_value_hex,
        );
        assert_raw_value(&store, &case.supply_key_hex, &case.supply_value_hex);

        let mut go_store = kvstore::Store::new();
        go_store
            .set(
                hex::decode(&case.balance_key_hex).expect("balance key hex"),
                hex::decode(&case.balance_value_hex).expect("balance value hex"),
            )
            .expect("seed Go balance");
        go_store
            .set(
                hex::decode(&case.supply_key_hex).expect("supply key hex"),
                hex::decode(&case.supply_value_hex).expect("supply value hex"),
            )
            .expect("seed Go supply");
        let keeper = Keeper::new(&mut go_store);
        assert_eq!(
            keeper.balance(&address, &case.denom).expect("Go balance"),
            coin
        );
        assert_eq!(keeper.supply(&case.denom).expect("Go supply"), coin);
    }
}

fn assert_raw_value(store: &kvstore::Store, key_hex: &str, value_hex: &str) {
    assert_eq!(
        store
            .get(&hex::decode(key_hex).expect("key hex"))
            .expect("read raw value"),
        Some(hex::decode(value_hex).expect("value hex"))
    );
}

fn load_fixture() -> Fixture {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../conformance/fixtures/bank/balances.json");
    let bytes = std::fs::read(path).expect("read bank fixture");
    serde_json::from_slice(&bytes).expect("decode bank fixture")
}

fn decode_address(value: &str) -> cosmos_types::AccountAddress {
    hex::decode(value)
        .expect("address hex")
        .try_into()
        .expect("20-byte account address")
}
