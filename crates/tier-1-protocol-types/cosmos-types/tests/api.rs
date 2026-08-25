//! API tests for the cosmos-types crate.

#![allow(clippy::expect_used)]

use cosmos_types::{
    Any, AuthInfo, Coin, Error, Fee, ModeInfo, SignDoc, SignMode, SignerInfo, Tip, Tx, TxBody,
    TxRaw, account_address_from_bech32, account_address_to_bech32, proto, secp256k1_public_key_any,
    valoper_address_from_bech32,
};
use prost::Message;

fn public_key() -> crypto::secp256k1::PublicKeyBytes {
    let secret_key = crypto::secp256k1::secret_key_from_secret(b"cosmos-types-api-test");
    crypto::secp256k1::public_key_from_secret_key(&secret_key).expect("public key")
}

fn message_any() -> Any {
    Any {
        type_url: "/cosmos.bank.v1beta1.MsgSend".to_owned(),
        value: vec![1, 2, 3],
    }
}

fn valid_body() -> TxBody {
    TxBody {
        messages: vec![message_any()],
        memo: "memo".to_owned(),
        timeout_height: 12,
        extension_options: Vec::new(),
        non_critical_extension_options: Vec::new(),
    }
}

fn valid_fee() -> Fee {
    Fee {
        amount: vec![Coin::new("utia", "123").expect("valid coin")],
        gas_limit: 80_000,
        payer: String::new(),
        granter: String::new(),
    }
}

fn valid_auth_info() -> AuthInfo {
    AuthInfo {
        signer_infos: vec![SignerInfo {
            public_key: Some(secp256k1_public_key_any(&public_key())),
            mode_info: ModeInfo::Single {
                mode: SignMode::Direct,
            },
            sequence: 7,
        }],
        fee: Some(valid_fee()),
        tip: None,
    }
}

fn encoded_body() -> Vec<u8> {
    valid_body().to_proto().encode_to_vec()
}

fn encoded_auth_info() -> Vec<u8> {
    valid_auth_info().to_proto().encode_to_vec()
}

#[test]
fn error_display_messages_are_stable() {
    let cases = [
        (
            Error::InvalidAddressLength { actual: 3 },
            "invalid Cosmos address length: 3",
        ),
        (Error::InvalidAddress, "invalid Cosmos address"),
        (
            Error::InvalidAddressPrefix {
                expected: "celestia",
                actual: "other".to_owned(),
            },
            "invalid Cosmos address prefix: expected celestia, got other",
        ),
        (Error::InvalidAny, "invalid Cosmos Any"),
        (Error::InvalidCoin, "invalid Cosmos coin"),
        (Error::InvalidDenom, "invalid Cosmos denom"),
        (Error::InvalidAmount, "invalid Cosmos amount"),
        (Error::InvalidFee, "invalid Cosmos fee"),
        (Error::InvalidModeInfo, "invalid Cosmos mode info"),
        (Error::InvalidSignMode, "invalid Cosmos sign mode"),
        (Error::InvalidSignerInfo, "invalid Cosmos signer info"),
        (Error::InvalidSignDoc, "invalid Cosmos sign doc"),
        (Error::InvalidTx, "invalid Cosmos tx"),
        (Error::InvalidPublicKey, "invalid Cosmos public key"),
        (
            Error::Decode("boom".to_owned()),
            "decode Cosmos protobuf: boom",
        ),
        (
            Error::Encode("boom".to_owned()),
            "encode Cosmos protobuf: boom",
        ),
    ];

    for (error, message) in cases {
        assert_eq!(error.to_string(), message);
    }
}

#[test]
fn address_helpers_round_trip_and_reject_wrong_prefix() {
    let address = [7u8; cosmos_types::ADDRESS_LENGTH];
    let encoded = account_address_to_bech32(&address);
    assert_eq!(
        account_address_from_bech32(&encoded).expect("decode account address"),
        address
    );

    assert_eq!(
        valoper_address_from_bech32(&encoded),
        Err(Error::InvalidAddressPrefix {
            expected: "celestiavaloper",
            actual: "celestia".to_owned(),
        })
    );
    assert_eq!(
        account_address_from_bech32("not-bech32"),
        Err(Error::InvalidAddress)
    );
}

#[test]
fn module_address_matches_cosmos_sdk_legacy_derivation() {
    assert_eq!(
        hex::encode(cosmos_types::module_address("fee_collector")),
        "f1829676db577682e944fc3493d451b67ff3e29f"
    );
}

#[test]
fn coin_validates_and_round_trips_proto() {
    let coin = Coin::new("transfer/channel-0/utia", "123").expect("valid coin");
    assert_eq!(
        Coin::from_proto(coin.to_proto()).expect("coin from proto"),
        coin
    );

    assert_eq!(Coin::new("1tia", "1"), Err(Error::InvalidDenom));
    assert_eq!(Coin::new("utia", "-1"), Err(Error::InvalidAmount));
    assert_eq!(Coin::new("utia", "1".repeat(79)), Err(Error::InvalidAmount));
}

