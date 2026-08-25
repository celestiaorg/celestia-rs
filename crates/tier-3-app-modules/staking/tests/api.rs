//! Public API and validation tests for the staking keeper.

#![allow(clippy::expect_used)]

use cosmos_types::proto::staking::{Delegation, Validator};
use prost::Message;
use staking::{Error, Keeper};

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
