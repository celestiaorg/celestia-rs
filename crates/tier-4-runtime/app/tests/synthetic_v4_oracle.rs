//! The synthetic chain across the v3 to v4 upgrade, as the Go oracle ran it
//! (`conformance/v4-oracle`, fixture `conformance/fixtures/synthetic/v4`).
//!
//! celestia-app v3.13.0 commits block 15 of the synthetic app version 1 to
//! 3 chain (`conformance/fixtures/synthetic/v1-v3`), whose EndBlocker
//! schedules plan `v4` at height 16. celestia-app v4.1.0 then opens the
//! same database, as the multiplexer does, and executes blocks 16 to 20.
//! These tests check that the fixture is the one the oracle wrote, that it
//! continues the v1-v3 chain, and pin what Go does at the upgrade block, so
//! the Rust hand-off has a fixed target.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::collections::{BTreeMap, BTreeSet};

use celestia_proto::celestia_core_proto::tendermint::types::ConsensusParams;
use common::{Diff, Kv};
use prost::Message as _;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v4"
);
const V1_V3: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v1-v3"
);
/// The last block of celestia-app v3.13.0.
const HANDOFF: i64 = 15;
/// The upgrade block: plan `v4` runs in its PreBlocker.
const UPGRADE: i64 = 16;

fn read(dir: &str, name: &str) -> Vec<u8> {
    std::fs::read(format!("{dir}/{name}")).unwrap()
}

