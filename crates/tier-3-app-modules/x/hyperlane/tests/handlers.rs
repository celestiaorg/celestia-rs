//! Message handler tests against an in-memory bank.

#![allow(clippy::unwrap_used, clippy::too_many_lines)]

use std::cell::RefCell;

use celestia_proto::Any;
use celestia_proto::hyperlane::core::interchain_security::v1 as ismpb;
use celestia_proto::hyperlane::core::post_dispatch::v1 as pdpb;
use celestia_proto::hyperlane::core::v1 as pb;
use k256::ecdsa::SigningKey;
use math::Int;
use prost::Message;
use sdk::{Context, ExecMode, Header, Kind, MultiStore, Result};
use x_hyperlane::coins::{Coin, Coins};
use x_hyperlane::ism::{self, Route};
use x_hyperlane::testing::FakeBank;
use x_hyperlane::util::{acc_address_string, eth_signing_hash, keccak256};
use x_hyperlane::{
    Deps, HexAddress, HyperlaneApp, HyperlaneMessage, STORE_KEY, StandardHookMetadata, core,
    genesis, handle_msg, post_dispatch,
};

fn store() -> MultiStore {
    let mut ms = MultiStore::new();
    ms.mount(STORE_KEY, Kind::Iavl).unwrap();
    ms
}

fn ctx(ms: &mut MultiStore) -> Context<'_> {
    Context::new(ms, Header::default(), ExecMode::Finalize)
}

fn owner() -> String {
    acc_address_string(&[1u8; 20])
}

/// Test recipient app: records handled messages; ISM is the mailbox default.
#[derive(Default)]
struct RecordingApp {
    handled: RefCell<Vec<HyperlaneMessage>>,
}

impl HyperlaneApp for RecordingApp {
    fn exists(&self, _: &mut Context<'_>, _: &Deps<'_>, _: &HexAddress) -> Result<bool> {
        Ok(true)
    }
    fn handle(
        &self,
        _: &mut Context<'_>,
        _: &Deps<'_>,
        _: &HexAddress,
        message: &HyperlaneMessage,
    ) -> Result<()> {
        self.handled.borrow_mut().push(message.clone());
        Ok(())
    }
    fn receiver_ism_id(
        &self,
        _: &mut Context<'_>,
        _: &Deps<'_>,
        _: &HexAddress,
    ) -> Result<HexAddress> {
        Err(x_hyperlane::errors::core::no_receiver_ism(""))
    }
}

fn exec<M: prost::Name>(ctx: &mut Context<'_>, deps: &Deps<'_>, m: &M) -> Result<Any> {
    handle_msg(ctx, deps, &Any::pack(m)).unwrap()
}

fn id_of(a: &Any) -> HexAddress {
    // All create responses carry the id in field 1.
    let r = pb::MsgCreateMailboxResponse::decode(a.value.as_slice()).unwrap();
    HexAddress::decode(&r.id).unwrap()
}

#[test]
fn genesis_writes_sequences_and_roundtrips() {
    let mut ms = store();
    let mut c = ctx(&mut ms);
    let json = serde_json::json!({
        "ism_genesis": {"isms": [], "validator_storage_locations": []},
        "post_dispatch_genesis": {"igps": [], "igp_gas_configs": [], "merkle_tree_hooks": [], "noop_hooks": []},
        "mailboxes": [], "messages": [],
        "ism_sequence": "0", "post_dispatch_sequence": "0", "app_sequence": "0"
    });
    genesis::init_genesis_json(&mut c, &json).unwrap();
    for k in [[0u8, 2], [0, 4], [0, 5], [0, 6]] {
        assert_eq!(c.get(STORE_KEY, &k).unwrap(), Some(vec![0; 8]));
    }
    let exported = genesis::export_genesis(&mut c).unwrap();
    assert_eq!(exported.mailboxes.len(), 0);
    assert_eq!(exported.ism_sequence, 0);

    // Missing post-dispatch lists abort like Go's panic.
    let mut bad = json.clone();
    bad["post_dispatch_genesis"] = serde_json::json!({"igps": []});
    assert!(genesis::init_genesis_json(&mut c, &bad).is_err());
}

