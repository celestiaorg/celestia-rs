//! The synthetic chain across the v8 to v9 upgrade, as the Go oracles ran
//! it: `conformance/v8-oracle -heights 10 -next-handoff` (fixture
//! `conformance/fixtures/synthetic/v8-to-v9`, blocks 47 to 56) and
//! `conformance/v9-oracle` (fixture `conformance/fixtures/synthetic/v9`,
//! celestia-app v9.0.8 without the fibre build tag, blocks 57 to 61).
//!
//! The validators signal version 9, a `MsgTryUpgrade` in block 53 reaches
//! quorum, and the EndBlocker of block 56 schedules plan `v9` at 57.
//! celestia-app v9.0.8 opens the same database, runs the `v9` handler and
//! executes blocks 57 to 61. These tests check that the fixtures continue
//! one chain and pin what Go does at the upgrade.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::collections::BTreeMap;

use celestia_proto::celestia::blob::v1::Params as BlobParams;
use celestia_proto::celestia_core_proto::tendermint::types::ConsensusParams;
use celestia_proto::ibc::core::connection::v1::Params as ConnectionParams;
use common::Kv;
use prost::Message as _;

const V8: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v8"
);
const V8_TO_V9: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v8-to-v9"
);
const V9: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v9"
);
const V1_V3: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v1-v3"
);
/// The last block of celestia-app v6.
const V8_HANDOFF: i64 = 46;
/// The last block of celestia-app v8: its EndBlocker schedules plan `v9`.
const HANDOFF: i64 = 56;
/// The upgrade block: plan `v9` runs in its PreBlocker.
const UPGRADE: i64 = 57;
const LAST: i64 = 61;

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
fn v9_fixture_files_match_their_record() {
    check_record(
        V8_TO_V9,
        "conformance/v8-oracle",
        "v8.0.8",
        V8_HANDOFF,
        HANDOFF,
    );
    check_record(V9, "conformance/v9-oracle", "v9.0.8", HANDOFF, LAST);
}

/// `v8-to-v9` runs the blocks of `v8` first.
#[test]
fn v8_to_v9_extends_the_v8_fixture() {
    let (short, long) = (heights(V8), heights(V8_TO_V9));
    for (h, want) in &short {
        assert_eq!(&long[h], want, "{h}");
    }
    let (short, long) = (blocks(V8), blocks(V8_TO_V9));
    for (h, want) in &short {
        assert_eq!(&long[h], want, "{h}");
    }
    assert_eq!(
        read(V8, "kv-handoff.jsonl.gz"),
        read(V8_TO_V9, "kv-handoff.jsonl.gz")
    );
    assert_eq!(state_at(V8, 51), state_at(V8_TO_V9, 51));
}

/// The v9 blocks continue the v8 chain, and every transaction of both
/// fixtures after block 51 succeeds.
#[test]
fn v9_fixture_continues_the_v8_chain() {
    let v8 = heights(V8_TO_V9);
    let v9 = heights(V9);
    assert_eq!(v9[&HANDOFF]["app_hash"], v8[&HANDOFF]["app_hash"]);
    assert_eq!(v9[&HANDOFF]["stores"], v8[&HANDOFF]["stores"]);
    let mut handoff = kv_lines(V9, "kv-handoff.jsonl.gz");
    handoff.retain(|_, kv| !kv.is_empty());
    assert_eq!(handoff, state_at(V8_TO_V9, HANDOFF));

    let (v8_blocks, v9_blocks) = (blocks(V8_TO_V9), blocks(V9));
    assert_eq!(
        v9_blocks.keys().copied().collect::<Vec<_>>(),
        (UPGRADE..=LAST).collect::<Vec<_>>()
    );
    let header = &v9_blocks[&UPGRADE]["block"]["header"];
    assert_eq!(header["last_block_id"], v8_blocks[&HANDOFF]["block_id"]);
    assert_eq!(header["app_hash"], v8[&HANDOFF]["app_hash"]);
    for h in UPGRADE + 1..=LAST {
        let header = &v9_blocks[&h]["block"]["header"];
        assert_eq!(header["app_hash"], v9[&(h - 1)]["app_hash"], "{h}");
        assert_eq!(
            header["last_block_id"],
            v9_blocks[&(h - 1)]["block_id"],
            "{h}"
        );
    }
    for (h, b) in &v8_blocks {
        assert_eq!(b["block"]["header"]["version"]["app"], "8", "{h}");
    }
    for (h, b) in &v9_blocks {
        assert_eq!(b["block"]["header"]["version"]["app"], "9", "{h}");
    }
    for dir in [V8_TO_V9, V9] {
        for d in common::gz_lines(&format!("{dir}/tx-results.jsonl.gz")) {
            if d["height"].as_i64().unwrap() <= 51 {
                continue;
            }
            for tx in d["txs"].as_array().unwrap() {
                assert_eq!(tx["code"], 0, "{}: {}", d["height"], tx["log"]);
            }
        }
    }
}

/// The `v9` handler (`kv-upgrade`): consensus BlockMaxBytes 32 MiB and
/// evidence MaxAgeNumBlocks 559,940; blob GovMaxSquareSize 256; IBC
/// connection MaxExpectedTimePerBlock 13 s; plan `v9` done at 57. No store
/// is added (fibre's stores exist only with the fibre build tag), and the
/// FinalizeBlock consensus params of block 57 on carry the new block and
/// evidence params, which the headers of 58 on commit to.
#[test]
fn v9_upgrade_sets_the_v9_params() {
    let upgrade = common::diffs_in(&format!("{V9}/kv-upgrade.jsonl.gz"));
    let stores = &upgrade[&UPGRADE];
    assert_eq!(
        stores.keys().map(String::as_str).collect::<Vec<_>>(),
        ["blob", "consensus", "ibc", "upgrade"]
    );
    let mut done = vec![0x01];
    done.extend(UPGRADE.to_be_bytes());
    done.extend(b"v9");
    assert!(stores["upgrade"].set.contains_key(&done));

    let cp =
        ConsensusParams::decode(&stores["consensus"].set[b"Consensus".as_slice()][..]).unwrap();
    assert_eq!(cp.block.unwrap().max_bytes, 32 * 1024 * 1024);
    assert_eq!(cp.evidence.unwrap().max_age_num_blocks, 559_940);
    let blob = BlobParams::decode(&stores["blob"].set[b"params".as_slice()][..]).unwrap();
    assert_eq!(blob.gov_max_square_size, 256);
    let ibc =
        ConnectionParams::decode(&stores["ibc"].set[b"connectionParams".as_slice()][..]).unwrap();
    assert_eq!(ibc.max_expected_time_per_block, 13_000_000_000);

    let v9 = heights(V9);
    let names = |h: i64| {
        v9[&h]["versions"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(names(HANDOFF), names(UPGRADE));

    for d in common::gz_lines(&format!("{V9}/tx-results.jsonl.gz")) {
        let p = &d["consensus_param_updates"];
        assert_eq!(p["block"]["max_bytes"], "33554432", "{}", d["height"]);
        assert_eq!(
            p["evidence"]["max_age_num_blocks"], "559940",
            "{}",
            d["height"]
        );
        assert_eq!(p["version"]["app"], "9", "{}", d["height"]);
    }
}
