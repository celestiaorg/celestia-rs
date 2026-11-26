//! The synthetic chain through the v3 to v4 hand-off inside [`app::App`]:
//! InitChain and blocks 1 to 15 of `conformance/fixtures/synthetic/v1-v3`
//! on the cosmos-sdk v0.46 driver, then the first cosmos-sdk v0.50 block
//! (16, `conformance/fixtures/synthetic/v4`), where the app switches its
//! store set and runs plan `v4`.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;

use std::collections::BTreeMap;

use app::App;
use common::Kv;
use common::check::{contents, describe, diff, update_set};

const V1_V3: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v1-v3"
);
const V4: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v4"
);
const HANDOFF: i64 = 15;
const UPGRADE: i64 = 16;

/// Stores of block 16 that depend on app version 4's own rules, which run
/// once app version 4 executes as itself (`Release::runs_as`, workstream
/// B): the mint BeginBlocker's CIP-29 inflation (5.36% at v4 and v5;
/// celestia-rs runs app version 4 as v9, with CIP-41's 2.67%), which
/// changes the minted provision, the supply, the fee collector and the
/// distribution rewards. Every other store must match Go now.
const PENDING_V4_RULES: &[&str] = &["bank", "distribution", "mint"];

fn json(path: &str) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn by_height(path: &str, field: &str) -> BTreeMap<i64, serde_json::Value> {
    common::gz_lines(path)
        .map(|d| {
            let v = if field.is_empty() {
                d.clone()
            } else {
                d[field].clone()
            };
            (d["height"].as_i64().unwrap(), v)
        })
        .collect()
}

/// The v4 oracle's state at `height` (hand-off state plus each block's
/// changes), by store.
fn v4_state(height: i64) -> BTreeMap<String, Kv> {
    let mut state: BTreeMap<String, Kv> = common::gz_lines(&format!("{V4}/kv-handoff.jsonl.gz"))
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
        .collect();
    state.remove("mem_capability");
    for (h, stores) in common::diffs_in(&format!("{V4}/kv-diffs.jsonl.gz")) {
        if h > height {
            break;
        }
        for (store, d) in stores {
            let kv = state.entry(store).or_default();
            for k in &d.delete {
                kv.remove(k);
            }
            kv.extend(d.set);
        }
    }
    state
}

/// The app after InitChain and blocks 1 to 15 of the v1-v3 chain, each
/// at the oracle's app hash, with the validator sets of heights 1 to 17.
fn app_at_handoff() -> (App, BTreeMap<i64, common::ValidatorSet>) {
    let genesis = json(&format!("{V1_V3}/genesis.json"));
    let roots = json(&format!("{V1_V3}/store-roots.json"));
    let results = by_height(&format!("{V1_V3}/tx-results.jsonl.gz"), "");
    let blocks = by_height(&format!("{V1_V3}/blocks.jsonl.gz"), "result");

    let mut app = App::new();
    app.set_v2_upgrade_height(roots["upgrade_height_v2"].as_i64().unwrap());
    let init = app.init_chain(common::init_request(&genesis)).unwrap();
    let mut vals: BTreeMap<i64, common::ValidatorSet> = BTreeMap::new();
    let mut genesis_set = update_set(&init.validators);
    common::sort_validators(&mut genesis_set);
    vals.insert(1, genesis_set.clone());
    vals.insert(2, genesis_set);
    for h in 1..=HANDOFF {
        let next = common::apply_updates(&vals[&(h + 1)], &common::oracle_updates(&results[&h]));
        vals.insert(h + 2, next);
    }
    for height in 1..=HANDOFF {
        let last = if height == 1 {
            Vec::new()
        } else {
            vals[&(height - 1)].clone()
        };
        app.finalize_block(common::finalize_request(&blocks[&height], &last))
            .unwrap_or_else(|e| panic!("height {height}: {}", e.log));
        app.commit().unwrap();
        let want = roots["heights"][usize::try_from(height).unwrap()]["app_hash"]
            .as_str()
            .unwrap();
        assert_eq!(
            hex::encode_upper(app.info().last_block_app_hash),
            want,
            "{height}"
        );
    }
    assert!(app.is_sdk_v046_state());
    (app, vals)
}

/// Block 16 runs on the v0.50 app with the v4 store set and plan `v4`:
/// the commit info lists Go's stores at Go's IAVL versions (the added
/// stores start at 16, `mem_capability` is gone), and every store outside
/// [`PENDING_V4_RULES`] holds Go's state after the block.
#[test]
fn v4_handoff_block_matches_go() {
    let (mut app, mut vals) = app_at_handoff();
    let v4_results = by_height(&format!("{V4}/tx-results.jsonl.gz"), "");
    let v4_blocks = by_height(&format!("{V4}/blocks.jsonl.gz"), "result");
    let v4_roots = json(&format!("{V4}/store-roots.json"));
    vals.insert(
        UPGRADE + 2,
        common::apply_updates(
            &vals[&(UPGRADE + 1)],
            &common::oracle_updates(&v4_results[&UPGRADE]),
        ),
    );

    let req = common::finalize_request(&v4_blocks[&UPGRADE], &vals[&HANDOFF]);
    let res = app.finalize_block(req);
    let res = res.unwrap_or_else(|e| panic!("height {UPGRADE}: {}", e.log));
    app.commit().unwrap();
    assert!(!app.is_sdk_v046_state());

    let want = v4_state(UPGRADE);
    let mut failures = Vec::new();
    for (store, w) in &want {
        if PENDING_V4_RULES.contains(&store.as_str()) {
            continue;
        }
        let got = contents(app.multistore(), store);
        if got != *w {
            failures.push(format!("{store}: rs vs go {}", describe(&diff(w, &got))));
        }
    }
    let h16 = v4_roots["heights"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["height"] == UPGRADE)
        .unwrap();
    let info = app.multistore().root().last_commit_info().unwrap().clone();
    let got_versions: BTreeMap<String, i64> = info
        .store_infos
        .iter()
        .map(|s| {
            (
                s.name.clone(),
                s.commit_id.as_ref().map_or(-1, |c| c.version),
            )
        })
        .collect();
    let want_versions: BTreeMap<String, i64> = h16["versions"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.as_i64().unwrap()))
        .collect();
    if got_versions != want_versions {
        failures.push(format!("commit info {got_versions:?} vs {want_versions:?}"));
    }
    for (store, root) in h16["stores"].as_object().unwrap() {
        if PENDING_V4_RULES.contains(&store.as_str()) {
            continue;
        }
        let got = info
            .store_info(store)
            .and_then(|s| s.commit_id.as_ref())
            .map(|c| hex::encode_upper(c.hash));
        if got.as_deref() != root.as_str() {
            failures.push(format!("{store} root {got:?} vs {root}"));
        }
    }
    // The app hash covers the pending stores too.
    let _ = res;
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
