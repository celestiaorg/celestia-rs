//! Conformance tests for the square crate against committed fixture files.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::struct_field_names,
    dead_code
)]

use serde::Deserialize;
use shares::{
    Blob, CompactShareSplitter, DEFAULT_SHARE_VERSION, Namespace, PAY_FOR_BLOB_NAMESPACE,
    SparseShareSplitter, TX_NAMESPACE,
};
use square::{
    BlobTx, Builder, LeoRsCodec, PROTO_BLOB_TX_TYPE_ID, Square, blob_min_square_size,
    blob_share_range, blob_shares_used_non_interactive_defaults, build,
    compute_extended_data_square, construct, create_commitment, create_commitments, deconstruct,
    empty_square, extend_shares, import_extended_data_square, is_power_of_two,
    merkle_mountain_range_sizes, next_share_index, round_down_power_of_two,
    round_up_by_multiple_of, round_up_power_of_two, round_up_power_of_two_strict, size,
    sub_tree_width, tx_share_range, write_square,
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

fn decode_bytes(hex: &str) -> Vec<u8> {
    hex::decode(hex).expect("valid fixture hex")
}

fn decode_bytes_vec(hexes: &[String]) -> Vec<Vec<u8>> {
    hexes.iter().map(|hex| decode_bytes(hex)).collect()
}

fn decode_namespace(hex: &str) -> shares::Result<Namespace> {
    Namespace::from_slice(&decode_bytes(hex))
}

fn hex_square(square: &Square) -> Vec<String> {
    square
        .as_shares()
        .iter()
        .map(|share| hex::encode(share.as_bytes()))
        .collect()
}

fn hex_bytes_vec(values: &[Vec<u8>]) -> Vec<String> {
    values.iter().map(hex::encode).collect()
}

fn hex_roots<const N: usize>(values: &[[u8; N]]) -> Vec<String> {
    values.iter().map(hex::encode).collect()
}

fn decode_shares(hexes: &[String]) -> square::Result<Vec<shares::Share>> {
    hexes
        .iter()
        .map(|hex| Ok(shares::Share::from_slice(&decode_bytes(hex))?))
        .collect()
}

fn decode_square(hexes: &[String]) -> square::Result<Square> {
    Square::new(decode_shares(hexes)?)
}

fn decode_pfb_sizes(outputs: &[PfbDecodeCase], tx: &[u8]) -> square::Result<Vec<u32>> {
    let tx_hex = hex::encode(tx);
    outputs
        .iter()
        .find(|output| output.tx_hex == tx_hex)
        .map(|output| output.blob_sizes.clone())
        .ok_or(square::Error::BuilderInvariant("missing decoder fixture"))
}

#[derive(Deserialize)]
struct PowersFixture {
    cases: Vec<PowerCase>,
}

#[derive(Deserialize)]
struct PowerCase {
    label: String,
    input: usize,
    round_up: usize,
    #[serde(default)]
    round_down: usize,
    round_down_error: bool,
    round_up_strict: usize,
    is_power_of_two: bool,
    square_size: usize,
    #[serde(default)]
    blob_min_square_size: usize,
}

#[derive(Deserialize)]
struct InclusionFixture {
    subtree_cases: Vec<SubtreeCase>,
    next_cases: Vec<NextCase>,
    used_cases: Vec<BlobSharesUsedCase>,
    round_up_cases: Vec<RoundUpCase>,
}

#[derive(Deserialize)]
struct BlobTxFixture {
    type_id: String,
    cases: Vec<BlobTxCase>,
    non_wrappers: Vec<NonWrapperCase>,
}

#[derive(Deserialize)]
struct BlobTxCase {
    label: String,
    tx_hex: String,
    blobs: Vec<BlobFixture>,
    encoded_hex: String,
}

#[derive(Deserialize)]
struct BlobFixture {
    namespace_hex: String,
    data_hex: String,
    share_version: u8,
}

#[derive(Deserialize)]
struct NonWrapperCase {
    label: String,
    input_hex: String,
}

#[derive(Deserialize)]
struct CommitmentsFixture {
    mmr_cases: Vec<MMRCase>,
    commitment_cases: Vec<CommitmentCase>,
    batch_case: CommitmentBatch,
}

#[derive(Deserialize)]
struct BuildersFixture {
    cases: Vec<BuilderCase>,
}

#[derive(Deserialize)]
struct SquaresFixture {
    empty_square_shares_hex: Vec<String>,
    empty_square_size: usize,
    write_cases: Vec<WriteSquareCase>,
    invalid_square_size_cases: Vec<InvalidSquareCase>,
}

#[derive(Deserialize)]
struct RangesFixture {
    cases: Vec<RangeCase>,
}

#[derive(Deserialize)]
struct DeconstructFixture {
    cases: Vec<DeconstructCase>,
    error_cases: Vec<DeconstructErrorCase>,
}

#[derive(Deserialize)]
struct ErasureFixture {
    cases: Vec<ErasureCase>,
    invalid_cases: Vec<ErasureInvalidCase>,
}

#[derive(Deserialize)]
struct MMRCase {
    label: String,
    total_size: u64,
    max_tree_size: u64,
    sizes: Vec<u64>,
}

#[derive(Clone, Deserialize)]
struct CommitmentCase {
    label: String,
    namespace_hex: String,
    data_hex: String,
    share_version: u8,
    subtree_root_threshold: usize,
    commitment_hex: String,
}

#[derive(Deserialize)]
struct CommitmentBatch {
    labels: Vec<String>,
    commitments_hex: Vec<String>,
}

#[derive(Deserialize)]
struct BuilderCase {
    label: String,
    max_square_size: usize,
    subtree_root_threshold: usize,
    input_txs_hex: Vec<String>,
    build_square_shares_hex: Vec<String>,
    included_txs_hex: Vec<String>,
    construct_error: bool,
    #[serde(default)]
    construct_square_shares_hex: Vec<String>,
    tx_ranges: Vec<TxRangeCase>,
    blob_ranges: Vec<BlobRangeCase>,
    wrapped_pfbs: Vec<WrappedPfbCase>,
}

#[derive(Deserialize)]
struct TxRangeCase {
    tx_index: usize,
    start: usize,
    end: usize,
}

#[derive(Deserialize)]
struct BlobRangeCase {
    tx_index: usize,
    blob_index: usize,
    start: usize,
    end: usize,
}

#[derive(Deserialize)]
struct WrappedPfbCase {
    tx_index: usize,
    tx_hex: String,
    share_indexes: Vec<u32>,
}

#[derive(Deserialize)]
struct WriteSquareCase {
    label: String,
    txs_hex: Vec<String>,
    pfb_txs_hex: Vec<String>,
    blob_ops: Vec<BlobWriteOp>,
    non_reserved_start: usize,
    square_size: usize,
    shares_hex: Vec<String>,
    wrapped_pfbs_hex: Vec<String>,
    is_empty: bool,
}

#[derive(Deserialize)]
struct BlobWriteOp {
    kind: String,
    namespace_hex: Option<String>,
    data_hex: Option<String>,
    #[serde(default)]
    share_version: u8,
    #[serde(default)]
    count: usize,
}

#[derive(Deserialize)]
struct InvalidSquareCase {
    label: String,
    shares_hex: Vec<String>,
}

#[derive(Deserialize)]
struct RangeCase {
    label: String,
    max_square_size: usize,
    subtree_root_threshold: usize,
    txs_hex: Vec<String>,
    tx_ranges: Vec<RangeTxExpectation>,
    blob_ranges: Vec<RangeBlobExpectation>,
}

#[derive(Deserialize)]
struct RangeTxExpectation {
    label: String,
    tx_index: usize,
    error: bool,
    #[serde(default)]
    start: usize,
    #[serde(default)]
    end: usize,
}

#[derive(Deserialize)]
struct RangeBlobExpectation {
    label: String,
    tx_index: usize,
    blob_index: usize,
    error: bool,
    #[serde(default)]
    start: usize,
    #[serde(default)]
    end: usize,
}

#[derive(Deserialize)]
struct DeconstructCase {
    label: String,
    square_shares_hex: Vec<String>,
    decoder_outputs: Vec<PfbDecodeCase>,
    deconstructed_txs_hex: Vec<String>,
}

#[derive(Deserialize)]
struct DeconstructErrorCase {
    label: String,
    square_shares_hex: Vec<String>,
    decoder_outputs: Vec<PfbDecodeCase>,
}

#[derive(Deserialize)]
struct PfbDecodeCase {
    tx_hex: String,
    blob_sizes: Vec<u32>,
}

#[derive(Deserialize)]
struct ErasureCase {
    label: String,
    ods_shares_hex: Vec<String>,
    extended_shares_hex: Vec<String>,
    flattened_ods_hex: Vec<String>,
    row_roots_hex: Vec<String>,
    column_roots_hex: Vec<String>,
    roots_hex: Vec<String>,
    original_width: usize,
    width: usize,
}

#[derive(Deserialize)]
struct ErasureInvalidCase {
    label: String,
    #[serde(default)]
    ods_shares_hex: Vec<String>,
    #[serde(default)]
    extended_shares_hex: Vec<String>,
    compute_error: bool,
    import_error: bool,
}

#[derive(Deserialize)]
struct SubtreeCase {
    label: String,
    share_count: usize,
    subtree_root_threshold: usize,
    blob_min_square_size: usize,
    sub_tree_width: usize,
}

#[derive(Deserialize)]
struct NextCase {
    label: String,
    cursor: usize,
    blob_share_len: usize,
    subtree_root_threshold: usize,
    next_share_index: usize,
}

#[derive(Deserialize)]
struct BlobSharesUsedCase {
    label: String,
    cursor: usize,
    subtree_root_threshold: usize,
    blob_share_lengths: Vec<usize>,
    shares_used: usize,
    indexes: Vec<u32>,
}

#[derive(Deserialize)]
struct RoundUpCase {
    label: String,
    cursor: usize,
    value: usize,
    rounded: usize,
}

#[test]
fn powers_of_two_fixtures_match_go_square() -> square::Result<()> {
    let fixture: PowersFixture = load_fixture("square/powers-of-two.json");
    for case in fixture.cases {
        assert_eq!(
            round_up_power_of_two(case.input),
            case.round_up,
            "{}",
            case.label
        );
        assert_eq!(
            round_up_power_of_two_strict(case.input),
            case.round_up_strict,
            "{}",
            case.label
        );
        assert_eq!(
            is_power_of_two(case.input),
            case.is_power_of_two,
            "{}",
            case.label
        );
        assert_eq!(size(case.input), case.square_size, "{}", case.label);

        if case.round_down_error {
            assert!(
                round_down_power_of_two(case.input).is_err(),
                "{}",
                case.label
            );
        } else {
            assert_eq!(
                round_down_power_of_two(case.input)?,
                case.round_down,
                "{}",
                case.label
            );
        }

        if case.input > 0 {
            assert_eq!(
                blob_min_square_size(case.input)?,
                case.blob_min_square_size,
                "{}",
                case.label
            );
        }
    }
    Ok(())
}

#[test]
fn inclusion_rule_fixtures_match_go_square() -> square::Result<()> {
    let fixture: InclusionFixture = load_fixture("square/inclusion-rules.json");

    for case in fixture.subtree_cases {
        assert_eq!(
            blob_min_square_size(case.share_count)?,
            case.blob_min_square_size,
            "{}",
            case.label
        );
        assert_eq!(
            sub_tree_width(case.share_count, case.subtree_root_threshold)?,
            case.sub_tree_width,
            "{}",
            case.label
        );
    }

    for case in fixture.next_cases {
        assert_eq!(
            next_share_index(
                case.cursor,
                case.blob_share_len,
                case.subtree_root_threshold,
            )?,
            case.next_share_index,
            "{}",
            case.label
        );
    }

    for case in fixture.used_cases {
        let (shares_used, indexes) = blob_shares_used_non_interactive_defaults(
            case.cursor,
            case.subtree_root_threshold,
            &case.blob_share_lengths,
        )?;
        assert_eq!(shares_used, case.shares_used, "{}", case.label);
        assert_eq!(indexes, case.indexes, "{}", case.label);
    }

    for case in fixture.round_up_cases {
        assert_eq!(
            round_up_by_multiple_of(case.cursor, case.value)?,
            case.rounded,
            "{}",
            case.label
        );
    }
    Ok(())
}

#[test]
fn blob_tx_fixtures_match_go_square() -> square::Result<()> {
    let fixture: BlobTxFixture = load_fixture("square/blob-tx.json");
    assert_eq!(fixture.type_id, PROTO_BLOB_TX_TYPE_ID);

    for case in fixture.cases {
        let tx = decode_bytes(&case.tx_hex);
        let blobs = case
            .blobs
            .iter()
            .map(|blob| {
                Blob::new(
                    decode_namespace(&blob.namespace_hex)?,
                    decode_bytes(&blob.data_hex),
                    blob.share_version,
                )
                .map_err(square::Error::from)
            })
            .collect::<square::Result<Vec<_>>>()?;
        let blob_tx = BlobTx::new(tx, blobs)?;
        let encoded = decode_bytes(&case.encoded_hex);
        assert_eq!(blob_tx.encode()?, encoded, "{}", case.label);
        assert_eq!(BlobTx::decode(&encoded)?, Some(blob_tx), "{}", case.label);
    }

    for case in fixture.non_wrappers {
        assert_eq!(
            BlobTx::decode(&decode_bytes(&case.input_hex))?,
            None,
            "{}",
            case.label
        );
    }
    Ok(())
}

#[test]
fn builder_fixtures_match_go_square() -> square::Result<()> {
    let fixture: BuildersFixture = load_fixture("square/builders.json");

    for case in fixture.cases {
        let txs = case
            .input_txs_hex
            .iter()
            .map(|tx| decode_bytes(tx))
            .collect::<Vec<_>>();

        let built = build(&txs, case.max_square_size, case.subtree_root_threshold)?;
        assert_eq!(
            hex_square(&built.square),
            case.build_square_shares_hex,
            "{}",
            case.label
        );
        assert_eq!(
            built
                .included_txs
                .iter()
                .map(hex::encode)
                .collect::<Vec<_>>(),
            case.included_txs_hex,
            "{}",
            case.label
        );

        let constructed = construct(&txs, case.max_square_size, case.subtree_root_threshold);
        if case.construct_error {
            assert!(constructed.is_err(), "{}", case.label);
        } else {
            assert_eq!(
                hex_square(&constructed?),
                case.construct_square_shares_hex,
                "{}",
                case.label
            );
        }

        for range in case.tx_ranges {
            assert_eq!(
                tx_share_range(
                    &txs,
                    range.tx_index,
                    case.max_square_size,
                    case.subtree_root_threshold,
                )?,
                shares::ShareRange::new(range.start, range.end),
                "{}",
                case.label
            );
        }

        for range in case.blob_ranges {
            assert_eq!(
                blob_share_range(
                    &txs,
                    range.tx_index,
                    range.blob_index,
                    case.max_square_size,
                    case.subtree_root_threshold,
                )?,
                shares::ShareRange::new(range.start, range.end),
                "{}",
                case.label
            );
        }

        if !case.wrapped_pfbs.is_empty() {
            let mut builder =
                Builder::new(case.max_square_size, case.subtree_root_threshold, &txs)?;
            for wrapped in case.wrapped_pfbs {
                let got = builder.get_wrapped_pfb(wrapped.tx_index)?;
                assert_eq!(hex::encode(got.tx), wrapped.tx_hex, "{}", case.label);
                assert_eq!(got.share_indexes, wrapped.share_indexes, "{}", case.label);
            }
        }
    }

    Ok(())
}

#[test]
fn square_layout_fixtures_match_go_square() -> square::Result<()> {
    let fixture: SquaresFixture = load_fixture("square/squares.json");

    let empty = empty_square();
    assert_eq!(hex_square(&empty), fixture.empty_square_shares_hex);
    assert_eq!(empty.size(), fixture.empty_square_size);
    assert!(empty.is_empty_square());

    for case in fixture.write_cases {
        let mut tx_writer = CompactShareSplitter::new(TX_NAMESPACE, DEFAULT_SHARE_VERSION)?;
        for tx in &case.txs_hex {
            tx_writer.write_tx(&decode_bytes(tx))?;
        }

        let mut pfb_writer =
            CompactShareSplitter::new(PAY_FOR_BLOB_NAMESPACE, DEFAULT_SHARE_VERSION)?;
        for pfb_tx in &case.pfb_txs_hex {
            pfb_writer.write_tx(&decode_bytes(pfb_tx))?;
        }

        let mut blob_writer = SparseShareSplitter::new();
        for op in &case.blob_ops {
            match op.kind.as_str() {
                "blob" => {
                    let namespace_hex = op
                        .namespace_hex
                        .as_deref()
                        .unwrap_or_else(|| panic!("missing namespace for {}", case.label));
                    let data_hex = op
                        .data_hex
                        .as_deref()
                        .unwrap_or_else(|| panic!("missing blob data for {}", case.label));
                    let blob = Blob::new(
                        decode_namespace(namespace_hex)?,
                        decode_bytes(data_hex),
                        op.share_version,
                    )?;
                    blob_writer.write(&blob)?;
                }
                "namespace-padding" => {
                    blob_writer.write_namespace_padding_shares(op.count)?;
                }
                other => panic!("unknown blob op kind {other} for {}", case.label),
            }
        }

        let square = write_square(
            tx_writer,
            pfb_writer,
            blob_writer,
            case.non_reserved_start,
            case.square_size,
        )?;
        assert_eq!(hex_square(&square), case.shares_hex, "{}", case.label);
        assert_eq!(square.size(), case.square_size, "{}", case.label);
        assert_eq!(square.is_empty_square(), case.is_empty, "{}", case.label);
        assert_eq!(
            square
                .wrapped_pfbs()?
                .iter()
                .map(hex::encode)
                .collect::<Vec<_>>(),
            case.wrapped_pfbs_hex,
            "{}",
            case.label
        );
    }

    for case in fixture.invalid_square_size_cases {
        let shares = decode_shares(&case.shares_hex)?;
        assert!(Square::new(shares).is_err(), "{}", case.label);
    }

    Ok(())
}

#[test]
fn share_range_fixtures_match_go_square() -> square::Result<()> {
    let fixture: RangesFixture = load_fixture("square/ranges.json");

    for case in fixture.cases {
        let txs = case
            .txs_hex
            .iter()
            .map(|tx| decode_bytes(tx))
            .collect::<Vec<_>>();

        for range in case.tx_ranges {
            let actual = tx_share_range(
                &txs,
                range.tx_index,
                case.max_square_size,
                case.subtree_root_threshold,
            );
            if range.error {
                assert!(
                    actual.is_err(),
                    "{}:{} expected tx range error",
                    case.label,
                    range.label
                );
            } else {
                assert_eq!(
                    actual?,
                    shares::ShareRange::new(range.start, range.end),
                    "{}:{}",
                    case.label,
                    range.label
                );
            }
        }

        for range in case.blob_ranges {
            let actual = blob_share_range(
                &txs,
                range.tx_index,
                range.blob_index,
                case.max_square_size,
                case.subtree_root_threshold,
            );
            if range.error {
                assert!(
                    actual.is_err(),
                    "{}:{} expected blob range error",
                    case.label,
                    range.label
                );
            } else {
                assert_eq!(
                    actual?,
                    shares::ShareRange::new(range.start, range.end),
                    "{}:{}",
                    case.label,
                    range.label
                );
            }
        }
    }

    Ok(())
}

#[test]
fn deconstruct_fixtures_match_go_square() -> square::Result<()> {
    let fixture: DeconstructFixture = load_fixture("square/deconstruct.json");

    for case in fixture.cases {
        let square = decode_square(&case.square_shares_hex)?;
        let txs = deconstruct(&square, |tx| decode_pfb_sizes(&case.decoder_outputs, tx))?;
        assert_eq!(
            txs.iter().map(hex::encode).collect::<Vec<_>>(),
            case.deconstructed_txs_hex,
            "{}",
            case.label
        );
    }

    for case in fixture.error_cases {
        let square = decode_square(&case.square_shares_hex)?;
        assert!(
            deconstruct(&square, |tx| decode_pfb_sizes(&case.decoder_outputs, tx)).is_err(),
            "{}",
            case.label
        );
    }

    Ok(())
}

#[test]
fn erasure_fixtures_match_rsmt2d() -> square::Result<()> {
    let fixture: ErasureFixture = load_fixture("square/erasure.json");
    let codec = LeoRsCodec::new();

    for case in fixture.cases {
        let label = case.label.as_str();
        let shares = decode_shares(&case.ods_shares_hex)?;
        let eds = compute_extended_data_square(&shares, &codec)?;
        assert_eq!(extend_shares(&shares)?, eds, "{label}");
        assert_eq!(eds.original_width(), case.original_width, "{label}");
        assert_eq!(eds.width(), case.width, "{label}");

        assert_eq!(
            hex_bytes_vec(&eds.flattened()).as_slice(),
            case.extended_shares_hex.as_slice(),
            "{label}"
        );
        assert_eq!(
            hex_bytes_vec(&eds.flattened_ods()).as_slice(),
            case.flattened_ods_hex.as_slice(),
            "{label}"
        );
        assert_eq!(
            hex_roots(&eds.row_roots()?).as_slice(),
            case.row_roots_hex.as_slice(),
            "{label}"
        );
        assert_eq!(
            hex_roots(&eds.column_roots()?).as_slice(),
            case.column_roots_hex.as_slice(),
            "{label}"
        );
        assert_eq!(
            hex_roots(&eds.roots()?).as_slice(),
            case.roots_hex.as_slice(),
            "{label}"
        );

        let imported =
            import_extended_data_square(decode_bytes_vec(&case.extended_shares_hex), &codec)?;
        assert_eq!(imported, eds, "{label}");
    }

    for case in fixture.invalid_cases {
        let label = case.label.as_str();
        if case.compute_error {
            let shares = decode_shares(&case.ods_shares_hex)?;
            assert!(
                compute_extended_data_square(&shares, &codec).is_err(),
                "{label}"
            );
        }
        if case.import_error {
            assert!(
                import_extended_data_square(decode_bytes_vec(&case.extended_shares_hex), &codec)
                    .is_err(),
                "{label}"
            );
        }
    }

    Ok(())
}

#[test]
fn commitment_fixtures_match_go_square() -> square::Result<()> {
    let fixture: CommitmentsFixture = load_fixture("square/commitments.json");

    for case in fixture.mmr_cases {
        assert_eq!(
            merkle_mountain_range_sizes(case.total_size, case.max_tree_size)?,
            case.sizes,
            "{}",
            case.label
        );
    }

    let mut by_label = BTreeMap::new();
    for case in fixture.commitment_cases {
        let blob = Blob::new(
            decode_namespace(&case.namespace_hex)?,
            decode_bytes(&case.data_hex),
            case.share_version,
        )?;
        assert_eq!(
            hex::encode(create_commitment(
                &blob,
                merkle::root_from_byte_slices,
                case.subtree_root_threshold,
            )?),
            case.commitment_hex,
            "{}",
            case.label
        );
        by_label.insert(case.label, (blob, case.subtree_root_threshold));
    }

    let mut batch_blobs = Vec::with_capacity(fixture.batch_case.labels.len());
    let mut threshold = None;
    for label in &fixture.batch_case.labels {
        let (blob, case_threshold) = by_label
            .get(label)
            .unwrap_or_else(|| panic!("unknown batch commitment label {label}"));
        threshold = Some(threshold.unwrap_or(*case_threshold));
        assert_eq!(threshold, Some(*case_threshold));
        batch_blobs.push(blob.clone());
    }
    let commitments = create_commitments(
        &batch_blobs,
        merkle::root_from_byte_slices,
        threshold.unwrap_or(64),
    )?;
    assert_eq!(
        commitments.iter().map(hex::encode).collect::<Vec<_>>(),
        fixture.batch_case.commitments_hex
    );
    Ok(())
}
