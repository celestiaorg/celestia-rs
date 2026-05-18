//! Conformance tests for the shares crate against committed fixture files.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::struct_excessive_bools,
    clippy::struct_field_names,
    dead_code
)]

use serde::Deserialize;
use shares::{
    Blob, COMPACT_SHARE_RESERVED_BYTES, Error, IndexWrapper, InfoByte, NAMESPACE_ID_SIZE,
    NAMESPACE_SIZE, NAMESPACE_VERSION_ZERO_ID_SIZE, NAMESPACE_VERSION_ZERO_PREFIX_SIZE, Namespace,
    PARITY_SHARES_NAMESPACE, PAY_FOR_BLOB_NAMESPACE, PRIMARY_RESERVED_PADDING_NAMESPACE,
    ReservedBytes, SHARE_SIZE, Share, ShareBuilder, ShareRange, TAIL_PADDING_NAMESPACE,
    TX_NAMESPACE, marshal_delimited_tx, namespace_padding_share, namespace_padding_shares,
    parse_blobs, parse_compact_shares, parse_sparse_shares, parse_txs, reserved_padding_share,
    reserved_padding_shares, split_blobs, split_txs, tail_padding_share, tail_padding_shares,
};
use std::collections::BTreeMap;
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
struct NamespacesFixture {
    namespace_size: usize,
    namespace_id_size: usize,
    namespace_version_zero_id_size: usize,
    namespace_version_zero_prefix_size: usize,
    constants: Vec<NamespaceCase>,
    constructors: Vec<NamespaceCase>,
    ordering_hex: Vec<String>,
    malformed: Vec<MalformedMiniCase>,
}

#[derive(Deserialize)]
struct NamespaceCase {
    label: String,
    namespace_hex: String,
    version: u8,
    id_hex: String,
}

#[derive(Deserialize)]
struct MalformedMiniCase {
    label: String,
    expected_error: String,
}

#[derive(Deserialize)]
struct InfoReservedFixture {
    info_cases: Vec<InfoCase>,
    reserved_cases: Vec<ReservedCase>,
    malformed: Vec<MalformedMiniCase>,
}

#[derive(Deserialize)]
struct InfoCase {
    label: String,
    version: u8,
    is_sequence_start: bool,
    byte_hex: String,
}

#[derive(Deserialize)]
struct ReservedCase {
    label: String,
    byte_index: u32,
    bytes_hex: String,
}

#[derive(Deserialize)]
struct BuildersFixture {
    cases: Vec<BuilderCase>,
}

#[derive(Deserialize)]
struct BuilderCase {
    label: String,
    namespace_hex: String,
    share_version: u8,
    is_first_share: bool,
    initial_available: usize,
    padding_bytes: usize,
    share_hex: String,
    raw_data_hex: String,
    #[serde(default)]
    raw_data_reserved_hex: String,
    #[serde(default)]
    reserved_byte_index: u32,
    #[serde(default)]
    sequence_len: u32,
    is_compact: bool,
    is_empty_before_data: bool,
    is_sequence_start_after: bool,
}

#[derive(Deserialize)]
struct CompactFixture {
    delimited_cases: Vec<DelimitedTxCase>,
    split_cases: Vec<CompactSplitCase>,
    index_wrapper: IndexWrapperFixture,
}

#[derive(Deserialize)]
struct DelimitedTxCase {
    label: String,
    tx_hex: String,
    out_hex: String,
}

