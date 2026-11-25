#![allow(clippy::unwrap_used, clippy::expect_used)]

use celestia_proto::celestia_core_proto::tendermint::types::VersionParams;
use pbjson_types::Timestamp;
use sdk::{Context, ExecMode, Header, Kind, MultiStore};
use x_staking::StakingHooks as _;

use super::*;
use crate::types::{checksum_hex, hex_to_address, is_hex_address};

fn store() -> MultiStore {
    let mut ms = MultiStore::new();
    for s in [STORE_KEY, "staking", x_params::STORE_KEY] {
        ms.mount(s, Kind::Iavl).unwrap();
    }
    ms.mount(x_params::TSTORE_KEY, Kind::Transient).unwrap();
    ms
}

fn ctx_at(ms: &mut MultiStore, height: i64, seconds: i64) -> Context<'_> {
    let header = Header {
        height,
        time: Timestamp { seconds, nanos: 0 },
        ..Header::default()
    };
    let mut ctx = Context::new(ms, header, ExecMode::Finalize);
    ctx.consensus_params.version = Some(VersionParams { app: 1 });
    ctx
}

/// EIP-55 vectors from the EIP and the mainnet height-1 valset.
#[test]
fn checksum_hex_is_eip55() {
    for s in [
        "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
        "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359",
        "0xdbF03B407c01E7cD3CBea99509d93f8DDDC8C6FB",
        "0xD1220A0cf47c7B9Be7A2E6BA89F429762e7b9aDb",
        "0xaCd6963C1641e8a51b2f0B069400d8Dce543575A",
    ] {
        assert_eq!(checksum_hex(&hex_to_address(s)), s);
    }
}

/// `IsHexAddress` and `HexToAddress` follow go-ethereum.
#[test]
fn hex_addresses_parse_like_go_ethereum() {
    assert!(is_hex_address("0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed"));
    assert!(is_hex_address("0X5aaeb6053f3e94c9b9a09f33669435e7ef1beaed"));
    assert!(is_hex_address("5aaeb6053f3e94c9b9a09f33669435e7ef1beaed"));
    assert!(!is_hex_address("0x5aaeb6053f3e94c9b9a09f33669435e7ef1beae"));
    assert!(!is_hex_address(
        "0x5aaeb6053f3e94c9b9a09f33669435e7ef1beaeg"
    ));
    // Short input is left-padded; an odd length gets a leading zero;
    // decoding stops at the first non-hex pair.
    let mut want = [0u8; 20];
    want[19] = 0x01;
    assert_eq!(hex_to_address("0x1"), want);
    want[18] = 0xab;
    want[19] = 0xcd;
    assert_eq!(hex_to_address("abcd"), want);
    let mut want = [0u8; 20];
    want[19] = 0xab;
    assert_eq!(hex_to_address("abzz12"), want);
}

/// The default EVM address is the last 20 bytes of the operator address.
#[test]
fn default_evm_address_takes_the_last_20_bytes() {
    let long: Vec<u8> = (0u8..32).collect();
    assert_eq!(
        types::default_evm_address(&long).to_vec(),
        long[12..].to_vec()
    );
    let mut want = [0u8; 20];
    want[17..].copy_from_slice(&[1, 2, 3]);
    assert_eq!(types::default_evm_address(&[1, 2, 3]), want);
}

/// Attestations round-trip through their `Any` encoding; other type URLs
/// are `ErrUnmarshalllAttestation`.
#[test]
fn attestations_round_trip() {
    let dc = Attestation::DataCommitment(DataCommitment {
        nonce: 4,
        begin_block: 1,
        end_block: 401,
        time: Some(Timestamp::default()),
    });
    let bz = dc.encode();
    assert_eq!(Attestation::decode(&bz).unwrap(), dc);
    // The non-nullable zero time is still encoded (`22 00`).
    assert!(bz.ends_with(&[0x22, 0x00]));
    let other = celestia_proto::Any {
        type_url: "/x".into(),
        value: vec![],
    };
    let e = Attestation::decode(&prost::Message::encode_to_vec(&other)).unwrap_err();
    assert_eq!((e.codespace, e.code), ("qgb", 26));
}

