//! Public API coverage for the `shares` crate.

#![allow(clippy::expect_used)]

use shares::{
    Blob, COMPACT_SHARE_RESERVED_BYTES, CONTINUATION_SPARSE_SHARE_CONTENT_SIZE,
    CompactShareSplitter, Error, FIRST_COMPACT_SHARE_CONTENT_SIZE, FIRST_SPARSE_SHARE_CONTENT_SIZE,
    IndexWrapper, InfoByte, MAX_PRIMARY_RESERVED_NAMESPACE, MAX_SHARE_VERSION,
    MIN_SECONDARY_RESERVED_NAMESPACE, NAMESPACE_ID_SIZE, NAMESPACE_SIZE,
    NAMESPACE_VERSION_ZERO_ID_SIZE, Namespace, PARITY_SHARES_NAMESPACE, PAY_FOR_BLOB_NAMESPACE,
    PRIMARY_RESERVED_PADDING_NAMESPACE, ReservedBytes, SEQUENCE_LEN_BYTES, SHARE_INFO_BYTES,
    SHARE_SIZE, Share, ShareBuilder, ShareRange, TAIL_PADDING_NAMESPACE, TX_NAMESPACE,
    get_share_range_for_namespace, marshal_delimited_tx, namespace_padding_share, parse_blobs,
    parse_compact_shares, parse_sparse_shares, parse_txs, reserved_padding_share, split_blobs,
    split_txs, tail_padding_share,
};

const SHARE_SIZE_U32: u32 = 512;

fn blob_namespace(byte: u8) -> shares::Result<Namespace> {
    let mut sub_id = [0; NAMESPACE_VERSION_ZERO_ID_SIZE];
    sub_id[0] = byte;
    Namespace::new_v0(&sub_id)
}

#[test]
fn namespace_constructors_and_reserved_constants() -> shares::Result<()> {
    let namespace = Namespace::new_v0(&[0xab, 0xcd])?;
    assert_eq!(namespace.as_bytes().len(), NAMESPACE_SIZE);
    assert_eq!(namespace.version(), 0);
    assert_eq!(
        &namespace.id()[..NAMESPACE_ID_SIZE - NAMESPACE_VERSION_ZERO_ID_SIZE],
        &[0; 18]
    );
    assert_eq!(&namespace.id()[26..], &[0xab, 0xcd]);
    assert_eq!(format!("{namespace:x}").len(), NAMESPACE_SIZE * 2);
    assert_eq!(namespace.repeat(2), vec![namespace, namespace]);

    assert_eq!(TX_NAMESPACE.as_bytes()[NAMESPACE_SIZE - 1], 0x01);
    assert_eq!(PAY_FOR_BLOB_NAMESPACE.as_bytes()[NAMESPACE_SIZE - 1], 0x04);
    assert!(PRIMARY_RESERVED_PADDING_NAMESPACE.is_primary_reserved_padding());
    assert_eq!(
        PRIMARY_RESERVED_PADDING_NAMESPACE,
        MAX_PRIMARY_RESERVED_NAMESPACE
    );
    assert!(MIN_SECONDARY_RESERVED_NAMESPACE.is_secondary_reserved());
    assert!(TAIL_PADDING_NAMESPACE.is_tail_padding());
    assert!(PARITY_SHARES_NAMESPACE.is_parity_shares());
    assert!(TX_NAMESPACE.is_tx());
    assert!(PAY_FOR_BLOB_NAMESPACE.is_pay_for_blob());
    assert!(TX_NAMESPACE.is_reserved());

    assert_eq!(
        Namespace::from_slice(&[0; 28]).expect_err("short namespace"),
        Error::InvalidNamespaceLength {
            actual: 28,
            expected: NAMESPACE_SIZE,
        }
    );
    assert_eq!(
        Namespace::new_v0(&[1; 11]).expect_err("long v0 sub id"),
        Error::InvalidNamespaceIdLength {
            actual: 11,
            expected: NAMESPACE_VERSION_ZERO_ID_SIZE,
        }
    );
    let mut invalid_id = [0; NAMESPACE_ID_SIZE];
    invalid_id[0] = 1;
    assert_eq!(
        Namespace::new(0, invalid_id).expect_err("invalid v0 prefix"),
        Error::InvalidNamespaceV0Prefix
    );
    assert_eq!(
        Namespace::new(1, [0; NAMESPACE_ID_SIZE]).expect_err("unsupported version"),
        Error::InvalidNamespaceVersion { version: 1 }
    );
    Ok(())
}

