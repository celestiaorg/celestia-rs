//! API contract tests for the encoding crate.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use encoding::{base64, bech32, hex, protobuf, varint};
use prost::Message;

#[derive(Clone, PartialEq, Message)]
struct FixtureMessage {
    #[prost(uint64, tag = "1")]
    id: u64,
    #[prost(string, tag = "2")]
    label: String,
    #[prost(bytes, tag = "3")]
    payload: Vec<u8>,
}

#[test]
fn protobuf_helpers_round_trip_raw_and_length_delimited() {
    let message = FixtureMessage {
        id: 42,
        label: String::from("celestia"),
        payload: vec![0x00, 0x7f, 0x80, 0xff],
    };

    let encoded = protobuf::encode_to_vec(&message);
    assert_eq!(protobuf::decode::<FixtureMessage>(&encoded).unwrap(), message);

    let delimited = protobuf::encode_length_delimited_to_vec(&message);
    assert_eq!(
        protobuf::decode_length_delimited::<FixtureMessage>(&delimited).unwrap(),
        message
    );
}

#[test]
fn unsigned_varint_matches_go_boundaries() {
    let cases = [
        (0, "00"),
        (1, "01"),
        (127, "7f"),
        (128, "8001"),
        (255, "ff01"),
        (300, "ac02"),
        (16_383, "ff7f"),
        (16_384, "808001"),
        (u64::from(u32::MAX), "ffffffff0f"),
        (u64::MAX, "ffffffffffffffffff01"),
    ];

    for (value, expected_hex) in cases {
        let encoded = varint::encode_uvarint(value);
        assert_eq!(hex::encode(&encoded), expected_hex);
        assert_eq!(
            varint::decode_uvarint(&encoded).unwrap(),
            (value, encoded.len())
        );
        assert_eq!(varint::encoded_uvarint_len(value), encoded.len());
    }
}

#[test]
fn signed_varint_matches_go_boundaries() {
    let cases = [
        (0, "00"),
        (1, "02"),
        (-1, "01"),
        (63, "7e"),
        (64, "8001"),
        (-64, "7f"),
        (-65, "8101"),
        (i64::from(i32::MIN), "ffffffff0f"),
        (i64::from(i32::MAX), "feffffff0f"),
        (i64::MIN, "ffffffffffffffffff01"),
        (i64::MAX, "feffffffffffffffff01"),
    ];

    for (value, expected_hex) in cases {
        let encoded = varint::encode_varint(value);
        assert_eq!(hex::encode(&encoded), expected_hex);
        assert_eq!(
            varint::decode_varint(&encoded).unwrap(),
            (value, encoded.len())
        );
        assert_eq!(varint::encoded_varint_len(value), encoded.len());
    }
}

#[test]
fn varint_rejects_malformed_inputs_and_preserves_small_buffers() {
    assert_eq!(varint::decode_uvarint(&[0x80]), Err(varint::Error::Incomplete));
    assert_eq!(
        varint::decode_uvarint(&[0x80, 0x00]),
        Err(varint::Error::NonCanonical)
    );
    assert_eq!(
        varint::decode_uvarint(&[0xff; 10]),
        Err(varint::Error::Overflow)
    );

    let mut buf = [0xaa, 0xbb];
    assert_eq!(
        varint::put_uvarint(&mut buf, 16_384),
        Err(varint::Error::BufferTooSmall {
            required: 3,
            available: 2
        })
    );
    assert_eq!(buf, [0xaa, 0xbb]);
}

#[test]
fn hex_is_lowercase_and_decodes_uppercase() {
    let bytes = [0x00, 0xab, 0xcd, 0xef];
    assert_eq!(hex::encode(&bytes), "00abcdef");
    assert_eq!(hex::decode("00ABCDEF").unwrap(), bytes);
    assert!(hex::decode("abc").is_err());
}

#[test]
fn base64_standard_and_url_safe_are_padded() {
    let bytes = [0xfb, 0xff, 0xef];
    assert_eq!(base64::encode_standard(&bytes), "+//v");
    assert_eq!(base64::decode_standard("+//v").unwrap(), bytes);
    assert_eq!(base64::encode_url_safe(&bytes), "-__v");
    assert_eq!(base64::decode_url_safe("-__v").unwrap(), bytes);

    let padded = b"hi";
    assert_eq!(base64::encode_standard(padded), "aGk=");
    assert_eq!(base64::encode_url_safe(padded), "aGk=");
}

#[test]
fn bech32_address_helpers_round_trip_and_enforce_prefixes() {
    let bytes: bech32::AddressBytes = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a,
        0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14,
    ];

    let account = bech32::encode_account_address(&bytes).unwrap();
    let operator = bech32::encode_validator_operator_address(&bytes).unwrap();
    let consensus = bech32::encode_validator_consensus_address(&bytes).unwrap();

    assert!(account.starts_with("celestia1"));
    assert!(operator.starts_with("celestiavaloper1"));
    assert!(consensus.starts_with("celestiavalcons1"));
    assert_eq!(bech32::decode_account_address(&account).unwrap(), bytes);
    assert_eq!(
        bech32::decode_validator_operator_address(&operator).unwrap(),
        bytes
    );
    assert_eq!(
        bech32::decode_validator_consensus_address(&consensus).unwrap(),
        bytes
    );
    assert!(matches!(
        bech32::decode_account_address(&operator),
        Err(bech32::Error::WrongPrefix { .. })
    ));
}
