//! Warp message handler tests against an in-memory bank.

#![allow(clippy::unwrap_used, clippy::too_many_lines)]

use celestia_proto::Any;
use celestia_proto::hyperlane::warp::v1 as pb;
use math::Int;
use prost::Message;
use sdk::{Context, ExecMode, Header, Kind, MultiStore, Result};
use x_hyperlane::testing::FakeBank;
use x_hyperlane::util::acc_address_string;
use x_hyperlane::{Deps, HexAddress, HyperlaneMessage, core, ism, module_address, post_dispatch};
use x_warp::{Warp, WarpPayload};

fn store() -> MultiStore {
    let mut ms = MultiStore::new();
    ms.mount(x_hyperlane::STORE_KEY, Kind::Iavl).unwrap();
    ms.mount(x_warp::STORE_KEY, Kind::Iavl).unwrap();
    ms
}

const SENDER: [u8; 20] = [1; 20];
const REMOTE: HexAddress = HexAddress([0xee; 32]);

fn owner() -> String {
    acc_address_string(&SENDER)
}

fn tx<T>(c: &mut Context<'_>, f: impl FnOnce(&mut Context<'_>) -> Result<T>) -> Result<T> {
    let cp = c.branch();
    let r = f(c);
    if r.is_ok() {
        c.write(cp);
    } else {
        c.discard(cp);
    }
    r
}

fn exec<M: prost::Name>(warp: &Warp, c: &mut Context<'_>, deps: &Deps<'_>, m: &M) -> Result<Any> {
    tx(c, |c| warp.handle_msg(c, deps, &Any::pack(m)).unwrap())
}

fn token_id(a: &Any) -> HexAddress {
    let r = pb::MsgCreateCollateralTokenResponse::decode(a.value.as_slice()).unwrap();
    HexAddress::decode(&r.id).unwrap()
}

fn setup(c: &mut Context<'_>, deps: &Deps<'_>) -> HexAddress {
    let noop_ism = ism::create_noop_ism(c, deps, &owner()).unwrap();
    let hook = post_dispatch::create_noop_hook(c, deps, &owner()).unwrap();
    core::create_mailbox(c, deps, &owner(), 7, noop_ism, Some(hook), Some(hook)).unwrap()
}

fn incoming(mailbox: HexAddress, token: HexAddress, nonce: u32, to: &[u8], amount: u64) -> Vec<u8> {
    let mut recipient = vec![0u8; 12];
    recipient.extend_from_slice(to);
    let body = WarpPayload::new(&recipient, amount.into()).unwrap().bytes();
    let _ = mailbox;
    HyperlaneMessage {
        version: 3,
        nonce,
        origin: 1,
        sender: REMOTE,
        destination: 7,
        recipient: token,
        body,
    }
    .bytes()
}

fn enroll(warp: &Warp, c: &mut Context<'_>, deps: &Deps<'_>, token: HexAddress) {
    exec(
        warp,
        c,
        deps,
        &pb::MsgEnrollRemoteRouter {
            owner: owner(),
            token_id: token.to_string(),
            remote_router: Some(pb::RemoteRouter {
                receiver_domain: 1,
                receiver_contract: REMOTE.to_string(),
                gas: "100".into(),
            }),
        },
    )
    .unwrap();
}

fn transfer(
    warp: &Warp,
    c: &mut Context<'_>,
    deps: &Deps<'_>,
    token: HexAddress,
    amount: &str,
) -> Result<Any> {
    exec(
        warp,
        c,
        deps,
        &pb::MsgRemoteTransfer {
            sender: owner(),
            token_id: token.to_string(),
            destination_domain: 1,
            recipient: HexAddress([0x22; 32]).to_string(),
            amount: amount.into(),
            custom_hook_id: String::new(),
            gas_limit: "0".into(),
            max_fee: Some(celestia_proto::cosmos::base::v1beta1::Coin {
                denom: "utia".into(),
                amount: "0".into(),
            }),
            custom_hook_metadata: String::new(),
        },
    )
}

#[test]
fn genesis_params_key_and_export() {
    let mut ms = store();
    let mut c = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
    let json = serde_json::json!({"params": {}, "tokens": [], "remote_routers": []});
    x_warp::init_genesis_json(&mut c, &json).unwrap();
    assert_eq!(c.get(x_warp::STORE_KEY, &[0]).unwrap(), Some(vec![]));
    let gs = x_warp::export_genesis(&mut c).unwrap();
    assert!(gs.tokens.is_empty() && gs.params.is_some());
}

