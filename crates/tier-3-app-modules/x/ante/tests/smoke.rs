//! End-to-end smoke test: a DIRECT-signed MsgSend through the full chain.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use celestia_proto::Any;
use celestia_proto::cosmos::auth::v1beta1::{BaseAccount, Params as AuthParams};
use celestia_proto::cosmos::bank::v1beta1::MsgSend;
use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;
use celestia_proto::cosmos::tx::v1beta1::{
    AuthInfo, Fee, ModeInfo, SignDoc, SignerInfo, TxBody, mode_info,
};
use prost::Message;
use sdk::coins::Coin;
use sdk::{Context, ExecMode, Header, Kind, MultiStore};
use x_ante::{StoreDeps, ante_handle};
use x_auth::Account;

const STORES: &[&str] = &["acc", "bank", "minfee", "blob", "circuit", "feegrant"];

#[test]
fn expired_feegrant_does_not_hide_revoke_out_of_gas() {
    use celestia_proto::cosmos::feegrant::v1beta1::BasicAllowance;
    let mut ms = setup();
    let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
    let (sponsor, payer) = ([1u8; 20], [2u8; 20]);
    let allowance = x_feegrant::Allowance::Basic(BasicAllowance {
        spend_limit: vec![],
        expiration: Some(pbjson_types::Timestamp {
            seconds: 1,
            nanos: 0,
        }),
    });
    x_feegrant::grant_allowance(&mut ctx, &sponsor, &payer, &allowance, None).unwrap();
    ctx.header.time.seconds = 2;
    ctx.gas = kvstore::GasMeter::infinite();
    x_feegrant::get_allowance(&mut ctx, &sponsor, &payer).unwrap();
    // Enough gas for the initial grant read, but not revoke's second read.
    ctx.gas = kvstore::GasMeter::new(ctx.gas.consumed());
    let error = x_feegrant::use_granted_fees(
        &mut ctx,
        &sponsor,
        &payer,
        &sdk::coins::Coins::default(),
        &[],
    )
    .unwrap_err();
    assert_eq!((error.codespace, error.code), ("sdk", 11));
    assert_eq!(ctx.gas.first_out_of_gas().unwrap().0, "ReadFlat");
}

fn setup() -> MultiStore {
    let mut ms = MultiStore::new();
    for s in STORES {
        ms.mount(s, Kind::Iavl).unwrap();
    }
    ms
}

fn key() -> ([u8; 32], Vec<u8>, Vec<u8>) {
    let sk = crypto::secp256k1::secret_key_from_secret(b"smoke");
    let pk = crypto::secp256k1::public_key_from_secret_key(&sk).unwrap();
    let addr = crypto::secp256k1::address(&pk).to_vec();
    (sk, pk.to_vec(), addr)
}

fn build_tx(
    sk: &[u8; 32],
    pk: &[u8],
    addr: &[u8],
    seq: u64,
    chain: &str,
    acc_num: u64,
) -> sdk::tx::Tx {
    let from = x_auth::bech32(addr).unwrap();
    let msg = MsgSend {
        from_address: from.clone(),
        to_address: x_auth::bech32(&[7u8; 20]).unwrap(),
        amount: vec![PbCoin {
            denom: "utia".into(),
            amount: "10".into(),
        }],
    };
    let body = TxBody {
        messages: vec![Any::pack(&msg)],
        ..Default::default()
    };
    sign_body(sk, pk, seq, chain, acc_num, body)
}

fn sign_body(
    sk: &[u8; 32],
    pk: &[u8],
    seq: u64,
    chain: &str,
    acc_num: u64,
    body: TxBody,
) -> sdk::tx::Tx {
    let pk_any = Any {
        type_url: "/cosmos.crypto.secp256k1.PubKey".into(),
        value: {
            let mut v = vec![0x0a, 33];
            v.extend_from_slice(pk);
            v
        },
    };
    let auth = AuthInfo {
        signer_infos: vec![SignerInfo {
            public_key: Some(pk_any),
            mode_info: Some(ModeInfo {
                sum: Some(mode_info::Sum::Single(mode_info::Single { mode: 1 })),
            }),
            sequence: seq,
        }],
        fee: Some(Fee {
            amount: vec![PbCoin {
                denom: "utia".into(),
                amount: "2000".into(),
            }],
            gas_limit: 100_000,
            payer: String::new(),
            granter: String::new(),
        }),
        ..Default::default()
    };
    let body_bytes = body.encode_to_vec();
    let auth_info_bytes = auth.encode_to_vec();
    let doc = SignDoc {
        body_bytes: body_bytes.clone(),
        auth_info_bytes: auth_info_bytes.clone(),
        chain_id: chain.into(),
        account_number: acc_num,
    };
    let sig = crypto::secp256k1::sign(sk, &doc.encode_to_vec()).unwrap();
    let raw = celestia_proto::cosmos::tx::v1beta1::TxRaw {
        body_bytes: body_bytes.clone(),
        auth_info_bytes: auth_info_bytes.clone(),
        signatures: vec![sig.to_vec()],
    };
    sdk::tx::Tx {
        tx_bytes: raw.encode_to_vec(),
        body_bytes,
        auth_info_bytes,
        signatures: vec![sig.to_vec()],
        body,
        auth_info: auth,
        ..Default::default()
    }
}

