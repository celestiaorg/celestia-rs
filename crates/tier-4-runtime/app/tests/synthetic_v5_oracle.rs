//! The synthetic chain across the v4 to v5 upgrade, as the Go oracles ran
//! it: `conformance/v4-oracle -heights 10 -next-handoff` (fixture
//! `conformance/fixtures/synthetic/v4-to-v5`, blocks 16 to 25) and
//! `conformance/v5-oracle` (fixture `conformance/fixtures/synthetic/v5`,
//! blocks 26 to 30).
//!
//! The validators signal version 5 in blocks 18 and 21, a `MsgTryUpgrade`
//! in block 22 reaches quorum, and the v4 EndBlocker of block 25 sets app
//! version 5 and schedules plan `v5` at 26. celestia-app v5.0.12 then opens
//! the same database, as the multiplexer does, runs the `v5` handler in the
//! PreBlocker of block 26 and executes blocks 26 to 30. These tests check
//! that the fixtures are the ones the oracles wrote, that they continue one
//! chain, and pin what Go does at the upgrade, so the Rust hand-off has a
//! fixed target.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::collections::BTreeMap;

use common::Kv;

const V4: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v4"
);
const V4_TO_V5: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v4-to-v5"
);
const V5: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v5"
);
const V1_V3: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v1-v3"
);
/// The last block of celestia-app v3.13.0.
const V4_HANDOFF: i64 = 15;
/// The last block of celestia-app v4: its EndBlocker schedules plan `v5`.
const HANDOFF: i64 = 25;
/// The upgrade block: plan `v5` runs in its PreBlocker.
const UPGRADE: i64 = 26;
const LAST: i64 = 30;

fn read(dir: &str, name: &str) -> Vec<u8> {
    std::fs::read(format!("{dir}/{name}")).unwrap()
}

fn gunzip(dir: &str, name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    std::io::Read::read_to_end(
        &mut flate2::read::GzDecoder::new(&read(dir, name)[..]),
        &mut out,
    )
    .unwrap();
    out
}

fn json(dir: &str, name: &str) -> serde_json::Value {
    serde_json::from_slice(&read(dir, name)).unwrap()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(hash::sha256(bytes))
}

fn heights(dir: &str) -> BTreeMap<i64, serde_json::Value> {
    json(dir, "store-roots.json")["heights"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| (h["height"].as_i64().unwrap(), h.clone()))
        .collect()
}

fn by_height(dir: &str, name: &str, field: &str) -> BTreeMap<i64, serde_json::Value> {
    common::gz_lines(&format!("{dir}/{name}"))
        .map(|d| (d["height"].as_i64().unwrap(), d[field].clone()))
        .collect()
}

fn blocks(dir: &str) -> BTreeMap<i64, serde_json::Value> {
    by_height(dir, "blocks.jsonl.gz", "result")
}

