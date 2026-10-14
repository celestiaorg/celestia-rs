//! Reward flow tests with the distribution staking hooks installed.
#![allow(deprecated)] // MsgCreateValidator.delegator_address

use std::sync::Arc;

use celestia_proto::Any;
use celestia_proto::celestia_core_proto::abci::{Validator as AbciValidator, VoteInfo};
use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;
use celestia_proto::cosmos::crypto::ed25519;
use celestia_proto::cosmos::distribution::v1beta1 as pb;
use celestia_proto::cosmos::staking::v1beta1 as spb;
use math::{Int, LegacyDec};
use sdk::coins::{Coin, Coins};
use sdk::{Context, ExecMode, Header, Kind, MultiStore};

use crate::store;

fn header(height: i64) -> Header {
    Header {
        chain_id: "test".into(),
        height,
        time: pbjson_types::Timestamp {
            seconds: 1_000_000 + height,
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
fn allocate_withdraw_rewards_and_commission() {
    x_staking::set_hooks(Arc::new(crate::Hooks));
    let mut m = MultiStore::new();
    for s in [
        x_staking::STORE_KEY,
        crate::STORE_KEY,
        x_auth::STORE_KEY,
        x_bank::STORE_KEY,
    ] {
        m.mount(s, Kind::Iavl).unwrap();
    }
    let mut ctx = Context::new(&mut m, header(0), ExecMode::Finalize);
    crate::init_genesis(
        &mut ctx,
        &serde_json::json!({
            "params": {"community_tax": "0.02", "base_proposer_reward": "0", "bonus_proposer_reward": "0", "withdraw_addr_enabled": true},
            "fee_pool": {"community_pool": []}
        }),
    )
    .unwrap();
    x_staking::init_genesis(
        &mut ctx,
        &serde_json::json!({
            "params": {"unbonding_time": "1000s", "max_validators": 10, "max_entries": 7,
                       "historical_entries": 0, "bond_denom": "utia", "min_commission_rate": "0.05"},
            "last_total_power": "0"
        }),
    )
    .unwrap();
    let op = vec![1u8; 20];
    let acct = x_auth::new_account_with_address(&mut ctx, &op).unwrap();
    x_auth::set_account(&mut ctx, &acct).unwrap();
    x_bank::set_balance(&mut ctx, &op, &Coin::new("utia", 10_000_000)).unwrap();
    let pk = ed25519::PubKey { key: vec![7; 32] };
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
            validator_address: x_staking::addr::val_to_string(&op).unwrap(),
            pubkey: Some(Any::pack(&pk)),
            value: Some(PbCoin {
                denom: "utia".into(),
                amount: "5000000".into(),
            }),
        },
    )
    .unwrap();
    x_staking::apply_and_return_validator_set_updates(&mut ctx).unwrap();
    // Starting info as in the Go devnet: previous period 1, reference 2.
    assert_eq!(store::get_current(&mut ctx, &op).unwrap().period, 2);
    assert_eq!(
        store::get_historical(&mut ctx, &op, 1)
            .unwrap()
            .reference_count,
        2
    );

    // Block 2: 1_000_000utia of fees, one vote with all the power.
    let mut ctx = Context::new(ctx.multistore(), header(2), ExecMode::Finalize);
    let fc = x_auth::get_module_account(&mut ctx, x_auth::FEE_COLLECTOR)
        .unwrap()
        .unwrap();
    let fc = x_auth::Account::Module(fc).address().unwrap();
    x_bank::set_balance(&mut ctx, &fc, &Coin::new("utia", 1_000_000)).unwrap();
    let cons = hash::sha256(&pk.key)[..20].to_vec();
    ctx.vote_infos = vec![VoteInfo {
        validator: Some(AbciValidator {
            address: cons,
            power: 5,
        }),
        block_id_flag: 2,
    }];
    crate::begin_block(&mut ctx).unwrap();
    let d = |s: &str| LegacyDec::from_str_go(s).unwrap();
    assert_eq!(
        store::get_fee_pool(&mut ctx).unwrap().amount_of("utia"),
        d("20000")
    );
    assert_eq!(
        store::get_commission(&mut ctx, &op)
            .unwrap()
            .amount_of("utia"),
        d("98000")
    );
    assert_eq!(
        store::get_outstanding(&mut ctx, &op)
            .unwrap()
            .amount_of("utia"),
        d("980000")
    );
    assert_eq!(
        store::get_current(&mut ctx, &op)
            .unwrap()
            .rewards
            .amount_of("utia"),
        d("882000")
    );

    let resp = crate::handle_withdraw_delegator_reward(
        &mut ctx,
        &pb::MsgWithdrawDelegatorReward {
            delegator_address: x_auth::bech32(&op).unwrap(),
            validator_address: x_staking::addr::val_to_string(&op).unwrap(),
        },
    )
    .unwrap();
    assert_eq!(
        Coins::from_proto(&resp.amount).unwrap().to_string(),
        "882000utia"
    );
    assert_eq!(
        store::get_historical(&mut ctx, &op, 2)
            .unwrap()
            .cumulative_reward_ratio
            .to_string(),
        "0.176400000000000000utia"
    );
    let resp = crate::handle_withdraw_validator_commission(
        &mut ctx,
        &pb::MsgWithdrawValidatorCommission {
            validator_address: x_staking::addr::val_to_string(&op).unwrap(),
        },
    )
    .unwrap();
    assert_eq!(
        Coins::from_proto(&resp.amount).unwrap().to_string(),
        "98000utia"
    );
    assert!(store::get_outstanding(&mut ctx, &op).unwrap().is_empty());
    assert_eq!(
        x_bank::get_balance(&mut ctx, &op, "utia").unwrap().amount,
        Int::from_u64(5_000_000 + 882_000 + 98_000)
    );
    let e = crate::handle_withdraw_validator_commission(
        &mut ctx,
        &pb::MsgWithdrawValidatorCommission {
            validator_address: x_staking::addr::val_to_string(&op).unwrap(),
        },
    )
    .unwrap_err();
    assert_eq!((e.codespace, e.code), ("distribution", 7));
}
