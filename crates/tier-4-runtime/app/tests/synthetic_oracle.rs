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

use app::legacy::{Driver, V1Modules};
use common::check::{check_response, check_tx_results, contents, describe, diff, update_set};
use common::{Diff, Kv};
use sdk::MultiStore;

/// The fixture directory.
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v1-v3"
);

/// The last block of the synthetic chain that celestia-rs executes as Go
/// does. Block 4 holds a `MsgSignalVersion` at app version 1, whose
/// cosmos-sdk v0.46 `ValidateBasic` is not ported yet; block 5 runs the
/// v1 to v2 upgrade.
const MATCHED_THROUGH: i64 = 4;

fn read(name: &str) -> Vec<u8> {
    std::fs::read(format!("{FIXTURE}/{name}")).unwrap()
}

fn gunzip(name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&read(name)[..]), &mut out)
        .unwrap();
    out
}

fn roots() -> serde_json::Value {
    serde_json::from_slice(&read("store-roots.json")).unwrap()
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
    common::gz_lines(&format!("{FIXTURE}/tx-results.jsonl.gz"))
        .map(|d| (d["height"].as_i64().unwrap(), d))
        .collect()
}

fn oracle_diffs() -> BTreeMap<i64, BTreeMap<String, Diff>> {
    common::diffs_in(&format!("{FIXTURE}/kv-diffs.jsonl.gz"))
}

fn blocks() -> BTreeMap<i64, serde_json::Value> {
    common::gz_lines(&format!("{FIXTURE}/blocks.jsonl.gz"))
        .map(|d| (d["height"].as_i64().unwrap(), d["result"].clone()))
        .collect()
}

/// The oracle's state at height 0, by store.
fn height_0() -> BTreeMap<String, Kv> {
    common::gz_lines(&format!("{FIXTURE}/kv-height-0.jsonl.gz"))
        .map(|d| {
            let kv = d["kv"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    (
                        hex::decode(p[0].as_str().unwrap()).unwrap(),
                        hex::decode(p[1].as_str().unwrap()).unwrap(),
                    )
                })
                .collect();
            (d["store"].as_str().unwrap().to_owned(), kv)
        })
        .collect()
}

fn v1_store() -> MultiStore {
    let mut ms = MultiStore::new();
    let v1 = sdk::version::Release::for_app_version(1).unwrap();
    app::stores::mount_release(&mut ms, v1).unwrap();
    ms
}

/// The IAVL stores of a height's commit info.
fn iavl_stores(roots: &serde_json::Value) -> Vec<String> {
    roots["stores"]
        .as_object()
        .unwrap()
        .keys()
        .filter(|s| *s != "mem_capability")
        .cloned()
        .collect()
}

#[test]
#[allow(clippy::too_many_lines)]
fn synthetic_blocks_match_the_oracle() {
    let genesis: serde_json::Value = serde_json::from_slice(&read("genesis.json")).unwrap();
    let r = roots();
    let heights = r["heights"].as_array().unwrap();
    let results = results();
    let mut want_diffs = oracle_diffs();
    let blocks = blocks();

    let mut ms = v1_store();
    let mut driver = Driver::new(V1Modules);
    let init = driver
        .init_chain(&mut ms, &common::init_request(&genesis))
        .unwrap();
    assert_eq!(init.app_hash, hash::sha256(&[]).to_vec());
    assert_eq!(r["init_chain_app_hash"], hex::encode_upper(&init.app_hash));

    // InitChain writes the oracle's height-0 state (Go commits it alone as
    // height 0; block 1 runs on it uncommitted).
    let state0 = height_0();
    let stores0 = iavl_stores(&heights[0]);
    assert_eq!(
        stores0,
        state0.keys().cloned().collect::<Vec<_>>(),
        "height 0 stores"
    );
    for (s, want) in &state0 {
        let got = contents(&ms, s);
        assert!(
            got == *want,
            "height 0 {s}: rs vs go {}",
            describe(&diff(want, &got))
        );
    }

    // Validator sets by height: the genesis set signs heights 1 and 2,
    // and each EndBlock(h)'s updates take effect at h + 2.
    let mut vals: BTreeMap<i64, common::ValidatorSet> = BTreeMap::new();
    let mut genesis_set = update_set(&init.validators);
    common::sort_validators(&mut genesis_set);
    vals.insert(1, genesis_set.clone());
    vals.insert(2, genesis_set);
    for h in 1..15 {
        let next = common::apply_updates(&vals[&(h + 1)], &common::oracle_updates(&results[&h]));
        vals.insert(h + 2, next);
    }

    let mut before = state0;
    for height in 1..=MATCHED_THROUGH {
        let last = if height == 1 {
            Vec::new()
        } else {
            vals[&(height - 1)].clone()
        };
        let req = common::finalize_request(&blocks[&height], &last);
        let res = driver
            .finalize_block(&mut ms, &req)
            .unwrap_or_else(|e| panic!("height {height}: {}", e.log));
        let id = driver.commit(&mut ms).unwrap();

        let want_roots = &heights[usize::try_from(height).unwrap()];
        let want_diff = want_diffs.remove(&height).unwrap_or_default();
        let stores = iavl_stores(want_roots);
        let info = ms.root().last_commit_info().unwrap().clone();
        let mut failures = Vec::new();
        let mut names: Vec<&str> = info.store_infos.iter().map(|s| s.name.as_str()).collect();
        names.sort_unstable();
        let want_names: Vec<&str> = want_roots["stores"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        if names != want_names {
            failures.push(format!("commit info stores {names:?}"));
        }
        let mut after = BTreeMap::new();
        for s in &stores {
            let now = if ms.is_mounted(s) {
                contents(&ms, s)
            } else {
                Kv::new()
            };
            let got = diff(before.get(s).unwrap_or(&Kv::new()), &now);
            let want = want_diff.get(s).cloned().unwrap_or_default();
            if got != want {
                failures.push(format!(
                    "{s}: kv diff\n  rs {}\n  go {}",
                    describe(&got),
                    describe(&want)
                ));
            }
            let root = info
                .store_info(s)
                .and_then(|si| si.commit_id.as_ref())
                .map(|c| hex::encode_upper(c.hash))
                .unwrap_or_default();
            if root != want_roots["stores"][s].as_str().unwrap() {
                failures.push(format!("{s}: root {root}"));
            }
            after.insert(s.clone(), now);
        }
        before = after;
        let app_hash = hex::encode_upper(id.hash);
        if app_hash != want_roots["app_hash"].as_str().unwrap()
            || hex::encode_upper(&res.app_hash) != app_hash
        {
            failures.push(format!(
                "app hash {app_hash}, want {}",
                want_roots["app_hash"]
            ));
        }
        failures.extend(check_tx_results(&res, &results[&height]));
        failures.extend(check_response(&res, &results[&height]));
        assert!(
            failures.is_empty(),
            "height {height}:\n{}",
            failures.join("\n")
        );
        println!("synthetic block {height} matches {app_hash}");
    }
}
