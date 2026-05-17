//! Conformance tests for cosmos-types against committed fixtures.

#![allow(clippy::expect_used, clippy::panic)]

use cosmos_types::{
    Any, AuthInfo, Coin, Error, ModeInfo, SignDoc, SignerInfo, TxBody, TxRaw,
    account_address_from_bech32, account_address_to_bech32, proto, secp256k1_public_key_any,
    valcons_address_from_bech32, valcons_address_to_bech32, valoper_address_from_bech32,
    valoper_address_to_bech32,
};
use prost::Message;
use serde::Deserialize;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    let mut dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for _ in 0..10 {
        let manifest = dir.join("Cargo.toml");
        if manifest.exists() {
            let body =
                std::fs::read_to_string(&manifest).expect("read candidate workspace Cargo.toml");
            if body.contains("[workspace]") {
                return dir;
            }
        }
        if !dir.pop() {
            break;
        }
    }
    panic!("no workspace Cargo.toml with [workspace] found above CARGO_MANIFEST_DIR")
}

fn load_fixture<T>(relative_path: &str) -> T
where
    T: for<'de> Deserialize<'de>,
{
    let path = workspace_root()
        .join("conformance/fixtures")
        .join(relative_path);
    let body =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&body).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

#[derive(Deserialize)]
struct AddressFixture {
    cases: Vec<AddressCase>,
}

#[derive(Deserialize)]
struct AddressCase {
    label: String,
    kind: String,
    address_hex: String,
    bech32: String,
}

#[derive(Deserialize)]
struct CoinFixture {
    valid_cases: Vec<CoinCase>,
    malformed_cases: Vec<MalformedCase>,
}

#[derive(Deserialize)]
struct CoinCase {
    label: String,
    denom: String,
    amount: String,
    proto_hex: String,
}

#[derive(Deserialize)]
struct PublicKeyFixture {
    cases: Vec<PublicKeyCase>,
}

#[derive(Deserialize)]
struct PublicKeyCase {
    label: String,
    public_key_hex: String,
    address_hex: String,
    any_type_url: String,
    any_value_hex: String,
    any_proto_hex: String,
}

#[derive(Deserialize)]
struct SignDocFixture {
    cases: Vec<SignDocCase>,
}

#[derive(Deserialize)]
struct SignDocCase {
    label: String,
    chain_id: String,
    account_number: u64,
    body_bytes_hex: String,
    auth_info_bytes_hex: String,
    sign_doc_proto_hex: String,
    sign_bytes_hex: String,
}

#[derive(Deserialize)]
struct TxRawFixture {
    cases: Vec<TxRawCase>,
}

#[derive(Deserialize)]
struct TxRawCase {
    label: String,
    body_bytes_hex: String,
    auth_info_bytes_hex: String,
    signatures_hex: Vec<String>,
    tx_raw_proto_hex: String,
    tx_hash_hex: String,
}

#[derive(Deserialize)]
struct MalformedFixture {
    cases: Vec<MalformedCase>,
}

#[derive(Deserialize)]
struct MalformedCase {
    label: String,
    kind: String,
    value: Option<String>,
    proto_hex: Option<String>,
    expected_error: String,
}

#[test]
fn conformance_addresses() {
    let fixture: AddressFixture = load_fixture("cosmos-types/addresses.json");

    for case in &fixture.cases {
        let address = decode_array::<20>(&case.address_hex);
        match case.kind.as_str() {
            "account" => {
                assert_eq!(account_address_to_bech32(&address), case.bech32);
                assert_eq!(
                    account_address_from_bech32(&case.bech32).expect("decode account address"),
                    address
                );
            }
            "valoper" => {
                assert_eq!(valoper_address_to_bech32(&address), case.bech32);
                assert_eq!(
                    valoper_address_from_bech32(&case.bech32).expect("decode valoper address"),
                    address
                );
            }
            "valcons" => {
                assert_eq!(valcons_address_to_bech32(&address), case.bech32);
                assert_eq!(
                    valcons_address_from_bech32(&case.bech32).expect("decode valcons address"),
                    address
                );
            }
            other => panic!("unknown address kind '{other}' for '{}'", case.label),
        }
    }
}

