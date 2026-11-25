//! The blobstream EndBlocker against the Go v1 oracle
//! (`conformance/fixtures/mainnet/v1-oracle`): for each mainnet block
//! 1 to 10, the oracle's `staking` and `params` stores after the block
//! (the staking EndBlocker runs before blobstream's) and its `qgb` store
//! before it must turn into the oracle's `qgb` store after the block, with
//! the block's `AttestationRequest` events.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, Read as _};

use celestia_proto::celestia_core_proto::abci::Event;
use celestia_proto::celestia_core_proto::tendermint::types::VersionParams;
use flate2::read::GzDecoder;
use sdk::{Context, ExecMode, Header, Kind, MultiStore};

const ORACLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../conformance/fixtures/mainnet/v1-oracle"
);
const BLOCKS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../conformance/fixtures/mainnet/blocks.jsonl.gz"
);
/// The stores the EndBlocker reads or writes.
const STORES: [&str; 3] = ["params", "qgb", "staking"];

type Kv = BTreeMap<Vec<u8>, Vec<u8>>;

fn gz_lines(path: &str) -> impl Iterator<Item = serde_json::Value> {
    BufReader::new(GzDecoder::new(std::fs::File::open(path).unwrap()))
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
}

fn hex(v: &serde_json::Value) -> Vec<u8> {
    hex::decode(v.as_str().unwrap()).unwrap()
}

/// The oracle state of [`STORES`] at heights 0 to 10.
fn oracle_states() -> Vec<BTreeMap<String, Kv>> {
    let mut text = String::new();
    GzDecoder::new(std::fs::File::open(format!("{ORACLE}/kv-height-0-modules.jsonl.gz")).unwrap())
        .read_to_string(&mut text)
        .unwrap();
    let mut state: BTreeMap<String, Kv> = BTreeMap::new();
    for line in text.lines() {
        let d: serde_json::Value = serde_json::from_str(line).unwrap();
        let store = d["store"].as_str().unwrap();
        if STORES.contains(&store) {
            let kv = d["kv"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| (hex(&p[0]), hex(&p[1])))
                .collect();
            state.insert(store.to_owned(), kv);
        }
    }
    let mut states = vec![state];
    for d in gz_lines(&format!("{ORACLE}/kv-diffs.jsonl.gz")) {
        let height = usize::try_from(d["height"].as_u64().unwrap()).unwrap();
        if height > 10 {
            break;
        }
        while states.len() <= height {
            states.push(states.last().unwrap().clone());
        }
        let Some(kv) = states[height].get_mut(d["store"].as_str().unwrap()) else {
            continue;
        };
        for p in d["set"].as_array().unwrap() {
            kv.insert(hex(&p[0]), hex(&p[1]));
        }
        for k in d["delete"].as_array().unwrap() {
            kv.remove(&hex(k));
        }
    }
    while states.len() <= 10 {
        states.push(states.last().unwrap().clone());
    }
    states
}

/// Block times of heights 1 to 10.
fn block_times() -> BTreeMap<i64, pbjson_types::Timestamp> {
    gz_lines(BLOCKS)
        .filter_map(|d| {
            let h = d["height"].as_i64().unwrap();
            (1..=10).contains(&h).then(|| {
                let t = d["result"]["block"]["header"]["time"].as_str().unwrap();
                let (seconds, nanos) = sdk::time::parse_rfc3339_nano(t.as_bytes()).unwrap();
                (h, pbjson_types::Timestamp { seconds, nanos })
            })
        })
        .collect()
}

/// The `AttestationRequest` end-block events of heights 1 to 10.
fn oracle_events() -> BTreeMap<i64, Vec<Event>> {
    gz_lines(&format!("{ORACLE}/tx-results.jsonl.gz"))
        .map(|d| {
            let events = d["end_block_events"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e["type"] == "AttestationRequest")
                .map(|e| {
                    sdk::event::event(
                        e["type"].as_str().unwrap(),
                        e["attributes"].as_array().unwrap().iter().map(|a| {
                            assert_eq!(a["index"], true);
                            (
                                a["key"].as_str().unwrap().to_owned(),
                                a["value"].as_str().unwrap().to_owned(),
                            )
                        }),
                    )
                })
                .collect();
            (d["height"].as_i64().unwrap(), events)
        })
        .collect()
}

#[test]
fn end_blocker_matches_the_oracle_for_blocks_1_to_10() {
    let states = oracle_states();
    let times = block_times();
    let events = oracle_events();
    for height in 1..=10i64 {
        let h = usize::try_from(height).unwrap();
        let mut ms = MultiStore::new();
        for store in STORES {
            ms.mount(store, Kind::Iavl).unwrap();
            let src = if store == "qgb" {
                &states[h - 1]
            } else {
                &states[h]
            };
            for (k, v) in &src[store] {
                ms.set(store, k.clone(), v.clone()).unwrap();
            }
        }
        let header = Header {
            chain_id: "celestia".into(),
            height,
            time: times[&height],
            ..Header::default()
        };
        let mut ctx = Context::new(&mut ms, header, ExecMode::Finalize);
        ctx.consensus_params.version = Some(VersionParams { app: 1 });
        x_qgb::end_block(&mut ctx).unwrap();
        let got_events = ctx.events.take();
        drop(ctx);
        let got: Kv = ms
            .range("qgb", None, None, false)
            .unwrap()
            .into_iter()
            .collect();
        let want = &states[h]["qgb"];
        for (k, v) in want {
            assert_eq!(
                got.get(k).map(hex::encode),
                Some(hex::encode(v)),
                "height {height}: qgb key {}",
                String::from_utf8_lossy(k)
            );
        }
        assert_eq!(got.len(), want.len(), "height {height}: qgb key count");
        assert_eq!(got_events, events[&height], "height {height}: events");
    }
}
