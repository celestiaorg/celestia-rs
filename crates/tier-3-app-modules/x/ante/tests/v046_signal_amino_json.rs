//! cosmos-sdk v0.46 `SIGN_MODE_LEGACY_AMINO_JSON` sign bytes of celestia's
//! signal messages: their `ModuleCdc` is a `ProtoCodec`, so
//! `GetSignBytes` is sorted proto3 JSON (gogo jsonpb, `OrigName`,
//! `EmitDefaults`), not amino JSON. Mainnet's version 3 signalling (app
//! version 2) included amino-signed transactions; Go accepted these.

#![allow(clippy::unwrap_used)]

use x_ante::txview;

/// Mainnet block 6500891, tx 0: `MsgSignalVersion` (version 4), signed by
/// one secp256k1 key in `SIGN_MODE_LEGACY_AMINO_JSON`, account 106236,
/// sequence 13.
const SIGNAL_SINGLE: &[&str] = &[
    "0a640a620a242f63656c65737469612e7369676e616c2e76312e4d73675369676e616c56657273696f6e123a0a366365",
    "6c657374696176616c6f706572316b6e6e3838796c30386374736472747876667033396a797774377270683970747637",
    "35333279100412680a500a460a1f2f636f736d6f732e63727970746f2e736563703235366b312e5075624b657912230a",
    "21039169e043b926b78c4c798f8a2d5da32b384f77c11aa52d4a0696de1546bdff9712040a02087f180d12140a0e0a04",
    "75746961120632313030303010d0e80c1a4017f888e21079ddf15b767f0948611b8428ed377d107e7617092fa40b7cbd",
    "dc311e0d5605ae6129549e899b18d6ff34fd3acedd6cd0e2102024ee0de8a380a7bb",
];

/// Mainnet block 6517081, tx 0: `MsgSignalVersion` from a
/// `LegacyAminoPubKey` account with three amino sub-signatures, account
/// 1626273, sequence 1.
const SIGNAL_MULTISIG: &[&str] = &[
    "0a640a620a242f63656c65737469612e7369676e616c2e76312e4d73675369676e616c56657273696f6e123a0a366365",
    "6c657374696176616c6f706572313376326a7a6d30773879757737703330636334683470337368763276643773713037",
    "657433711004128b030af2020ad0020a292f636f736d6f732e63727970746f2e6d756c74697369672e4c656761637941",
    "6d696e6f5075624b657912a202080312460a1f2f636f736d6f732e63727970746f2e736563703235366b312e5075624b",
    "657912230a21033055886900d975fd84e86b5c9439bacc290f67372af053cb3cf6dad65487887312460a1f2f636f736d",
    "6f732e63727970746f2e736563703235366b312e5075624b657912230a2102761f3a66fa4f83f0a2a0366203a54dab71",
    "862a320824c135aca5d5ad30c2996412460a1f2f636f736d6f732e63727970746f2e736563703235366b312e5075624b",
    "657912230a2103f7313315fc0ac8e87b0063a4099fedaf3540aea924517b2bcc8d83745ea4371e12460a1f2f636f736d",
    "6f732e63727970746f2e736563703235366b312e5075624b657912230a2103c206f32b295cee7c21b23924b91d5683a6",
    "b52991267603e141dbf6ed5d996663121b12190a05080412017012040a02087f12040a02087f12040a02087f18011214",
    "0a0e0a0475746961120632313030303010c09a0c1ac6010a40d68bf29ae01c003734b870541973e9ac655ce67c5a9df5",
    "c785510ea986b5e3566692527862227a57d5f1c5f58ececaecc0c000457fd005198bcef754187ab7cc0a40f53db5734a",
    "bfc915a758927733f3cf4a3afe4c91aad11901dda4d782f05c2569792c940b2ce72c79bd57fd3ebc5616dbf67dbf8d92",
    "ac99b69845852c2b88e9c40a40d4f55d062264bc733e86b93e88ca182fafee9d66c1dc726a3437136257ed1746431af3",
    "e29985ab927d292c3026f60ab5b3edfe19b741bb08a47f2daec3f2d17a",
];

/// Mainnet block 6579539, tx 1: the `MsgTryUpgrade` that scheduled app
/// version 3, signed in `SIGN_MODE_LEGACY_AMINO_JSON`, account 99629,
/// sequence 32.
const TRY_UPGRADE: &[&str] = &[
    "0a580a560a212f63656c65737469612e7369676e616c2e76312e4d73675472795570677261646512310a2f63656c6573",
    "746961316a6b7577387278787273676e3970713030393938376b7a656c6b703436636763637a7578703512680a500a46",
    "0a1f2f636f736d6f732e63727970746f2e736563703235366b312e5075624b657912230a21026c72bc1372b429706e67",
    "5de40e53ccac5500d01f70a239cd7a2961bff202bffc12040a02087f182012140a0e0a04757469611206313030303030",
    "10c0843d1a40edae4ebed99fa45469845209c5c6dd2471d838dfc18ed13ded30c05d4aa864c76537ac47fbc95db83e92",
    "c519cf59b5fc92ca3164d996d51aa1963c0b94b0d96f",
];

/// The sign bytes of `hex` at app version 2 for the given signer, after
/// verifying the transaction's signature with them.
fn verify(hex_tx: &[&str], address: &str, account_number: u64, sequence: u64) -> String {
    let raw = hex::decode(hex_tx.concat()).unwrap();
    let _v046 = sdk::version::SdkV046Scope::enter(2);
    let tx = txdecode::decode_v1(&raw).unwrap();
    let pk = txview::pub_keys(&tx).unwrap()[0].clone().unwrap();
    let sd = x_ante::signing::SignerData {
        address: address.into(),
        chain_id: "celestia".into(),
        account_number,
        sequence,
        pub_key: pk.to_any(),
    };
    let si = &tx.auth_info.signer_infos[0];
    let data =
        txview::mode_info_to_sig_data(si.mode_info.as_ref().unwrap(), &tx.signatures[0]).unwrap();
    let verified = x_ante::signing::verify_signature(&pk, &sd, Some(&data), &tx).unwrap();
    assert_eq!(verified, Ok(()), "{address}");
    String::from_utf8(x_ante::aminojson::sign_bytes(&sd, &tx).unwrap()).unwrap()
}

#[test]
fn v046_amino_json_signs_signal_version_as_proto_json() {
    let text = verify(
        SIGNAL_SINGLE,
        "celestia1knn88yl08ctsdrtxvfp39jywt7rph9ptfpkguz",
        106_236,
        13,
    );
    assert!(
        text.contains(r#""msgs":[{"validator_address":"celestiavaloper1knn88yl08ctsdrtxvfp39jywt7rph9ptv7532y","version":"4"}]"#),
        "{text}"
    );
}

#[test]
fn v046_amino_json_verifies_a_multisig_signal_version() {
    verify(
        SIGNAL_MULTISIG,
        "celestia13v2jzm0w8yuw7p30cc4h4p3shv2vd7sq2pmj8x",
        1_626_273,
        1,
    );
}

#[test]
fn v046_amino_json_signs_try_upgrade_as_proto_json() {
    let text = verify(
        TRY_UPGRADE,
        "celestia1jkuw8rxxrsgn9pq009987kzelkp46cgcczuxp5",
        99_629,
        32,
    );
    assert!(
        text.contains(r#""msgs":[{"signer":"celestia1jkuw8rxxrsgn9pq009987kzelkp46cgcczuxp5"}]"#),
        "{text}"
    );
}