#[test]
fn conformance_coins() {
    let fixture: CoinFixture = load_fixture("cosmos-types/coins.json");

    for case in &fixture.valid_cases {
        let encoded = hex::decode(&case.proto_hex).expect("valid coin protobuf hex");
        let proto = decode_proto::<proto::base::Coin>(&encoded)
            .unwrap_or_else(|e| panic!("decode coin for '{}': {e}", case.label));
        let coin = Coin::from_proto(proto)
            .unwrap_or_else(|e| panic!("coin from proto for '{}': {e}", case.label));
        assert_eq!(coin.denom, case.denom);
        assert_eq!(coin.amount, case.amount);
        assert_eq!(
            hex::encode(coin.to_proto().encode_to_vec()),
            case.proto_hex,
            "coin protobuf round-trip mismatch for '{}'",
            case.label
        );
    }

    for case in &fixture.malformed_cases {
        assert_malformed(case);
    }
}

#[test]
fn conformance_public_keys() {
    let fixture: PublicKeyFixture = load_fixture("cosmos-types/public-keys.json");

    for case in &fixture.cases {
        let public_key = decode_array::<33>(&case.public_key_hex);
        assert_eq!(
            hex::encode(crypto::secp256k1::address(&public_key)),
            case.address_hex,
            "public key address mismatch for '{}'",
            case.label
        );

        let packed = secp256k1_public_key_any(&public_key);
        assert_eq!(packed.type_url, case.any_type_url);
        assert_eq!(hex::encode(&packed.value), case.any_value_hex);

        let encoded = hex::decode(&case.any_proto_hex).expect("valid Any protobuf hex");
        let any = Any::from_proto(
            decode_proto::<proto::google::Any>(&encoded)
                .unwrap_or_else(|e| panic!("decode Any for '{}': {e}", case.label)),
        )
        .unwrap_or_else(|e| panic!("Any from proto for '{}': {e}", case.label));
        assert_eq!(any, packed);
    }
}

#[test]
fn conformance_sign_docs() {
    let fixture: SignDocFixture = load_fixture("cosmos-types/sign-docs.json");

    for case in &fixture.cases {
        let encoded = hex::decode(&case.sign_doc_proto_hex).expect("valid sign doc protobuf hex");
        let sign_doc = SignDoc::decode(&encoded)
            .unwrap_or_else(|e| panic!("decode sign doc for '{}': {e}", case.label));
        assert_eq!(hex::encode(&sign_doc.body_bytes), case.body_bytes_hex);
        assert_eq!(
            hex::encode(&sign_doc.auth_info_bytes),
            case.auth_info_bytes_hex
        );
        assert_eq!(sign_doc.chain_id, case.chain_id);
        assert_eq!(sign_doc.account_number, case.account_number);
        assert_eq!(
            hex::encode(sign_doc.sign_bytes().expect("sign bytes")),
            case.sign_bytes_hex,
            "sign bytes mismatch for '{}'",
            case.label
        );

        TxBody::from_proto(
            decode_proto::<proto::tx::TxBody>(&sign_doc.body_bytes)
                .expect("decode body bytes from sign doc"),
        )
        .expect("body bytes validate");
        AuthInfo::from_proto(
            decode_proto::<proto::tx::AuthInfo>(&sign_doc.auth_info_bytes)
                .expect("decode auth info bytes from sign doc"),
        )
        .expect("auth info bytes validate");
    }
}

#[test]
fn conformance_tx_raw() {
    let fixture: TxRawFixture = load_fixture("cosmos-types/tx-raw.json");

    for case in &fixture.cases {
        let encoded = hex::decode(&case.tx_raw_proto_hex).expect("valid tx raw protobuf hex");
        let tx_raw = TxRaw::decode(&encoded)
            .unwrap_or_else(|e| panic!("decode tx raw for '{}': {e}", case.label));
        assert_eq!(hex::encode(&tx_raw.body_bytes), case.body_bytes_hex);
        assert_eq!(
            hex::encode(&tx_raw.auth_info_bytes),
            case.auth_info_bytes_hex
        );
        assert_eq!(
            tx_raw
                .signatures
                .iter()
                .map(hex::encode)
                .collect::<Vec<_>>(),
            case.signatures_hex
        );
        assert_eq!(
            hex::encode(tx_raw.encode_to_vec().expect("encode tx raw")),
            case.tx_raw_proto_hex,
            "tx raw protobuf round-trip mismatch for '{}'",
            case.label
        );
        assert_eq!(
            hex::encode(tx_raw.hash().expect("tx hash")),
            case.tx_hash_hex,
            "tx hash mismatch for '{}'",
            case.label
        );
    }
}

