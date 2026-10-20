//! Keeper flow tests (no hooks installed).
#![allow(deprecated)] // MsgCreateValidator.delegator_address

use celestia_proto::Any;
use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;
use celestia_proto::cosmos::crypto::ed25519;
use celestia_proto::cosmos::staking::v1beta1 as pb;
use math::{Int, LegacyDec};
use sdk::coins::Coin;
use sdk::{Context, ExecMode, Header, Kind, MultiStore};

use crate::types::{BondStatus, dec_str};
use crate::{BONDED_POOL, NOT_BONDED_POOL, store};

fn ms() -> MultiStore {
    let mut m = MultiStore::new();
    for s in [crate::STORE_KEY, x_auth::STORE_KEY, x_bank::STORE_KEY] {
        m.mount(s, Kind::Iavl).unwrap();
    }
    m
}

fn header(height: i64, secs: i64) -> Header {
    Header {
        chain_id: "test".into(),
        height,
        time: pbjson_types::Timestamp {
            seconds: secs,
            nanos: 0,
        },
        ..Header::default()
    }
}

fn acc(n: u8) -> Vec<u8> {
    vec![n; 20]
}

fn fund(ctx: &mut Context<'_>, a: &[u8], amt: u64) {
    let acct = x_auth::new_account_with_address(ctx, a).unwrap();
    x_auth::set_account(ctx, &acct).unwrap();
    x_bank::set_balance(ctx, a, &Coin::new("utia", amt)).unwrap();
}

fn setup(ctx: &mut Context<'_>) {
    let genesis = serde_json::json!({
        "params": {
            "unbonding_time": "1000s",
            "max_validators": 2,
            "max_entries": 7,
            "historical_entries": 3,
            "bond_denom": "utia",
            "min_commission_rate": "0.050000000000000000"
        },
        "last_total_power": "0",
        "exported": false
    });
    crate::init_genesis(ctx, &genesis).unwrap();
}

fn pubkey(n: u8) -> Any {
    Any::pack(&ed25519::PubKey { key: vec![n; 32] })
}

fn create(ctx: &mut Context<'_>, n: u8, amt: u64) -> pb::MsgCreateValidator {
    let msg = pb::MsgCreateValidator {
        description: Some(pb::Description {
            moniker: format!("v{n}"),
            ..Default::default()
        }),
        commission: Some(pb::CommissionRates {
            rate: dec_str(&LegacyDec::from_str_go("0.1").unwrap()),
            max_rate: dec_str(&LegacyDec::from_str_go("0.2").unwrap()),
            max_change_rate: dec_str(&LegacyDec::from_str_go("0.01").unwrap()),
        }),
        min_self_delegation: "1".into(),
        delegator_address: String::new(),
        validator_address: crate::addr::val_to_string(&acc(n)).unwrap(),
        pubkey: Some(pubkey(n)),
        value: Some(PbCoin {
            denom: "utia".into(),
            amount: amt.to_string(),
        }),
    };
    crate::handle_create_validator(ctx, &msg).unwrap();
    msg
}

fn coin(amt: u64) -> Option<PbCoin> {
    Some(PbCoin {
        denom: "utia".into(),
        amount: amt.to_string(),
    })
}

fn pool(ctx: &mut Context<'_>, name: &str) -> Int {
    x_bank::get_balance(ctx, &x_auth::module_address(name), "utia")
        .unwrap()
        .amount
}

