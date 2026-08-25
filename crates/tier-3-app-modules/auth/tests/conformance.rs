//! Conformance tests for auth storage against Go-generated SDK fixtures.

#![allow(clippy::expect_used, clippy::panic)]

use auth::{BaseAccount, Keeper, STORE_KEY};
use prost::Message;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct Fixture {
    scenario: String,
    store_key: String,
    account_key_prefix_hex: String,
    account_number_key_hex: String,
    account_number: u64,
    account_number_value_hex: String,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    label: String,
    address_hex: String,
    account_number: u64,
    sequence: u64,
    #[serde(default)]
    public_key_any_hex: String,
    store_key_hex: String,
    store_value_hex: String,
    account_number_index_key_hex: String,
    account_number_index_value_hex: String,
}

#[test]
fn base_account_store_bytes_match_go() {
    let fixture: Fixture = load_fixture();
    assert_eq!(fixture.scenario, "auth-accounts");
    assert_eq!(fixture.store_key, STORE_KEY);
    assert_eq!(fixture.account_key_prefix_hex, "01");

    for case in fixture.cases {
        let address = decode_address(&case.address_hex);
        let public_key = if case.public_key_any_hex.is_empty() {
            None
        } else {
            let bytes = hex::decode(&case.public_key_any_hex).expect("public key hex");
            let proto =
                cosmos_types::proto::google::Any::decode(bytes.as_slice()).expect("public key any");
            Some(cosmos_types::Any::from_proto(proto).expect("valid public key any"))
        };
        let expected = BaseAccount {
            address,
            public_key,
            account_number: case.account_number,
            sequence: case.sequence,
        };

        let mut store = kvstore::Store::new();
        Keeper::new(&mut store)
            .set_account(&expected)
            .unwrap_or_else(|error| panic!("{}: set account: {error}", case.label));
        assert_eq!(
            store
                .get(&hex::decode(&case.store_key_hex).expect("store key hex"))
                .expect("read stored account"),
            Some(hex::decode(&case.store_value_hex).expect("store value hex")),
            "{}",
            case.label
        );
        assert_eq!(
            store
                .get(
                    &hex::decode(&case.account_number_index_key_hex)
                        .expect("account number index key hex")
                )
                .expect("read account number index"),
            Some(
                hex::decode(&case.account_number_index_value_hex)
                    .expect("account number index value hex")
            ),
            "{}",
            case.label
        );

        let mut go_store = kvstore::Store::new();
        go_store
            .set(
                hex::decode(&case.store_key_hex).expect("store key hex"),
                hex::decode(&case.store_value_hex).expect("store value hex"),
            )
            .expect("seed Go bytes");
        assert_eq!(
            Keeper::new(&mut go_store)
                .get_account(&address)
                .unwrap_or_else(|error| panic!("{}: decode Go account: {error}", case.label)),
            Some(expected),
            "{}",
            case.label
        );
    }
}

#[test]
fn global_account_number_encoding_matches_go() {
    let fixture: Fixture = load_fixture();
    let key = hex::decode(fixture.account_number_key_hex).expect("account number key hex");
    let value = hex::decode(fixture.account_number_value_hex).expect("account number value hex");
    let mut store = kvstore::Store::new();
    store.set(key.clone(), value).expect("seed Go sequence");

    {
        let mut keeper = Keeper::new(&mut store);
        assert_eq!(
            keeper.peek_next_account_number().expect("peek sequence"),
            fixture.account_number
        );
        assert_eq!(
            keeper.next_account_number().expect("increment sequence"),
            fixture.account_number
        );
    }
    assert_eq!(
        store.get(&key).expect("read incremented sequence"),
        Some((fixture.account_number + 1).to_be_bytes().to_vec())
    );
}

fn load_fixture() -> Fixture {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../conformance/fixtures/auth/accounts.json");
    let bytes = std::fs::read(path).expect("read auth fixture");
    serde_json::from_slice(&bytes).expect("decode auth fixture")
}

fn decode_address(value: &str) -> cosmos_types::AccountAddress {
    hex::decode(value)
        .expect("address hex")
        .try_into()
        .expect("20-byte account address")
}
