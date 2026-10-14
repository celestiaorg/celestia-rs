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
    let mut ms = setup();
    let (sk, pk, addr) = key();
    let header = Header {
        chain_id: "test".into(),
        height: 5,
        ..Default::default()
    };
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
                address: x_auth::bech32(&addr).unwrap(),
                pub_key: None,
                account_number: n,
                sequence: 0,
            }),
        )
        .unwrap();
        x_bank::set_balance(&mut ctx, &addr, &Coin::new("utia", 1_000_000)).unwrap();
        let p = celestia_proto::celestia::minfee::v1::Params {
            network_min_gas_price: "1000000000000".into(),
        };
        ctx.set("minfee", b"params".to_vec(), p.encode_to_vec())
            .unwrap();
    }
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
