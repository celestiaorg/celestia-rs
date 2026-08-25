//! Public API and validation tests for the staking keeper.

#![allow(clippy::expect_used, clippy::too_many_lines)]

use cosmos_types::proto::staking::{Delegation, MsgDelegate, Validator};
use cosmos_types::{Any, Coin, proto};
use prost::Message;
use staking::{BONDED_POOL_NAME, Error, Keeper, MSG_DELEGATE_TYPE_URL, NOT_BONDED_POOL_NAME};

#[test]
fn validator_crud_rejects_address_mismatches_and_malformed_state() {
    let address = [1; 20];
    let mut validator = validator(address);
    let mut store = kvstore::Store::new();
    {
        let mut keeper = Keeper::new(&mut store);
        keeper.set_validator(&validator).expect("set validator");
        assert_eq!(
            keeper.validator(&address).expect("validator"),
            Some(validator.clone())
        );

        validator.operator_address = cosmos_types::valoper_address_to_bech32(&[2; 20]);
        keeper
            .set_validator(&validator)
            .expect("set second validator");

        let mut invalid = validator.clone();
        invalid.tokens = "01".to_owned();
        assert!(matches!(
            keeper.set_validator(&invalid),
            Err(Error::InvalidValidator)
        ));
        invalid.tokens = "1".to_owned();
        invalid.min_self_delegation = "0".to_owned();
        assert!(matches!(
            keeper.set_validator(&invalid),
            Err(Error::InvalidValidator)
        ));
    }
    store
        .set(
            [vec![0x21, 20], address.to_vec()].concat(),
            validator.encode_to_vec(),
        )
        .expect("seed mismatched validator");
    assert!(matches!(
        Keeper::new(&mut store).validator(&address),
        Err(Error::InvalidValidator)
    ));
    store
        .set([vec![0x21, 20], address.to_vec()].concat(), vec![0xff])
        .expect("seed malformed validator");
    assert!(matches!(
        Keeper::new(&mut store).validator(&address),
        Err(Error::InvalidValidator)
    ));

    let mut keeper = Keeper::new(&mut store);
    keeper.remove_validator(&address).expect("remove validator");
    assert_eq!(keeper.validator(&address).expect("removed"), None);
}

#[test]
fn delegation_crud_updates_both_indexes_and_validates_shares() {
    let delegator = [3; 20];
    let validator_address = [4; 20];
    let delegation_record = delegation(delegator, validator_address, "1250000000000000000");
    let mut store = kvstore::Store::new();
    let mut keeper = Keeper::new(&mut store);
    keeper
        .set_delegation(&delegation_record)
        .expect("set delegation");
    assert_eq!(
        keeper
            .delegation(&delegator, &validator_address)
            .expect("delegation"),
        Some(delegation_record.clone())
    );
    assert_eq!(
        keeper
            .validator_delegations(&validator_address)
            .expect("by validator"),
        vec![delegation_record.clone()]
    );

    let invalid = delegation(delegator, validator_address, "01");
    assert!(matches!(
        keeper.set_delegation(&invalid),
        Err(Error::InvalidShares)
    ));
    let zero = delegation(delegator, validator_address, "0");
    assert!(matches!(
        keeper.set_delegation(&zero),
        Err(Error::InvalidShares)
    ));
    keeper
        .remove_delegation(&delegator, &validator_address)
        .expect("remove delegation");
    assert_eq!(
        keeper
            .delegation(&delegator, &validator_address)
            .expect("removed"),
        None
    );
    assert!(
        keeper
            .validator_delegations(&validator_address)
            .expect("empty index")
            .is_empty()
    );
}

#[test]
fn indexed_missing_and_malformed_delegations_are_typed_errors() {
    let delegator = [5; 20];
    let validator_address = [6; 20];
    let primary = [
        vec![0x31, 20],
        delegator.to_vec(),
        vec![20],
        validator_address.to_vec(),
    ]
    .concat();
    let index = [
        vec![0x71, 20],
        validator_address.to_vec(),
        delegator.to_vec(),
    ]
    .concat();
    let mut store = kvstore::Store::new();
    store.set(index, Vec::new()).expect("seed orphan index");
    assert!(matches!(
        Keeper::new(&mut store).validator_delegations(&validator_address),
        Err(Error::MissingDelegation)
    ));
    store
        .set(primary, vec![0xff])
        .expect("seed malformed delegation");
    assert!(matches!(
        Keeper::new(&mut store).delegation(&delegator, &validator_address),
        Err(Error::InvalidDelegation)
    ));
}