#[test]
fn create_bond_delegate_undelegate_and_mature() {
    let mut m = ms();
    let mut ctx = Context::new(&mut m, header(1, 1_000_000), ExecMode::Finalize);
    setup(&mut ctx);
    fund(&mut ctx, &acc(1), 10_000_000);
    fund(&mut ctx, &acc(9), 10_000_000);
    create(&mut ctx, 1, 5_000_000);
    assert_eq!(pool(&mut ctx, NOT_BONDED_POOL), Int::from_u64(5_000_000));

    let updates = crate::end_block(&mut ctx).unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].power, 5);
    assert_eq!(pool(&mut ctx, BONDED_POOL), Int::from_u64(5_000_000));
    assert_eq!(
        store::get_last_total_power(&mut ctx).unwrap(),
        Int::from_u64(5)
    );

    let val = crate::addr::val_to_string(&acc(1)).unwrap();
    let del = x_auth::bech32(&acc(9)).unwrap();
    crate::handle_delegate(
        &mut ctx,
        &pb::MsgDelegate {
            delegator_address: del.clone(),
            validator_address: val.clone(),
            amount: coin(3_000_000),
        },
    )
    .unwrap();
    assert_eq!(pool(&mut ctx, BONDED_POOL), Int::from_u64(8_000_000));
    let updates = crate::end_block(&mut ctx).unwrap();
    assert_eq!(updates[0].power, 8);

    // Undelegate 1_000_000 at height 2; matures 1000s later.
    let mut ctx = Context::new(ctx.multistore(), header(2, 1_000_010), ExecMode::Finalize);
    let resp = crate::handle_undelegate(
        &mut ctx,
        &pb::MsgUndelegate {
            delegator_address: del.clone(),
            validator_address: val.clone(),
            amount: coin(1_000_000),
        },
    )
    .unwrap();
    assert_eq!(resp.completion_time.unwrap().seconds, 1_001_010);
    assert_eq!(pool(&mut ctx, NOT_BONDED_POOL), Int::from_u64(1_000_000));
    let ubd = store::get_ubd(&mut ctx, &acc(9), &acc(1)).unwrap().unwrap();
    assert_eq!(ubd.entries[0].unbonding_id, 1);

    // Cancel half of it.
    crate::handle_cancel_unbonding_delegation(
        &mut ctx,
        &pb::MsgCancelUnbondingDelegation {
            delegator_address: del.clone(),
            validator_address: val.clone(),
            amount: coin(400_000),
            creation_height: 2,
        },
    )
    .unwrap();
    let ubd = store::get_ubd(&mut ctx, &acc(9), &acc(1)).unwrap().unwrap();
    assert_eq!(ubd.entries[0].balance, Int::from_u64(600_000));
    assert_eq!(pool(&mut ctx, BONDED_POOL), Int::from_u64(7_400_000));

    // Not mature yet.
    crate::end_block(&mut ctx).unwrap();
    assert!(
        store::get_ubd(&mut ctx, &acc(9), &acc(1))
            .unwrap()
            .is_some()
    );
    let mut ctx = Context::new(ctx.multistore(), header(3, 1_001_010), ExecMode::Finalize);
    crate::end_block(&mut ctx).unwrap();
    assert!(
        store::get_ubd(&mut ctx, &acc(9), &acc(1))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        x_bank::get_balance(&mut ctx, &acc(9), "utia")
            .unwrap()
            .amount,
        Int::from_u64(7_600_000)
    );
    let ev = ctx.events.events().last().unwrap();
    assert_eq!(ev.r#type, "complete_unbonding");
    assert_eq!(ev.attributes[0].value, "600000utia");
}

#[test]
fn msg_validation_errors() {
    let mut m = ms();
    let mut ctx = Context::new(&mut m, header(1, 1_000_000), ExecMode::Finalize);
    setup(&mut ctx);
    fund(&mut ctx, &acc(1), 10_000_000);
    let mut msg = create(&mut ctx, 1, 1_000_000);
    // Same operator again.
    let e = crate::handle_create_validator(&mut ctx, &msg).unwrap_err();
    assert_eq!((e.codespace, e.code), ("staking", 4));
    // Commission above the celestia maximum (60%).
    fund(&mut ctx, &acc(2), 10_000_000);
    msg.validator_address = crate::addr::val_to_string(&acc(2)).unwrap();
    msg.pubkey = Some(pubkey(2));
    msg.commission = Some(pb::CommissionRates {
        rate: dec_str(&LegacyDec::from_str_go("0.7").unwrap()),
        max_rate: dec_str(&LegacyDec::from_str_go("0.8").unwrap()),
        max_change_rate: dec_str(&LegacyDec::from_str_go("0.01").unwrap()),
    });
    let e = crate::handle_create_validator(&mut ctx, &msg).unwrap_err();
    assert_eq!((e.codespace, e.code), ("staking", 11));
    // Below the minimum commission rate.
    msg.commission = Some(pb::CommissionRates {
        rate: dec_str(&LegacyDec::from_str_go("0.01").unwrap()),
        max_rate: dec_str(&LegacyDec::from_str_go("0.2").unwrap()),
        max_change_rate: dec_str(&LegacyDec::from_str_go("0.01").unwrap()),
    });
    let e = crate::handle_create_validator(&mut ctx, &msg).unwrap_err();
    assert_eq!((e.codespace, e.code), ("staking", 40));
    // Self-redelegation.
    let v1 = crate::addr::val_to_string(&acc(1)).unwrap();
    let e = crate::handle_begin_redelegate(
        &mut ctx,
        &pb::MsgBeginRedelegate {
            delegator_address: x_auth::bech32(&acc(1)).unwrap(),
            validator_src_address: v1.clone(),
            validator_dst_address: v1,
            amount: coin(10),
        },
    )
    .unwrap_err();
    assert_eq!((e.codespace, e.code), ("staking", 29));
}

