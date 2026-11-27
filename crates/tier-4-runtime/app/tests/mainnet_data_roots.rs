//! Block sync's data check on real mainnet blocks of every app version.
//!
//! `conformance/mainnet-rpc/data-root/campaign.py` sampled mainnet history
//! era by era (evenly spaced heights, the first and last 50 blocks of each
//! era, the largest blocks found in random windows) and rebuilt each
//! block's data square with `app::proposal::verified_block_eds`;
//! `hard-cases.json` keeps, per era, the blocks that stress the square
//! builders: era edges, the largest square, the most blobs and txs, share
//! version 1 blobs, PFB-only and empty blocks, and the longest tx. These
//! tests pin them: the small ones offline from
//! `fixtures/mainnet/data-root-hard-cases.jsonl.gz`, all of them from a
//! fresh fetch when `DATA_ROOT_BLOCKS` names one.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::BufRead as _;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::Value;

const CASES: &str = include_str!("../../../../conformance/mainnet-rpc/data-root/hard-cases.json");
const BODIES: &[u8] =
    include_bytes!("../../../../conformance/fixtures/mainnet/data-root-hard-cases.jsonl.gz");

/// Every app version mainnet has run (there was no app version 7).
const ERAS: [u64; 9] = [1, 2, 3, 4, 5, 6, 8, 9, 10];

fn cases() -> BTreeMap<u64, Value> {
    let v: Value = serde_json::from_str(CASES).unwrap();
    v["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["height"].as_u64().unwrap(), c.clone()))
        .collect()
}

fn blocks(reader: impl std::io::Read) -> impl Iterator<Item = Value> {
    std::io::BufReader::new(reader)
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
}

/// Rebuilds `block`'s square and requires the case's committed data hash
/// and square size.
fn check(block: &Value, case: &Value) {
    let height = block["height"].as_u64().unwrap();
    let version = block["app_version"].as_u64().unwrap();
    assert_eq!(case["app_version"], version, "{height}");
    assert_eq!(case["data_hash"], block["data_hash"], "{height}");
    assert_eq!(case["square_size"], block["square_size"], "{height}");
    let txs: Vec<Vec<u8>> = block["txs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| STANDARD.decode(t.as_str().unwrap()).unwrap())
        .collect();
    let hash = hex::decode(block["data_hash"].as_str().unwrap()).unwrap();
    let (eds, _) = app::proposal::verified_block_eds(version, &txs, &hash)
        .unwrap_or_else(|e| panic!("height {height} (v{version}): {e}"));
    assert_eq!(
        eds.width() as u64 / 2,
        block["square_size"].as_u64().unwrap(),
        "{height}"
    );
    if let Some(tx) = txs.first() {
        let mut changed = txs.clone();
        changed[0] = [tx.as_slice(), &[0]].concat();
        assert!(
            app::proposal::verified_block_eds(version, &changed, &hash).is_err(),
            "height {height}: a changed tx still matches"
        );
    }
}

/// The hard cases cover every era, and each era has its edges, its
/// largest square and an empty block; from v6 on, blobs with share version
/// 1 (a signer). The campaign found none in the 1,280 blocks it sampled
/// from v3 to v5.
#[test]
fn hard_cases_cover_every_era() {
    let mut reasons = BTreeMap::<u64, BTreeSet<String>>::new();
    for c in cases().values() {
        let v = c["app_version"].as_u64().unwrap();
        for r in c["reasons"].as_array().unwrap() {
            reasons
                .entry(v)
                .or_default()
                .insert(r.as_str().unwrap().to_owned());
        }
    }
    assert_eq!(reasons.keys().copied().collect::<Vec<_>>(), ERAS);
    for (v, rs) in &reasons {
        for want in [
            "era_first",
            "era_last",
            "largest_square",
            "most_blobs",
            "empty",
        ] {
            assert!(rs.contains(want), "v{v} has no {want} case");
        }
        if *v >= 6 {
            assert!(rs.contains("share_v1"), "v{v} has no share version 1 case");
        }
    }
}

/// The committed blocks of the hard cases rebuild their data root offline,
/// and a changed tx does not.
#[test]
fn committed_hard_cases_rebuild_their_data_root() {
    let cases = cases();
    let mut seen = BTreeSet::new();
    for block in blocks(flate2::read::GzDecoder::new(BODIES)) {
        let height = block["height"].as_u64().unwrap();
        check(&block, &cases[&height]);
        seen.insert(height);
    }
    let want: BTreeSet<u64> = cases
        .iter()
        .filter(|(_, c)| c["body_in_fixture"] == true)
        .map(|(h, _)| *h)
        .collect();
    assert_eq!(seen, want);
    let versions: BTreeSet<u64> = seen
        .iter()
        .map(|h| cases[h]["app_version"].as_u64().unwrap())
        .collect();
    assert_eq!(versions.into_iter().collect::<Vec<_>>(), ERAS);
}

/// Every hard case, large blocks included, from a fresh fetch:
///
/// ```sh
/// python3 conformance/mainnet-rpc/data-root/campaign.py fetch \
///   $(jq -r '.cases[].height' conformance/mainnet-rpc/data-root/hard-cases.json) > /tmp/blocks.jsonl
/// DATA_ROOT_BLOCKS=/tmp/blocks.jsonl cargo test -p app --test mainnet_data_roots -- --ignored
/// ```
#[test]
#[ignore = "needs DATA_ROOT_BLOCKS, fetched from mainnet RPC"]
fn fetched_hard_cases_rebuild_their_data_root() {
    let cases = cases();
    let path = std::env::var_os("DATA_ROOT_BLOCKS").expect("DATA_ROOT_BLOCKS");
    let mut seen = BTreeSet::new();
    for block in blocks(std::fs::File::open(path).unwrap()) {
        let height = block["height"].as_u64().unwrap();
        check(&block, &cases[&height]);
        seen.insert(height);
    }
    assert_eq!(seen, cases.keys().copied().collect());
}