#[test]
fn first_delegation_uses_not_bonded_pool_and_creates_shares() {
    let delegator = [7; 20];
    let validator_address = [8; 20];
    let mut validator = validator(validator_address);
    validator.status = 1;
    validator.tokens = "0".to_owned();
    validator.delegator_shares = "0".to_owned();
    let mut staking_store = kvstore::Store::new();
    Keeper::new(&mut staking_store)
        .set_validator(&validator)
        .expect("set validator");
    let mut bank_store = kvstore::Store::new();
    bank::Keeper::new(&mut bank_store)
        .set_balance(&delegator, &coin("utia", "12"))
        .expect("seed balance");

    let result = execute_delegate(
        &mut staking_store,
        &mut bank_store,
        delegator,
        validator_address,
        Some(proto_coin("utia", "7")),
    )
    .expect("delegate");
    assert_eq!(result.events[2].ty, "delegate");
    assert_eq!(result.events[2].attributes[3].value, "7.000000000000000000");

    let staking = Keeper::new(&mut staking_store);
    let updated = staking
        .validator(&validator_address)
        .expect("validator")
        .expect("present");
    assert_eq!(updated.tokens, "7");
    assert_eq!(updated.delegator_shares, "7000000000000000000");
    assert_eq!(
        staking
            .delegation(&delegator, &validator_address)
            .expect("delegation")
            .expect("present")
            .shares,
        "7000000000000000000"
    );
    assert_eq!(
        staking
            .validator_by_power_index(&updated)
            .expect("power index"),
        Some(validator_address)
    );
    let pool = cosmos_types::module_address(NOT_BONDED_POOL_NAME);
    let bank = bank::Keeper::new(&mut bank_store);
    assert_eq!(
        bank.balance(&delegator, "utia").expect("sender").amount,
        "5"
    );
    assert_eq!(bank.balance(&pool, "utia").expect("pool").amount, "7");
}

#[test]
fn power_index_skips_jailed_validators_and_can_be_removed() {
    let address = [9; 20];
    let mut validator = validator(address);
    let mut store = kvstore::Store::new();
    let mut keeper = Keeper::new(&mut store);
    keeper
        .set_validator_by_power_index(&validator)
        .expect("set power index");
    assert_eq!(
        keeper
            .validator_by_power_index(&validator)
            .expect("power index"),
        Some(address)
    );
    keeper
        .remove_validator_by_power_index(&validator)
        .expect("remove power index");
    assert_eq!(
        keeper
            .validator_by_power_index(&validator)
            .expect("removed"),
        None
    );
    validator.jailed = true;
    keeper
        .set_validator_by_power_index(&validator)
        .expect("jailed is omitted");
    assert_eq!(
        keeper
            .validator_by_power_index(&validator)
            .expect("jailed absent"),
        None
    );
}

