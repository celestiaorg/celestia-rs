//! The synthetic app version 1 to 3 chain against the Go oracle
//! (`conformance/v1-oracle -synthetic`, fixture
//! `conformance/fixtures/synthetic/v1-v3`).
//!
//! celestia-app v3.13.0 runs the chain from a genesis at app version 1:
//! the v1 to v2 upgrade at the EndBlocker of block 5
//! (`--v2-upgrade-height 6`), the signalled upgrade to app version 3 at
//! block 10 and the scheduling of the v4 upgrade at block 15. The test
//! runs InitChain and the blocks through the cosmos-sdk v0.46 driver and
//! requires, for each block up to [`MATCHED_THROUGH`], the oracle's KV
//! changes of every store, its store roots, the app hash, every
//! transaction result and the block's events, validator updates,
//! consensus params and timeouts.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::print_stdout)]

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::Diff;
use common::check::update_set;
use common::fixture::Fixture;

fn fix() -> Fixture {
    Fixture::synthetic("v1-v3")
}

/// The last block of the synthetic chain that celestia-rs executes as Go
/// does: every block, through the scheduling of the v4 upgrade.
const MATCHED_THROUGH: i64 = 15;

fn read(name: &str) -> Vec<u8> {
    fix().read(name)
}

fn gunzip(name: &str) -> Vec<u8> {
    fix().gunzip(name)
}

fn roots() -> serde_json::Value {
    fix().roots()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(hash::sha256(bytes))
}

/// The fixture files are the ones the oracle wrote: each SHA-256 that
/// `store-roots.json` records matches.
#[test]
fn synthetic_fixture_files_match_their_record() {
    let r = roots();
    assert_eq!(r["chain_id"], "test");
    assert_eq!(r["upgrade_height_v2"], 6);
    assert_eq!(r["genesis_sha256"], sha256_hex(&read("genesis.json")));
    assert_eq!(r["blocks_sha256"], sha256_hex(&gunzip("blocks.jsonl.gz")));
    assert_eq!(
        r["tx_results_sha256"],
        sha256_hex(&gunzip("tx-results.jsonl.gz"))
    );
    assert_eq!(
        r["kv"]["kv-height-0.jsonl"],
        sha256_hex(&gunzip("kv-height-0.jsonl.gz"))
    );
    assert_eq!(
        r["kv"]["kv-diffs.jsonl"],
        sha256_hex(&gunzip("kv-diffs.jsonl.gz"))
    );
    let heights: Vec<i64> = r["heights"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["height"].as_i64().unwrap())
        .collect();
    assert_eq!(heights, (0..=15).collect::<Vec<_>>());
}

/// What the chain is meant to exercise is in the oracle's record: the
/// app version of each block's consensus params, the store set before
/// and after the v1 to v2 migration, and the scheduled v4 plan.
#[test]
fn synthetic_chain_crosses_app_versions_1_to_3() {
    let results = results();
    let version = |h: i64| {
        results[&h]["consensus_param_updates"]["version"]["app_version"]
            .as_u64()
            .unwrap()
    };
    let versions: Vec<u64> = (1..=15).map(version).collect();
    assert_eq!(
        versions,
        [1, 1, 1, 1, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 4],
        "EndBlock app versions"
    );
    let r = roots();
    let stores = |h: usize| -> BTreeSet<String> {
        r["heights"][h]["stores"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    };
    let v1 = stores(4);
    let v2 = stores(5);
    assert!(v1.contains("qgb") && !v2.contains("qgb"));
    for s in ["icahost", "packetfowardmiddleware", "signal"] {
        assert!(!v1.contains(s) && v2.contains(s), "{s}");
    }
    assert_eq!(stores(15), v2);
    // The upgrade store holds the plan only after block 15.
    let diffs = oracle_diffs();
    assert!(diffs[&15].contains_key("upgrade"));
    assert!((1..15).all(|h| !diffs.get(&h).is_some_and(|d| d.contains_key("upgrade"))));
}

fn results() -> BTreeMap<i64, serde_json::Value> {
    fix().results()
}

fn oracle_diffs() -> BTreeMap<i64, BTreeMap<String, Diff>> {
    fix().diffs()
}

fn blocks() -> BTreeMap<i64, serde_json::Value> {
    fix().blocks()
}

#[test]
fn synthetic_blocks_match_the_oracle() {
    fix().run_blocks(MATCHED_THROUGH);
}

/// The node's [`app::App`] on a persistent home runs the whole synthetic
/// chain with the oracle's app hashes, reopening after the v1 to v2
/// upgrade (block 5), inside app version 2 (block 8) and inside app
/// version 3 (block 12). After block 15 the state is at app version 4
/// with the v4 plan scheduled, and the next block runs on the cosmos-sdk
/// v0.50 app.
#[test]
fn synthetic_app_runs_the_chain_across_reopens() {
    let genesis: serde_json::Value = serde_json::from_slice(&read("genesis.json")).unwrap();
    let r = roots();
    let heights = r["heights"].as_array().unwrap();
    let results = results();
    let blocks = blocks();
    let upgrade_height_v2 = r["upgrade_height_v2"].as_i64().unwrap();
    let home = tempfile::tempdir().unwrap();
    let open = || {
        let mut app = app::App::open(home.path()).unwrap();
        app.set_v2_upgrade_height(upgrade_height_v2);
        app
    };
    let mut app = open();
    let init = app.init_chain(common::init_request(&genesis)).unwrap();
    let mut vals: BTreeMap<i64, common::ValidatorSet> = BTreeMap::new();
    let mut genesis_set = update_set(&init.validators);
    common::sort_validators(&mut genesis_set);
    vals.insert(1, genesis_set.clone());
    vals.insert(2, genesis_set);
    for h in 1..15 {
        let next = common::apply_updates(&vals[&(h + 1)], &common::oracle_updates(&results[&h]));
        vals.insert(h + 2, next);
    }
    let versions = [(5, 2), (8, 2), (12, 3)];
    for height in 1..=15i64 {
        let last = if height == 1 {
            Vec::new()
        } else {
            vals[&(height - 1)].clone()
        };
        let res = app
            .finalize_block(common::finalize_request(&blocks[&height], &last))
            .unwrap_or_else(|e| panic!("height {height}: {}", e.log));
        app.commit().unwrap();
        let want = heights[usize::try_from(height).unwrap()]["app_hash"]
            .as_str()
            .unwrap();
        assert_eq!(
            hex::encode_upper(&res.app_hash),
            want,
            "app hash at {height}"
        );
        if let Some(&(_, version)) = versions.iter().find(|(h, _)| *h == height) {
            drop(app);
            app = open();
            assert!(app.is_sdk_v046_state());
            let info = app.info();
            assert_eq!(
                (info.last_block_height, info.app_version),
                (height, version)
            );
            assert_eq!(hex::encode_upper(&info.last_block_app_hash), want);
        }
    }
    assert_eq!(app.info().app_version, 4);
    // Block 16 is the scheduled v4 upgrade: the app hands off to the
    // cosmos-sdk v0.50 app (`tests/v4_handoff_oracle.rs` checks that block
    // against the Go oracle).
    let mut next = common::finalize_request(&blocks[&15], &vals[&15]);
    next.height = 16;
    app.finalize_block(next)
        .unwrap_or_else(|e| panic!("height 16: {}", e.log));
    assert!(!app.is_sdk_v046_state());
}