#[test]
fn slash_burns_and_changes_exchange_rate() {
    let mut m = ms();
    let mut ctx = Context::new(&mut m, header(1, 1_000_000), ExecMode::Finalize);
    setup(&mut ctx);
    fund(&mut ctx, &acc(1), 10_000_000);
    create(&mut ctx, 1, 3_000_000);
    crate::end_block(&mut ctx).unwrap();
    let mut ctx = Context::new(ctx.multistore(), header(5, 1_000_050), ExecMode::Finalize);
    let v = store::validator(&mut ctx, &acc(1)).unwrap();
    let cons = v.cons_addr().unwrap();
    let burned = crate::slash(
        &mut ctx,
        &cons,
        5,
        3,
        &LegacyDec::from_str_go("0.01").unwrap(),
    )
    .unwrap();
    assert_eq!(burned, Int::from_u64(30_000));
    let v = store::validator(&mut ctx, &acc(1)).unwrap();
    assert_eq!(v.tokens, Int::from_u64(2_970_000));
    assert_eq!(pool(&mut ctx, BONDED_POOL), Int::from_u64(2_970_000));
    // shares/tokens now != 1: 1_000_000 tokens buy 3e6/2.97e6 * 1e6 shares.
    let shares = v.shares_from_tokens(&Int::from_u64(1_000_000)).unwrap();
    assert_eq!(shares.to_string(), "1010101.010101010101010101");
    let t = v.tokens_from_shares_truncated(&shares).unwrap();
    assert_eq!(t.to_string(), "999999.999999999999999999");

    crate::jail(&mut ctx, &cons).unwrap();
    let updates = crate::end_block(&mut ctx).unwrap();
    assert_eq!(updates[0].power, 0);
    let v = store::validator(&mut ctx, &acc(1)).unwrap();
    assert_eq!(v.status, BondStatus::Unbonding);
    assert_eq!(v.unbonding_time.seconds, 1_001_050);
    assert_eq!(pool(&mut ctx, NOT_BONDED_POOL), Int::from_u64(2_970_000));
}

#[test]
fn json_msg_to_wire_rewrites_decimals() {
    let mut v = serde_json::json!({
        "@type": "/cosmos.staking.v1beta1.MsgCreateValidator",
        "commission": {"rate": "0.100000000000000000", "max_rate": "0.2", "max_change_rate": "0.01"}
    });
    crate::msgs::json_msg_to_wire(&mut v).unwrap();
    assert_eq!(v["commission"]["rate"], "100000000000000000");
    assert_eq!(v["commission"]["max_change_rate"], "10000000000000000");
}

