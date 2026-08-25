//! Public API and malformed-state tests for the auth keeper.

#![allow(clippy::expect_used)]

use ante::{AccountReader, AccountWriter};
use auth::{BaseAccount, Error, Keeper};
use cosmos_types::Any;

#[test]
fn creates_updates_lists_and_removes_accounts() {
    let mut store = kvstore::Store::new();
    let first_address = [1; 20];
    let second_address = [2; 20];
    let public_key = Any {
        type_url: "/cosmos.crypto.secp256k1.PubKey".to_owned(),
        value: vec![10, 33, 2],
    };

    let mut keeper = Keeper::new(&mut store);
    assert_eq!(
        keeper.peek_next_account_number().expect("empty sequence"),
        0
    );
    assert_eq!(
        keeper
            .create_account(second_address)
            .expect("second account"),
        BaseAccount::new(second_address, 0)
    );
    assert_eq!(
        keeper.create_account(first_address).expect("first account"),
        BaseAccount::new(first_address, 1)
    );
    assert!(matches!(
        keeper.create_account(first_address),
        Err(Error::AccountAlreadyExists)
    ));

    keeper
        .set_public_key(&first_address, public_key.clone())
        .expect("set public key through ante trait");
    keeper
        .set_sequence(&first_address, 9)
        .expect("set sequence through ante trait");
    assert_eq!(
        keeper.account(&first_address).expect("ante account"),
        Some(ante::Account {
            address: first_address,
            account_number: 1,
            sequence: 9,
            public_key: Some(public_key),
        })
    );
    assert_eq!(
        keeper
            .address_by_account_number(1)
            .expect("account number index"),
        Some(first_address)
    );
    assert_eq!(
        keeper
            .all_accounts()
            .expect("ordered accounts")
            .into_iter()
            .map(|account| account.address)
            .collect::<Vec<_>>(),
        vec![first_address, second_address]
    );

    assert!(
        keeper
            .remove_account(&first_address)
            .expect("remove account")
    );
    assert_eq!(
        keeper
            .address_by_account_number(1)
            .expect("removed account number index"),
        None
    );
    assert!(
        !keeper
            .remove_account(&first_address)
            .expect("remove missing")
    );
    assert!(!keeper.has_account(&first_address).expect("missing account"));
}

#[test]
fn ante_updates_report_missing_accounts_as_state_errors() {
    let mut store = kvstore::Store::new();
    let mut keeper = Keeper::new(&mut store);
    let error = keeper
        .set_sequence(&[9; 20], 1)
        .expect_err("missing account must fail");
    assert_eq!(error, ante::Error::State("account not found".to_owned()));
}

#[test]
fn rejects_malformed_account_state() {
    let address = [3; 20];
    let key = [vec![1], address.to_vec()].concat();

    let mut malformed_store = kvstore::Store::new();
    malformed_store
        .set(key.clone(), vec![0xff])
        .expect("seed malformed bytes");
    assert!(matches!(
        Keeper::new(&mut malformed_store).get_account(&address),
        Err(Error::MalformedAccount)
    ));

    let mut unsupported_store = kvstore::Store::new();
    unsupported_store
        .set(
            key,
            prost::Message::encode_to_vec(&cosmos_types::proto::google::Any {
                type_url: "/cosmos.auth.v1beta1.ModuleAccount".to_owned(),
                value: Vec::new(),
            }),
        )
        .expect("seed unsupported account");
    assert!(matches!(
        Keeper::new(&mut unsupported_store).get_account(&address),
        Err(Error::UnsupportedAccountType(type_url))
            if type_url == "/cosmos.auth.v1beta1.ModuleAccount"
    ));
}

#[test]
fn rejects_address_mismatch_and_malformed_global_number() {
    let address = [4; 20];
    let other = [5; 20];
    let key = [vec![1], address.to_vec()].concat();
    let mut store = kvstore::Store::new();
    let proto = cosmos_types::proto::auth::BaseAccount {
        address: cosmos_types::account_address_to_bech32(&other),
        pub_key: None,
        account_number: 0,
        sequence: 0,
    };
    store
        .set(
            key,
            prost::Message::encode_to_vec(&cosmos_types::proto::google::Any {
                type_url: auth::BASE_ACCOUNT_TYPE_URL.to_owned(),
                value: prost::Message::encode_to_vec(&proto),
            }),
        )
        .expect("seed mismatched account");
    assert!(matches!(
        Keeper::new(&mut store).get_account(&address),
        Err(Error::AddressMismatch)
    ));

    store
        .set(vec![2], vec![0, 1])
        .expect("seed malformed sequence");
    assert!(matches!(
        Keeper::new(&mut store).peek_next_account_number(),
        Err(Error::MalformedAccountNumber)
    ));
}

#[test]
fn rejects_duplicate_and_malformed_account_number_indexes() {
    let first = [6; 20];
    let second = [7; 20];
    let mut store = kvstore::Store::new();
    let mut keeper = Keeper::new(&mut store);
    keeper
        .set_account(&BaseAccount::new(first, 5))
        .expect("seed account");
    assert!(matches!(
        keeper.set_account(&BaseAccount::new(second, 5)),
        Err(Error::AccountNumberAlreadyExists)
    ));

    let malformed_index_key = [b"accountNumber".as_slice(), &9_u64.to_be_bytes()].concat();
    store
        .set(malformed_index_key, vec![1, 2, 3])
        .expect("seed malformed index");
    assert!(matches!(
        Keeper::new(&mut store).address_by_account_number(9),
        Err(Error::MalformedAccountNumberIndex)
    ));
}