fn gunzip(name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    std::io::Read::read_to_end(
        &mut flate2::read::GzDecoder::new(&read(FIXTURE, name)[..]),
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

fn roots() -> serde_json::Value {
    json(FIXTURE, "store-roots.json")
}

fn height(h: i64) -> serde_json::Value {
    let r = roots();
    r["heights"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["height"] == h)
        .unwrap()
        .clone()
}

fn stores(h: i64) -> BTreeSet<String> {
    height(h)["stores"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect()
}

fn diffs() -> BTreeMap<i64, BTreeMap<String, Diff>> {
    common::diffs_in(&format!("{FIXTURE}/kv-diffs.jsonl.gz"))
}

fn results() -> BTreeMap<i64, serde_json::Value> {
    common::gz_lines(&format!("{FIXTURE}/tx-results.jsonl.gz"))
        .map(|d| (d["height"].as_i64().unwrap(), d))
        .collect()
}

/// The oracle's state at the hand-off height, by store.
fn handoff_state() -> BTreeMap<String, Kv> {
    common::gz_lines(&format!("{FIXTURE}/kv-handoff.jsonl.gz"))
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

/// The fixture files are the ones the oracle wrote, for the v1-v3 chain.
#[test]
fn v4_fixture_files_match_their_record() {
    let r = roots();
    assert_eq!(r["chain_id"], "test");
    assert_eq!(r["celestia_app"], "v4.1.0");
    assert_eq!(r["handoff_height"], HANDOFF);
    assert_eq!(
        r["genesis_sha256"],
        sha256_hex(&read(V1_V3, "genesis.json"))
    );
    assert_eq!(r["blocks_sha256"], sha256_hex(&gunzip("blocks.jsonl.gz")));
    assert_eq!(
        r["tx_results_sha256"],
        sha256_hex(&gunzip("tx-results.jsonl.gz"))
    );
    assert_eq!(
        r["kv"]["kv-handoff.jsonl"],
        sha256_hex(&gunzip("kv-handoff.jsonl.gz"))
    );
    assert_eq!(
        r["kv"]["kv-upgrade.jsonl"],
        sha256_hex(&gunzip("kv-upgrade.jsonl.gz"))
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
    assert_eq!(heights, (HANDOFF..=20).collect::<Vec<_>>());
}

/// The v4 blocks continue the v1-v3 chain: the hand-off state is the
/// v1-v3 oracle's state after block 15, and block 16 points at block 15.
#[test]
fn v4_fixture_continues_the_v1_v3_chain() {
    let v1 = json(V1_V3, "store-roots.json");
    let v1_15 = &v1["heights"][usize::try_from(HANDOFF).unwrap()];
    assert_eq!(v1_15["height"], HANDOFF);
    let h15 = height(HANDOFF);
    assert_eq!(h15["app_hash"], v1_15["app_hash"]);
    assert_eq!(h15["stores"], v1_15["stores"]);
    assert_eq!(h15["keys"], v1_15["keys"]);

    let v1_blocks: BTreeMap<i64, serde_json::Value> =
        common::gz_lines(&format!("{V1_V3}/blocks.jsonl.gz"))
            .map(|d| (d["height"].as_i64().unwrap(), d["result"].clone()))
            .collect();
    let blocks: BTreeMap<i64, serde_json::Value> =
        common::gz_lines(&format!("{FIXTURE}/blocks.jsonl.gz"))
            .map(|d| (d["height"].as_i64().unwrap(), d["result"].clone()))
            .collect();
    assert_eq!(
        blocks.keys().copied().collect::<Vec<_>>(),
        (16..=20).collect::<Vec<_>>()
    );
    let header = &blocks[&UPGRADE]["block"]["header"];
    assert_eq!(header["last_block_id"], v1_blocks[&HANDOFF]["block_id"]);
    assert_eq!(header["app_hash"], v1_15["app_hash"]);
    assert_eq!(header["version"]["app"], "4");
    for h in 17..=20 {
        let header = &blocks[&h]["block"]["header"];
        assert_eq!(header["app_hash"], height(h - 1)["app_hash"], "{h}");
        assert_eq!(header["last_block_id"], blocks[&(h - 1)]["block_id"], "{h}");
    }
}

/// `UpgradeStoreLoader(16, …)`: the commit info of block 16 adds circuit,
/// consensus, hyperlane, minfee and warp (IAVL trees whose first version
/// is 16, like every other store's) and drops the v0.46 memory store
/// `mem_capability`; the `Deleted` stores crisis and blobstream were never
/// mounted.
#[test]
fn v4_upgrade_block_changes_the_store_set() {
    let before = stores(HANDOFF);
    let after = stores(UPGRADE);
    assert!(before.contains("mem_capability"));
    assert!(!before.iter().any(|s| s == "crisis" || s == "blobstream"));
    let added: Vec<&String> = after.difference(&before).collect();
    let removed: Vec<&String> = before.difference(&after).collect();
    assert_eq!(
        added,
        ["circuit", "consensus", "hyperlane", "minfee", "warp"]
    );
    assert_eq!(removed, ["mem_capability"]);
    let h16 = height(UPGRADE);
    for (store, version) in h16["versions"].as_object().unwrap() {
        assert_eq!(version, UPGRADE, "{store}");
    }
    assert_eq!(stores(20), after);
}

/// The app version stays 4: the v3 EndBlocker of block 15 set it in the
/// legacy params subspace `baseapp`; the `v4` handler copies the block,
/// evidence and validator params from there (cosmos-sdk v0.50
/// `baseapp.GetConsensusParams` does not read `VersionParams`), so it
/// stores `Version{App: 3}`, and `x/upgrade` `ApplyUpgrade` then sets
/// 3 + 1 = 4. The legacy subspace is left as it was.
#[test]
fn v4_upgrade_block_keeps_app_version_4() {
    let results = results();
    for (h, r) in &results {
        assert_eq!(r["consensus_param_updates"]["version"]["app"], "4", "{h}");
    }
    let state = handoff_state();
    let params = &state["params"];
    let legacy = |k: &str| -> serde_json::Value {
        serde_json::from_slice(&params[format!("baseapp/{k}").as_bytes()]).unwrap()
    };
    assert_eq!(legacy("VersionParams")["app_version"], "4");

    let diffs = diffs();
    let d16 = &diffs[&UPGRADE];
    assert!(
        !d16.contains_key("params"),
        "the upgrade leaves x/params alone"
    );
    let consensus = &d16["consensus"];
    assert!(consensus.delete.is_empty());
    assert_eq!(consensus.set.len(), 1);
    let cp = ConsensusParams::decode(&consensus.set[b"Consensus".as_slice()][..]).unwrap();
    assert_eq!(cp.version.unwrap().app, 4);
    let block = cp.block.unwrap();
    let blk = legacy("BlockParams");
    assert_eq!(
        block.max_bytes.to_string(),
        blk["max_bytes"].as_str().unwrap()
    );
    assert_eq!(block.max_gas.to_string(), blk["max_gas"].as_str().unwrap());
    let evidence = cp.evidence.unwrap();
    let ev = legacy("EvidenceParams");
    assert_eq!(
        evidence.max_age_num_blocks.to_string(),
        ev["max_age_num_blocks"].as_str().unwrap()
    );
    assert_eq!(
        evidence.max_bytes.to_string(),
        ev["max_bytes"].as_str().unwrap()
    );
    assert_eq!(cp.validator.unwrap().pub_key_types, ["ed25519"]);
    assert!(cp.abci.is_none());
    for h in 17..=20 {
        assert!(
            !diffs.get(&h).is_some_and(|d| d.contains_key("consensus")),
            "{h}"
        );
    }
}

/// What plan `v4` writes to x/upgrade and x/circuit at block 16: the plan
/// is cleared and marked done at 16, the module version map is the v4
/// one, and the circuit breaker disables the three upgrade messages.
#[test]
fn v4_upgrade_block_runs_plan_v4() {
    let diffs = diffs();
    let upgrade = &diffs[&UPGRADE]["upgrade"];
    assert_eq!(upgrade.delete, BTreeSet::from([vec![0x00]]), "plan cleared");
    let mut done = vec![0x01];
    done.extend_from_slice(&UPGRADE.to_be_bytes());
    done.extend_from_slice(b"v4");
    assert_eq!(upgrade.set[&done], [0x01]);
    let versions: BTreeMap<String, u64> = upgrade
        .set
        .iter()
        .filter(|(k, _)| k[0] == 0x02)
        .map(|(k, v)| {
            (
                String::from_utf8(k[1..].to_vec()).unwrap(),
                u64::from_be_bytes(v[..].try_into().unwrap()),
            )
        })
        .collect();
    let want: BTreeMap<String, u64> = [
        ("06-solomachine", 0),
        ("07-tendermint", 0),
        ("auth", 5),
        ("bank", 4),
        ("blob", 3),
        ("circuit", 1),
        ("consensus", 1),
        ("distribution", 4),
        ("gov", 5),
        ("hyperlane", 1),
        ("ibc", 6),
        ("minfee", 2),
        ("packetfowardmiddleware", 3),
        ("signal", 3),
        ("slashing", 4),
        ("staking", 5),
        ("transfer", 5),
        ("upgrade", 3),
        ("warp", 1),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    assert_eq!(versions, want, "module versions written at block 16");

    let circuit: BTreeSet<Vec<u8>> = diffs[&UPGRADE]["circuit"].set.keys().cloned().collect();
    let want: BTreeSet<Vec<u8>> = [
        "/cosmos.upgrade.v1beta1.MsgCancelUpgrade",
        "/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade",
        "/ibc.core.client.v1.MsgIBCSoftwareUpgrade",
    ]
    .iter()
    .map(|m| [&[0x02][..], m.as_bytes()].concat())
    .collect();
    assert_eq!(circuit, want);
}

/// Every transaction of the v4 blocks succeeds, and the oracle's timeouts
/// are celestia-app v4's.
#[test]
fn v4_blocks_execute() {
    let results = results();
    assert_eq!(
        results.keys().copied().collect::<Vec<_>>(),
        (16..=20).collect::<Vec<_>>()
    );
    for (h, r) in &results {
        for tx in r["txs"].as_array().unwrap() {
            assert_eq!(tx["code"], 0, "height {h}: {}", tx["log"]);
        }
        assert_eq!(
            r["timeouts"]["timeout_propose_ns"], 3_500_000_000_i64,
            "{h}"
        );
        assert_eq!(r["timeouts"]["timeout_commit_ns"], 4_200_000_000_i64, "{h}");
    }
}
