//! Public API coverage for the `square` crate.

use shares::{
    Blob, CompactShareSplitter, DEFAULT_SHARE_VERSION, Namespace, PAY_FOR_BLOB_NAMESPACE,
    SparseShareSplitter, TX_NAMESPACE,
};
use square::{
    BlobTx, Error, PROTO_BLOB_TX_TYPE_ID, Square, blob_min_square_size,
    blob_shares_used_non_interactive_defaults, empty_square, is_power_of_two, next_share_index,
    round_down_power_of_two, round_up_by_multiple_of, round_up_power_of_two,
    round_up_power_of_two_strict, size, sub_tree_width, write_square,
};

#[test]
fn power_of_two_helpers_match_go_rules() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(round_up_power_of_two(0), 1);
    assert_eq!(round_up_power_of_two(1), 1);
    assert_eq!(round_up_power_of_two(3), 4);
    assert_eq!(round_up_power_of_two(16), 16);
    assert_eq!(round_down_power_of_two(17)?, 16);
    assert_eq!(round_up_power_of_two_strict(16), 32);
    assert_eq!(round_up_power_of_two_strict(17), 32);
    assert!(is_power_of_two(128));
    assert!(!is_power_of_two(0));
    assert!(!is_power_of_two(129));
    Ok(())
}

#[test]
fn inclusion_helpers_align_blob_starts() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(blob_min_square_size(1)?, 1);
    assert_eq!(blob_min_square_size(2)?, 2);
    assert_eq!(blob_min_square_size(5)?, 4);
    assert_eq!(sub_tree_width(11, 4)?, 4);
    assert_eq!(next_share_index(13, 11, 4)?, 16);
    assert_eq!(round_up_by_multiple_of(16, 4)?, 16);
    assert_eq!(round_up_by_multiple_of(17, 4)?, 20);

    let (used, indexes) = blob_shares_used_non_interactive_defaults(3, 4, &[2, 11, 1])?;
    assert_eq!(used, 17);
    assert_eq!(indexes, vec![3, 8, 19]);
    Ok(())
}

#[test]
fn inclusion_helpers_reject_invalid_inputs() {
    assert!(blob_min_square_size(0).is_err());
    assert!(sub_tree_width(1, 0).is_err());
    assert!(sub_tree_width(0, 1).is_err());
    assert!(next_share_index(0, 0, 1).is_err());
    assert!(round_up_by_multiple_of(1, 0).is_err());
    assert!(round_down_power_of_two(0).is_err());
}

#[test]
fn blob_tx_round_trips_as_go_protobuf_shape() -> Result<(), Box<dyn std::error::Error>> {
    let namespace = Namespace::new_v0(&[1, 2, 3])?;
    let blob = Blob::new(namespace, b"blob-data".to_vec(), DEFAULT_SHARE_VERSION)?;
    let blob_tx = BlobTx::new(b"sdk-tx".to_vec(), vec![blob.clone()])?;

    let encoded = blob_tx.encode()?;
    let decoded =
        BlobTx::decode(&encoded)?.ok_or_else(|| std::io::Error::other("expected blob tx"))?;

    assert_eq!(decoded.tx, b"sdk-tx");
    assert_eq!(decoded.blobs, vec![blob]);
    assert!(BlobTx::decode(b"not protobuf")?.is_none());
    Ok(())
}

#[test]
fn blob_tx_decode_handles_non_wrappers_and_malformed_wrappers()
-> Result<(), Box<dyn std::error::Error>> {
    assert!(BlobTx::new(Vec::new(), Vec::new()).is_err());

    let not_blob = encode_test_blob_tx(&TestBlobTxProto {
        tx: b"tx".to_vec(),
        blobs: Vec::new(),
        type_id: "NOPE".to_owned(),
    })?;
    assert!(BlobTx::decode(&not_blob)?.is_none());

    let empty_blob_list = encode_test_blob_tx(&TestBlobTxProto {
        tx: b"tx".to_vec(),
        blobs: Vec::new(),
        type_id: PROTO_BLOB_TX_TYPE_ID.to_owned(),
    })?;
    assert!(BlobTx::decode(&empty_blob_list)?.is_none());

    let malformed_blob = encode_test_blob_tx(&TestBlobTxProto {
        tx: b"tx".to_vec(),
        blobs: vec![TestBlobProto {
            namespace_id: vec![1, 2, 3],
            data: b"blob".to_vec(),
            share_version: u32::from(DEFAULT_SHARE_VERSION),
            namespace_version: 0,
        }],
        type_id: PROTO_BLOB_TX_TYPE_ID.to_owned(),
    })?;
    assert!(BlobTx::decode(&malformed_blob).is_err());

    Ok(())
}

#[test]
fn square_size_and_empty_square_are_stable() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(size(0), 1);
    assert_eq!(size(1), 1);
    assert_eq!(size(2), 2);
    assert_eq!(size(5), 4);

    let square = empty_square();
    assert_eq!(square.size(), 1);
    assert_eq!(square.len(), 1);
    assert!(!square.is_empty());
    assert!(square.is_empty_square());
    assert_eq!(square.to_bytes().len(), 1);
    assert_eq!(square.as_ref().len(), 1);
    assert!(Square::from_shares_unchecked(Vec::new()).is_empty());
    assert!(Square::new(square.into_shares())?.is_empty_square());
    assert!(Square::new(Vec::new()).is_err());
    assert!(Square::new(vec![shares::tail_padding_share(); 2]).is_err());
    Ok(())
}

