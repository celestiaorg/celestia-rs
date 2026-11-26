//! cosmos-sdk v0.46 `SIGN_MODE_LEGACY_AMINO_JSON` sign bytes (app
//! version 1): legacy amino `GetSignBytes`, not x/tx's encoder, decides.

#![allow(clippy::unwrap_used)]

use celestia_proto::cosmos::tx::v1beta1::{AuthInfo, TxRaw};
use prost::Message as _;

/// Mainnet block 6651, tx 4: an ibc-go v6 `MsgTransfer` without
/// `timeout_height`, signed with `SIGN_MODE_LEGACY_AMINO_JSON` by account
/// 33227 at sequence 1 (Go accepted it).
const TX: &[&str] = &[
    "0a88060ae0050a292f6962632e6170706c69636174696f6e732e7472616e736665722e76312e4d73675472616e736665",
    "7212b2050a087472616e7366657212096368616e6e656c2d321a100a047574696112083135303030303030222f63656c",
    "657374696131366a6e713667357776676a683978733674366437616b346633386834636878367372793932642a3f6f73",
    "6d6f31646d7337683567307a6c7164383234616e3475343064786e306a6a727773326e30307868617372376b63636774",
    "6d7478657061716438747a75323880ceb585a8d3dfc917428c047b227761736d223a7b22636f6e7472616374223a226f",
    "736d6f31646d7337683567307a6c7164383234616e3475343064786e306a6a727773326e30307868617372376b636367",
    "746d7478657061716438747a7532222c226d7367223a7b22737761705f616e645f616374696f6e223a7b22757365725f",
    "73776170223a7b22737761705f65786163745f636f696e5f696e223a7b22737761705f76656e75655f6e616d65223a22",
    "6f736d6f7369732d706f6f6c6d616e61676572222c226f7065726174696f6e73223a5b7b22706f6f6c223a2231323438",
    "222c2264656e6f6d5f696e223a226962632f443739453744383341423339394246464639333433334535344641413438",
    "30433139313234384643353536393234413241383335314145323633384233383737222c2264656e6f6d5f6f7574223a",
    "22756f736d6f227d5d7d7d2c226d696e5f636f696e223a7b2264656e6f6d223a22756f736d6f222c22616d6f756e7422",
    "3a223936393234353730227d2c2274696d656f75745f74696d657374616d70223a313639383834303638363439373730",
    "373331362c22706f73745f737761705f616374696f6e223a7b2262616e6b5f73656e64223a7b22746f5f616464726573",
    "73223a226f736d6f31366a6e713667357776676a683978733674366437616b34663338683463687836666a7839786a22",
    "7d7d2c22616666696c6961746573223a5b5d7d7d7d7d122353776170706564207669612048616c6c737761702028636f",
    "696e68616c6c2e6f72672912670a500a460a1f2f636f736d6f732e63727970746f2e736563703235366b312e5075624b",
    "657912230a2103e459dcd1053aae6a887ac7458ab732c6fcae4b8ee93891e369837430d17bc30912040a02087f180112",
    "130a0d0a04757469611205323533353410b0de071a40f212e8a28a6b7a5f447f285180e3cd7fee783a8b097843f3baad",
    "627fdc3f7b8169dc7638bf8ceafa095b1bded169c8977d5828a429bd84930b1e1f43f5b95122",
];

