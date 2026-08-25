//! Conformance tests for staking storage against Go-generated SDK fixtures.

#![allow(clippy::expect_used, clippy::panic)]

use cosmos_types::proto::staking::{Delegation, Validator};
use prost::Message;
use serde::Deserialize;
use staking::{Keeper, STORE_KEY};
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct Fixture {
    scenario: String,
    store_key: String,
    validator: ValidatorCase,
    delegations: Vec<DelegationCase>,
}

#[derive(Debug, Deserialize)]
struct ValidatorCase {
    operator_address: String,
    operator_address_hex: String,
    tokens: String,
    delegator_shares_atomics: String,
    key_hex: String,
    value_hex: String,
}

#[derive(Debug, Deserialize)]
struct DelegationCase {
    label: String,
    delegator_address: String,
    delegator_address_hex: String,
    validator_address: String,
    validator_address_hex: String,
    shares_atomics: String,
    key_hex: String,
    value_hex: String,
    index_key_hex: String,
    index_value_hex: String,
}

#[test]
fn validator_and_delegation_bytes_match_go() {
    let fixture = load_fixture();
    assert_eq!(fixture.scenario, "staking-state");
    assert_eq!(fixture.store_key, STORE_KEY);

    let validator_bytes = hex::decode(&fixture.validator.value_hex).expect("validator value hex");
    let validator = Validator::decode(validator_bytes.as_slice()).expect("Go validator protobuf");
    assert_eq!(
        validator.operator_address,
        fixture.validator.operator_address
    );
    assert_eq!(validator.tokens, fixture.validator.tokens);
    assert_eq!(
        validator.delegator_shares,
        fixture.validator.delegator_shares_atomics
    );

    let mut store = kvstore::Store::new();
    {
        let mut keeper = Keeper::new(&mut store);
        keeper.set_validator(&validator).expect("set validator");
        for case in &fixture.delegations {
            let bytes = hex::decode(&case.value_hex).expect("delegation value hex");
            let delegation = Delegation::decode(bytes.as_slice()).expect("Go delegation protobuf");
            assert_eq!(
                delegation.delegator_address, case.delegator_address,
                "{}",
                case.label
            );
            assert_eq!(
                delegation.validator_address, case.validator_address,
                "{}",
                case.label
            );
            assert_eq!(delegation.shares, case.shares_atomics, "{}", case.label);
            keeper
                .set_delegation(&delegation)
                .unwrap_or_else(|error| panic!("{}: set delegation: {error}", case.label));
        }
    }

    assert_raw_value(
        &store,
        &fixture.validator.key_hex,
        &fixture.validator.value_hex,
    );
    for case in &fixture.delegations {
        assert_raw_value(&store, &case.key_hex, &case.value_hex);
        assert_raw_value(&store, &case.index_key_hex, &case.index_value_hex);
    }

    let keeper = Keeper::new(&mut store);
    let validator_address = decode_valoper(&fixture.validator.operator_address_hex);
    assert_eq!(
        keeper.validator(&validator_address).expect("get validator"),
        Some(validator)
    );
    let by_validator = keeper
        .validator_delegations(&validator_address)
        .expect("validator delegations");
    assert_eq!(by_validator.len(), fixture.delegations.len());
    for (actual, expected) in by_validator.iter().zip(&fixture.delegations) {
        assert_eq!(actual.delegator_address, expected.delegator_address);
        let delegator = decode_account(&expected.delegator_address_hex);
        assert_eq!(
            keeper
                .delegator_delegations(&delegator)
                .expect("delegator delegations"),
            vec![actual.clone()]
        );
        assert_eq!(
            decode_valoper(&expected.validator_address_hex),
            validator_address
        );
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
        .join("../../../conformance/fixtures/staking/state.json");
    let bytes = std::fs::read(path).expect("read staking fixture");
    serde_json::from_slice(&bytes).expect("decode staking fixture")
}

fn decode_account(value: &str) -> cosmos_types::AccountAddress {
    hex::decode(value)
        .expect("account address hex")
        .try_into()
        .expect("20-byte account address")
}

fn decode_valoper(value: &str) -> cosmos_types::ValoperAddress {
    hex::decode(value)
        .expect("validator address hex")
        .try_into()
        .expect("20-byte validator address")
}
