//! Public API coverage for the `square` crate.

use shares::{
    Blob, CompactShareSplitter, DEFAULT_SHARE_VERSION, Namespace, PAY_FOR_BLOB_NAMESPACE,
    SHARE_SIZE, SHARE_VERSION_ONE, SIGNER_SIZE, SparseShareSplitter, TX_NAMESPACE,
};
use square::{
    BlobTx, Builder, DAH_HASH_SIZE, DAH_ROOT_SIZE, DataAvailabilityHeader, ErasureCodec, Error,
    ExtendedDataSquare, LeoRsCodec, PROTO_BLOB_TX_TYPE_ID, Square, blob_min_square_size,
    blob_share_range, blob_shares_used_non_interactive_defaults, build,
    compute_extended_data_square, construct, data_availability_header_from_proto, deconstruct,
    empty_square, extend_shares, import_extended_data_square, is_power_of_two,
    merkle_mountain_range_sizes, min_data_availability_header, new_data_availability_header,
    next_share_index, round_down_power_of_two, round_up_by_multiple_of, round_up_power_of_two,
    round_up_power_of_two_strict, size, sub_tree_width, tx_share_range, write_square,
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
    let signer = [9; SIGNER_SIZE];
    let v1_blob = Blob::new_v1(namespace, b"v1-blob-data".to_vec(), signer)?;
    let blob_tx = BlobTx::new(b"sdk-tx".to_vec(), vec![blob.clone(), v1_blob.clone()])?;

    let encoded = blob_tx.encode()?;
    let decoded =
        BlobTx::decode(&encoded)?.ok_or_else(|| std::io::Error::other("expected blob tx"))?;

    assert_eq!(decoded.tx, b"sdk-tx");
    assert_eq!(decoded.blobs, vec![blob, v1_blob]);
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
            signer: Vec::new(),
        }],
        type_id: PROTO_BLOB_TX_TYPE_ID.to_owned(),
    })?;
    assert!(BlobTx::decode(&malformed_blob).is_err());

    let malformed_signer = encode_test_blob_tx(&TestBlobTxProto {
        tx: b"tx".to_vec(),
        blobs: vec![TestBlobProto {
            namespace_id: vec![7; shares::NAMESPACE_VERSION_ZERO_ID_SIZE],
            data: b"blob".to_vec(),
            share_version: u32::from(SHARE_VERSION_ONE),
            namespace_version: 0,
            signer: vec![1; SIGNER_SIZE - 1],
        }],
        type_id: PROTO_BLOB_TX_TYPE_ID.to_owned(),
    })?;
    assert!(BlobTx::decode(&malformed_signer).is_err());

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
fn builder_exports_square_and_finds_ranges() -> Result<(), Box<dyn std::error::Error>> {
    let ordinary_tx = b"ordinary".to_vec();
    let blob = Blob::new(
        Namespace::new_v0(&[3])?,
        b"blob-data".to_vec(),
        DEFAULT_SHARE_VERSION,
    )?;
    let blob_tx = BlobTx::new(b"pfb".to_vec(), vec![blob.clone()])?.encode()?;
    let txs = vec![ordinary_tx.clone(), blob_tx.clone()];

    let mut builder = Builder::new(4, 64, &txs)?;
    assert_eq!(builder.num_txs(), 2);
    assert_eq!(builder.num_pfbs(), 1);
    assert_eq!(builder.subtree_root_threshold(), 64);
    assert!(builder.current_size() >= 3);

    let square = builder.export()?;
    assert_eq!(square.size(), 2);

    let ordinary_range = builder.find_tx_share_range(0)?;
    let pfb_range = builder.find_tx_share_range(1)?;
    assert_eq!(ordinary_range, shares::ShareRange::new(0, 1));
    assert_eq!(pfb_range, shares::ShareRange::new(1, 2));
    assert_eq!(tx_share_range(&txs, 0, 4, 64)?, ordinary_range);

    let blob_start = builder.find_blob_starting_index(1, 0)?;
    let blob_len = builder.blob_share_length(1, 0)?;
    assert_eq!(blob_len, 1);
    assert_eq!(
        blob_share_range(&txs, 1, 0, 4, 64)?,
        shares::ShareRange::new(blob_start, blob_start + blob_len)
    );

    let wrapped = builder.get_wrapped_pfb(1)?;
    assert_eq!(wrapped.tx, b"pfb");
    assert_eq!(wrapped.share_indexes, vec![u32::try_from(blob_start)?]);
    Ok(())
}