#[test]
fn direct_msg_send_passes_and_increments_sequence() {
    let (sk, pk, addr) = key();
    let header = Header {
        chain_id: "test".into(),
        height: 5,
        ..Default::default()
    };
    let mut ms = funded_ms(&header, &addr);
    let tx = build_tx(&sk, &pk, &addr, 0, "test", 1);
    let mut ctx = Context::new(&mut ms, header, ExecMode::Finalize);
    ctx.tx_bytes = tx.tx_bytes.clone();
    let out = ante_handle(&mut ctx, &tx, false, &StoreDeps::default()).expect("ante passes");
    assert_eq!(out.gas_wanted, 100_000);
    assert_eq!(out.priority, 2000 * 1_000_000 / 100_000);
    let acc = x_auth::get_account(&mut ctx, &addr).unwrap().unwrap();
    assert_eq!(acc.sequence(), 1);
    assert!(acc.pub_key().is_some());
    let types: Vec<_> = ctx
        .events
        .events()
        .iter()
        .map(|e| e.r#type.clone())
        .collect();
    assert_eq!(
        types,
        [
            "coin_spent",
            "coin_received",
            "transfer",
            "message",
            "tx",
            "tx",
            "tx"
        ]
    );
    assert!(ctx.gas.consumed() > 0);

    // Replaying the same tx now fails on the sequence.
    let err = ante_handle(&mut ctx, &tx, false, &StoreDeps::default()).unwrap_err();
    assert_eq!((err.error.codespace, err.error.code), ("sdk", 32));
}

fn funded_ms(header: &Header, addr: &[u8]) -> MultiStore {
    let mut ms = setup();
    {
        let mut ctx = Context::new(&mut ms, header.clone(), ExecMode::Finalize);
        x_auth::set_params(
            &mut ctx,
            &AuthParams {
                max_memo_characters: 256,
                tx_sig_limit: 7,
                tx_size_cost_per_byte: 10,
                sig_verify_cost_ed25519: 590,
                sig_verify_cost_secp256k1: 1000,
            },
        )
        .unwrap();
        x_auth::get_module_account(&mut ctx, x_auth::FEE_COLLECTOR).unwrap();
        let n = x_auth::next_account_number(&mut ctx).unwrap();
        x_auth::set_account(
            &mut ctx,
            &Account::Base(BaseAccount {
                address: x_auth::bech32(addr).unwrap(),
                pub_key: None,
                account_number: n,
                sequence: 0,
            }),
        )
        .unwrap();
        x_bank::set_balance(&mut ctx, addr, &Coin::new("utia", 1_000_000)).unwrap();
        let p = celestia_proto::celestia::minfee::v1::Params {
            network_min_gas_price: "1000000000000".into(),
        };
        ctx.set("minfee", b"params".to_vec(), p.encode_to_vec())
            .unwrap();
    }
    ms
}

#[test]
#[allow(clippy::too_many_lines)] // Keep the signed callback and rollback assertions together.
fn ibc_callback_cannot_translate_or_swallow_out_of_gas() {
    use celestia_proto::ibc::core::channel::v1::{MsgAcknowledgement, Packet};
    use x_ante::deps::{IbcHooks, IbcPacketMsg};

    // Model the PFM refund callback's MintCoins -> plain-error translation.
    // The hook is deliberately not a proof-validating IBC lifecycle fixture.
    fn refund(ctx: &mut Context<'_>) -> sdk::Result<()> {
        let remaining = ctx.gas.limit().unwrap() - ctx.gas.consumed();
        ctx.gas.consume(remaining, "before refund").unwrap();
        x_bank::mint_coins(
            ctx,
            "transfer",
            &sdk::coins::Coins(vec![Coin::new("utia", 1)]),
        )
    }
    fn translated(ctx: &mut Context<'_>, _: IbcPacketMsg, _: &Any, _: bool) -> sdk::Result<bool> {
        refund(ctx).map_err(|e| sdk::SdkError::new("undefined", 1, e.log))?;
        Ok(false)
    }
    fn later_panic(ctx: &mut Context<'_>, _: IbcPacketMsg, _: &Any, _: bool) -> sdk::Result<bool> {
        let _ = refund(ctx);
        let _ = ctx.gas.consume(17, "after original unwind");
        Err(sdk::error::go_panic("translated bank failure"))
    }
    fn swallowed(ctx: &mut Context<'_>, _: IbcPacketMsg, _: &Any, _: bool) -> sdk::Result<bool> {
        let _ = refund(ctx);
        Ok(false)
    }
    for mode in [ExecMode::Check, ExecMode::ReCheck] {
        for hook in [translated, later_panic, swallowed] {
            let (sk, pk, addr) = key();
            let header = Header {
                chain_id: "test".into(),
                height: 5,
                ..Default::default()
            };
            let mut ms = funded_ms(&header, &addr);
            {
                let mut ctx = Context::new(&mut ms, header.clone(), ExecMode::Finalize);
                let params = celestia_proto::celestia::blob::v1::Params {
                    gas_per_blob_byte: 8,
                    gov_max_square_size: 256,
                };
                ctx.set("blob", b"params".to_vec(), params.encode_to_vec())
                    .unwrap();
                x_auth::get_module_account(&mut ctx, "transfer").unwrap();
                let cp = ctx.branch();
                x_bank::mint_coins(
                    &mut ctx,
                    "transfer",
                    &sdk::coins::Coins(vec![Coin::new("utia", 1)]),
                )
                .unwrap();
                ctx.discard(cp);
            }
            ms.flush().unwrap();
            ms.commit().unwrap();
            let body = TxBody {
                messages: vec![Any::pack(&MsgAcknowledgement {
                    packet: Some(Packet {
                        sequence: 1,
                        source_port: "transfer".into(),
                        source_channel: "channel-0".into(),
                        destination_port: "transfer".into(),
                        destination_channel: "channel-1".into(),
                        data: vec![1],
                        timeout_timestamp: u64::MAX,
                        ..Default::default()
                    }),
                    acknowledgement: br#"{"error":"refund"}"#.to_vec(),
                    proof_acked: vec![1],
                    proof_height: Some(celestia_proto::ibc::core::client::v1::Height {
                        revision_number: 0,
                        revision_height: 1,
                    }),
                    signer: x_auth::bech32(&addr).unwrap(),
                })],
                ..Default::default()
            };
            let tx = sign_body(&sk, &pk, 0, "test", 1, body);
            let deps = StoreDeps {
                ibc: Some(IbcHooks {
                    packet_is_noop: hook,
                    update_client_check: |_, _, _| Ok(()),
                }),
                ..StoreDeps::default()
            };
            let mut ctx = Context::new(&mut ms, header, mode);
            ctx.gas.consume(13, "original meter").unwrap();
            ctx.tx_bytes = tx.tx_bytes.clone();
            let error = ante_handle(&mut ctx, &tx, false, &deps).unwrap_err();
            assert_eq!(
                (error.error.codespace, error.error.code),
                ("sdk", 11),
                "{}",
                error.error.log
            );
            assert!(!error.panicked && !error.zero_ctx);
            assert_eq!(error.gas_wanted, 100_000);
            let (location, used) = ctx.gas.first_out_of_gas().unwrap();
            assert_eq!(location, "ReadFlat");
            assert_eq!(ctx.gas.consumed(), used);
            assert!(
                error
                    .error
                    .log
                    .contains("out of gas in location: ReadFlat;")
            );
            assert!(ctx.events.events().is_empty());
            ctx.gas = kvstore::GasMeter::infinite();
            let account = x_auth::get_account(&mut ctx, &addr).unwrap().unwrap();
            assert_eq!(account.sequence(), 0);
            assert!(account.pub_key().is_none());
            assert_eq!(
                x_bank::get_balance(&mut ctx, &addr, "utia").unwrap().amount,
                Coin::new("utia", 1_000_000).amount
            );
        }
    }
}

#[test]
fn unsupported_msgs_are_rejected_from_app_version_11() {
    use celestia_proto::celestia_core_proto::tendermint::types::VersionParams;
    use celestia_proto::cosmos::evidence::v1beta1::MsgSubmitEvidence;
    let (sk, pk, _) = key();
    let body = TxBody {
        messages: vec![Any::pack(&MsgSubmitEvidence::default())],
        ..Default::default()
    };
    let tx = sign_body(&sk, &pk, 0, "test", 1, body);
    for (app, rejected) in [(0, false), (9, false), (10, false), (11, true)] {
        let mut ms = setup();
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        ctx.consensus_params.version = Some(VersionParams { app });
        ctx.tx_bytes = tx.tx_bytes.clone();
        let err = ante_handle(&mut ctx, &tx, false, &StoreDeps::default()).unwrap_err();
        let not_supported = (err.error.codespace, err.error.code) == ("sdk", 37);
        assert_eq!(
            not_supported, rejected,
            "app version {app}: {}",
            err.error.log
        );
    }
}