#[test]
fn info_byte_and_reserved_bytes_match_share_layout() -> shares::Result<()> {
    let info = InfoByte::new(7, true)?;
    assert_eq!(info.into_byte(), 0b0000_1111);
    assert_eq!(InfoByte::parse(0b0000_1110)?.version(), 7);
    assert!(!InfoByte::parse(0b0000_1110)?.is_sequence_start());
    assert_eq!(
        InfoByte::new(MAX_SHARE_VERSION + 1, false).expect_err("invalid version"),
        Error::InvalidShareVersion {
            version: MAX_SHARE_VERSION + 1,
            max: MAX_SHARE_VERSION,
        }
    );

    let reserved = ReservedBytes::new(38)?;
    assert_eq!(reserved.to_bytes(), 38u32.to_be_bytes());
    assert_eq!(ReservedBytes::parse(&reserved.to_bytes())?, reserved);
    assert_eq!(
        ReservedBytes::parse(&[0; COMPACT_SHARE_RESERVED_BYTES - 1])
            .expect_err("short reserved bytes"),
        Error::InvalidReservedBytesLength {
            actual: COMPACT_SHARE_RESERVED_BYTES - 1,
            expected: COMPACT_SHARE_RESERVED_BYTES,
        }
    );
    assert_eq!(
        ReservedBytes::new(SHARE_SIZE_U32).expect_err("reserved index outside share"),
        Error::InvalidReservedByteIndex {
            actual: SHARE_SIZE_U32,
            max_exclusive: SHARE_SIZE_U32,
        }
    );
    Ok(())
}

#[test]
fn share_builder_populates_headers_and_raw_offsets() -> shares::Result<()> {
    let mut compact = ShareBuilder::new(TX_NAMESPACE, 0, true)?;
    assert_eq!(compact.available_bytes(), FIRST_COMPACT_SHARE_CONTENT_SIZE);
    assert!(compact.is_empty_share());
    compact.maybe_write_reserved_bytes()?;
    assert_eq!(compact.add_data(b"abc"), None);
    compact.write_sequence_len(3)?;
    assert_eq!(
        compact.zero_pad_if_necessary(),
        FIRST_COMPACT_SHARE_CONTENT_SIZE - 3
    );
    let compact_share = compact.build()?;
    assert_eq!(compact_share.len(), SHARE_SIZE);
    assert_eq!(compact_share.namespace()?, TX_NAMESPACE);
    assert_eq!(compact_share.info_byte()?, InfoByte::new(0, true)?);
    assert_eq!(compact_share.sequence_len()?, Some(3));
    assert!(compact_share.is_compact_share()?);
    assert_eq!(&compact_share.raw_data()?[..3], b"abc");
    assert_eq!(&compact_share.raw_data_using_reserved()?[..3], b"abc");

    let namespace = blob_namespace(9)?;
    let mut sparse = ShareBuilder::new(namespace, 0, true)?;
    assert_eq!(sparse.available_bytes(), FIRST_SPARSE_SHARE_CONTENT_SIZE);
    sparse.write_sequence_len(2)?;
    assert_eq!(sparse.add_data(b"zz"), None);
    sparse.zero_pad_if_necessary();
    let sparse_share = sparse.build()?;
    assert!(!sparse_share.is_compact_share()?);
    assert_eq!(&sparse_share.raw_data()?[..2], b"zz");

    let mut continuation = ShareBuilder::new(namespace, 0, false)?;
    assert_eq!(
        continuation.available_bytes(),
        CONTINUATION_SPARSE_SHARE_CONTENT_SIZE
    );
    continuation.flip_sequence_start();
    continuation.zero_pad_if_necessary();
    assert!(continuation.build()?.is_sequence_start()?);
    Ok(())
}

#[test]
fn compact_split_and_parse_round_trip_transactions() -> shares::Result<()> {
    let txs = vec![
        b"alpha".to_vec(),
        vec![7; FIRST_COMPACT_SHARE_CONTENT_SIZE + 10],
    ];
    let result = split_txs(&txs)?;
    assert!(result.pfb_shares.is_empty());
    assert_eq!(parse_txs(&result.tx_shares)?, txs);
    assert_eq!(result.share_ranges.len(), 2);
    assert_eq!(
        result.share_ranges[&hash::sha256(b"alpha")],
        ShareRange::new(0, 1)
    );

    let mut splitter = CompactShareSplitter::new(TX_NAMESPACE, 0)?;
    splitter.write_tx(b"a")?;
    assert_eq!(splitter.count(), 1);
    let first_export = splitter.export()?;
    let second_export = splitter.export()?;
    assert_eq!(first_export, second_export);
    assert_eq!(
        parse_compact_shares(&first_export, &[0])?,
        vec![b"a".to_vec()]
    );

    assert_eq!(marshal_delimited_tx(b"abc"), vec![3, b'a', b'b', b'c']);
    assert_eq!(
        CompactShareSplitter::new(blob_namespace(1)?, 0).expect_err("sparse namespace"),
        Error::InvalidCompactShare
    );
    Ok(())
}

