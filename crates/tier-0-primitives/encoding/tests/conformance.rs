//! Conformance tests for the encoding crate against committed fixture files.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use encoding::{base64, bech32, hex, protobuf, varint};
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

#[derive(Clone, PartialEq, Message)]
struct FixtureMessage {
    #[prost(uint64, tag = "1")]
    id: u64,
    #[prost(string, tag = "2")]
    label: String,
    #[prost(bytes, tag = "3")]
    payload: Vec<u8>,
}

#[derive(Deserialize)]
struct ProtobufFixture {
    cases: Vec<ProtobufCase>,
}

#[derive(Deserialize)]
struct ProtobufCase {
    label: String,
    id: u64,
    message_label: String,
    payload_hex: String,
    wire_hex: String,
    length_delimited_hex: String,
}

#[derive(Deserialize)]
struct VarintFixture {
    unsigned_cases: Vec<UnsignedVarintCase>,
    signed_cases: Vec<SignedVarintCase>,
    malformed_uvarints: Vec<MalformedVarintCase>,
    malformed_varints: Vec<MalformedVarintCase>,
}

#[derive(Deserialize)]
struct UnsignedVarintCase {
    label: String,
    value: u64,
    encoded_hex: String,
}

#[derive(Deserialize)]
struct SignedVarintCase {
    label: String,
    value: i64,
    encoded_hex: String,
}

#[derive(Deserialize)]
struct MalformedVarintCase {
    label: String,
    input_hex: String,
    expected_error: String,
}

#[derive(Deserialize)]
struct HexBase64Fixture {
    cases: Vec<HexBase64Case>,
    invalid: Vec<InvalidTextCase>,
}

#[derive(Deserialize)]
struct HexBase64Case {
    label: String,
    input_hex: String,
    upper_hex: String,
    standard_base64: String,
    url_safe_base64: String,
}

#[derive(Deserialize)]
struct InvalidTextCase {
    label: String,
    kind: String,
    input: String,
}

#[derive(Deserialize)]
struct Bech32Fixture {
    account_prefix: String,
    validator_operator_prefix: String,
    validator_consensus_prefix: String,
    cases: Vec<Bech32Case>,
    invalid_address_length_case_hex: String,
}

#[derive(Deserialize)]
struct Bech32Case {
    label: String,
    address_hex: String,
    account_address: String,
    validator_operator_address: String,
    validator_consensus_address: String,
}

#[test]
fn conformance_protobuf() {
    let fixture: ProtobufFixture = load_fixture("encoding/protobuf.json");

    for case in &fixture.cases {
        let message = FixtureMessage {
            id: case.id,
            label: case.message_label.clone(),
            payload: hex::decode(&case.payload_hex).expect("fixture payload hex"),
        };

        let raw = protobuf::encode_to_vec(&message);
        assert_eq!(
            hex::encode(&raw),
            case.wire_hex,
            "raw protobuf mismatch for '{}'",
            case.label
        );
        assert_eq!(
            protobuf::decode::<FixtureMessage>(&raw).expect("decode raw protobuf"),
            message,
            "raw protobuf decode mismatch for '{}'",
            case.label
        );

        let delimited = protobuf::encode_length_delimited_to_vec(&message);
        assert_eq!(
            hex::encode(&delimited),
            case.length_delimited_hex,
            "length-delimited protobuf mismatch for '{}'",
            case.label
        );
        assert_eq!(
            protobuf::decode_length_delimited::<FixtureMessage>(&delimited)
                .expect("decode length-delimited protobuf"),
            message,
            "length-delimited protobuf decode mismatch for '{}'",
            case.label
        );
    }
}

#[test]
fn conformance_varint() {
    let fixture: VarintFixture = load_fixture("encoding/varint.json");

    for case in &fixture.unsigned_cases {
        let encoded = varint::encode_uvarint(case.value);
        assert_eq!(
            hex::encode(&encoded),
            case.encoded_hex,
            "uvarint encoding mismatch for '{}'",
            case.label
        );
        assert_eq!(
            varint::decode_uvarint(&encoded).expect("decode uvarint"),
            (case.value, encoded.len()),
            "uvarint decode mismatch for '{}'",
            case.label
        );
    }

    for case in &fixture.signed_cases {
        let encoded = varint::encode_varint(case.value);
        assert_eq!(
            hex::encode(&encoded),
            case.encoded_hex,
            "varint encoding mismatch for '{}'",
            case.label
        );
        assert_eq!(
            varint::decode_varint(&encoded).expect("decode varint"),
            (case.value, encoded.len()),
            "varint decode mismatch for '{}'",
            case.label
        );
    }

    for case in &fixture.malformed_uvarints {
        let bytes = hex::decode(&case.input_hex).expect("fixture malformed hex");
        assert_eq!(
            varint::decode_uvarint(&bytes),
            Err(expected_varint_error(&case.expected_error)),
            "malformed uvarint mismatch for '{}'",
            case.label
        );
    }

    for case in &fixture.malformed_varints {
        let bytes = hex::decode(&case.input_hex).expect("fixture malformed hex");
        assert_eq!(
            varint::decode_varint(&bytes),
            Err(expected_varint_error(&case.expected_error)),
            "malformed varint mismatch for '{}'",
            case.label
        );
    }
}

