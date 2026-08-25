//! Public API and state-transition tests for the bank keeper.

#![allow(clippy::expect_used)]

use ante::FeeKeeper;
use bank::{Error, Keeper, MSG_MULTI_SEND_TYPE_URL, MSG_SEND_TYPE_URL, SendOutput};
use cosmos_types::{Any, Coin, proto};
use prost::Message;

#[test]
fn balances_are_canonical_ordered_and_zero_is_deleted() {
    let address = [1; 20];
    let mut store = kvstore::Store::new();
    let mut keeper = Keeper::new(&mut store);
    assert_eq!(
        keeper
            .balance(&address, "utia")
            .expect("zero balance")
            .amount,
        "0"
    );
    keeper
        .set_balance(&address, &coin("utia", "00042"))
        .expect("set canonical balance");
    keeper
        .set_balance(&address, &coin("uatom", "7"))
        .expect("set second balance");
    assert_eq!(
        keeper
            .all_balances(&address)
            .expect("all balances")
            .into_iter()
            .map(|value| (value.denom, value.amount))
            .collect::<Vec<_>>(),
        vec![
            ("uatom".to_owned(), "7".to_owned()),
            ("utia".to_owned(), "42".to_owned())
        ]
    );
    keeper
        .set_balance(&address, &coin("utia", "0"))
        .expect("delete zero balance");
    assert_eq!(
        keeper
            .balance(&address, "utia")
            .expect("deleted balance")
            .amount,
        "0"
    );
}

#[test]
fn send_is_atomic_and_preserves_supply() {
    let from = [2; 20];
    let to = [3; 20];
    let mut store = kvstore::Store::new();
    let mut keeper = Keeper::new(&mut store);
    keeper
        .set_balance(&from, &coin("uatom", "10"))
        .expect("uatom");
    keeper
        .set_balance(&from, &coin("utia", "20"))
        .expect("utia");
    keeper
        .set_supply(&coin("uatom", "10"))
        .expect("uatom supply");
    keeper.set_supply(&coin("utia", "20")).expect("utia supply");

    keeper
        .send_coins(&from, &from, &[coin("uatom", "3")])
        .expect("send to self");
    assert_eq!(
        keeper.balance(&from, "uatom").expect("unchanged").amount,
        "10"
    );
    assert!(matches!(
        keeper.send_coins(&from, &from, &[coin("uatom", "11")]),
        Err(Error::InsufficientFunds)
    ));

    assert!(matches!(
        keeper.send_coins(&from, &to, &[coin("uatom", "5"), coin("utia", "21")]),
        Err(Error::InsufficientFunds)
    ));
    assert_eq!(
        keeper.balance(&from, "uatom").expect("unchanged").amount,
        "10"
    );
    keeper
        .send_coins(&from, &to, &[coin("uatom", "5"), coin("utia", "20")])
        .expect("send coins");
    assert_eq!(keeper.balance(&from, "uatom").expect("debit").amount, "5");
    assert_eq!(
        keeper.balance(&from, "utia").expect("zero debit").amount,
        "0"
    );
    assert_eq!(keeper.balance(&to, "uatom").expect("credit").amount, "5");
    assert_eq!(keeper.balance(&to, "utia").expect("credit").amount, "20");
    assert_eq!(keeper.supply("uatom").expect("supply").amount, "10");
    assert_eq!(keeper.supply("utia").expect("supply").amount, "20");
}

#[test]
fn mint_and_burn_update_balance_and_supply() {
    let address = [4; 20];
    let mut store = kvstore::Store::new();
    let mut keeper = Keeper::new(&mut store);
    keeper
        .mint_coins(&address, &[coin("utia", "100")])
        .expect("mint");
    assert_eq!(
        keeper.balance(&address, "utia").expect("balance").amount,
        "100"
    );
    assert_eq!(keeper.supply("utia").expect("supply").amount, "100");
    keeper
        .burn_coins(&address, &[coin("utia", "00040")])
        .expect("burn");
    assert_eq!(
        keeper.balance(&address, "utia").expect("balance").amount,
        "60"
    );
    assert_eq!(keeper.supply("utia").expect("supply").amount, "60");
    assert!(matches!(
        keeper.burn_coins(&address, &[coin("utia", "61")]),
        Err(Error::InsufficientFunds)
    ));
}

#[test]
fn multi_send_aggregates_recipients_and_preflights_totals() {
    let from = [8; 20];
    let recipient = [9; 20];
    let mut store = kvstore::Store::new();
    let mut keeper = Keeper::new(&mut store);
    keeper
        .set_balance(&from, &coin("utia", "100"))
        .expect("sender balance");
    keeper
        .multi_send(
            &from,
            &[coin("utia", "100")],
            &[
                SendOutput {
                    address: from,
                    coins: vec![coin("utia", "10")],
                },
                SendOutput {
                    address: recipient,
                    coins: vec![coin("utia", "30")],
                },
                SendOutput {
                    address: recipient,
                    coins: vec![coin("utia", "60")],
                },
            ],
        )
        .expect("multi-send");
    assert_eq!(keeper.balance(&from, "utia").expect("sender").amount, "10");
    assert_eq!(
        keeper
            .balance(&recipient, "utia")
            .expect("recipient")
            .amount,
        "90"
    );

    assert!(matches!(
        keeper.multi_send(
            &from,
            &[coin("utia", "10")],
            &[SendOutput {
                address: recipient,
                coins: vec![coin("utia", "11")],
            }],
        ),
        Err(Error::InvalidInputOutput)
    ));
    assert!(matches!(
        keeper.multi_send(
            &from,
            &[],
            &[SendOutput {
                address: recipient,
                coins: Vec::new(),
            }],
        ),
        Err(Error::InvalidInputOutput)
    ));
    assert_eq!(
        keeper.balance(&from, "utia").expect("unchanged").amount,
        "10"
    );
}

