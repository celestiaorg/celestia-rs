#![allow(deprecated)] // MsgCreateValidator.delegator_address
use super::*;

use std::sync::Arc;

use celestia_proto::celestia_core_proto::abci::{Validator as AbciValidator, VoteInfo};
use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;
use celestia_proto::cosmos::staking::v1beta1 as spb;
use sdk::{ExecMode, Header, Kind, MultiStore};

fn header(height: i64, secs: i64) -> Header {
    Header {
        chain_id: "test".into(),
        height,
        time: Timestamp {
            seconds: secs,
            nanos: 0,
        },
        ..Header::default()
    }
}

fn dec(s: &str) -> String {
    String::from_utf8(LegacyDec::from_str_go(s).unwrap().marshal()).unwrap()
}

#[test]
#[allow(clippy::too_many_lines)]
fn downtime_jails_slashes_and_unjail_waits() {
    x_staking::set_hooks(Arc::new(Hooks));
    let mut m = MultiStore::new();
    for s in [
        x_staking::STORE_KEY,
        STORE_KEY,
        x_auth::STORE_KEY,
        x_bank::STORE_KEY,
    ] {
        m.mount(s, Kind::Iavl).unwrap();
    }
    let mut ctx = Context::new(&mut m, header(0, 1000), ExecMode::Finalize);
    x_staking::init_genesis(
        &mut ctx,
        &serde_json::json!({
            "params": {"unbonding_time": "1000s", "max_validators": 10, "max_entries": 7,
                       "historical_entries": 0, "bond_denom": "utia", "min_commission_rate": "0.05"},
            "last_total_power": "0"
        }),
    )
    .unwrap();
    init_genesis(
        &mut ctx,
        &serde_json::json!({"params": {
            "signed_blocks_window": "4",
            "min_signed_per_window": "0.5",
            "downtime_jail_duration": "60s",
            "slash_fraction_double_sign": "0.05",
            "slash_fraction_downtime": "0.01"
        }}),
    )
    .unwrap();
    let op = vec![1u8; 20];
    let acct = x_auth::new_account_with_address(&mut ctx, &op).unwrap();
    x_auth::set_account(&mut ctx, &acct).unwrap();
    x_bank::set_balance(&mut ctx, &op, &sdk::coins::Coin::new("utia", 10_000_000)).unwrap();
    let pk = ed25519::PubKey { key: vec![7; 32] };
    let valoper = x_staking::addr::val_to_string(&op).unwrap();
    x_staking::handle_create_validator(
        &mut ctx,
        &spb::MsgCreateValidator {
            description: Some(spb::Description {
                moniker: "v".into(),
                ..Default::default()
            }),
            commission: Some(spb::CommissionRates {
                rate: dec("0.1"),
                max_rate: dec("0.2"),
                max_change_rate: dec("0.01"),
            }),
            min_self_delegation: "1".into(),
            delegator_address: String::new(),
            validator_address: valoper.clone(),
            pubkey: Some(Any::pack(&pk)),
            value: Some(PbCoin {
                denom: "utia".into(),
                amount: "5000000".into(),
            }),
        },
    )
    .unwrap();
    x_staking::apply_and_return_validator_set_updates(&mut ctx).unwrap();
    let cons = hash::sha256(&pk.key)[..20].to_vec();
    assert_eq!(
        get_signing_info(&mut ctx, &cons)
            .unwrap()
            .unwrap()
            .start_height,
        0
    );
    assert!(
        ctx.get(STORE_KEY, &addr_pubkey_key(&cons))
            .unwrap()
            .is_some()
    );

    for h in 1..=5 {
        let mut c = Context::new(ctx.multistore(), header(h, 1000 + h), ExecMode::Finalize);
        c.vote_infos = vec![VoteInfo {
            validator: Some(AbciValidator {
                address: cons.clone(),
                power: 5,
            }),
            block_id_flag: BLOCK_ID_FLAG_ABSENT,
        }];
        begin_block(&mut c).unwrap();
        if h < 5 {
            let info = get_signing_info(&mut c, &cons).unwrap().unwrap();
            assert_eq!(info.missed_blocks_counter, h);
            assert!(get_missed_block(&mut c, &cons, h - 1).unwrap());
        } else {
            let ev: Vec<_> = c.events.events().iter().map(|e| e.r#type.clone()).collect();
            assert_eq!(ev, ["liveness", "coin_spent", "burn", "slash"]);
            let slash_ev = &c.events.events()[3];
            assert_eq!(slash_ev.attributes[4].value, "50000");
        }
    }
    let mut ctx = Context::new(ctx.multistore(), header(5, 1005), ExecMode::Finalize);
    let info = get_signing_info(&mut ctx, &cons).unwrap().unwrap();
    assert_eq!((info.missed_blocks_counter, info.index_offset), (0, 0));
    assert_eq!(info.jailed_until.seconds, 1065);
    assert!(
        ctx.prefix_iter(STORE_KEY, &missed_bitmap_prefix(&cons))
            .unwrap()
            .next(&mut ctx)
            .unwrap()
            .is_none()
    );
    let v = x_staking::store::validator(&mut ctx, &op).unwrap();
    assert!(v.jailed);
    assert_eq!(v.tokens, math::Int::from_u64(4_950_000));

    let msg = pb::MsgUnjail {
        validator_addr: valoper,
    };
    let e = handle_unjail(&mut ctx, &msg).unwrap_err();
    assert_eq!((e.codespace, e.code), ("slashing", 4));
    let mut ctx = Context::new(ctx.multistore(), header(70, 1065), ExecMode::Finalize);
    handle_unjail(&mut ctx, &msg).unwrap();
    assert!(!x_staking::store::validator(&mut ctx, &op).unwrap().jailed);
    let e = handle_unjail(&mut ctx, &msg).unwrap_err();
    assert_eq!((e.codespace, e.code), ("slashing", 5));
}

#[test]
fn bitmap_chunk_roundtrip_matches_bitset_encoding() {
    let mut words = [0u64; CHUNK_WORDS];
    words[0] = 1;
    words[15] = 1 << 63;
    let b = encode_chunk(&words);
    assert_eq!(b.len(), 136);
    assert_eq!(&b[..8], &1024u64.to_be_bytes());
    assert_eq!(&b[8..16], &1u64.to_be_bytes());
    assert_eq!(decode_chunk(&b).unwrap(), words);
    assert_eq!(bit_pos(1025), (1, 0, 1));
    assert_eq!(bit_pos(1023), (0, 15, 63));
}

#[test]
fn params_wire_matches_fixture() {
    // slashing 0x00 at height 1 of the `empty` devnet scenario.
    let hex = "08904e1210313030303030303030303030303030301a02083c221132303030303030303030303030303030302a0130";
    let b: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    let mut json = serde_json::json!({
        "signed_blocks_window": "10000",
        "min_signed_per_window": "0.001000000000000000",
        "downtime_jail_duration": "60s",
        "slash_fraction_double_sign": "0.020000000000000000",
        "slash_fraction_downtime": "0.000000000000000000"
    });
    normalize_params_json(&mut json).unwrap();
    let p: pb::Params = serde_json::from_value(json).unwrap();
    let p = Params::from_wire(&p).unwrap();
    assert_eq!(p.encode(), b);
    assert_eq!(p.min_signed_per_window_blocks().unwrap(), 10);
}

#[test]
fn params_wire_nil_decimals_match_go_validation_order() {
    let zero = LegacyDec::zero().marshal();
    let mut wire = pb::Params {
        signed_blocks_window: 100,
        min_signed_per_window: zero.clone(),
        downtime_jail_duration: Some(Duration {
            seconds: 60,
            nanos: 0,
        }),
        slash_fraction_double_sign: zero.clone(),
        slash_fraction_downtime: zero,
    };
    let validate = |wire: &pb::Params| {
        Params::from_wire(wire)
            .unwrap()
            .validate_wire(wire)
            .map_err(|e| e.log)
    };
    validate(&wire).unwrap();

    wire.min_signed_per_window.clear();
    assert_eq!(
        validate(&wire).unwrap_err(),
        "min signed per window cannot be nil: <nil>"
    );
    wire.signed_blocks_window = 0;
    assert_eq!(
        validate(&wire).unwrap_err(),
        "signed blocks window must be positive: 0"
    );

    wire.signed_blocks_window = 100;
    wire.min_signed_per_window = LegacyDec::zero().marshal();
    wire.slash_fraction_double_sign.clear();
    assert_eq!(
        validate(&wire).unwrap_err(),
        "double sign slash fraction cannot be nil: <nil>"
    );
    wire.downtime_jail_duration = Some(Duration::default());
    assert_eq!(
        validate(&wire).unwrap_err(),
        "downtime jail duration must be positive: 0s"
    );

    wire.downtime_jail_duration = Some(Duration {
        seconds: 60,
        nanos: 0,
    });
    wire.slash_fraction_double_sign = LegacyDec::zero().marshal();
    wire.slash_fraction_downtime.clear();
    assert_eq!(
        validate(&wire).unwrap_err(),
        "downtime slash fraction cannot be nil: <nil>"
    );
}