#[test]
fn write_square_lays_out_reserved_blob_and_tail_padding() -> Result<(), Box<dyn std::error::Error>>
{
    let mut tx_writer = CompactShareSplitter::new(TX_NAMESPACE, DEFAULT_SHARE_VERSION)?;
    tx_writer.write_tx(b"ordinary")?;
    let mut pfb_writer = CompactShareSplitter::new(PAY_FOR_BLOB_NAMESPACE, DEFAULT_SHARE_VERSION)?;
    pfb_writer.write_tx(b"wrapped-pfb")?;
    let non_reserved_start = tx_writer.count() + pfb_writer.count() + 1;

    let namespace = Namespace::new_v0(&[9])?;
    let blob = Blob::new(namespace, b"blob-data".to_vec(), DEFAULT_SHARE_VERSION)?;
    let mut blob_writer = SparseShareSplitter::new();
    blob_writer.write(&blob)?;

    let square = write_square(tx_writer, pfb_writer, blob_writer, non_reserved_start, 4)?;
    let shares = square.as_shares();
    assert_eq!(shares.len(), 16);
    assert_eq!(shares[0].namespace()?, TX_NAMESPACE);
    assert_eq!(shares[1].namespace()?, PAY_FOR_BLOB_NAMESPACE);
    assert!(shares[2].namespace()?.is_primary_reserved_padding());
    assert_eq!(shares[3].namespace()?, namespace);
    assert!(shares[4].namespace()?.is_tail_padding());
    Ok(())
}

#[test]
fn square_extracts_wrapped_pfbs() -> Result<(), Box<dyn std::error::Error>> {
    let tx_writer = CompactShareSplitter::new(TX_NAMESPACE, DEFAULT_SHARE_VERSION)?;
    let mut pfb_writer = CompactShareSplitter::new(PAY_FOR_BLOB_NAMESPACE, DEFAULT_SHARE_VERSION)?;
    pfb_writer.write_tx(b"wrapped-pfb")?;
    let square = write_square(tx_writer, pfb_writer, SparseShareSplitter::new(), 1, 2)?;

    assert_eq!(square.wrapped_pfbs()?, vec![b"wrapped-pfb".to_vec()]);
    Ok(())
}

#[test]
fn write_square_rejects_invalid_layouts() -> Result<(), Box<dyn std::error::Error>> {
    let mut tx_writer = CompactShareSplitter::new(TX_NAMESPACE, DEFAULT_SHARE_VERSION)?;
    tx_writer.write_tx(b"ordinary")?;
    let pfb_writer = CompactShareSplitter::new(PAY_FOR_BLOB_NAMESPACE, DEFAULT_SHARE_VERSION)?;
    let blob_writer = SparseShareSplitter::new();

    assert!(
        write_square(
            tx_writer.clone(),
            pfb_writer.clone(),
            blob_writer.clone(),
            0,
            1
        )
        .is_err()
    );
    assert!(write_square(tx_writer, pfb_writer, blob_writer, 1, 3).is_err());

    let namespace = Namespace::new_v0(&[7])?;
    let blob = Blob::new(namespace, b"blob-data".to_vec(), DEFAULT_SHARE_VERSION)?;
    let mut too_large_blob_writer = SparseShareSplitter::new();
    too_large_blob_writer.write(&blob)?;
    assert!(
        write_square(
            CompactShareSplitter::new(TX_NAMESPACE, DEFAULT_SHARE_VERSION)?,
            CompactShareSplitter::new(PAY_FOR_BLOB_NAMESPACE, DEFAULT_SHARE_VERSION)?,
            too_large_blob_writer,
            1,
            1
        )
        .is_err()
    );

    Ok(())
}

#[test]
fn error_display_messages_are_stable() {
    let cases = [
        (
            Error::InvalidSquareSize { actual: 0 },
            "invalid square size or share count: 0",
        ),
        (
            Error::NotPowerOfTwo { actual: 3 },
            "value is not a power of two: 3",
        ),
        (Error::ZeroDivisor, "divisor must be non-zero"),
        (Error::ArithmeticOverflow, "arithmetic overflow"),
        (
            Error::ShareIndexOverflow { actual: usize::MAX },
            "share index overflows uint32: 18446744073709551615",
        ),
        (Error::InvalidBlobTx, "invalid blob transaction wrapper"),
        (
            Error::Share(shares::Error::InvalidBlob),
            "share error: invalid blob",
        ),
        (
            Error::Decode("bad bytes".to_owned()),
            "decode square data: bad bytes",
        ),
        (
            Error::Encode("bad value".to_owned()),
            "encode square data: bad value",
        ),
    ];

    for (error, message) in cases {
        assert_eq!(error.to_string(), message);
    }
}

#[derive(Clone, PartialEq, prost::Message)]
struct TestBlobProto {
    #[prost(bytes = "vec", tag = "1")]
    namespace_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    data: Vec<u8>,
    #[prost(uint32, tag = "3")]
    share_version: u32,
    #[prost(uint32, tag = "4")]
    namespace_version: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
struct TestBlobTxProto {
    #[prost(bytes = "vec", tag = "1")]
    tx: Vec<u8>,
    #[prost(message, repeated, tag = "2")]
    blobs: Vec<TestBlobProto>,
    #[prost(string, tag = "3")]
    type_id: String,
}

fn encode_test_blob_tx(proto: &TestBlobTxProto) -> Result<Vec<u8>, prost::EncodeError> {
    let mut out = Vec::with_capacity(prost::Message::encoded_len(proto));
    prost::Message::encode(proto, &mut out)?;
    Ok(out)
}