fn kv_lines(dir: &str, name: &str) -> BTreeMap<String, Kv> {
    common::gz_lines(&format!("{dir}/{name}"))
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

/// A fixture's state at `height`: its hand-off state plus each block's
/// changes.
fn state_at(dir: &str, height: i64) -> BTreeMap<String, Kv> {
    let mut state = kv_lines(dir, "kv-handoff.jsonl.gz");
    for (h, stores) in common::diffs_in(&format!("{dir}/kv-diffs.jsonl.gz")) {
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
    state.retain(|_, kv| !kv.is_empty());
    state
}

fn check_record(dir: &str, generator: &str, app: &str, handoff: i64, last: i64) {
    let r = json(dir, "store-roots.json");
    assert_eq!(r["chain_id"], "test");
    assert_eq!(r["generator"], generator);
    assert_eq!(r["celestia_app"], app);
    assert_eq!(r["handoff_height"], handoff);
    assert_eq!(
        r["genesis_sha256"],
        sha256_hex(&read(V1_V3, "genesis.json"))
    );
    assert_eq!(
        r["blocks_sha256"],
        sha256_hex(&gunzip(dir, "blocks.jsonl.gz"))
    );
    assert_eq!(
        r["tx_results_sha256"],
        sha256_hex(&gunzip(dir, "tx-results.jsonl.gz"))
    );
    for name in ["kv-handoff", "kv-upgrade", "kv-diffs"] {
        assert_eq!(
            r["kv"][format!("{name}.jsonl")],
            sha256_hex(&gunzip(dir, &format!("{name}.jsonl.gz"))),
            "{name}"
        );
    }
    assert_eq!(
        heights(dir).keys().copied().collect::<Vec<_>>(),
        (handoff..=last).collect::<Vec<_>>()
    );
}

/// The fixture files are the ones the oracles wrote, for the v1-v3 chain.
#[test]
fn v5_fixture_files_match_their_record() {
    check_record(
        V4_TO_V5,
        "conformance/v4-oracle",
        "v4.1.0",
        V4_HANDOFF,
        HANDOFF,
    );
    check_record(V5, "conformance/v5-oracle", "v5.0.12", HANDOFF, LAST);
}

/// `v4-to-v5` runs the blocks of `v4` first: heights 15 to 20 are the same
/// blocks with the same results and state.
#[test]
fn v4_to_v5_extends_the_v4_fixture() {
    let (short, long) = (heights(V4), heights(V4_TO_V5));
    for (h, want) in &short {
        assert_eq!(&long[h], want, "{h}");
    }
    let (short, long) = (blocks(V4), blocks(V4_TO_V5));
    for (h, want) in &short {
        assert_eq!(&long[h], want, "{h}");
    }
    assert_eq!(
        read(V4, "kv-handoff.jsonl.gz"),
        read(V4_TO_V5, "kv-handoff.jsonl.gz")
    );
    assert_eq!(state_at(V4, 20), state_at(V4_TO_V5, 20));
}

/// The v5 blocks continue the v4 chain: the hand-off state is the v4
/// oracle's state after block 25, and block 26 points at block 25.
#[test]
fn v5_fixture_continues_the_v4_chain() {
    let v4 = heights(V4_TO_V5);
    let v5 = heights(V5);
    assert_eq!(v5[&HANDOFF]["app_hash"], v4[&HANDOFF]["app_hash"]);
    assert_eq!(v5[&HANDOFF]["stores"], v4[&HANDOFF]["stores"]);
    assert_eq!(v5[&HANDOFF]["keys"], v4[&HANDOFF]["keys"]);
    // kv-handoff lists empty stores too; the diffs do not.
    let mut handoff = kv_lines(V5, "kv-handoff.jsonl.gz");
    handoff.retain(|_, kv| !kv.is_empty());
    assert_eq!(handoff, state_at(V4_TO_V5, HANDOFF));

    let (v4_blocks, v5_blocks) = (blocks(V4_TO_V5), blocks(V5));
    assert_eq!(
        v5_blocks.keys().copied().collect::<Vec<_>>(),
        (UPGRADE..=LAST).collect::<Vec<_>>()
    );
    let header = &v5_blocks[&UPGRADE]["block"]["header"];
    assert_eq!(header["last_block_id"], v4_blocks[&HANDOFF]["block_id"]);
    assert_eq!(header["app_hash"], v4[&HANDOFF]["app_hash"]);
    for h in UPGRADE + 1..=LAST {
        let header = &v5_blocks[&h]["block"]["header"];
        assert_eq!(header["app_hash"], v5[&(h - 1)]["app_hash"], "{h}");
        assert_eq!(
            header["last_block_id"],
            v5_blocks[&(h - 1)]["block_id"],
            "{h}"
        );
    }
}

/// The signal and upgrade path on v4: every transaction of blocks 16 to 25
/// succeeds; the EndBlocker of block 25 returns app version 5 in its
/// consensus params and clears the signal store (`ResetTally`), so the
/// headers of blocks 16 to 25 carry app version 4 and those of 26 to 30
/// app version 5.
#[test]
fn v4_endblocker_of_block_25_sets_app_version_5() {
    let results = by_height(V4_TO_V5, "tx-results.jsonl.gz", "txs");
    for (h, txs) in &results {
        for tx in txs.as_array().unwrap() {
            assert_eq!(tx["code"], 0, "{h}: {}", tx["log"]);
        }
    }
    let params = by_height(V4_TO_V5, "tx-results.jsonl.gz", "consensus_param_updates");
    for (h, p) in &params {
        let want = if *h == HANDOFF { "5" } else { "4" };
        assert_eq!(p["version"]["app"], want, "{h}");
    }
    for (h, b) in blocks(V4_TO_V5) {
        assert_eq!(b["block"]["header"]["version"]["app"], "4", "{h}");
    }
    for (h, b) in blocks(V5) {
        assert_eq!(b["block"]["header"]["version"]["app"], "5", "{h}");
    }
    let signal = |h| state_at(V4_TO_V5, h).remove("signal").unwrap_or_default();
    assert!(!signal(HANDOFF - 1).is_empty());
    assert!(signal(HANDOFF).is_empty());
}

/// The `v5` upgrade changes no store but x/upgrade's: the PreBlocker of
/// block 26 alone deletes the plan (key 0x00) and records plan `v5` as done
/// at 26 (key 0x01 || height || name). The store set and the IAVL
/// versions do not change.
#[test]
fn v5_upgrade_touches_only_the_upgrade_store() {
    let upgrade = common::diffs_in(&format!("{V5}/kv-upgrade.jsonl.gz"));
    assert_eq!(upgrade.keys().copied().collect::<Vec<_>>(), [UPGRADE]);
    let stores = &upgrade[&UPGRADE];
    assert_eq!(stores.keys().collect::<Vec<_>>(), ["upgrade"]);
    let mut done = vec![0x01];
    done.extend(UPGRADE.to_be_bytes());
    done.extend(b"v5");
    assert_eq!(
        stores["upgrade"].set,
        Kv::from([(done.clone(), vec![0x01])])
    );
    assert_eq!(
        stores["upgrade"].delete.iter().collect::<Vec<_>>(),
        [&vec![0x00]]
    );
    let before = state_at(V4_TO_V5, HANDOFF);
    let plan = &before["upgrade"][&vec![0x00]];
    assert!(
        plan.windows(2).any(|w| w == b"v5"),
        "plan {}",
        hex::encode(plan)
    );

    let v5 = heights(V5);
    let names = |h: i64| {
        v5[&h]["versions"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(names(HANDOFF), names(UPGRADE));
    for (store, v) in v5[&UPGRADE]["versions"].as_object().unwrap() {
        assert_eq!(v, UPGRADE, "{store}");
    }
}
