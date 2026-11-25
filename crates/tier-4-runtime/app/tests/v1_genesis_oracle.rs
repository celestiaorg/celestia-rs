//! App version 1 InitGenesis against the Go v1 oracle
//! (`conformance/v1-oracle`): InitChain on the canonical mainnet genesis,
//! then a genesis-only Commit, must reproduce the oracle's height-0 state
//! store by store, and its app hash.
//!
//! Needs the mainnet genesis (88 MB, SHA-256 `9727aac9…7793`, URL in
//! `crates/tier-5-node/celestia-rs/src/networks.rs`):
//!
//! ```sh
//! CELESTIA_MAINNET_GENESIS=<genesis.json> cargo test -p app --test v1_genesis_oracle -- --ignored
//! ```
//!
//! The committed dump has every store but `acc` and `bank`; those two are
//! checked by root here, and by content against the full oracle dump
//! (`kv-height-0.jsonl.gz`, regenerated with the oracle) when
//! `CELESTIA_V1_ORACLE_KV` names it.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::print_stdout)]

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, Read as _};

use app::legacy::{Driver, V1Modules};
use celestia_proto::celestia_core_proto::abci::RequestInitChain;
use celestia_proto::celestia_core_proto::tendermint::types::{
    BlockParams, ConsensusParams, EvidenceParams, ValidatorParams, VersionParams,
};
use flate2::read::GzDecoder;
use sdk::MultiStore;

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/mainnet/v1-oracle"
);
const GENESIS_SHA256: &str = "9727aac9bbfb021ce7fc695a92f901986421283a891b89e0af97bc9fad187793";

/// The stores expected to match the oracle at height 0.
const MATCHING: &[&str] = &[
    "acc",
    "authz",
    "bank",
    "blob",
    "capability",
    "distribution",
    "evidence",
    "feegrant",
    "gov",
    "ibc",
    "mint",
    "params",
    "qgb",
    "slashing",
    "staking",
    "transfer",
    "upgrade",
];

fn int(v: &serde_json::Value) -> i64 {
    v.as_str().unwrap().parse().unwrap()
}

/// `RequestInitChain` as Tendermint v0.34 builds it from the genesis.
fn init_request(g: &serde_json::Value) -> RequestInitChain {
    let cp = &g["consensus_params"];
    let nanos = int(&cp["evidence"]["max_age_duration"]);
    let (secs, frac) =
        sdk::time::parse_rfc3339_nano(g["genesis_time"].as_str().unwrap().as_bytes())
            .expect("genesis time");
    RequestInitChain {
        time: Some(pbjson_types::Timestamp {
            seconds: secs,
            nanos: frac,
        }),
        chain_id: g["chain_id"].as_str().unwrap().to_owned(),
        consensus_params: Some(ConsensusParams {
            block: Some(BlockParams {
                max_bytes: int(&cp["block"]["max_bytes"]),
                max_gas: int(&cp["block"]["max_gas"]),
            }),
            evidence: Some(EvidenceParams {
                max_age_num_blocks: int(&cp["evidence"]["max_age_num_blocks"]),
                max_age_duration: Some(pbjson_types::Duration {
                    seconds: nanos / 1_000_000_000,
                    nanos: i32::try_from(nanos % 1_000_000_000).unwrap(),
                }),
                max_bytes: int(&cp["evidence"]["max_bytes"]),
            }),
            validator: Some(ValidatorParams {
                pub_key_types: cp["validator"]["pub_key_types"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|t| t.as_str().unwrap().to_owned())
                    .collect(),
            }),
            version: Some(VersionParams {
                app: u64::try_from(int(&cp["version"]["app_version"])).unwrap(),
            }),
            ..ConsensusParams::default()
        }),
        validators: Vec::new(),
        app_state_bytes: serde_json::to_vec(&g["app_state"]).unwrap(),
        initial_height: int(&g["initial_height"]),
    }
}

struct Oracle {
    roots: BTreeMap<String, String>,
    app_hash: String,
    kv: BTreeMap<String, Vec<(String, String)>>,
}

fn oracle() -> Oracle {
    let roots: serde_json::Value =
        serde_json::from_slice(&std::fs::read(format!("{FIXTURES}/store-roots.json")).unwrap())
            .unwrap();
    let h0 = &roots["heights"][0];
    assert_eq!(h0["height"], 0);
    let mut text = String::new();
    GzDecoder::new(
        std::fs::File::open(format!("{FIXTURES}/kv-height-0-modules.jsonl.gz")).unwrap(),
    )
    .read_to_string(&mut text)
    .unwrap();
    let kv = text
        .lines()
        .map(|l| {
            let d: serde_json::Value = serde_json::from_str(l).unwrap();
            let pairs = d["kv"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    (
                        p[0].as_str().unwrap().to_owned(),
                        p[1].as_str().unwrap().to_owned(),
                    )
                })
                .collect();
            (d["store"].as_str().unwrap().to_owned(), pairs)
        })
        .collect();
    Oracle {
        roots: h0["stores"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
            .collect(),
        app_hash: h0["app_hash"].as_str().unwrap().to_owned(),
        kv,
    }
}