#[test]
fn build_construct_and_deconstruct_round_trip_blob_transactions()
-> Result<(), Box<dyn std::error::Error>> {
    let ordinary_tx = b"ordinary".to_vec();
    let blob = Blob::new(
        Namespace::new_v0(&[4])?,
        b"blob-data".to_vec(),
        DEFAULT_SHARE_VERSION,
    )?;
    let blob_tx = BlobTx::new(b"pfb".to_vec(), vec![blob.clone()])?.encode()?;

    let built = build(&[blob_tx.clone(), ordinary_tx.clone()], 4, 64)?;
    assert_eq!(
        built.included_txs,
        vec![ordinary_tx.clone(), blob_tx.clone()]
    );

    let square = construct(&[ordinary_tx.clone(), blob_tx.clone()], 4, 64)?;
    assert_eq!(square, built.square);

    let deconstructed = deconstruct(&square, |tx| {
        assert_eq!(tx, b"pfb");
        let blob_len = u32::try_from(blob.data_len()).map_err(|_| Error::ArithmeticOverflow)?;
        Ok(vec![blob_len])
    })?;
    assert_eq!(deconstructed, vec![ordinary_tx, blob_tx]);
    Ok(())
}

#[test]
fn builder_rejects_invalid_order_and_indexes() -> Result<(), Box<dyn std::error::Error>> {
    let ordinary_tx = b"ordinary".to_vec();
    let blob = Blob::new(
        Namespace::new_v0(&[5])?,
        b"blob-data".to_vec(),
        DEFAULT_SHARE_VERSION,
    )?;
    let blob_tx = BlobTx::new(b"pfb".to_vec(), vec![blob])?.encode()?;

    assert!(Builder::new(3, 64, &[]).is_err());
    assert!(construct(&[blob_tx.clone(), ordinary_tx], 4, 64).is_err());

    let mut builder = Builder::new(4, 64, &[blob_tx])?;
    assert!(builder.find_tx_share_range(2).is_err());
    assert!(builder.get_wrapped_pfb(0).is_ok());
    assert!(builder.get_wrapped_pfb(1).is_err());
    assert!(builder.find_blob_starting_index(0, 1).is_err());
    Ok(())
}

#[test]
fn merkle_mountain_ranges_match_commitment_rules() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(merkle_mountain_range_sizes(0, 4)?, Vec::<u64>::new());
    assert_eq!(merkle_mountain_range_sizes(4, 4)?, vec![4]);
    assert_eq!(merkle_mountain_range_sizes(11, 4)?, vec![4, 4, 2, 1]);
    assert!(merkle_mountain_range_sizes(1, 0).is_err());
    Ok(())
}

#[test]
fn blob_commitments_are_deterministic_and_data_dependent() -> Result<(), Box<dyn std::error::Error>>
{
    let namespace = Namespace::new_v0(&[6])?;
    let blob = Blob::new(namespace, b"blob-data".repeat(80), DEFAULT_SHARE_VERSION)?;
    let changed_blob = Blob::new(namespace, b"blob-data!".repeat(80), DEFAULT_SHARE_VERSION)?;

    let commitment = square::create_commitment(&blob, merkle::root_from_byte_slices, 64)?;
    let repeated = square::create_commitment(&blob, merkle::root_from_byte_slices, 64)?;
    let single_tree_commitment =
        square::create_commitment(&blob, merkle::root_from_byte_slices, 1)?;
    let changed = square::create_commitment(&changed_blob, merkle::root_from_byte_slices, 64)?;
    let commitments =
        square::create_commitments(&[blob, changed_blob], merkle::root_from_byte_slices, 64)?;

    assert_eq!(commitment, repeated);
    assert_ne!(commitment, single_tree_commitment);
    assert_ne!(commitment, changed);
    assert_eq!(commitments, vec![commitment, changed]);
    assert!(
        square::create_commitment(
            &Blob {
                namespace,
                data: Vec::new(),
                share_version: DEFAULT_SHARE_VERSION,
                signer: None,
            },
            merkle::root_from_byte_slices,
            64,
        )
        .is_err()
    );
    assert!(
        square::create_commitment(
            &commitments_blob(namespace)?,
            merkle::root_from_byte_slices,
            0
        )
        .is_err()
    );
    Ok(())
}