#[derive(Deserialize)]
struct CompactSplitCase {
    label: String,
    input_txs_hex: Vec<String>,
    tx_shares_hex: Vec<String>,
    pfb_shares_hex: Vec<String>,
    parsed_txs_hex: Vec<String>,
    parsed_pfb_hex: Vec<String>,
    share_ranges: Vec<ShareRangeCase>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct ShareRangeCase {
    tx_hash_hex: String,
    start: usize,
    end: usize,
}

#[derive(Deserialize)]
struct IndexWrapperFixture {
    tx_hex: String,
    share_indexes: Vec<u32>,
    encoded_hex: String,
    non_wrapper_hex: String,
}

#[derive(Deserialize)]
struct SparseFixture {
    cases: Vec<SparseCase>,
}

#[derive(Deserialize)]
struct SparseCase {
    label: String,
    namespace_hex: String,
    data_hex: String,
    share_version: u8,
    shares_hex: Vec<String>,
    parsed_blobs: Vec<ParsedBlobCase>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct ParsedBlobCase {
    namespace_hex: String,
    data_hex: String,
    share_version: u8,
}

#[derive(Deserialize)]
struct PaddingFixture {
    namespace_padding_share_hex: String,
    reserved_padding_share_hex: String,
    tail_padding_share_hex: String,
    namespace_padding_count_hex: Vec<String>,
    reserved_padding_count_hex: Vec<String>,
    tail_padding_count_hex: Vec<String>,
}

#[derive(Deserialize)]
struct MalformedFixture {
    cases: Vec<MalformedCase>,
}

#[derive(Deserialize)]
struct MalformedCase {
    label: String,
    operation: String,
    #[serde(default)]
    input_hex: String,
    expected_error: String,
    #[serde(default)]
    supported_versions: Vec<u8>,
}

fn decode_bytes(hex: &str) -> Vec<u8> {
    hex::decode(hex).expect("valid fixture hex")
}

fn decode_namespace(hex: &str) -> shares::Result<Namespace> {
    Namespace::from_slice(&decode_bytes(hex))
}

fn decode_share(hex: &str) -> shares::Result<Share> {
    Share::from_slice(&decode_bytes(hex))
}

fn decode_shares(values: &[String]) -> shares::Result<Vec<Share>> {
    values.iter().map(|value| decode_share(value)).collect()
}

fn decode_byte_vecs(values: &[String]) -> Vec<Vec<u8>> {
    values.iter().map(|value| decode_bytes(value)).collect()
}

fn hex_shares(shares: &[Share]) -> Vec<String> {
    shares
        .iter()
        .map(|share| hex::encode(share.as_bytes()))
        .collect()
}

fn parsed_blob_cases(blobs: &[Blob]) -> Vec<ParsedBlobCase> {
    blobs
        .iter()
        .map(|blob| ParsedBlobCase {
            namespace_hex: hex::encode(blob.namespace().as_bytes()),
            data_hex: hex::encode(blob.data()),
            share_version: blob.share_version(),
        })
        .collect()
}

fn range_cases(ranges: &BTreeMap<[u8; 32], ShareRange>) -> Vec<ShareRangeCase> {
    ranges
        .iter()
        .map(|(hash, range)| ShareRangeCase {
            tx_hash_hex: hex::encode(hash),
            start: range.start,
            end: range.end,
        })
        .collect()
}

fn error_name(error: &Error) -> &'static str {
    match error {
        Error::InvalidNamespaceLength { .. }
        | Error::InvalidNamespaceVersion { .. }
        | Error::InvalidNamespaceIdLength { .. }
        | Error::InvalidNamespaceV0Prefix => "invalid_namespace",
        Error::InvalidShareLength { .. } => "invalid_share_length",
        Error::InvalidShareVersion { .. }
        | Error::InvalidInfoByte
        | Error::InvalidReservedBytesLength { .. } => "invalid_share",
        Error::UnsupportedShareVersion { .. } => "unsupported_share_version",
        Error::InvalidReservedByteIndex { .. } => "invalid_reserved_byte_index",
        Error::InvalidSequenceLen => "invalid_sequence_len",
        Error::InvalidShareRange => "invalid_share_range",
        Error::InvalidPadding => "invalid_padding",
        Error::InvalidBlob => "invalid_blob",
        Error::InvalidCompactShare => "invalid_compact_share",
        Error::InvalidSparseShare => "invalid_sparse_share",
        Error::InvalidIndexWrapper => "invalid_index_wrapper",
        Error::Decode(_) => "decode",
        Error::Encode(_) => "encode",
    }
}

#[test]
fn namespace_fixtures_match_go_square() -> shares::Result<()> {
    let fixture: NamespacesFixture = load_fixture("shares/namespaces.json");
    assert_eq!(fixture.namespace_size, NAMESPACE_SIZE);
    assert_eq!(fixture.namespace_id_size, NAMESPACE_ID_SIZE);
    assert_eq!(
        fixture.namespace_version_zero_id_size,
        NAMESPACE_VERSION_ZERO_ID_SIZE
    );
    assert_eq!(
        fixture.namespace_version_zero_prefix_size,
        NAMESPACE_VERSION_ZERO_PREFIX_SIZE
    );

    for case in fixture.constants.iter().chain(fixture.constructors.iter()) {
        let namespace = decode_namespace(&case.namespace_hex)?;
        assert_eq!(namespace.version(), case.version, "{}", case.label);
        assert_eq!(hex::encode(namespace.id()), case.id_hex, "{}", case.label);
        match case.label.as_str() {
            "tx" => assert_eq!(namespace, TX_NAMESPACE),
            "pay-for-blob" => assert_eq!(namespace, PAY_FOR_BLOB_NAMESPACE),
            "primary-reserved-padding" | "max-primary-reserved" => {
                assert_eq!(namespace, PRIMARY_RESERVED_PADDING_NAMESPACE);
            }
            "tail-padding" => assert_eq!(namespace, TAIL_PADDING_NAMESPACE),
            "parity-shares" | "new-max-version" => assert_eq!(namespace, PARITY_SHARES_NAMESPACE),
            _ => {}
        }
    }

    let mut sorted = fixture.ordering_hex.clone();
    sorted.sort();
    assert_eq!(fixture.ordering_hex, sorted);
    for case in fixture.malformed {
        assert_eq!(case.expected_error, "invalid_namespace", "{}", case.label);
    }
    Ok(())
}

#[test]
fn info_and_reserved_fixtures_match_go_square() -> shares::Result<()> {
    let fixture: InfoReservedFixture = load_fixture("shares/info-reserved.json");
    for case in fixture.info_cases {
        let expected = decode_bytes(&case.byte_hex);
        assert_eq!(expected.len(), 1, "{}", case.label);
        let info = InfoByte::new(case.version, case.is_sequence_start)?;
        assert_eq!(info.into_byte(), expected[0], "{}", case.label);
        assert_eq!(InfoByte::parse(expected[0])?, info, "{}", case.label);
    }
    for case in fixture.reserved_cases {
        let bytes = decode_bytes(&case.bytes_hex);
        let reserved = ReservedBytes::new(case.byte_index)?;
        assert_eq!(reserved.to_bytes(), bytes.as_slice(), "{}", case.label);
        assert_eq!(ReservedBytes::parse(&bytes)?, reserved, "{}", case.label);
    }
    for case in fixture.malformed {
        assert_eq!(case.expected_error, "invalid_share", "{}", case.label);
    }
    Ok(())
}

#[test]
fn builder_fixtures_match_go_square() -> shares::Result<()> {
    let fixture: BuildersFixture = load_fixture("shares/builders.json");
    for case in fixture.cases {
        let namespace = decode_namespace(&case.namespace_hex)?;
        let mut builder = ShareBuilder::new(namespace, case.share_version, case.is_first_share)?;
        assert_eq!(
            builder.available_bytes(),
            case.initial_available,
            "{}",
            case.label
        );
        assert_eq!(
            builder.is_empty_share(),
            case.is_empty_before_data,
            "{}",
            case.label
        );

        if case.is_compact {
            builder.maybe_write_reserved_bytes()?;
        }
        if case.is_first_share {
            builder.write_sequence_len(case.sequence_len)?;
        }
        let raw_data = decode_bytes(&case.raw_data_hex);
        let data_len = usize::try_from(case.sequence_len).map_err(|_| Error::InvalidSequenceLen)?;
        assert_eq!(
            builder.add_data(&raw_data[..data_len]),
            None,
            "{}",
            case.label
        );
        assert_eq!(
            builder.zero_pad_if_necessary(),
            case.padding_bytes,
            "{}",
            case.label
        );
        let share = builder.build()?;
        assert_eq!(
            hex::encode(share.as_bytes()),
            case.share_hex,
            "{}",
            case.label
        );
        assert_eq!(
            hex::encode(share.raw_data()?),
            case.raw_data_hex,
            "{}",
            case.label
        );
        assert_eq!(
            share.is_sequence_start()?,
            case.is_sequence_start_after,
            "{}",
            case.label
        );
        if case.is_compact {
            assert_eq!(
                hex::encode(share.raw_data_using_reserved()?),
                case.raw_data_reserved_hex,
                "{}",
                case.label
            );
            let reserved_start = NAMESPACE_SIZE + 1 + if case.is_first_share { 4 } else { 0 };
            let reserved = ReservedBytes::parse(
                &share.as_bytes()[reserved_start..reserved_start + COMPACT_SHARE_RESERVED_BYTES],
            )?;
            assert_eq!(
                reserved.byte_index(),
                case.reserved_byte_index,
                "{}",
                case.label
            );
        }
    }
    Ok(())
}

#[test]
fn compact_fixtures_match_go_square() -> shares::Result<()> {
    let fixture: CompactFixture = load_fixture("shares/compact.json");
    for case in fixture.delimited_cases {
        let tx = decode_bytes(&case.tx_hex);
        assert_eq!(
            hex::encode(marshal_delimited_tx(&tx)),
            case.out_hex,
            "{}",
            case.label
        );
    }

    let wrapper_bytes = decode_bytes(&fixture.index_wrapper.encoded_hex);
    let wrapper = IndexWrapper::decode(&wrapper_bytes)?.expect("fixture is an index wrapper");
    assert_eq!(hex::encode(&wrapper.tx), fixture.index_wrapper.tx_hex);
    assert_eq!(wrapper.share_indexes, fixture.index_wrapper.share_indexes);
    assert_eq!(wrapper.encode()?, wrapper_bytes);
    assert_eq!(
        IndexWrapper::decode(&decode_bytes(&fixture.index_wrapper.non_wrapper_hex))?,
        None
    );

    for case in fixture.split_cases {
        let input_txs = decode_byte_vecs(&case.input_txs_hex);
        let result = split_txs(&input_txs)?;
        assert_eq!(
            hex_shares(&result.tx_shares),
            case.tx_shares_hex,
            "{}",
            case.label
        );
        assert_eq!(
            hex_shares(&result.pfb_shares),
            case.pfb_shares_hex,
            "{}",
            case.label
        );
        assert_eq!(
            parse_txs(&decode_shares(&case.tx_shares_hex)?)?
                .iter()
                .map(hex::encode)
                .collect::<Vec<_>>(),
            case.parsed_txs_hex,
            "{}",
            case.label
        );
        assert_eq!(
            parse_compact_shares(&decode_shares(&case.pfb_shares_hex)?, &[0])?
                .iter()
                .map(hex::encode)
                .collect::<Vec<_>>(),
            case.parsed_pfb_hex,
            "{}",
            case.label
        );
        assert_eq!(
            range_cases(&result.share_ranges),
            case.share_ranges,
            "{}",
            case.label
        );
    }
    Ok(())
}

#[test]
fn sparse_fixtures_match_go_square() -> shares::Result<()> {
    let fixture: SparseFixture = load_fixture("shares/sparse.json");
    for case in fixture.cases {
        let namespace = decode_namespace(&case.namespace_hex)?;
        let data = decode_bytes(&case.data_hex);
        let blob = Blob::new(namespace, data, case.share_version)?;
        let shares = split_blobs(core::slice::from_ref(&blob))?;
        assert_eq!(hex_shares(&shares), case.shares_hex, "{}", case.label);
        assert_eq!(
            parsed_blob_cases(&parse_blobs(&decode_shares(&case.shares_hex)?)?),
            case.parsed_blobs,
            "{}",
            case.label
        );
    }
    Ok(())
}

#[test]
fn padding_fixtures_match_go_square() -> shares::Result<()> {
    let fixture: PaddingFixture = load_fixture("shares/padding.json");
    let namespace_padding = decode_share(&fixture.namespace_padding_share_hex)?;
    let namespace = namespace_padding.namespace()?;
    let version = namespace_padding.version()?;
    assert_eq!(
        namespace_padding_share(namespace, version)?,
        namespace_padding
    );
    assert_eq!(
        hex_shares(&namespace_padding_shares(namespace, version, 2)?),
        fixture.namespace_padding_count_hex
    );

    assert_eq!(
        hex::encode(reserved_padding_share().as_bytes()),
        fixture.reserved_padding_share_hex
    );
    assert_eq!(
        hex_shares(&reserved_padding_shares(2)),
        fixture.reserved_padding_count_hex
    );
    assert_eq!(
        hex::encode(tail_padding_share().as_bytes()),
        fixture.tail_padding_share_hex
    );
    assert_eq!(
        hex_shares(&tail_padding_shares(2)),
        fixture.tail_padding_count_hex
    );
    assert!(reserved_padding_share().is_padding()?);
    assert!(tail_padding_share().is_padding()?);
    Ok(())
}

#[test]
fn malformed_fixtures_match_expected_errors() -> shares::Result<()> {
    let fixture: MalformedFixture = load_fixture("shares/malformed.json");
    for case in fixture.cases {
        let supported = if case.supported_versions.is_empty() {
            vec![0]
        } else {
            case.supported_versions.clone()
        };
        let error = match case.operation.as_str() {
            "share_from_slice" => Share::from_slice(&decode_bytes(&case.input_hex))
                .expect_err("fixture should be malformed"),
            "parse_sparse" => {
                let share = decode_share(&case.input_hex)?;
                parse_sparse_shares(&[share], &supported).expect_err("fixture should be malformed")
            }
            "parse_compact" => {
                let share = decode_share(&case.input_hex)?;
                parse_compact_shares(&[share], &supported).expect_err("fixture should be malformed")
            }
            other => panic!("unknown malformed operation {other}"),
        };
        assert_eq!(error_name(&error), case.expected_error, "{}", case.label);
    }
    assert_eq!(SHARE_SIZE, 512);
    Ok(())
}