#[test]
fn collateral_round_trip() {
    let bank = FakeBank::default();
    let warp = Warp::celestia();
    let mut deps = Deps::new(&bank);
    warp.register(&mut deps).unwrap();
    let mut ms = store();
    let mut c = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
    let mailbox = setup(&mut c, &deps);

    let e = exec(
        &warp,
        &mut c,
        &deps,
        &pb::MsgCreateCollateralToken {
            owner: owner(),
            origin_mailbox: mailbox.to_string(),
            origin_denom: "x".into(),
        },
    )
    .unwrap_err();
    assert_eq!((e.codespace, e.code), ("undefined", 1));
    let token = token_id(
        &exec(
            &warp,
            &mut c,
            &deps,
            &pb::MsgCreateCollateralToken {
                owner: owner(),
                origin_mailbox: mailbox.to_string(),
                origin_denom: "utia".into(),
            },
        )
        .unwrap(),
    );
    assert_eq!((token.module_type(), token.internal_id()), (1, 0));

    // No router yet.
    bank.fund(&SENDER, "utia", 1_000);
    let e = transfer(&warp, &mut c, &deps, token, "500").unwrap_err();
    assert!(
        e.log
            .contains("no enrolled router found for destination domain 1")
    );
    // The fake bank lives outside the store, so the discarded tx still moved
    // the coins; put them back.
    bank.fund(&SENDER, "utia", 500);
    enroll(&warp, &mut c, &deps, token);

    c.events.take();
    transfer(&warp, &mut c, &deps, token, "500").unwrap();
    assert_eq!(bank.balance(&SENDER, "utia"), Int::from(500_i64));
    // 500 escrowed now plus 500 from the discarded attempt above.
    assert_eq!(
        bank.balance(&module_address("warp"), "utia"),
        Int::from(1_000_i64)
    );
    let t = x_warp::get_token(&mut c, 0).unwrap().unwrap();
    assert_eq!(t.collateral_balance, Int::from(500_i64));
    assert_eq!(core::get_mailbox(&mut c, &mailbox).unwrap().message_sent, 1);
    let types: Vec<String> = c.events.take().into_iter().map(|e| e.r#type).collect();
    assert_eq!(
        types,
        [
            "hyperlane.warp.v1.EventSendRemoteTransfer",
            "hyperlane.core.v1.EventDispatch"
        ]
    );

    // Insufficient balance: bank error propagates, nothing changes.
    let e = transfer(&warp, &mut c, &deps, token, "501").unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 5));

    // Incoming transfer releases collateral.
    let to = [7u8; 20];
    let raw = incoming(mailbox, token, 0, &to, 200);
    tx(&mut c, |c| {
        core::process_message(c, &deps, mailbox, &raw, &[])
    })
    .unwrap();
    assert_eq!(bank.balance(&to, "utia"), Int::from(200_i64));
    let t = x_warp::get_token(&mut c, 0).unwrap().unwrap();
    assert_eq!(t.collateral_balance, Int::from(300_i64));
    let ev = c.events.take();
    let recv = ev
        .iter()
        .find(|e| e.r#type == "hyperlane.warp.v1.EventReceiveRemoteTransfer")
        .unwrap();
    assert_eq!(recv.attributes[0].key, "amount");
    assert_eq!(recv.attributes[0].value, "\"200utia\"");

    let raw = incoming(mailbox, token, 1, &to, 301);
    let e = tx(&mut c, |c| {
        core::process_message(c, &deps, mailbox, &raw, &[])
    })
    .unwrap_err();
    assert_eq!((e.codespace, e.code), ("warp", 1));

    // Unknown origin router.
    let mut m = HyperlaneMessage::parse(&incoming(mailbox, token, 2, &to, 1)).unwrap();
    m.origin = 5;
    let e = tx(&mut c, |c| {
        core::process_message(c, &deps, mailbox, &m.bytes(), &[])
    })
    .unwrap_err();
    assert!(e.log.contains("no enrolled router"));

    // Ownership / ISM updates.
    let other = acc_address_string(&[2; 20]);
    assert!(
        exec(
            &warp,
            &mut c,
            &deps,
            &pb::MsgSetToken {
                owner: other.clone(),
                token_id: token.to_string(),
                new_owner: owner(),
                ..Default::default()
            }
        )
        .is_err()
    );
    let e = exec(
        &warp,
        &mut c,
        &deps,
        &pb::MsgSetToken {
            owner: owner(),
            token_id: token.to_string(),
            ism_id: HexAddress::generate(&[0; 20], 0, 42).to_string(),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(e.log.contains("does not exist"));
    exec(
        &warp,
        &mut c,
        &deps,
        &pb::MsgUnrollRemoteRouter {
            owner: owner(),
            token_id: token.to_string(),
            receiver_domain: 1,
        },
    )
    .unwrap();
    assert!(!x_warp::has_enrolled_router(&mut c, 0, 1).unwrap());
    assert!(
        exec(
            &warp,
            &mut c,
            &deps,
            &pb::MsgUnrollRemoteRouter {
                owner: owner(),
                token_id: token.to_string(),
                receiver_domain: 1,
            }
        )
        .is_err()
    );
    exec(
        &warp,
        &mut c,
        &deps,
        &pb::MsgSetToken {
            owner: owner(),
            token_id: token.to_string(),
            new_owner: other.clone(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(x_warp::get_token(&mut c, 0).unwrap().unwrap().owner, other);
}

#[test]
fn synthetic_mint_and_burn() {
    let bank = FakeBank::default();
    let warp = Warp::celestia();
    let mut deps = Deps::new(&bank);
    warp.register(&mut deps).unwrap();
    let mut ms = store();
    let mut c = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
    let mailbox = setup(&mut c, &deps);
    x_warp::init_genesis(&mut c, &pb::GenesisState::default()).unwrap();

    let token = token_id(
        &exec(
            &warp,
            &mut c,
            &deps,
            &pb::MsgCreateSyntheticToken {
                owner: owner(),
                origin_mailbox: mailbox.to_string(),
            },
        )
        .unwrap(),
    );
    assert_eq!(token.module_type(), 2);
    let denom = format!("hyperlane/{token}");
    enroll(&warp, &mut c, &deps, token);

    let raw = incoming(mailbox, token, 0, &SENDER, 50);
    tx(&mut c, |c| {
        core::process_message(c, &deps, mailbox, &raw, &[])
    })
    .unwrap();
    assert_eq!(bank.balance(&SENDER, &denom), Int::from(50_i64));
    assert_eq!(bank.supply(&denom), Int::from(50_i64));

    let r = transfer(&warp, &mut c, &deps, token, "20").unwrap();
    let id = pb::MsgRemoteTransferResponse::decode(r.value.as_slice())
        .unwrap()
        .message_id;
    assert_eq!(id.len(), 66);
    assert_eq!(bank.balance(&SENDER, &denom), Int::from(30_i64));
    assert_eq!(bank.supply(&denom), Int::from(30_i64));

    // Native synthetic tokens are authority-gated.
    let e = exec(
        &warp,
        &mut c,
        &deps,
        &pb::MsgCreateNativeSyntheticToken {
            owner: owner(),
            origin_mailbox: mailbox.to_string(),
            origin_denom: "unative".into(),
        },
    )
    .unwrap_err();
    assert!(e.log.contains("invalid authority"));
    let e = exec(
        &warp,
        &mut c,
        &deps,
        &pb::MsgCreateNativeSyntheticToken {
            owner: warp.authority.clone(),
            origin_mailbox: mailbox.to_string(),
            origin_denom: denom.clone(),
        },
    )
    .unwrap_err();
    assert!(e.log.contains("already exists"));
    exec(
        &warp,
        &mut c,
        &deps,
        &pb::MsgCreateNativeSyntheticToken {
            owner: warp.authority.clone(),
            origin_mailbox: mailbox.to_string(),
            origin_denom: "unative".into(),
        },
    )
    .unwrap();
    assert_eq!(
        warp.authority,
        "celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7"
    );

    let gs = x_warp::export_genesis(&mut c).unwrap();
    assert_eq!((gs.tokens.len(), gs.remote_routers.len()), (2, 1));
}

#[test]
fn payload_encoding() {
    let p = WarpPayload::new(&[0u8; 32], 1000u64.into()).unwrap();
    let b = p.bytes();
    assert_eq!(
        hex::encode(&b),
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000003e8"
    );
    assert_eq!(
        WarpPayload::parse(&b).unwrap().cosmos_account(),
        vec![0u8; 20]
    );
    assert!(WarpPayload::parse(&b[..63]).is_err());
}