fn commitments_blob(namespace: Namespace) -> Result<Blob, shares::Error> {
    Blob::new(namespace, b"blob-data".to_vec(), DEFAULT_SHARE_VERSION)
}

#[test]
fn data_availability_header_hashes_and_round_trips_proto() -> Result<(), Box<dyn std::error::Error>>
{
    assert_eq!(DAH_HASH_SIZE, 32);
    assert_eq!(DAH_ROOT_SIZE, 90);

    let row_roots = vec![root_bytes(1), root_bytes(2)];
    let column_roots = vec![root_bytes(3), root_bytes(4)];
    let dah = DataAvailabilityHeader::new(row_roots.clone(), column_roots.clone())?;

    let expected_items = vec![
        row_roots[0].to_vec(),
        row_roots[1].to_vec(),
        column_roots[0].to_vec(),
        column_roots[1].to_vec(),
    ];
    assert_eq!(dah.hash(), merkle::root_from_byte_slices(&expected_items));
    assert!(!dah.is_zero());
    assert!(dah.equals(&dah.clone()));

    let encoded = dah.to_proto_bytes()?;
    let decoded = DataAvailabilityHeader::from_proto_bytes(&encoded)?;
    let decoded_via_free_fn = data_availability_header_from_proto(&encoded)?;
    assert_eq!(decoded, dah);
    assert_eq!(decoded_via_free_fn, dah);
    Ok(())
}

#[test]
fn data_availability_header_rejects_invalid_shapes() -> Result<(), Box<dyn std::error::Error>> {
    assert!(DataAvailabilityHeader::new(Vec::new(), Vec::new()).is_err());
    assert!(
        DataAvailabilityHeader::new(vec![root_bytes(1), root_bytes(2)], vec![root_bytes(3)])
            .is_err()
    );
    assert!(
        DataAvailabilityHeader::new(vec![root_bytes(1); 257], vec![root_bytes(2); 257]).is_err()
    );
    let malformed_root = encode_test_dah(&TestDahProto {
        row_roots: vec![vec![1; DAH_ROOT_SIZE - 1], root_bytes(2).to_vec()],
        column_roots: vec![root_bytes(3).to_vec(), root_bytes(4).to_vec()],
    })?;
    assert!(data_availability_header_from_proto(&malformed_root).is_err());

    let zero = DataAvailabilityHeader {
        row_roots: Vec::new(),
        column_roots: vec![root_bytes(1)],
    };
    assert!(zero.is_zero());
    Ok(())
}

#[test]
fn leopard_codec_encodes_and_reconstructs_shards() -> Result<(), Box<dyn std::error::Error>> {
    let codec = LeoRsCodec::new();
    let data = vec![vec![1; SHARE_SIZE], vec![2; SHARE_SIZE]];

    assert_eq!(codec.name(), "Leopard");
    assert_eq!(codec.max_chunks(), 128 * 128);
    codec.validate_chunk_size(SHARE_SIZE)?;
    assert!(codec.validate_chunk_size(7).is_err());

    let parity = codec.encode(&data)?;
    assert_eq!(parity.len(), data.len());
    assert!(parity.iter().all(|shard| shard.len() == SHARE_SIZE));

    let mut shards = vec![
        Some(data[0].clone()),
        None,
        Some(parity[0].clone()),
        Some(parity[1].clone()),
    ];
    let reconstructed = codec.decode(&mut shards)?;
    assert_eq!(&reconstructed[..data.len()], data.as_slice());
    assert!(shards.iter().all(Option::is_some));
    Ok(())
}

#[test]
fn extended_data_square_extends_imports_and_roots() -> Result<(), Box<dyn std::error::Error>> {
    let codec = LeoRsCodec::new();
    let square = empty_square();

    let eds = compute_extended_data_square(square.as_shares(), &codec)?;
    let default_eds = extend_shares(square.as_shares())?;
    assert_eq!(eds, default_eds);
    assert_eq!(eds.original_width(), 1);
    assert_eq!(eds.width(), 2);
    assert_eq!(eds.flattened().len(), 4);
    assert_eq!(
        eds.flattened_ods(),
        vec![square.as_shares()[0].as_bytes().to_vec()]
    );
    assert_eq!(eds.row(0)?.len(), 2);
    assert_eq!(eds.column(0)?.len(), 2);
    assert!(eds.row(2).is_err());
    assert!(eds.column(2).is_err());

    let imported = import_extended_data_square(eds.flattened(), &codec)?;
    assert_eq!(imported, eds);
    assert_eq!(eds.row_roots()?.len(), 2);
    assert_eq!(eds.column_roots()?.len(), 2);
    assert_eq!(eds.roots()?.len(), 4);

    let dah = new_data_availability_header(&eds)?;
    assert_eq!(dah.row_roots, eds.row_roots()?);
    assert_eq!(dah.column_roots, eds.column_roots()?);
    assert_eq!(min_data_availability_header(), dah);
    Ok(())
}