#[test]
fn conformance_malformed() {
    let fixture: MalformedFixture = load_fixture("cosmos-types/malformed.json");

    for case in &fixture.cases {
        assert_malformed(case);
    }
}

fn assert_malformed(case: &MalformedCase) {
    let result = malformed_result(case);
    let Err(error) = result else {
        panic!("malformed case '{}' unexpectedly succeeded", case.label);
    };
    assert_eq!(
        error_code(&error),
        case.expected_error,
        "malformed error mismatch for '{}'",
        case.label
    );
}

fn malformed_result(case: &MalformedCase) -> Result<(), Error> {
    match case.kind.as_str() {
        "account_address" => account_address_from_bech32(
            case.value
                .as_deref()
                .unwrap_or_else(|| panic!("missing value for '{}'", case.label)),
        )
        .map(|_| ()),
        "any" => decode_fixture_proto::<proto::google::Any>(case)
            .and_then(Any::from_proto)
            .map(|_| ()),
        "coin" => decode_fixture_proto::<proto::base::Coin>(case)
            .and_then(Coin::from_proto)
            .map(|_| ()),
        "mode_info" => decode_fixture_proto::<proto::tx::ModeInfo>(case)
            .and_then(ModeInfo::from_proto)
            .map(|_| ()),
        "signer_info" => decode_fixture_proto::<proto::tx::SignerInfo>(case)
            .and_then(SignerInfo::from_proto)
            .map(|_| ()),
        "sign_doc" => {
            let encoded = decode_fixture_hex(case);
            SignDoc::decode(&encoded).map(|_| ())
        }
        "tx_raw" => {
            let encoded = decode_fixture_hex(case);
            TxRaw::decode(&encoded).map(|_| ())
        }
        other => panic!("unknown malformed kind '{other}'"),
    }
}

fn decode_fixture_proto<T>(case: &MalformedCase) -> Result<T, Error>
where
    T: Message + Default,
{
    let encoded = decode_fixture_hex(case);
    decode_proto(&encoded)
}

fn decode_fixture_hex(case: &MalformedCase) -> Vec<u8> {
    hex::decode(
        case.proto_hex
            .as_deref()
            .unwrap_or_else(|| panic!("missing proto_hex for '{}'", case.label)),
    )
    .expect("valid malformed protobuf hex")
}

fn decode_proto<T>(bytes: &[u8]) -> Result<T, Error>
where
    T: Message + Default,
{
    <T as Message>::decode(bytes).map_err(|e| Error::Decode(e.to_string()))
}

fn decode_array<const N: usize>(hex: &str) -> [u8; N] {
    let bytes = hex::decode(hex).expect("valid fixture hex");
    assert_eq!(bytes.len(), N, "fixture byte length");
    bytes
        .try_into()
        .unwrap_or_else(|_| panic!("{N}-byte fixture"))
}

fn error_code(error: &Error) -> &'static str {
    match error {
        Error::InvalidAddressLength { .. } => "invalid_address_length",
        Error::InvalidAddress => "invalid_address",
        Error::InvalidAddressPrefix { .. } => "invalid_address_prefix",
        Error::InvalidAny => "invalid_any",
        Error::InvalidCoin => "invalid_coin",
        Error::InvalidDenom => "invalid_denom",
        Error::InvalidAmount => "invalid_amount",
        Error::InvalidFee => "invalid_fee",
        Error::InvalidModeInfo => "invalid_mode_info",
        Error::InvalidSignMode => "invalid_sign_mode",
        Error::InvalidSignerInfo => "invalid_signer_info",
        Error::InvalidSignDoc => "invalid_sign_doc",
        Error::InvalidTx => "invalid_tx",
        Error::InvalidPublicKey => "invalid_public_key",
        Error::Decode(_) => "decode",
        Error::Encode(_) => "encode",
    }
}
