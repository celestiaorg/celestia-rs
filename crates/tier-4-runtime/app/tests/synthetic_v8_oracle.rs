//! The synthetic chain across the v6 to v8 upgrade (mainnet skipped v7),
//! as the Go oracles ran it: `conformance/v6-oracle -heights 10
//! -next-handoff` (fixture `conformance/fixtures/synthetic/v6-to-v8`,
//! blocks 37 to 46) and `conformance/v8-oracle` (fixture
//! `conformance/fixtures/synthetic/v8`, celestia-app v8.0.8, blocks 47 to
//! 51).
//!
//! In the v6 blocks accounts send to module accounts (v6 blocks none), bob
//! becomes a validator at 25% with a 30% max, the validators signal version
//! 8 and the EndBlocker of block 46 schedules plan `v8` at 47.
//! celestia-app v8.0.8 opens the same database, adds the zkism store, runs
//! the `v8` handler and executes blocks 47 to 51, where the same sends fail
//! except the one to gov. These tests check that the fixtures continue one
//! chain and pin what Go does at the upgrade.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::collections::BTreeMap;

use celestia_proto::cosmos::staking::v1beta1::{Params, Validator};
use common::Kv;
use prost::Message as _;

const V6: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v6"
);
const V6_TO_V8: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v6-to-v8"
);
const V8: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v8"
);
const V1_V3: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v1-v3"
);
/// The last block of celestia-app v5.
const V6_HANDOFF: i64 = 36;
/// The last block of celestia-app v6: its EndBlocker schedules plan `v8`.
const HANDOFF: i64 = 46;
/// The upgrade block: plan `v8` runs in its PreBlocker.
const UPGRADE: i64 = 47;
const LAST: i64 = 51;
/// One `LegacyDec` unit: 10^18.
const DEC: u128 = 1_000_000_000_000_000_000;

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
fn v8_fixture_files_match_their_record() {
    check_record(
        V6_TO_V8,
        "conformance/v6-oracle",
        "v6.4.4",
        V6_HANDOFF,
        HANDOFF,
    );
    check_record(V8, "conformance/v8-oracle", "v8.0.8", HANDOFF, LAST);
}

/// `v6-to-v8` runs the blocks of `v6` first.
#[test]
fn v6_to_v8_extends_the_v6_fixture() {
    let (short, long) = (heights(V6), heights(V6_TO_V8));
    for (h, want) in &short {
        assert_eq!(&long[h], want, "{h}");
    }
    let (short, long) = (blocks(V6), blocks(V6_TO_V8));
    for (h, want) in &short {
        assert_eq!(&long[h], want, "{h}");
    }
    assert_eq!(
        read(V6, "kv-handoff.jsonl.gz"),
        read(V6_TO_V8, "kv-handoff.jsonl.gz")
    );
    assert_eq!(state_at(V6, 41), state_at(V6_TO_V8, 41));
}

/// The v8 blocks continue the v6 chain: the hand-off state is the v6
/// oracle's state after block 46, and block 47 points at block 46.
#[test]
fn v8_fixture_continues_the_v6_chain() {
    let v6 = heights(V6_TO_V8);
    let v8 = heights(V8);
    assert_eq!(v8[&HANDOFF]["app_hash"], v6[&HANDOFF]["app_hash"]);
    assert_eq!(v8[&HANDOFF]["stores"], v6[&HANDOFF]["stores"]);
    let mut handoff = kv_lines(V8, "kv-handoff.jsonl.gz");
    handoff.retain(|_, kv| !kv.is_empty());
    assert_eq!(handoff, state_at(V6_TO_V8, HANDOFF));

    let (v6_blocks, v8_blocks) = (blocks(V6_TO_V8), blocks(V8));
    assert_eq!(
        v8_blocks.keys().copied().collect::<Vec<_>>(),
        (UPGRADE..=LAST).collect::<Vec<_>>()
    );
    let header = &v8_blocks[&UPGRADE]["block"]["header"];
    assert_eq!(header["last_block_id"], v6_blocks[&HANDOFF]["block_id"]);
    assert_eq!(header["app_hash"], v6[&HANDOFF]["app_hash"]);
    for h in UPGRADE + 1..=LAST {
        let header = &v8_blocks[&h]["block"]["header"];
        assert_eq!(header["app_hash"], v8[&(h - 1)]["app_hash"], "{h}");
        assert_eq!(
            header["last_block_id"],
            v8_blocks[&(h - 1)]["block_id"],
            "{h}"
        );
    }
    for (h, b) in &v6_blocks {
        assert_eq!(b["block"]["header"]["version"]["app"], "6", "{h}");
    }
    for (h, b) in &v8_blocks {
        assert_eq!(b["block"]["header"]["version"]["app"], "8", "{h}");
    }
}