/// An unset non-nullable `MsgTransfer.timeout_height` is `{}` in v0.46's
/// legacy amino JSON (x/tx refuses it): the mainnet signature verifies.
#[test]
fn v046_amino_json_writes_an_unset_non_nullable_message_as_empty() {
    let raw = hex::decode(TX.concat()).unwrap();
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let tx = txdecode::decode_v1(&raw).unwrap();
    let sd = x_ante::signing::SignerData {
        address: "celestia16jnq6g5wvgjh9xs6t6d7ak4f38h4chx6sry92d".into(),
        chain_id: "celestia".into(),
        account_number: 33227,
        sequence: 1,
        pub_key: celestia_proto::Any::default(),
    };
    let bytes = x_ante::aminojson::sign_bytes(&sd, &tx).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.contains(r#""timeout_height":{}"#), "{text}");
    let raw_tx = TxRaw::decode(raw.as_slice()).unwrap();
    let auth = AuthInfo::decode(raw_tx.auth_info_bytes.as_slice()).unwrap();
    let pk_any = auth.signer_infos[0].public_key.clone().unwrap();
    let pk = x_ante::pubkey::PubKey::from_any(&pk_any).unwrap();
    assert!(pk.verify_signature(&bytes, &raw_tx.signatures[0]).unwrap());
}

/// Mainnet block 558496, tx 2: a `MsgExec` of an ibc-go v6 `MsgTransfer`
/// (with `timeout_height` and a memo), signed with
/// `SIGN_MODE_LEGACY_AMINO_JSON` by account 259624 at sequence 1375 (Go
/// accepted it).
const EXEC_TRANSFER_TX: &[&str] = &[
    "0a9f030a9c030a1d2f636f736d6f732e617574687a2e763162657461312e4d73674578656312fa020a2f63656c657374",
    "696131666c38357168397077346a7534387a793265717238683068366438687767707135383830776412c6020a292f69",
    "62632e6170706c69636174696f6e732e7472616e736665722e76312e4d73675472616e736665721298020a087472616e",
    "7366657212096368616e6e656c2d321a100a047574696112083330363233333339222f63656c65737469613176723030",
    "656772636b38613064617836386667676c726d336e387634797a39776a6a37636a322a3f6f736d6f3166357666637068",
    "32647666657163716b6865747776373566646136397a3765356332646c646d336b76676a323363726b76367771636e34",
    "3761303207080110e9efa90642747b227761736d223a7b22636f6e7472616374223a226f736d6f316635766663706832",
    "647666657163716b6865747776373566646136397a3765356332646c646d336b76676a323363726b76367771636e3437",
    "6130222c226d7367223a7b22726563656976655f72657761726473223a7b7d7d7d7d12670a510a460a1f2f636f736d6f",
    "732e63727970746f2e736563703235366b312e5075624b657912230a21034c2b762207b57e9de1c299e47fb238fa8728",
    "0a601ea93919a0a0741aeca5c13412040a02087f18df0a12120a0c0a047574696112043234373710b7c7071a40240c57",
    "73d496abfea2ef5d72ebb53bf42be5cfff53d41a517d234443a430cf4074c90667f8a97dfc8e2f0da099e9d6e7e9d278",
    "419f13158c5c83e3a94c777fbc",
];

/// v0.46 `MsgExec.GetSignBytes` marshals its messages with the authz
/// amino codec, where ibc-go v6 does not register `MsgTransfer`: go-amino
/// writes an unregistered concrete type as its bare value, without the
/// `{"type","value"}` wrapper. The mainnet signature verifies.
#[test]
fn v046_amino_json_writes_an_unregistered_exec_message_bare() {
    let raw = hex::decode(EXEC_TRANSFER_TX.concat()).unwrap();
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let tx = txdecode::decode_v1(&raw).unwrap();
    let sd = x_ante::signing::SignerData {
        address: "celestia1fl85qh9pw4ju48zy2eqr8h0h6d8hwgpq5880wd".into(),
        chain_id: "celestia".into(),
        account_number: 259_624,
        sequence: 1375,
        pub_key: celestia_proto::Any::default(),
    };
    let bytes = x_ante::aminojson::sign_bytes(&sd, &tx).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.contains(r#""type":"cosmos-sdk/MsgExec""#), "{text}");
    assert!(!text.contains("cosmos-sdk/MsgTransfer"), "{text}");
    let raw_tx = TxRaw::decode(raw.as_slice()).unwrap();
    let auth = AuthInfo::decode(raw_tx.auth_info_bytes.as_slice()).unwrap();
    let pk_any = auth.signer_infos[0].public_key.clone().unwrap();
    let pk = x_ante::pubkey::PubKey::from_any(&pk_any).unwrap();
    assert!(
        pk.verify_signature(&bytes, &raw_tx.signatures[0]).unwrap(),
        "{text}"
    );
}
