//! Conformance tests for staking storage against Go-generated SDK fixtures.

#![allow(clippy::expect_used, clippy::panic, clippy::too_many_lines)]

use cosmos_types::proto::staking::{Delegation, Validator};
use cosmos_types::{Any, Coin};
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

#[derive(Debug, Deserialize)]
struct DelegateFixture {
    scenario: String,
    type_url: String,
    value_hex: String,
    delegator_address_hex: String,
    validator_address_hex: String,
    pool_address_hex: String,
    initial_delegator_balance: String,
    expected_delegator_balance: String,
    initial_pool_balance: String,
    expected_pool_balance: String,
    validator_key_hex: String,
    initial_validator_value_hex: String,
    expected_validator_value_hex: String,
    delegation_key_hex: String,
    initial_delegation_value_hex: String,
    expected_delegation_value_hex: String,
    delegation_index_key_hex: String,
    old_power_key_hex: String,
    new_power_key_hex: String,
    power_value_hex: String,
    events: Vec<ExpectedEvent>,
}

#[derive(Debug, Deserialize)]
struct ExpectedEvent {
    #[serde(rename = "type")]
    ty: String,
    attributes: Vec<ExpectedAttribute>,
}

#[derive(Debug, Deserialize)]
struct ExpectedAttribute {
    key: String,
    value: String,
    index: bool,
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

#[test]
fn msg_delegate_matches_go_state_balances_power_index_and_events() {
    let fixture: DelegateFixture = load_named_fixture("delegate.json");
    assert_eq!(fixture.scenario, "staking-delegate");
    let delegator = decode_account(&fixture.delegator_address_hex);
    let validator_address = decode_valoper(&fixture.validator_address_hex);
    let pool = decode_account(&fixture.pool_address_hex);
    let validator = Validator::decode(
        hex::decode(&fixture.initial_validator_value_hex)
            .expect("validator hex")
            .as_slice(),
    )
    .expect("initial validator");
    let delegation = Delegation::decode(
        hex::decode(&fixture.initial_delegation_value_hex)
            .expect("delegation hex")
            .as_slice(),
    )
    .expect("initial delegation");

    let mut staking_store = kvstore::Store::new();
    let mut bank_store = kvstore::Store::new();
    {
        let mut staking = Keeper::new(&mut staking_store);
        staking.set_validator(&validator).expect("seed validator");
        staking
            .set_validator_by_power_index(&validator)
            .expect("seed power index");
        staking
            .set_delegation(&delegation)
            .expect("seed delegation");
    }
    assert_raw_value(
        &staking_store,
        &fixture.validator_key_hex,
        &fixture.initial_validator_value_hex,
    );
    assert_raw_value(
        &staking_store,
        &fixture.delegation_key_hex,
        &fixture.initial_delegation_value_hex,
    );
    assert_raw_value(&staking_store, &fixture.delegation_index_key_hex, "");
    assert_raw_value(
        &staking_store,
        &fixture.old_power_key_hex,
        &fixture.power_value_hex,
    );

    let result = {
        let mut bank = bank::Keeper::new(&mut bank_store);
        bank.set_balance(
            &delegator,
            &Coin::new("utia", &fixture.initial_delegator_balance).expect("delegator balance"),
        )
        .expect("seed delegator balance");
        bank.set_balance(
            &pool,
            &Coin::new("utia", &fixture.initial_pool_balance).expect("pool balance"),
        )
        .expect("seed pool balance");
        Keeper::new(&mut staking_store)
            .execute_message(
                &mut bank,
                &Any {
                    type_url: fixture.type_url.clone(),
                    value: hex::decode(&fixture.value_hex).expect("message hex"),
                },
            )
            .expect("execute MsgDelegate")
    };

    assert_events(&result.events, &fixture.events);
    assert_raw_value(
        &staking_store,
        &fixture.validator_key_hex,
        &fixture.expected_validator_value_hex,
    );
    assert_raw_value(
        &staking_store,
        &fixture.delegation_key_hex,
        &fixture.expected_delegation_value_hex,
    );
    assert_eq!(
        staking_store
            .get(&hex::decode(&fixture.old_power_key_hex).expect("old power key"))
            .expect("read old power key"),
        None
    );
    assert_raw_value(
        &staking_store,
        &fixture.new_power_key_hex,
        &fixture.power_value_hex,
    );

    let bank = bank::Keeper::new(&mut bank_store);
    assert_eq!(
        bank.balance(&delegator, "utia")
            .expect("delegator balance")
            .amount,
        fixture.expected_delegator_balance
    );
    assert_eq!(
        bank.balance(&pool, "utia").expect("pool balance").amount,
        fixture.expected_pool_balance
    );
    assert_eq!(
        Keeper::new(&mut staking_store)
            .validator(&validator_address)
            .expect("updated validator")
            .expect("validator exists")
            .encode_to_vec(),
        hex::decode(&fixture.expected_validator_value_hex).expect("expected validator hex")
    );
}

fn assert_events(actual: &[abci_types::Event], expected: &[ExpectedEvent]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert_eq!(actual.ty, expected.ty);
        assert_eq!(actual.attributes.len(), expected.attributes.len());
        for (actual, expected) in actual.attributes.iter().zip(&expected.attributes) {
            assert_eq!(actual.key, expected.key);
            assert_eq!(actual.value, expected.value);
            assert_eq!(actual.index, expected.index);
        }
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
    load_named_fixture("state.json")
}

fn load_named_fixture<T: serde::de::DeserializeOwned>(name: &str) -> T {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../conformance/fixtures/staking")
        .join(name);
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