#[test]
fn extended_data_square_rejects_invalid_shapes() -> Result<(), Box<dyn std::error::Error>> {
    let codec = LeoRsCodec::new();

    assert!(ExtendedDataSquare::new(&codec, 3, SHARE_SIZE).is_err());
    assert!(ExtendedDataSquare::new(&codec, 2, 7).is_err());
    assert!(import_extended_data_square(vec![vec![0; SHARE_SIZE]; 3], &codec).is_err());
    assert!(import_extended_data_square(vec![vec![0; 7]; 4], &codec).is_err());
    assert!(compute_extended_data_square(&[], &codec).is_err());
    assert!(compute_extended_data_square(&shares::tail_padding_shares(2), &codec).is_err());

    let empty = ExtendedDataSquare::new(&codec, 2, SHARE_SIZE)?;
    assert_eq!(empty.flattened().len(), 4);
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
            Error::TxIndexOutOfRange { index: 9 },
            "transaction index out of range: 9",
        ),
        (
            Error::BlobIndexOutOfRange { index: 2 },
            "blob index out of range: 2",
        ),
        (
            Error::NotPayForBlob { index: 1 },
            "transaction index does not identify a PFB: 1",
        ),
        (
            Error::NormalTxAfterBlobTx { index: 3 },
            "normal transaction after blob transaction at index 3",
        ),
        (
            Error::BuilderInvariant("bad layout"),
            "builder invariant failed: bad layout",
        ),
        (
            Error::InvalidNamespacedMerkleTree("empty tree"),
            "invalid namespaced Merkle tree: empty tree",
        ),
        (
            Error::InvalidDataAvailabilityRootLength {
                actual: 89,
                expected: 90,
            },
            "invalid data availability root length: got 89, want 90",
        ),
        (
            Error::InvalidDataAvailabilityRootCount { actual: 1 },
            "invalid data availability root count: 1",
        ),
        (
            Error::DataAvailabilityAxisMismatch {
                rows: 2,
                columns: 3,
            },
            "data availability row/column root count mismatch: rows 2, columns 3",
        ),
        (
            Error::InvalidErasureShardCount { actual: 3 },
            "invalid erasure shard count: 3",
        ),
        (
            Error::InvalidErasureChunkSize { actual: 7 },
            "invalid erasure chunk size: 7",
        ),
        (
            Error::ExtendedDataSquareIndexOutOfRange {
                axis: "row",
                index: 2,
                width: 2,
            },
            "extended data square row index out of range: 2 >= 2",
        ),
        (
            Error::ErasureCodec {
                codec: "Leopard",
                message: "bad shards".to_owned(),
            },
            "Leopard erasure codec error: bad shards",
        ),
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
    #[prost(bytes = "vec", tag = "5")]
    signer: Vec<u8>,
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

#[derive(Clone, PartialEq, prost::Message)]
struct TestDahProto {
    #[prost(bytes = "vec", repeated, tag = "1")]
    row_roots: Vec<Vec<u8>>,
    #[prost(bytes = "vec", repeated, tag = "2")]
    column_roots: Vec<Vec<u8>>,
}

fn encode_test_blob_tx(proto: &TestBlobTxProto) -> Result<Vec<u8>, prost::EncodeError> {
    let mut out = Vec::with_capacity(prost::Message::encoded_len(proto));
    prost::Message::encode(proto, &mut out)?;
    Ok(out)
}

fn encode_test_dah(proto: &TestDahProto) -> Result<Vec<u8>, prost::EncodeError> {
    let mut out = Vec::with_capacity(prost::Message::encoded_len(proto));
    prost::Message::encode(proto, &mut out)?;
    Ok(out)
}

fn root_bytes(fill: u8) -> [u8; DAH_ROOT_SIZE] {
    [fill; DAH_ROOT_SIZE]
}