#[test]
fn delegation_rejects_invalid_messages_without_moving_funds() {
    let delegator = [10; 20];
    let validator_address = [11; 20];
    let mut staking_store = kvstore::Store::new();
    let mut bank_store = kvstore::Store::new();
    bank::Keeper::new(&mut bank_store)
        .set_balance(&delegator, &coin("utia", "10"))
        .expect("seed balance");
    {
        let mut staking = Keeper::new(&mut staking_store);
        assert!(matches!(
            staking.execute_message(
                &mut bank::Keeper::new(&mut bank_store),
                &Any {
                    type_url: "/unsupported".to_owned(),
                    value: Vec::new(),
                }
            ),
            Err(Error::UnsupportedMessage(_))
        ));
        assert!(matches!(
            staking.execute_message(
                &mut bank::Keeper::new(&mut bank_store),
                &Any {
                    type_url: MSG_DELEGATE_TYPE_URL.to_owned(),
                    value: vec![0xff],
                }
            ),
            Err(Error::MalformedMessage)
        ));
    }

    assert!(matches!(
        execute_delegate(
            &mut staking_store,
            &mut bank_store,
            delegator,
            validator_address,
            Some(proto_coin("utia", "1")),
        ),
        Err(Error::ValidatorNotFound)
    ));
    let mut invalid_exchange = validator(validator_address);
    invalid_exchange.status = 3;
    invalid_exchange.tokens = "0".to_owned();
    invalid_exchange.delegator_shares = "1".to_owned();
    Keeper::new(&mut staking_store)
        .set_validator(&invalid_exchange)
        .expect("seed invalid exchange rate");
    assert!(matches!(
        execute_delegate(
            &mut staking_store,
            &mut bank_store,
            delegator,
            validator_address,
            Some(proto_coin("utia", "1")),
        ),
        Err(Error::InvalidExchangeRate)
    ));

    let mut valid = validator(validator_address);
    valid.status = 0;
    Keeper::new(&mut staking_store)
        .set_validator(&valid)
        .expect("seed invalid status");
    assert!(matches!(
        execute_delegate(
            &mut staking_store,
            &mut bank_store,
            delegator,
            validator_address,
            Some(proto_coin("utia", "1")),
        ),
        Err(Error::InvalidValidatorStatus)
    ));
    valid.status = 3;
    Keeper::new(&mut staking_store)
        .set_validator(&valid)
        .expect("seed bonded validator");
    assert!(matches!(
        execute_delegate(
            &mut staking_store,
            &mut bank_store,
            delegator,
            validator_address,
            Some(proto_coin("uatom", "1")),
        ),
        Err(Error::InvalidBondDenom)
    ));
    assert!(matches!(
        execute_delegate(
            &mut staking_store,
            &mut bank_store,
            delegator,
            validator_address,
            None,
        ),
        Err(Error::InvalidCoin)
    ));
    assert!(matches!(
        execute_delegate(
            &mut staking_store,
            &mut bank_store,
            delegator,
            validator_address,
            Some(proto_coin("utia", "11")),
        ),
        Err(Error::Bank(bank::Error::InsufficientFunds))
    ));
    assert_eq!(
        bank::Keeper::new(&mut bank_store)
            .balance(&delegator, "utia")
            .expect("unchanged")
            .amount,
        "10"
    );
    assert_eq!(
        bank::Keeper::new(&mut bank_store)
            .balance(&cosmos_types::module_address(BONDED_POOL_NAME), "utia")
            .expect("empty bonded pool")
            .amount,
        "0"
    );
}

fn execute_delegate(
    staking_store: &mut kvstore::Store,
    bank_store: &mut kvstore::Store,
    delegator: cosmos_types::AccountAddress,
    validator: cosmos_types::ValoperAddress,
    amount: Option<proto::base::Coin>,
) -> staking::Result<staking::MessageResult> {
    let message = MsgDelegate {
        delegator_address: cosmos_types::account_address_to_bech32(&delegator),
        validator_address: cosmos_types::valoper_address_to_bech32(&validator),
        amount,
    };
    Keeper::new(staking_store).execute_message(
        &mut bank::Keeper::new(bank_store),
        &Any {
            type_url: MSG_DELEGATE_TYPE_URL.to_owned(),
            value: message.encode_to_vec(),
        },
    )
}

fn coin(denom: &str, amount: &str) -> Coin {
    Coin::new(denom, amount).expect("valid coin")
}

fn proto_coin(denom: &str, amount: &str) -> proto::base::Coin {
    proto::base::Coin {
        denom: denom.to_owned(),
        amount: amount.to_owned(),
    }
}

fn validator(address: cosmos_types::ValoperAddress) -> Validator {
    Validator {
        operator_address: cosmos_types::valoper_address_to_bech32(&address),
        tokens: "100".to_owned(),
        delegator_shares: "100000000000000000000".to_owned(),
        min_self_delegation: "1".to_owned(),
        ..Validator::default()
    }
}

fn delegation(
    delegator: cosmos_types::AccountAddress,
    validator: cosmos_types::ValoperAddress,
    shares: &str,
) -> Delegation {
    Delegation {
        delegator_address: cosmos_types::account_address_to_bech32(&delegator),
        validator_address: cosmos_types::valoper_address_to_bech32(&validator),
        shares: shares.to_owned(),
    }
}