/// The hooks write at app version 1 only, and only where `qgb` is
/// mounted; a second validator with the same default address fails.
#[test]
fn hooks_act_at_app_version_1() {
    let val = [7u8; 20];
    let mut ms = store();
    let mut ctx = ctx_at(&mut ms, 12, 0);
    Hooks.after_validator_created(&mut ctx, &val).unwrap();
    assert_eq!(get_evm_address(&mut ctx, &val).unwrap(), Some(val));
    let e = Hooks.after_validator_created(&mut ctx, &val).unwrap_err();
    assert_eq!((e.codespace, e.code), ("qgb", 37));
    Hooks
        .after_validator_begin_unbonding(&mut ctx, &[], &val)
        .unwrap();
    assert_eq!(latest_unbonding_block_height(&mut ctx).unwrap(), 12);

    ctx.consensus_params.version = Some(VersionParams { app: 2 });
    ctx.header.height = 13;
    Hooks.after_validator_created(&mut ctx, &[8; 20]).unwrap();
    Hooks
        .after_validator_begin_unbonding(&mut ctx, &[], &val)
        .unwrap();
    assert_eq!(get_evm_address(&mut ctx, &[8; 20]).unwrap(), None);
    assert_eq!(latest_unbonding_block_height(&mut ctx).unwrap(), 12);
    drop(ctx);

    let mut ms = MultiStore::new();
    ms.mount("staking", Kind::Iavl).unwrap();
    let mut ctx = ctx_at(&mut ms, 1, 0);
    Hooks.after_validator_created(&mut ctx, &val).unwrap();
    Hooks
        .after_validator_begin_unbonding(&mut ctx, &[], &val)
        .unwrap();
}

fn store_attestation(ctx: &mut Context<'_>, nonce: u64, seconds: i64) {
    let at = Attestation::DataCommitment(DataCommitment {
        nonce,
        begin_block: 0,
        end_block: 0,
        time: Some(Timestamp { seconds, nanos: 0 }),
    });
    set_attestation_request(ctx, &at).unwrap();
}

/// The latest nonce must grow by one, and attestations are never
/// overwritten.
#[test]
fn attestation_nonces_only_increment() {
    let mut ms = store();
    let mut ctx = ctx_at(&mut ms, 10, 0);
    init_genesis(&mut ctx).unwrap();
    store_attestation(&mut ctx, 1, 0);
    let skip = Attestation::DataCommitment(DataCommitment {
        nonce: 3,
        ..DataCommitment::default()
    });
    assert!(set_attestation_request(&mut ctx, &skip).is_err());
    ctx.set(
        STORE_KEY,
        LATEST_ATTESTATION_NONCE.to_vec(),
        1u64.to_be_bytes().to_vec(),
    )
    .unwrap();
    let again = Attestation::DataCommitment(DataCommitment {
        nonce: 1,
        ..DataCommitment::default()
    });
    assert!(set_attestation_request(&mut ctx, &again).is_err());
}

/// `MsgRegisterEVMAddress` checks the message and that the validator
/// exists.
#[test]
fn register_evm_address_validates() {
    let val = x_staking::addr::val_to_string(&[7u8; 20]).unwrap();
    let ok = types::MsgRegisterEvmAddress {
        validator_address: val.clone(),
        evm_address: "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed".into(),
    };
    msgs::validate_basic(&ok).unwrap();
    assert_eq!(msgs::signers(&ok).unwrap(), vec![vec![7u8; 20]]);
    let bad = types::MsgRegisterEvmAddress {
        validator_address: val,
        evm_address: "0x12".into(),
    };
    let e = msgs::validate_basic(&bad).unwrap_err();
    assert_eq!((e.codespace, e.code), ("qgb", 36));
    let mut ms = store();
    let mut ctx = ctx_at(&mut ms, 1, 0);
    let e = register_evm_address(&mut ctx, &ok).unwrap_err();
    assert_eq!(e.codespace, "staking");
}