#[test]
fn message_router_rejects_unsupported_and_malformed_messages() {
    let address = cosmos_types::account_address_to_bech32(&[10; 20]);
    let mut store = kvstore::Store::new();
    let mut keeper = Keeper::new(&mut store);
    assert!(matches!(
        keeper.execute_message(&Any {
            type_url: "/test.Msg".to_owned(),
            value: Vec::new(),
        }),
        Err(Error::UnsupportedMessage(value)) if value == "/test.Msg"
    ));
    assert!(matches!(
        keeper.execute_message(&Any {
            type_url: MSG_SEND_TYPE_URL.to_owned(),
            value: vec![0xff],
        }),
        Err(Error::MalformedMessage)
    ));

    let empty_send = proto::bank::MsgSend {
        from_address: address.clone(),
        to_address: address.clone(),
        amount: Vec::new(),
    };
    assert!(matches!(
        keeper.execute_message(&Any {
            type_url: MSG_SEND_TYPE_URL.to_owned(),
            value: empty_send.encode_to_vec(),
        }),
        Err(Error::InvalidCoin)
    ));

    let invalid_address = proto::bank::MsgSend {
        from_address: "cosmos1invalid".to_owned(),
        to_address: address.clone(),
        amount: vec![proto_coin("utia", "1")],
    };
    assert!(matches!(
        keeper.execute_message(&Any {
            type_url: MSG_SEND_TYPE_URL.to_owned(),
            value: invalid_address.encode_to_vec(),
        }),
        Err(Error::InvalidAddress)
    ));

    let no_inputs = proto::bank::MsgMultiSend {
        inputs: Vec::new(),
        outputs: vec![proto::bank::Output {
            address: address.clone(),
            coins: vec![proto_coin("utia", "1")],
        }],
    };
    assert!(matches!(
        keeper.execute_message(&Any {
            type_url: MSG_MULTI_SEND_TYPE_URL.to_owned(),
            value: no_inputs.encode_to_vec(),
        }),
        Err(Error::InvalidInputOutput)
    ));

    let multiple_inputs = proto::bank::MsgMultiSend {
        inputs: vec![
            proto::bank::Input {
                address: address.clone(),
                coins: vec![proto_coin("utia", "1")],
            },
            proto::bank::Input {
                address: address.clone(),
                coins: vec![proto_coin("utia", "1")],
            },
        ],
        outputs: Vec::new(),
    };
    assert!(matches!(
        keeper.execute_message(&Any {
            type_url: MSG_MULTI_SEND_TYPE_URL.to_owned(),
            value: multiple_inputs.encode_to_vec(),
        }),
        Err(Error::InvalidInputOutput)
    ));
}

#[test]
fn fee_keeper_moves_fees_to_module_collector() {
    let payer = [5; 20];
    let collector = Keeper::fee_collector_address();
    let mut store = kvstore::Store::new();
    let mut keeper = Keeper::new(&mut store);
    keeper
        .set_balance(&payer, &coin("utia", "1000"))
        .expect("payer balance");
    keeper
        .deduct_fees(&payer, &[coin("utia", "250")])
        .expect("deduct fees");
    assert_eq!(keeper.balance(&payer, "utia").expect("payer").amount, "750");
    assert_eq!(
        keeper
            .balance(&collector, "utia")
            .expect("collector")
            .amount,
        "250"
    );
    assert_eq!(
        keeper
            .spendable_balance(&payer, "utia")
            .expect("spendable")
            .expect("coin")
            .amount,
        "750"
    );
}

#[test]
fn rejects_invalid_coin_sets_overflow_and_malformed_state() {
    let address = [6; 20];
    let mut store = kvstore::Store::new();
    let mut keeper = Keeper::new(&mut store);
    assert!(matches!(
        keeper.send_coins(&address, &[7; 20], &[coin("utia", "0")]),
        Err(Error::InvalidCoin)
    ));
    assert!(matches!(
        keeper.send_coins(&address, &[7; 20], &[coin("utia", "1"), coin("uatom", "1")]),
        Err(Error::InvalidCoin)
    ));
    let maximum = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
    keeper
        .set_balance(&address, &coin("umax", maximum))
        .expect("maximum balance");
    assert!(matches!(
        keeper.mint_coins(&address, &[coin("umax", "1")]),
        Err(Error::AmountOverflow)
    ));

    let malformed_key = [vec![2, 20], address.to_vec(), b"ubad".to_vec()].concat();
    store
        .set(malformed_key, b"01".to_vec())
        .expect("seed malformed balance");
    assert!(matches!(
        Keeper::new(&mut store).balance(&address, "ubad"),
        Err(Error::InvalidAmount)
    ));
}

fn coin(denom: &str, amount: &str) -> Coin {
    Coin::new(denom, amount).expect("valid test coin")
}

fn proto_coin(denom: &str, amount: &str) -> proto::base::Coin {
    proto::base::Coin {
        denom: denom.to_owned(),
        amount: amount.to_owned(),
    }
}
