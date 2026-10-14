//! Public API coverage for the `shares` crate.

#![allow(clippy::expect_used)]

use shares::{
    Blob, COMPACT_SHARE_RESERVED_BYTES, CONTINUATION_SPARSE_SHARE_CONTENT_SIZE,
    CompactShareSplitter, Error, FIRST_COMPACT_SHARE_CONTENT_SIZE, FIRST_SPARSE_SHARE_CONTENT_SIZE,
    FIRST_SPARSE_SHARE_CONTENT_SIZE_WITH_SIGNER, IndexWrapper, InfoByte,
    MAX_PRIMARY_RESERVED_NAMESPACE, MAX_SHARE_VERSION, MIN_SECONDARY_RESERVED_NAMESPACE,
    MIN_SHARE_COUNT, MIN_SQUARE_SIZE, NAMESPACE_ID_SIZE, NAMESPACE_SIZE, NAMESPACE_VERSION_MAX,
    NAMESPACE_VERSION_SIZE, NAMESPACE_VERSION_ZERO, NAMESPACE_VERSION_ZERO_ID_SIZE,
    NAMESPACE_VERSION_ZERO_PREFIX_SIZE, Namespace, PARITY_SHARES_NAMESPACE, PAY_FOR_BLOB_NAMESPACE,
    PRIMARY_RESERVED_PADDING_NAMESPACE, ReservedBytes, SEQUENCE_LEN_BYTES, SHARE_INFO_BYTES,
    SHARE_SIZE, SHARE_VERSION_ONE, SHARE_VERSION_ZERO, SIGNER_SIZE, SUPPORTED_SHARE_VERSIONS,
    Share, ShareBuilder, ShareRange, SparseShareSplitter, TAIL_PADDING_NAMESPACE, TX_NAMESPACE,
    get_share_range_for_namespace, marshal_delimited_tx, namespace_padding_share,
    namespace_padding_shares, parse_blobs, parse_compact_shares, parse_reserved_bytes,
    parse_sparse_shares, parse_txs, reserved_padding_share, reserved_padding_shares, split_blobs,
    split_txs, tail_padding_share, tail_padding_shares,
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
    assert_eq!(NAMESPACE_VERSION_SIZE, 1);
    assert_eq!(NAMESPACE_VERSION_ZERO, 0);
    assert_eq!(NAMESPACE_VERSION_MAX, u8::MAX);
    assert_eq!(NAMESPACE_VERSION_ZERO_PREFIX_SIZE, 18);
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
    assert!(!namespace.is_reserved());

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
fn error_display_messages_are_stable() {
    let cases = [
        (
            Error::InvalidNamespaceLength {
                actual: 1,
                expected: NAMESPACE_SIZE,
            },
            "invalid namespace length: got 1, want 29",
        ),
        (
            Error::InvalidNamespaceVersion { version: 7 },
            "invalid namespace version: 7",
        ),
        (
            Error::InvalidNamespaceIdLength {
                actual: 1,
                expected: NAMESPACE_ID_SIZE,
            },
            "invalid namespace ID length: got 1, want 28",
        ),
        (
            Error::InvalidNamespaceV0Prefix,
            "invalid version-zero namespace prefix",
        ),
        (
            Error::InvalidShareLength {
                actual: 1,
                expected: SHARE_SIZE,
            },
            "invalid share length: got 1, want 512",
        ),
        (
            Error::InvalidShareVersion {
                version: 128,
                max: MAX_SHARE_VERSION,
            },
            "invalid share version: got 128, max 127",
        ),
        (
            Error::UnsupportedShareVersion {
                version: 2,
                supported: vec![0, 1],
            },
            "unsupported share version 2; supported versions: [0, 1]",
        ),
        (
            Error::InvalidSignerLength {
                actual: 3,
                expected: SIGNER_SIZE,
            },
            "invalid signer length: got 3, want 20",
        ),
        (Error::InvalidInfoByte, "invalid share info byte"),
        (
            Error::InvalidReservedBytesLength {
                actual: 3,
                expected: COMPACT_SHARE_RESERVED_BYTES,
            },
            "invalid reserved bytes length: got 3, want 4",
        ),
        (
            Error::InvalidReservedByteIndex {
                actual: SHARE_SIZE_U32,
                max_exclusive: SHARE_SIZE_U32,
            },
            "invalid reserved byte index: got 512, want < 512",
        ),
        (Error::InvalidSequenceLen, "invalid share sequence length"),
        (Error::InvalidShareRange, "invalid share range"),
        (Error::InvalidPadding, "invalid share padding"),
        (Error::InvalidBlob, "invalid blob"),
        (Error::InvalidCompactShare, "invalid compact share"),
        (Error::InvalidSparseShare, "invalid sparse share"),
        (Error::InvalidIndexWrapper, "invalid index wrapper"),
        (
            Error::Decode("bad bytes".to_owned()),
            "decode share data: bad bytes",
        ),
        (
            Error::Encode("bad value".to_owned()),
            "encode share data: bad value",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
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
    assert_eq!(parse_reserved_bytes(&reserved.to_bytes())?, reserved);
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

    let mut empty = ShareBuilder::new_empty();
    assert!(empty.is_empty_share());
    assert_eq!(empty.available_bytes(), SHARE_SIZE);
    empty.flip_sequence_start();
    assert_eq!(empty.zero_pad_if_necessary(), SHARE_SIZE);
    assert!(ShareBuilder::default().is_empty_share());

    let mut sparse_header = ShareBuilder::new(namespace, 0, true)?;
    assert_eq!(
        sparse_header
            .maybe_write_reserved_bytes()
            .expect_err("not compact"),
        Error::InvalidCompactShare
    );
    let mut continuation_header = ShareBuilder::new(namespace, 0, false)?;
    assert_eq!(
        continuation_header
            .write_sequence_len(1)
            .expect_err("not first share"),
        Error::InvalidSequenceLen
    );
    assert_eq!(
        continuation_header
            .import_raw_share(vec![0; SHARE_SIZE + 1])
            .expect_err("oversized raw share"),
        Error::InvalidShareLength {
            actual: SHARE_SIZE + 1,
            expected: SHARE_SIZE,
        }
    );

    let mut compact_without_reserved = ShareBuilder::new(TX_NAMESPACE, 0, true)?;
    compact_without_reserved.zero_pad_if_necessary();
    assert_eq!(
        compact_without_reserved
            .build()?
            .raw_data_using_reserved()?,
        &[]
    );
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
    assert_eq!(
        IndexWrapper::decode(&[0x1a, 0x04, b'N', b'O', b'P', b'E'])?,
        None
    );

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

    let signer = [7; SIGNER_SIZE];
    let v1_blob = Blob::new_v1(
        namespace,
        vec![2; FIRST_SPARSE_SHARE_CONTENT_SIZE_WITH_SIGNER + 3],
        signer,
    )?;
    assert!(v1_blob.has_signer());
    assert_eq!(v1_blob.signer(), Some(signer));
    let v1_shares = v1_blob.to_shares()?;
    assert_eq!(v1_shares.len(), 2);
    assert_eq!(v1_shares[0].version()?, SHARE_VERSION_ONE);
    assert!(v1_shares[0].contains_signer()?);
    assert_eq!(v1_shares[0].signer()?, Some(signer));
    assert_eq!(&v1_shares[0].raw_data()?[..3], &[2; 3]);
    assert_eq!(parse_blobs(&v1_shares)?, vec![v1_blob.clone()]);
    assert_eq!(
        parse_sparse_shares(&v1_shares, &[SHARE_VERSION_ZERO])
            .expect_err("version one not in explicit supported list"),
        Error::UnsupportedShareVersion {
            version: SHARE_VERSION_ONE,
            supported: vec![SHARE_VERSION_ZERO],
        }
    );
    assert_eq!(
        Blob::new(namespace, b"missing signer".to_vec(), SHARE_VERSION_ONE)
            .expect_err("version one signer required"),
        Error::InvalidSignerLength {
            actual: 0,
            expected: SIGNER_SIZE,
        }
    );
    assert_eq!(
        Blob::new_with_signer(
            namespace,
            b"unexpected signer".to_vec(),
            SHARE_VERSION_ZERO,
            Some(signer),
        )
        .expect_err("version zero signer rejected"),
        Error::InvalidBlob,
    );
    let v1_padding = namespace_padding_share(namespace, SHARE_VERSION_ONE)?;
    assert!(v1_padding.is_padding()?);
    assert!(v1_padding.contains_signer()?);
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
    assert_eq!(
        Blob::new(namespace, b"abc".to_vec(), MAX_SHARE_VERSION + 1)
            .expect_err("invalid blob share version"),
        Error::InvalidShareVersion {
            version: MAX_SHARE_VERSION + 1,
            max: MAX_SHARE_VERSION,
        }
    );
    assert_eq!(SHARE_INFO_BYTES + SEQUENCE_LEN_BYTES, 5);
    Ok(())
}

#[test]
fn sparse_splitter_padding_and_error_paths() -> shares::Result<()> {
    assert_eq!(MIN_SQUARE_SIZE, 1);
    assert_eq!(MIN_SHARE_COUNT, 1);
    assert_eq!(SHARE_VERSION_ZERO, 0);
    assert_eq!(SHARE_VERSION_ONE, 1);
    assert_eq!(SUPPORTED_SHARE_VERSIONS, &[0, 1, 2]);

    let namespace = blob_namespace(0x66)?;
    let blob = Blob::new(namespace, b"blob".to_vec(), 0)?;
    let mut splitter = SparseShareSplitter::default();
    assert_eq!(splitter.count(), 0);
    splitter.write_namespace_padding_shares(0)?;
    assert_eq!(splitter.count(), 0);
    assert_eq!(
        splitter
            .write_namespace_padding_shares(1)
            .expect_err("padding on empty splitter"),
        Error::InvalidPadding
    );
    splitter.write(&blob)?;
    assert_eq!(splitter.count(), 1);
    splitter.write_namespace_padding_shares(2)?;
    assert_eq!(splitter.count(), 3);
    let exported = splitter.export();
    assert!(exported[1].is_padding()?);
    assert_eq!(
        namespace_padding_shares(namespace, 0, 2)?,
        exported[1..].to_vec()
    );
    assert_eq!(
        reserved_padding_shares(2),
        vec![reserved_padding_share(); 2]
    );
    assert_eq!(tail_padding_shares(2), vec![tail_padding_share(); 2]);

    let unsupported = Blob::new(namespace, b"blob".to_vec(), 3)?;
    assert_eq!(
        SparseShareSplitter::new()
            .write(&unsupported)
            .expect_err("unsupported sparse share version"),
        Error::UnsupportedShareVersion {
            version: 3,
            supported: vec![0, 1, 2],
        }
    );
    Ok(())
}

#[test]
fn parse_and_range_edge_paths() -> shares::Result<()> {
    assert_eq!(
        get_share_range_for_namespace(&[], blob_namespace(1)?)?,
        ShareRange::empty()
    );

    let blob_a = Blob::new_v0(blob_namespace(1)?, b"a".to_vec())?;
    let blob_c = Blob::new_v0(blob_namespace(3)?, b"c".to_vec())?;
    let mut shares = blob_a.to_shares()?;
    shares.extend(blob_c.to_shares()?);
    assert_eq!(
        get_share_range_for_namespace(&shares, blob_namespace(2)?)?,
        ShareRange::empty()
    );
    let mut overflowing = ShareRange::new(usize::MAX, usize::MAX);
    assert_eq!(
        overflowing.add(1).expect_err("range overflow"),
        Error::InvalidShareRange
    );

    let mut continuation = ShareBuilder::new(blob_namespace(4)?, 0, false)?;
    continuation.add_data(b"tail");
    continuation.zero_pad_if_necessary();
    let continuation = continuation.build()?;
    assert_eq!(continuation.sequence_len()?, None);
    assert!(!continuation.is_padding()?);
    assert!(!continuation.is_compact_share()?);
    assert!(!continuation.raw_data_using_reserved()?.is_empty());
    continuation.validate()?;
    let _bytes = continuation.clone().into_bytes();

    let mut oversized = ShareBuilder::new(blob_namespace(5)?, 0, true)?;
    oversized.write_sequence_len(999)?;
    oversized.zero_pad_if_necessary();
    assert_eq!(
        parse_sparse_shares(&[oversized.build()?], &[0]).expect_err("declared length too long"),
        Error::InvalidSequenceLen
    );

    assert!(parse_compact_shares(&[], &[0])?.is_empty());
    assert!(parse_sparse_shares(&[], &[0])?.is_empty());
    Ok(())
}
