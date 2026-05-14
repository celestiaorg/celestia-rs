//! Property tests for the encoding crate.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use encoding::{protobuf, varint};
use proptest::prelude::*;
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

fn ascii_string() -> impl Strategy<Value = String> {
    prop::collection::vec(0x20u8..=0x7eu8, 0..=32)
        .prop_map(|bytes| String::from_utf8(bytes).expect("ASCII bytes are valid UTF-8"))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn unsigned_varint_round_trips(value in any::<u64>()) {
        let encoded = varint::encode_uvarint(value);
        prop_assert_eq!(varint::decode_uvarint(&encoded), Ok((value, encoded.len())));
        prop_assert_eq!(varint::encoded_uvarint_len(value), encoded.len());

        let mut buf = [0u8; varint::MAX_VARINT_LEN64];
        let written = varint::put_uvarint(&mut buf, value).expect("10-byte buffer is enough");
        prop_assert_eq!(written, encoded.len());
        prop_assert_eq!(&buf[..written], encoded.as_slice());
    }

    #[test]
    fn signed_varint_round_trips(value in any::<i64>()) {
        let encoded = varint::encode_varint(value);
        prop_assert_eq!(varint::decode_varint(&encoded), Ok((value, encoded.len())));
        prop_assert_eq!(varint::encoded_varint_len(value), encoded.len());

        let mut buf = [0u8; varint::MAX_VARINT_LEN64];
        let written = varint::put_varint(&mut buf, value).expect("10-byte buffer is enough");
        prop_assert_eq!(written, encoded.len());
        prop_assert_eq!(&buf[..written], encoded.as_slice());
    }

    #[test]
    fn protobuf_helpers_round_trip(id in any::<u64>(), label in ascii_string(), payload in prop::collection::vec(any::<u8>(), 0..=512)) {
        let message = FixtureMessage { id, label, payload };

        let raw = protobuf::encode_to_vec(&message);
        let raw_decoded = protobuf::decode::<FixtureMessage>(&raw).expect("raw protobuf decodes");
        prop_assert!(raw_decoded == message);

        let delimited = protobuf::encode_length_delimited_to_vec(&message);
        let delimited_decoded = protobuf::decode_length_delimited::<FixtureMessage>(&delimited)
            .expect("length-delimited protobuf decodes");
        prop_assert!(delimited_decoded == message);
    }
}