#[test]
fn index_wrapper_routes_pay_for_blob_transactions() -> shares::Result<()> {
    let wrapper = IndexWrapper::new(b"pfb".to_vec(), vec![4, 9]);
    let encoded = wrapper.encode()?;
    assert_eq!(IndexWrapper::decode(&encoded)?, Some(wrapper));
    assert_eq!(IndexWrapper::decode(b"not protobuf")?, None);

    let ordinary_tx = b"ordinary".to_vec();
    let result = split_txs(&[ordinary_tx.clone(), encoded.clone()])?;
    assert_eq!(parse_txs(&result.tx_shares)?, vec![ordinary_tx]);
    assert_eq!(parse_txs(&result.pfb_shares)?, vec![encoded]);
    Ok(())
}

#[test]
fn sparse_split_parse_and_padding_round_trip_blobs() -> shares::Result<()> {
    let namespace = blob_namespace(0x44)?;
    let blob = Blob::new_v0(namespace, vec![1; FIRST_SPARSE_SHARE_CONTENT_SIZE + 3])?;
    assert_eq!(blob.namespace(), namespace);
    assert_eq!(blob.data_len(), FIRST_SPARSE_SHARE_CONTENT_SIZE + 3);
    assert_eq!(blob.share_version(), 0);
    assert!(!blob.is_empty());

    let shares = split_blobs(core::slice::from_ref(&blob))?;
    assert_eq!(shares.len(), 2);
    assert_eq!(
        shares[0].sequence_len()?,
        Some(u32::try_from(blob.data_len()).map_err(|_| Error::InvalidBlob)?)
    );
    assert_eq!(parse_blobs(&shares)?, vec![blob.clone()]);
    assert_eq!(blob.to_shares()?, shares);

    let padding = namespace_padding_share(namespace, 0)?;
    assert!(padding.is_padding()?);
    assert_eq!(padding.sequence_len()?, Some(0));
    assert!(reserved_padding_share().is_padding()?);
    assert!(tail_padding_share().is_padding()?);

    let mut with_padding = shares;
    with_padding.push(padding);
    assert_eq!(parse_sparse_shares(&with_padding, &[0])?, vec![blob]);
    Ok(())
}

#[test]
fn share_ranges_assume_sorted_namespaces() -> shares::Result<()> {
    let a = Blob::new_v0(blob_namespace(1)?, b"a".to_vec())?.to_shares()?;
    let b = Blob::new_v0(blob_namespace(2)?, b"b".to_vec())?.to_shares()?;
    let c = Blob::new_v0(blob_namespace(3)?, b"c".to_vec())?.to_shares()?;
    let mut shares = Vec::new();
    shares.extend(a);
    shares.extend(b.clone());
    shares.extend(c);

    assert_eq!(
        get_share_range_for_namespace(&shares, blob_namespace(2)?)?,
        ShareRange::new(1, 2)
    );
    assert_eq!(
        get_share_range_for_namespace(&shares, blob_namespace(9)?)?,
        ShareRange::empty()
    );
    assert!(ShareRange::empty().is_empty());
    let mut range = ShareRange::new(1, 2);
    range.add(3)?;
    assert_eq!(range, ShareRange::new(4, 5));

    let bytes = shares::shares_to_bytes(&b);
    assert_eq!(shares::shares_from_bytes(&bytes)?, b);
    assert_eq!(
        Share::from_slice(&[0; SHARE_SIZE - 1]).expect_err("short share"),
        Error::InvalidShareLength {
            actual: SHARE_SIZE - 1,
            expected: SHARE_SIZE,
        }
    );
    Ok(())
}

#[test]
fn sparse_parser_rejects_malformed_sequences_and_versions() -> shares::Result<()> {
    let namespace = blob_namespace(0x55)?;
    let mut builder = ShareBuilder::new(namespace, 0, false)?;
    builder.add_data(b"continuation");
    builder.zero_pad_if_necessary();
    let continuation = builder.build()?;
    assert_eq!(
        parse_sparse_shares(&[continuation], &[0]).expect_err("orphan continuation"),
        Error::InvalidSparseShare
    );

    let blob = Blob::new_v0(namespace, b"abc".to_vec())?;
    let shares = blob.to_shares()?;
    assert_eq!(
        parse_sparse_shares(&shares, &[1]).expect_err("unsupported version"),
        Error::UnsupportedShareVersion {
            version: 0,
            supported: vec![1],
        }
    );

    assert_eq!(
        Blob::new_v0(namespace, Vec::new()).expect_err("empty blob"),
        Error::InvalidBlob
    );
    assert_eq!(SHARE_INFO_BYTES + SEQUENCE_LEN_BYTES, 5);
    Ok(())
}