/// InitChain plus a genesis-only Commit (the oracle's height 0).
fn genesis_state() -> MultiStore {
    let path = std::env::var("CELESTIA_MAINNET_GENESIS")
        .expect("set CELESTIA_MAINNET_GENESIS to the mainnet genesis.json");
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(hex::encode(hash::sha256(&bytes)), GENESIS_SHA256);
    let g: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mut ms = MultiStore::new();
    let v1 = sdk::version::Release::for_app_version(1).unwrap();
    app::stores::mount_release(&mut ms, v1).unwrap();
    let res = Driver::new(V1Modules)
        .init_chain(&mut ms, &init_request(&g))
        .unwrap();
    assert_eq!(res.app_hash, hash::sha256(&[]).to_vec());
    assert_eq!(res.validators.len(), 75);
    ms.flush().unwrap();
    ms.commit().unwrap();
    ms
}

fn hex_kv(ms: &MultiStore, store: &str) -> Vec<(String, String)> {
    ms.range(store, None, None, false)
        .unwrap()
        .into_iter()
        .map(|(k, v)| (hex::encode(k), hex::encode(v)))
        .collect()
}

/// Every store's root, key/value contents (where committed) and the app
/// hash, as a table; the stores in [`MATCHING`] must match.
#[test]
#[ignore = "needs the mainnet genesis: CELESTIA_MAINNET_GENESIS=<genesis.json>"]
fn v1_init_genesis_matches_the_oracle() {
    let oracle = oracle();
    let ms = genesis_state();
    let info = ms.root().last_commit_info().unwrap().clone();
    let mut failures = Vec::new();
    println!("store          keys(rs/go)     kv     root");
    for (store, want_root) in &oracle.roots {
        let Some(si) = info.store_info(store) else {
            failures.push(format!("{store}: not in the commit info"));
            continue;
        };
        let root = si
            .commit_id
            .as_ref()
            .map(|c| hex::encode_upper(c.hash))
            .unwrap_or_default();
        let root_ok = root == *want_root;
        let (kv_cell, kv_ok, keys) = match oracle.kv.get(store) {
            Some(want) => {
                let got = hex_kv(&ms, store);
                let ok = got == *want;
                if !ok {
                    let first = got.iter().zip(want).find(|(a, b)| a != b).map_or_else(
                        || "length".to_owned(),
                        |(a, b)| format!("rs {a:?} go {b:?}"),
                    );
                    println!("  {store}: first difference: {first}");
                    let count = |kv: &[(String, String)]| {
                        let mut m = BTreeMap::<String, usize>::new();
                        for (k, _) in kv {
                            *m.entry(k[..2].to_owned()).or_default() += 1;
                        }
                        m
                    };
                    println!(
                        "  {store}: keys by first byte rs {:?} go {:?}",
                        count(&got),
                        count(want)
                    );
                }
                (
                    if ok { "match" } else { "DIFF" },
                    ok,
                    format!("{}/{}", got.len(), want.len()),
                )
            }
            None if store == "mem_capability" => ("-", true, "-".to_owned()),
            None => ("root", root_ok, format!("{}/-", hex_kv(&ms, store).len())),
        };
        println!(
            "{store:<14} {keys:<15} {kv_cell:<6} {}",
            if root_ok { "match" } else { "DIFF" }
        );
        if MATCHING.contains(&store.as_str()) && !(kv_ok && root_ok) {
            failures.push(format!("{store}: kv {kv_cell}, root {root} != {want_root}"));
        }
    }
    let app_hash = hex::encode_upper(ms.root().last_commit_id().unwrap().hash);
    println!("app hash {app_hash} (oracle {})", oracle.app_hash);
    if MATCHING.len() + 1 == oracle.roots.len() {
        assert_eq!(app_hash, oracle.app_hash);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// `acc` and `bank` against the full oracle dump, streamed.
#[test]
#[ignore = "needs CELESTIA_MAINNET_GENESIS and CELESTIA_V1_ORACLE_KV=<kv-height-0.jsonl.gz>"]
fn v1_init_genesis_acc_and_bank_match_the_full_dump() {
    let path = std::env::var("CELESTIA_V1_ORACLE_KV")
        .expect("set CELESTIA_V1_ORACLE_KV to the oracle's kv-height-0.jsonl.gz");
    let ms = genesis_state();
    let reader = BufReader::new(GzDecoder::new(std::fs::File::open(path).unwrap()));
    let mut seen = Vec::new();
    for line in reader.lines() {
        let d: serde_json::Value = serde_json::from_str(&line.unwrap()).unwrap();
        let store = d["store"].as_str().unwrap();
        if store != "acc" && store != "bank" {
            continue;
        }
        let got = hex_kv(&ms, store);
        let want = d["kv"].as_array().unwrap();
        assert_eq!(got.len(), want.len(), "{store} key count");
        for (g, w) in got.iter().zip(want) {
            assert_eq!(
                (g.0.as_str(), g.1.as_str()),
                (w[0].as_str().unwrap(), w[1].as_str().unwrap()),
                "{store}"
            );
        }
        seen.push(store.to_owned());
    }
    assert_eq!(seen, ["acc", "bank"]);
}
