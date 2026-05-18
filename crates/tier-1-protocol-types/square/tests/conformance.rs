//! Conformance tests for the square crate against committed fixture files.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::struct_field_names,
    dead_code
)]

use serde::Deserialize;
use square::{
    blob_min_square_size, blob_shares_used_non_interactive_defaults, is_power_of_two,
    next_share_index, round_down_power_of_two, round_up_by_multiple_of, round_up_power_of_two,
    round_up_power_of_two_strict, size, sub_tree_width,
};
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