#[test]
fn dispatch_with_merkle_and_igp_hooks() {
    let bank = FakeBank::default();
    let app = RecordingApp::default();
    let mut deps = Deps::new(&bank);
    deps.apps.register(9, &app).unwrap();
    let mut ms = store();
    let mut c = ctx(&mut ms);
    let o = owner();

    let noop_ism = id_of(
        &exec(
            &mut c,
            &deps,
            &ismpb::MsgCreateNoopIsm { creator: o.clone() },
        )
        .unwrap(),
    );
    assert_eq!(noop_ism.module_type(), 0);
    let mailbox = id_of(
        &exec(
            &mut c,
            &deps,
            &pb::MsgCreateMailbox {
                owner: o.clone(),
                local_domain: 7,
                default_ism: noop_ism.to_string(),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    assert_eq!(mailbox.internal_id(), 0);
    let merkle = id_of(
        &exec(
            &mut c,
            &deps,
            &pdpb::MsgCreateMerkleTreeHook {
                owner: o.clone(),
                mailbox_id: mailbox.to_string(),
            },
        )
        .unwrap(),
    );
    assert_eq!((merkle.module_type(), merkle.internal_id()), (3, 0));
    let igp = id_of(
        &exec(
            &mut c,
            &deps,
            &pdpb::MsgCreateIgp {
                owner: o.clone(),
                denom: "utia".into(),
            },
        )
        .unwrap(),
    );
    assert_eq!((igp.module_type(), igp.internal_id()), (4, 1));
    exec(
        &mut c,
        &deps,
        &pdpb::MsgSetDestinationGasConfig {
            owner: o.clone(),
            igp_id: igp.to_string(),
            destination_gas_config: Some(pdpb::DestinationGasConfig {
                remote_domain: 1,
                gas_oracle: Some(pdpb::GasOracle {
                    token_exchange_rate: "10000000000".into(),
                    gas_price: "2".into(),
                }),
                gas_overhead: "100".into(),
            }),
        },
    )
    .unwrap();

    // Dispatch before hooks are set fails with hyperlane/2.
    let md = StandardHookMetadata {
        address: vec![1; 20],
        gas_limit: Int::from(50_i64),
        custom_hook_metadata: vec![],
    };
    let fee = Coins::new([Coin {
        denom: "utia".into(),
        amount: Int::from(1000_i64),
    }])
    .unwrap();
    let e = core::dispatch_message(
        &mut c,
        &deps,
        &mailbox,
        HexAddress([5; 32]),
        &fee,
        1,
        HexAddress([6; 32]),
        vec![],
        &md,
        None,
    )
    .unwrap_err();
    assert_eq!((e.codespace, e.code), ("hyperlane", 2));

    // Unknown hook ids are rejected.
    let e = exec(
        &mut c,
        &deps,
        &pb::MsgSetMailbox {
            owner: o.clone(),
            mailbox_id: mailbox.to_string(),
            required_hook: HexAddress::generate(&[0; 20], 3, 99).to_string(),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!((e.codespace, e.code), ("undefined", 1));

    exec(
        &mut c,
        &deps,
        &pb::MsgSetMailbox {
            owner: o.clone(),
            mailbox_id: mailbox.to_string(),
            required_hook: merkle.to_string(),
            default_hook: igp.to_string(),
            ..Default::default()
        },
    )
    .unwrap();

    bank.fund(&[1; 20], "utia", 10_000);
    c.events.take();
    let id = core::dispatch_message(
        &mut c,
        &deps,
        &mailbox,
        HexAddress([5; 32]),
        &fee,
        1,
        HexAddress([6; 32]),
        vec![0xaa],
        &md,
        None,
    )
    .unwrap();
    // (50 + 100) * 2 * 1e10 / 1e10 = 300.
    assert_eq!(bank.balance(&[1; 20], "utia"), Int::from(9_700_i64));
    let types: Vec<String> = c.events.take().into_iter().map(|e| e.r#type).collect();
    assert_eq!(
        types,
        [
            "hyperlane.core.v1.EventDispatch",
            "hyperlane.core.post_dispatch.v1.EventInsertedIntoTree",
            "hyperlane.core.post_dispatch.v1.EventGasPayment"
        ]
    );
    assert!(core::message_delivered(&mut c, 0, &id).unwrap());
    let mb = core::get_mailbox(&mut c, &mailbox).unwrap();
    assert_eq!(mb.message_sent, 1);
    let hook = post_dispatch::get_merkle_tree_hook(&mut c, 0)
        .unwrap()
        .unwrap();
    assert_eq!(hook.tree.unwrap().count, 1);
    let stored = post_dispatch::get_igp(&mut c, 1).unwrap().unwrap();
    assert_eq!(stored.claimable_fees.to_string(), "300utia");

    // Quote matches the charge.
    let q = core::quote_dispatch(
        &mut c,
        &deps,
        &mailbox,
        &HexAddress::default(),
        &md,
        &HyperlaneMessage {
            destination: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(q.to_string(), "300utia");

    // Max fee too low.
    let low = Coins::new([Coin {
        denom: "utia".into(),
        amount: Int::from(10_i64),
    }])
    .unwrap();
    assert!(
        core::dispatch_message(
            &mut c,
            &deps,
            &mailbox,
            HexAddress([5; 32]),
            &low,
            1,
            HexAddress([6; 32]),
            vec![],
            &md,
            None
        )
        .is_err()
    );

    // Claim by a non-owner fails, by the owner pays out.
    assert!(
        exec(
            &mut c,
            &deps,
            &pdpb::MsgClaim {
                sender: acc_address_string(&[2; 20]),
                igp_id: igp.to_string()
            }
        )
        .is_err()
    );
    c.events.take();
    exec(
        &mut c,
        &deps,
        &pdpb::MsgClaim {
            sender: o.clone(),
            igp_id: igp.to_string(),
        },
    )
    .unwrap();
    assert_eq!(bank.balance(&[1; 20], "utia"), Int::from(10_000_i64));
    let ev = c.events.take();
    assert_eq!(ev[0].attributes[0].key, "amount");
    assert_eq!(ev[0].attributes[0].value, "\"\"");
}

fn incoming(recipient: HexAddress, nonce: u32) -> HyperlaneMessage {
    HyperlaneMessage {
        version: 3,
        nonce,
        origin: 1,
        sender: HexAddress([3; 32]),
        destination: 7,
        recipient,
        body: vec![1],
    }
}

fn setup_mailbox(c: &mut Context<'_>, deps: &Deps<'_>, ism: HexAddress) -> HexAddress {
    core::create_mailbox(c, deps, &owner(), 7, ism, None, None).unwrap()
}

#[test]
fn process_with_noop_and_routing_isms() {
    let bank = FakeBank::default();
    let app = RecordingApp::default();
    let mut deps = Deps::new(&bank);
    deps.apps.register(9, &app).unwrap();
    let mut ms = store();
    let mut c = ctx(&mut ms);
    let noop = ism::create_noop_ism(&mut c, &deps, &owner()).unwrap();
    let mailbox = setup_mailbox(&mut c, &deps, noop);
    let recipient = HexAddress::generate(&[0; 20], 9, 1);

    let msg = incoming(recipient, 0);
    let hexmsg = format!("0x{}", hex::encode(msg.bytes()));
    let gas0 = c.gas.consumed();
    exec(
        &mut c,
        &deps,
        &pb::MsgProcessMessage {
            mailbox_id: mailbox.to_string(),
            relayer: owner(),
            metadata: String::new(),
            message: hexmsg.clone(),
        },
    )
    .unwrap();
    assert!(c.gas.consumed() - gas0 > 10_000);
    assert_eq!(app.handled.borrow().len(), 1);
    assert_eq!(
        core::get_mailbox(&mut c, &mailbox)
            .unwrap()
            .message_received,
        1
    );
    // Replay is rejected.
    let e = core::process_message(&mut c, &deps, mailbox, &msg.bytes(), &[]).unwrap_err();
    assert!(e.log.contains("already received"));
    // Wrong destination / version.
    let mut m2 = incoming(recipient, 1);
    m2.destination = 8;
    assert!(core::process_message(&mut c, &deps, mailbox, &m2.bytes(), &[]).is_err());
    let mut m3 = incoming(recipient, 2);
    m3.version = 1;
    assert!(core::process_message(&mut c, &deps, mailbox, &m3.bytes(), &[]).is_err());
    assert!(
        exec(
            &mut c,
            &deps,
            &pb::MsgProcessMessage {
                mailbox_id: mailbox.to_string(),
                relayer: owner(),
                metadata: String::new(),
                message: String::new(),
            }
        )
        .is_err()
    );

    // Routing ISM: domain 1 -> noop; unknown route ISM rejected with ism/7.
    let e = ism::create_routing_ism(
        &mut c,
        &deps,
        &owner(),
        vec![Route {
            ism: HexAddress::generate(&[0; 20], 0, 77),
            domain: 1,
        }],
    )
    .unwrap_err();
    assert_eq!((e.codespace, e.code), ("ism", 7));
    let e = ism::create_routing_ism(
        &mut c,
        &deps,
        &owner(),
        vec![
            Route {
                ism: noop,
                domain: 1,
            },
            Route {
                ism: noop,
                domain: 1,
            },
        ],
    )
    .unwrap_err();
    assert_eq!((e.codespace, e.code), ("ism", 11));
    let routing = ism::create_routing_ism(&mut c, &deps, &owner(), vec![]).unwrap();
    assert_eq!(routing.module_type(), 1);
    let m4 = incoming(recipient, 4);
    let e = core::verify(&mut c, &deps, &routing, &[], &m4).unwrap_err();
    assert_eq!((e.codespace, e.code), ("ism", 8));
    ism::set_routing_ism_domain(
        &mut c,
        &deps,
        &routing,
        &owner(),
        Route {
            ism: noop,
            domain: 1,
        },
    )
    .unwrap();
    assert!(core::verify(&mut c, &deps, &routing, &[], &m4).unwrap());
    let e = ism::set_routing_ism_domain(
        &mut c,
        &deps,
        &routing,
        &acc_address_string(&[9; 20]),
        Route {
            ism: noop,
            domain: 2,
        },
    )
    .unwrap_err();
    assert_eq!((e.codespace, e.code), ("ism", 9));
    ism::remove_routing_ism_domain(&mut c, &routing, &owner(), 1).unwrap();
    assert!(core::verify(&mut c, &deps, &routing, &[], &m4).is_err());
    let e = ism::update_routing_ism_owner(&mut c, &routing, &owner(), "", false).unwrap_err();
    assert_eq!((e.codespace, e.code), ("ism", 10));
    ism::update_routing_ism_owner(&mut c, &routing, &owner(), "", true).unwrap();
    // Noop ISM is not a routing ISM.
    let e = ism::remove_routing_ism_domain(&mut c, &noop, &owner(), 1).unwrap_err();
    assert_eq!((e.codespace, e.code), ("ism", 6));
}

/// Runs `f` like a transaction: state changes are discarded on error.
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

fn eth_address(sk: &SigningKey) -> [u8; 20] {
    let pk = sk.verifying_key().to_encoded_point(false);
    let h = keccak256(&[&pk.as_bytes()[1..]]);
    h[12..].try_into().unwrap()
}

fn sign(sk: &SigningKey, digest: &[u8; 32]) -> Vec<u8> {
    let (sig, rid) = sk.sign_prehash_recoverable(digest).unwrap();
    let mut v = sig.to_bytes().to_vec();
    v.push(rid.to_byte() + 27);
    v
}

#[test]
fn message_id_multisig_and_announce() {
    let bank = FakeBank::default();
    let app = RecordingApp::default();
    let mut deps = Deps::new(&bank);
    deps.apps.register(9, &app).unwrap();
    let mut ms = store();
    let mut c = ctx(&mut ms);

    let mut keys: Vec<SigningKey> = (1u8..=3)
        .map(|i| SigningKey::from_slice(&[i; 32]).unwrap())
        .collect();
    keys.sort_by_key(eth_address);
    let validators: Vec<String> = keys
        .iter()
        .map(|k| format!("0x{}", hex::encode(eth_address(k))))
        .collect();

    // Unsorted validators are rejected with ism/2.
    let mut unsorted = validators.clone();
    unsorted.reverse();
    let e = ism::create_multisig_ism(&mut c, &deps, false, &owner(), unsorted, 2).unwrap_err();
    assert_eq!((e.codespace, e.code), ("ism", 2));
    let multisig =
        ism::create_multisig_ism(&mut c, &deps, false, &owner(), validators.clone(), 2).unwrap();
    assert_eq!(multisig.module_type(), 5);
    // The failed creation consumed sequence 0.
    assert_eq!(multisig.internal_id(), 1);

    let mailbox = setup_mailbox(&mut c, &deps, multisig);
    let recipient = HexAddress::generate(&[0; 20], 9, 1);
    let msg = incoming(recipient, 0);
    let hook = [4u8; 32];
    let root = [8u8; 32];
    let digest = ism::checkpoint_digest(msg.origin, &hook, &root, 3, &msg.id().0);
    let mut metadata = Vec::new();
    metadata.extend_from_slice(&hook);
    metadata.extend_from_slice(&root);
    metadata.extend_from_slice(&3u32.to_be_bytes());
    let one_sig = {
        let mut m = metadata.clone();
        m.extend(sign(&keys[0], &digest));
        m
    };
    let e = tx(&mut c, |c| {
        core::process_message(c, &deps, mailbox, &msg.bytes(), &one_sig)
    })
    .unwrap_err();
    assert!(e.log.contains("threshold"));
    // Out-of-order signatures do not verify.
    let mut wrong = metadata.clone();
    wrong.extend(sign(&keys[2], &digest));
    wrong.extend(sign(&keys[0], &digest));
    let e = tx(&mut c, |c| {
        core::process_message(c, &deps, mailbox, &msg.bytes(), &wrong)
    })
    .unwrap_err();
    assert!(e.log.contains("ism verification failed"));
    metadata.extend(sign(&keys[0], &digest));
    metadata.extend(sign(&keys[2], &digest));
    core::process_message(&mut c, &deps, mailbox, &msg.bytes(), &metadata).unwrap();
    assert_eq!(app.handled.borrow().len(), 1);

    // Validator announcement.
    let location = "s3://bucket";
    let ad = ism::announcement_digest(location, 7, &mailbox.0);
    let sig = sign(&keys[1], &eth_signing_hash(&ad));
    let announce = |c: &mut Context<'_>, loc: &str, sig: &[u8]| {
        exec(
            c,
            &deps,
            &ismpb::MsgAnnounceValidator {
                validator: format!("0x{}", hex::encode(eth_address(&keys[1]))),
                storage_location: loc.into(),
                signature: format!("0x{}", hex::encode(sig)),
                mailbox_id: mailbox.to_string(),
                creator: owner(),
            },
        )
    };
    announce(&mut c, location, &sig).unwrap();
    let e = announce(&mut c, location, &sig).unwrap_err();
    assert_eq!((e.codespace, e.code), ("ism", 3));
    let e = announce(&mut c, "other", &sig).unwrap_err();
    assert_eq!((e.codespace, e.code), ("ism", 5));
    let ad2 = ism::announcement_digest("other", 7, &mailbox.0);
    announce(&mut c, "other", &sign(&keys[1], &eth_signing_hash(&ad2))).unwrap();
    assert_eq!(
        ism::storage_locations(&mut c, 0, &eth_address(&keys[1])).unwrap(),
        ["s3://bucket", "other"]
    );
    let exported = ism::export_genesis(&mut c).unwrap();
    assert_eq!(exported.validator_storage_locations.len(), 2);
    assert_eq!(exported.validator_storage_locations[1].index, 1);
}

#[test]
fn igp_owner_and_noop_hook() {
    let bank = FakeBank::default();
    let deps = Deps::new(&bank);
    let mut ms = store();
    let mut c = ctx(&mut ms);
    let igp = post_dispatch::create_igp(&mut c, &deps, &owner(), "utia").unwrap();
    assert!(post_dispatch::create_igp(&mut c, &deps, &owner(), "u").is_err());
    let e = post_dispatch::set_igp_owner(&mut c, &owner(), &igp, "bad", false).unwrap_err();
    assert_eq!((e.codespace, e.code), ("post_dispatch", 5));
    post_dispatch::set_igp_owner(&mut c, &owner(), &igp, &acc_address_string(&[2; 20]), false)
        .unwrap();
    let noop = post_dispatch::create_noop_hook(&mut c, &deps, &owner()).unwrap();
    assert!(core::post_dispatch_hook_exists(&mut c, &deps, &noop).unwrap());
    let missing = HexAddress::generate(&[0; 20], 0, 50);
    let e = core::post_dispatch_hook_exists(&mut c, &deps, &missing).unwrap_err();
    assert_eq!((e.codespace, e.code), ("post_dispatch", 3));
    let e = post_dispatch::create_merkle_tree_hook(&mut c, &deps, &owner(), &missing).unwrap_err();
    assert_eq!((e.codespace, e.code), ("post_dispatch", 1));
    let gs = post_dispatch::export_genesis(&mut c).unwrap();
    assert_eq!((gs.igps.len(), gs.noop_hooks.len()), (1, 1));
}