#[test]
fn any_validates_and_public_key_packing_matches_proto() {
    let any = message_any();
    any.validate_basic().expect("valid any");
    assert_eq!(
        Any::from_proto(any.to_proto()).expect("any from proto"),
        any
    );

    assert_eq!(
        Any {
            type_url: "cosmos.bank.v1beta1.MsgSend".to_owned(),
            value: Vec::new(),
        }
        .validate_basic(),
        Err(Error::InvalidAny)
    );

    let public_key = public_key();
    let packed = secp256k1_public_key_any(&public_key);
    assert_eq!(packed.type_url, "/cosmos.crypto.secp256k1.PubKey");
    let proto = proto::crypto::secp256k1::PubKey::decode(packed.value.as_slice())
        .expect("decode public key proto");
    assert_eq!(proto.key, public_key);
}

#[test]
fn sign_modes_match_cosmos_sdk_values() {
    let cases = [
        (0, SignMode::Unspecified),
        (1, SignMode::Direct),
        (2, SignMode::Textual),
        (3, SignMode::DirectAux),
        (127, SignMode::LegacyAminoJson),
        (191, SignMode::Eip191),
    ];

    for (value, mode) in cases {
        assert_eq!(SignMode::try_from(value).expect("known mode"), mode);
        assert_eq!(i32::from(mode), value);
    }
    assert_eq!(SignMode::try_from(999), Err(Error::InvalidSignMode));
}

#[test]
fn mode_info_rejects_missing_invalid_and_round_trips_proto() {
    let mode_info = ModeInfo::Multi {
        bitarray_extra_bits_stored: 1,
        bitarray_elems: vec![0b1000_0000],
        mode_infos: vec![ModeInfo::Single {
            mode: SignMode::Direct,
        }],
    };
    assert_eq!(
        ModeInfo::from_proto(mode_info.to_proto()).expect("mode info from proto"),
        mode_info
    );

    assert_eq!(
        ModeInfo::from_proto(proto::tx::ModeInfo { sum: None }),
        Err(Error::InvalidModeInfo)
    );
    assert_eq!(
        ModeInfo::Single {
            mode: SignMode::Unspecified,
        }
        .validate_basic(),
        Err(Error::InvalidSignMode)
    );
}

#[test]
fn signer_auth_body_and_tx_round_trip_proto() {
    let body = valid_body();
    assert_eq!(
        TxBody::from_proto(body.to_proto()).expect("body from proto"),
        body
    );

    let auth_info = valid_auth_info();
    assert_eq!(
        AuthInfo::from_proto(auth_info.to_proto()).expect("auth info from proto"),
        auth_info
    );

    let tx = Tx {
        body: Some(body),
        auth_info: Some(auth_info),
        signatures: vec![vec![9; crypto::secp256k1::SIGNATURE_LENGTH]],
    };
    assert_eq!(Tx::from_proto(tx.to_proto()).expect("tx from proto"), tx);

    let mut invalid_tx = tx;
    invalid_tx.signatures.clear();
    assert_eq!(invalid_tx.validate_basic(), Err(Error::InvalidTx));
}

#[test]
fn fee_and_tip_validation_covers_coin_and_tipper_paths() {
    let fee = valid_fee();
    fee.validate_basic().expect("valid fee");
    assert_eq!(
        Fee::from_proto(fee.to_proto()).expect("fee from proto"),
        fee
    );

    let tip = Tip {
        amount: vec![Coin::new("utia", "1").expect("valid coin")],
        tipper: "celestia1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqcrl88e".to_owned(),
    };
    tip.validate_basic().expect("valid tip");
    assert_eq!(
        Tip::from_proto(tip.to_proto()).expect("tip from proto"),
        tip
    );

    let invalid_tip = Tip {
        amount: Vec::new(),
        tipper: String::new(),
    };
    assert_eq!(invalid_tip.validate_basic(), Err(Error::InvalidFee));
}

#[test]
fn tx_raw_encode_decode_hash_and_signature_count_are_deterministic() {
    let tx_raw = TxRaw {
        body_bytes: encoded_body(),
        auth_info_bytes: encoded_auth_info(),
        signatures: vec![vec![9; crypto::secp256k1::SIGNATURE_LENGTH]],
    };

    let encoded = tx_raw.encode_to_vec().expect("encode tx raw");
    assert_eq!(TxRaw::decode(&encoded).expect("decode tx raw"), tx_raw);
    assert_eq!(tx_raw.hash().expect("tx hash"), hash::sha256(&encoded));

    let mut missing_body = tx_raw.clone();
    missing_body.body_bytes.clear();
    assert_eq!(missing_body.validate_basic(), Err(Error::InvalidTx));

    let mut bad_signature_count = tx_raw;
    bad_signature_count.signatures.clear();
    assert_eq!(bad_signature_count.validate_basic(), Err(Error::InvalidTx));
}

#[test]
fn sign_doc_sign_bytes_are_encoded_proto_bytes() {
    let sign_doc = SignDoc {
        body_bytes: encoded_body(),
        auth_info_bytes: encoded_auth_info(),
        chain_id: "private-testnet".to_owned(),
        account_number: 42,
    };

    let encoded = sign_doc.encode_to_vec().expect("encode sign doc");
    assert_eq!(
        SignDoc::decode(&encoded).expect("decode sign doc"),
        sign_doc
    );
    assert_eq!(sign_doc.sign_bytes().expect("sign bytes"), encoded);

    let mut invalid = sign_doc;
    invalid.chain_id.clear();
    assert_eq!(invalid.validate_basic(), Err(Error::InvalidSignDoc));
}