/// The transaction codes of a fixture, by height.
fn codes(dir: &str) -> BTreeMap<i64, Vec<i64>> {
    common::gz_lines(&format!("{dir}/tx-results.jsonl.gz"))
        .map(|d| {
            let codes = d["txs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["code"].as_i64().unwrap())
                .collect();
            (d["height"].as_i64().unwrap(), codes)
        })
        .collect()
}

/// v6 lets every module account receive (its `BlockedAddresses` hashes the
/// module addresses twice): the sends of blocks 42 and 45 to the fee
/// collector, distribution, gov and the bonded pool succeed. v8 blocks
/// every module account but gov's: in blocks 47 and 48 the sends to the fee
/// collector, distribution and the bonded pool fail with `sdk` code 4
/// (unauthorized), and the sends to gov (47, 50) succeed.
#[test]
fn v8_blocks_module_accounts_but_gov() {
    for (h, c) in codes(V6_TO_V8) {
        assert!(c.iter().all(|&c| c == 0), "{h}: {c:?}");
    }
    let v8 = codes(V8);
    assert_eq!(v8[&47], [4, 4, 0]);
    assert_eq!(v8[&48], [0, 4]);
    for h in 49..=LAST {
        assert!(v8[&h].iter().all(|&c| c == 0), "{h}: {:?}", v8[&h]);
    }
    let lines: BTreeMap<i64, serde_json::Value> =
        common::gz_lines(&format!("{V8}/tx-results.jsonl.gz"))
            .map(|d| (d["height"].as_i64().unwrap(), d))
            .collect();
    for (h, i) in [(47, 0), (47, 1), (48, 1)] {
        let tx = &lines[&h]["txs"][i];
        assert_eq!(tx["codespace"], "sdk");
        assert!(
            tx["log"]
                .as_str()
                .unwrap()
                .ends_with("is not allowed to receive funds: unauthorized"),
            "{h}/{i}: {}",
            tx["log"]
        );
    }
}

fn validators(state: &BTreeMap<String, Kv>) -> BTreeMap<String, Validator> {
    state["staking"]
        .iter()
        .filter(|(k, _)| k.first() == Some(&0x21))
        .map(|(_, v)| {
            let v = Validator::decode(&v[..]).unwrap();
            (v.description.clone().unwrap().moniker, v)
        })
        .collect()
}

fn dec(s: &str) -> u128 {
    s.parse().unwrap()
}

/// The `v8` upgrade: the store loader adds zkism at 47 (an IAVL tree whose
/// first version is 47); the handler sets a 20% minimum commission and
/// raises alice (10%, max 20%) and carol (10%, max 10%) to 20% with a 60%
/// max and bob (25%, max 30%) to a 60% max, at block 47's time, and leaves
/// the genesis validators (50%, max 100%) alone; the migrations record the
/// versions of zkism and forwarding.
#[test]
fn v8_upgrade_adds_zkism_and_raises_commissions() {
    let v8 = heights(V8);
    let names = |h: i64| {
        v8[&h]["versions"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    };
    let (before, after) = (names(HANDOFF), names(UPGRADE));
    let added: Vec<&String> = after.iter().filter(|s| !before.contains(s)).collect();
    assert_eq!(added, ["zkism"]);
    assert!(before.iter().all(|s| after.contains(s)));
    assert_eq!(v8[&UPGRADE]["versions"]["zkism"], UPGRADE);

    let upgrade = common::diffs_in(&format!("{V8}/kv-upgrade.jsonl.gz"));
    let stores = &upgrade[&UPGRADE];
    assert_eq!(
        stores.keys().map(String::as_str).collect::<Vec<_>>(),
        ["staking", "upgrade"]
    );
    let version_keys: Vec<Vec<u8>> = stores["upgrade"]
        .set
        .keys()
        .filter(|k| k.first() == Some(&0x02))
        .cloned()
        .collect();
    assert_eq!(
        version_keys,
        [b"\x02forwarding".to_vec(), b"\x02zkism".to_vec()]
    );

    let before = state_at(V6_TO_V8, HANDOFF);
    let mut after = before.clone();
    for (store, d) in stores {
        let kv = after.entry(store.clone()).or_default();
        for k in &d.delete {
            kv.remove(k);
        }
        kv.extend(d.set.clone());
    }
    let params = Params::decode(&after["staking"][&vec![0x51]][..]).unwrap();
    assert_eq!(dec(&params.min_commission_rate), DEC / 5);

    let time = blocks(V8)[&UPGRADE]["block"]["header"]["time"].clone();
    let time = common::timestamp(time.as_str().unwrap());
    let (old, new) = (validators(&before), validators(&after));
    assert_eq!(old.len(), 7);
    for (name, rate, max_rate, new_rate) in [
        ("alice", 10, 20, 20),
        ("carol", 10, 10, 20),
        ("bob", 25, 30, 25),
    ] {
        let c = old[name]
            .commission
            .clone()
            .unwrap()
            .commission_rates
            .unwrap();
        assert_eq!(dec(&c.rate), DEC * rate / 100, "{name}");
        assert_eq!(dec(&c.max_rate), DEC * max_rate / 100, "{name}");
        let commission = new[name].commission.clone().unwrap();
        let c = commission.commission_rates.unwrap();
        assert_eq!(dec(&c.rate), DEC * new_rate / 100, "{name}");
        assert_eq!(dec(&c.max_rate), DEC * 60 / 100, "{name}");
        assert_eq!(commission.update_time.unwrap(), time, "{name}");
    }
    for name in ["val0", "val1", "val2", "val3"] {
        assert_eq!(old[name], new[name], "{name}");
    }
}