#[test]
fn conformance_hex_base64() {
    let fixture: HexBase64Fixture = load_fixture("encoding/hex-base64.json");

    for case in &fixture.cases {
        let bytes = hex::decode(&case.input_hex).expect("fixture input hex");
        assert_eq!(
            hex::encode(&bytes),
            case.input_hex,
            "lowercase hex mismatch for '{}'",
            case.label
        );
        assert_eq!(
            hex::decode(&case.upper_hex).expect("fixture upper hex"),
            bytes,
            "uppercase hex decode mismatch for '{}'",
            case.label
        );
        assert_eq!(
            base64::encode_standard(&bytes),
            case.standard_base64,
            "standard base64 mismatch for '{}'",
            case.label
        );
        assert_eq!(
            base64::decode_standard(&case.standard_base64).expect("fixture standard base64"),
            bytes,
            "standard base64 decode mismatch for '{}'",
            case.label
        );
        assert_eq!(
            base64::encode_url_safe(&bytes),
            case.url_safe_base64,
            "url-safe base64 mismatch for '{}'",
            case.label
        );
        assert_eq!(
            base64::decode_url_safe(&case.url_safe_base64).expect("fixture url-safe base64"),
            bytes,
            "url-safe base64 decode mismatch for '{}'",
            case.label
        );
    }

    for case in &fixture.invalid {
        match case.kind.as_str() {
            "hex" => assert!(hex::decode(&case.input).is_err(), "{}", case.label),
            "standard_base64" => {
                assert!(
                    base64::decode_standard(&case.input).is_err(),
                    "{}",
                    case.label
                );
            }
            "url_safe_base64" => {
                assert!(
                    base64::decode_url_safe(&case.input).is_err(),
                    "{}",
                    case.label
                );
            }
            other => panic!("unknown invalid fixture kind {other}"),
        }
    }
}

#[test]
fn conformance_bech32_addresses() {
    let fixture: Bech32Fixture = load_fixture("encoding/bech32-addresses.json");
    assert_eq!(fixture.account_prefix, bech32::ACCOUNT_PREFIX);
    assert_eq!(
        fixture.validator_operator_prefix,
        bech32::VALIDATOR_OPERATOR_PREFIX
    );
    assert_eq!(
        fixture.validator_consensus_prefix,
        bech32::VALIDATOR_CONSENSUS_PREFIX
    );

    for case in &fixture.cases {
        let bytes = address_bytes(&case.address_hex);
        assert_eq!(
            bech32::encode_account_address(&bytes).expect("encode account"),
            case.account_address,
            "account bech32 mismatch for '{}'",
            case.label
        );
        assert_eq!(
            bech32::encode_validator_operator_address(&bytes).expect("encode operator"),
            case.validator_operator_address,
            "validator operator bech32 mismatch for '{}'",
            case.label
        );
        assert_eq!(
            bech32::encode_validator_consensus_address(&bytes).expect("encode consensus"),
            case.validator_consensus_address,
            "validator consensus bech32 mismatch for '{}'",
            case.label
        );
        assert_eq!(
            bech32::decode_account_address(&case.account_address).expect("decode account"),
            bytes,
            "account bech32 decode mismatch for '{}'",
            case.label
        );
        assert_eq!(
            bech32::decode_validator_operator_address(&case.validator_operator_address)
                .expect("decode operator"),
            bytes,
            "validator operator bech32 decode mismatch for '{}'",
            case.label
        );
        assert_eq!(
            bech32::decode_validator_consensus_address(&case.validator_consensus_address)
                .expect("decode consensus"),
            bytes,
            "validator consensus bech32 decode mismatch for '{}'",
            case.label
        );
        assert!(matches!(
            bech32::decode_account_address(&case.validator_operator_address),
            Err(bech32::Error::WrongPrefix { .. })
        ));
    }

    let short = hex::decode(&fixture.invalid_address_length_case_hex).expect("short address hex");
    let encoded_short = bech32::encode(bech32::ACCOUNT_PREFIX, &short).expect("encode short");
    assert!(matches!(
        bech32::decode_account_address(&encoded_short),
        Err(bech32::Error::WrongLength {
            expected: bech32::ADDRESS_LENGTH,
            actual: 3
        })
    ));
}

fn expected_varint_error(value: &str) -> varint::Error {
    match value {
        "incomplete" => varint::Error::Incomplete,
        "overflow" => varint::Error::Overflow,
        "non_canonical" => varint::Error::NonCanonical,
        other => panic!("unknown varint error {other}"),
    }
}

fn address_bytes(input_hex: &str) -> bech32::AddressBytes {
    let bytes = hex::decode(input_hex).expect("fixture address hex");
    let mut out = [0u8; bech32::ADDRESS_LENGTH];
    out.copy_from_slice(&bytes);
    out
}