/// Redelegating away from an unbonded validator completes at once; Go's
/// `getBeginInfo` returns its zero `time.Time`, so the response's
/// `completion_time` is 0001-01-01T00:00:00Z (seconds -62135596800), not
/// the Unix epoch. Found on mainnet block 14,894,680 (the tx's `data`, and
/// so `LastResultsHash`, differed).
#[test]
fn redelegation_from_unbonded_validator_completes_at_go_zero_time() {
    let mut m = ms();
    let mut ctx = Context::new(&mut m, header(1, 1_000_000), ExecMode::Finalize);
    setup(&mut ctx);
    fund(&mut ctx, &acc(1), 10_000_000);
    fund(&mut ctx, &acc(2), 10_000_000);
    fund(&mut ctx, &acc(9), 10_000_000);
    // No EndBlock: both validators stay unbonded.
    create(&mut ctx, 1, 5_000_000);
    create(&mut ctx, 2, 5_000_000);
    let src = crate::addr::val_to_string(&acc(1)).unwrap();
    let dst = crate::addr::val_to_string(&acc(2)).unwrap();
    let del = x_auth::bech32(&acc(9)).unwrap();
    crate::handle_delegate(
        &mut ctx,
        &pb::MsgDelegate {
            delegator_address: del.clone(),
            validator_address: src.clone(),
            amount: coin(1_000_000),
        },
    )
    .unwrap();
    assert_eq!(
        store::get_validator(&mut ctx, &acc(1))
            .unwrap()
            .unwrap()
            .status,
        BondStatus::Unbonded
    );
    let resp = crate::handle_begin_redelegate(
        &mut ctx,
        &pb::MsgBeginRedelegate {
            delegator_address: del,
            validator_src_address: src,
            validator_dst_address: dst,
            amount: coin(400_000),
        },
    )
    .unwrap();
    assert_eq!(resp.completion_time, Some(sdk::time::go_zero()));
    let ev = ctx.events.events().last().unwrap();
    assert_eq!(ev.r#type, "redelegate");
    assert_eq!(ev.attributes[3].value, "0001-01-01T00:00:00Z");
}

/// Redelegating the last shares away from an unbonded validator: `Unbond`
/// removes the validator, and Go's `getBeginInfo` then returns
/// `ErrNoValidatorFound` (its named `err`), failing the redelegation.
/// Rust succeeded (deliver-diff `staking-lifecycle` seed 5017, #74).
#[test]
fn redelegating_last_shares_of_unbonded_validator_fails() {
    let mut m = ms();
    let mut ctx = Context::new(&mut m, header(1, 1_000_000), ExecMode::Finalize);
    setup(&mut ctx);
    fund(&mut ctx, &acc(1), 10_000_000);
    fund(&mut ctx, &acc(2), 10_000_000);
    // No EndBlock: both validators stay unbonded.
    create(&mut ctx, 1, 5_000_000);
    create(&mut ctx, 2, 5_000_000);
    let msg = |amount: u64| pb::MsgBeginRedelegate {
        delegator_address: x_auth::bech32(&acc(1)).unwrap(),
        validator_src_address: crate::addr::val_to_string(&acc(1)).unwrap(),
        validator_dst_address: crate::addr::val_to_string(&acc(2)).unwrap(),
        amount: coin(amount),
    };
    // Part of the self-delegation: the validator keeps shares.
    crate::handle_begin_redelegate(&mut ctx, &msg(1_000_000)).unwrap();
    // The rest: the validator is removed, then the lookup fails.
    let e = crate::handle_begin_redelegate(&mut ctx, &msg(4_000_000)).unwrap_err();
    assert_eq!((e.codespace, e.code), ("staking", 3));
    assert_eq!(e.log, "validator does not exist");
}

#[test]
fn edit_validator_checks_min_self_before_nil_rate() {
    // Go: a non-positive `min_self_delegation` fails before the nil
    // `commission_rate` is dereferenced; a nil `min_self_delegation`
    // panics.
    let mut m = ms();
    let mut ctx = Context::new(&mut m, header(1, 1_000_000), ExecMode::Finalize);
    setup(&mut ctx);
    let msg = pb::MsgEditValidator {
        description: Some(pb::Description {
            moniker: "m".into(),
            ..Default::default()
        }),
        validator_address: crate::addr::val_to_string(&acc(1)).unwrap(),
        commission_rate: String::new(),
        min_self_delegation: "-0".into(),
    };
    // description {moniker "m"}, commission_rate "", min_self_delegation "-0".
    let raw = [
        0x0a, 0x03, 0x0a, 0x01, b'm', 0x1a, 0x00, 0x22, 0x02, b'-', b'0',
    ];
    let e = crate::handle_edit_validator_wire(&mut ctx, &msg, &raw).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 18));
    let raw = [0x0a, 0x03, 0x0a, 0x01, b'm', 0x1a, 0x00, 0x22, 0x00];
    let msg = pb::MsgEditValidator {
        min_self_delegation: String::new(),
        ..msg
    };
    let e = crate::handle_edit_validator_wire(&mut ctx, &msg, &raw).unwrap_err();
    assert!(e.is_panic());
}
