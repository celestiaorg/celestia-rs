//! Inputs shared by the Go v1 oracle tests: the mainnet InitChain request,
//! the archived mainnet blocks and the oracle fixtures
//! (`conformance/fixtures/mainnet/v1-oracle`).

#![allow(clippy::expect_used, clippy::unwrap_used, dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead as _, BufReader};

use celestia_proto::celestia_core_proto::abci::{
    self, CommitInfo, Event, EventAttribute, RequestFinalizeBlock, RequestInitChain, VoteInfo,
};
use celestia_proto::celestia_core_proto::tendermint::types::{
    BlockId, BlockParams, ConsensusParams, EvidenceParams, Header, PartSetHeader, ValidatorParams,
    VersionParams,
};
use celestia_proto::celestia_core_proto::tendermint::version::Consensus;
use flate2::read::GzDecoder;

/// The oracle fixtures.
pub const ORACLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/mainnet/v1-oracle"
);
/// The archived mainnet blocks (`blocks.jsonl.gz`).
pub const BLOCKS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/mainnet/blocks.jsonl.gz"
);
/// SHA-256 of the canonical mainnet genesis.
pub const GENESIS_SHA256: &str = "9727aac9bbfb021ce7fc695a92f901986421283a891b89e0af97bc9fad187793";

/// Key/value contents of one store.
pub type Kv = BTreeMap<Vec<u8>, Vec<u8>>;

fn int(v: &serde_json::Value) -> i64 {
    v.as_str().unwrap().parse().unwrap()
}

/// `RequestInitChain` as Tendermint v0.34 builds it from the genesis.
pub fn init_request(g: &serde_json::Value) -> RequestInitChain {
    let cp = &g["consensus_params"];
    let nanos = int(&cp["evidence"]["max_age_duration"]);
    RequestInitChain {
        time: Some(timestamp(g["genesis_time"].as_str().unwrap())),
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

/// The verified mainnet genesis named by `CELESTIA_MAINNET_GENESIS`.
pub fn mainnet_genesis() -> serde_json::Value {
    let path = std::env::var("CELESTIA_MAINNET_GENESIS")
        .expect("set CELESTIA_MAINNET_GENESIS to the mainnet genesis.json");
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(hex::encode(hash::sha256(&bytes)), GENESIS_SHA256);
    serde_json::from_slice(&bytes).unwrap()
}

/// The lines of a gzipped JSON-lines file.
pub fn gz_lines(path: &str) -> impl Iterator<Item = serde_json::Value> {
    BufReader::new(GzDecoder::new(std::fs::File::open(path).unwrap()))
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
}

fn hex_field(v: &serde_json::Value) -> Vec<u8> {
    hex::decode(v.as_str().unwrap()).unwrap()
}

/// An RFC 3339 time.
pub fn timestamp(s: &str) -> pbjson_types::Timestamp {
    let (seconds, nanos) = sdk::time::parse_rfc3339_nano(s.as_bytes()).unwrap();
    pbjson_types::Timestamp { seconds, nanos }
}

/// The archived mainnet blocks 1 to 11 (`result` of the RPC `block`).
pub fn archive() -> BTreeMap<i64, serde_json::Value> {
    gz_lines(BLOCKS)
        .filter_map(|d| {
            let h = d["height"].as_i64().unwrap();
            (1..=11).contains(&h).then(|| (h, d["result"].clone()))
        })
        .collect()
}

/// The block's header as a proto.
pub fn header(block: &serde_json::Value) -> Header {
    let h = &block["block"]["header"];
    let id = &h["last_block_id"];
    Header {
        version: Some(Consensus {
            block: u64::try_from(int(&h["version"]["block"])).unwrap(),
            app: u64::try_from(int(&h["version"]["app"])).unwrap(),
        }),
        chain_id: h["chain_id"].as_str().unwrap().to_owned(),
        height: int(&h["height"]),
        time: Some(timestamp(h["time"].as_str().unwrap())),
        last_block_id: Some(BlockId {
            hash: hex_field(&id["hash"]),
            part_set_header: Some(PartSetHeader {
                total: u32::try_from(id["parts"]["total"].as_u64().unwrap()).unwrap(),
                hash: hex_field(&id["parts"]["hash"]),
            }),
        }),
        last_commit_hash: hex_field(&h["last_commit_hash"]),
        data_hash: hex_field(&h["data_hash"]),
        validators_hash: hex_field(&h["validators_hash"]),
        next_validators_hash: hex_field(&h["next_validators_hash"]),
        consensus_hash: hex_field(&h["consensus_hash"]),
        app_hash: hex_field(&h["app_hash"]),
        last_results_hash: hex_field(&h["last_results_hash"]),
        evidence_hash: hex_field(&h["evidence_hash"]),
        proposer_address: hex_field(&h["proposer_address"]),
    }
}

/// A Tendermint validator set: `(address, power)` sorted by power
/// descending, then address.
pub type ValidatorSet = Vec<(Vec<u8>, i64)>;

/// Sorts a validator set as Tendermint orders it.
pub fn sort_validators(set: &mut ValidatorSet) {
    set.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
}

/// Applies EndBlock validator updates (`(address, power)`, power 0
/// removes).
pub fn apply_updates(set: &ValidatorSet, updates: &[(Vec<u8>, i64)]) -> ValidatorSet {
    let mut m: BTreeMap<Vec<u8>, i64> = set.iter().cloned().collect();
    for (a, p) in updates {
        if *p == 0 {
            m.remove(a);
        } else {
            m.insert(a.clone(), *p);
        }
    }
    let mut out: ValidatorSet = m.into_iter().collect();
    sort_validators(&mut out);
    out
}

/// The last commit of `block` as vote infos over the validator set that
/// signed it (Tendermint v0.34 `getBeginBlockValidatorInfo`).
pub fn decided_last_commit(block: &serde_json::Value, last_vals: &ValidatorSet) -> CommitInfo {
    let lc = &block["block"]["last_commit"];
    let sigs = lc["signatures"].as_array().unwrap();
    if int(&lc["height"]) == 0 {
        assert!(sigs.is_empty());
        return CommitInfo::default();
    }
    assert_eq!(sigs.len(), last_vals.len(), "last commit size");
    let votes = sigs
        .iter()
        .zip(last_vals)
        .map(|(s, (addr, power))| {
            let flag = i32::try_from(s["block_id_flag"].as_i64().unwrap()).unwrap();
            if flag != 1 {
                assert_eq!(hex_field(&s["validator_address"]), *addr);
            }
            VoteInfo {
                validator: Some(abci::Validator {
                    address: addr.clone(),
                    power: *power,
                }),
                block_id_flag: flag,
            }
        })
        .collect();
    CommitInfo {
        round: i32::try_from(lc["round"].as_i64().unwrap()).unwrap(),
        votes,
    }
}

/// `RequestFinalizeBlock` for an archived block.
pub fn finalize_request(
    block: &serde_json::Value,
    last_vals: &ValidatorSet,
) -> RequestFinalizeBlock {
    let h = header(block);
    assert!(
        block["block"]["evidence"]["evidence"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let txs = block["block"]["data"]["txs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD
                .decode(t.as_str().unwrap())
                .unwrap()
        })
        .collect();
    RequestFinalizeBlock {
        txs,
        decided_last_commit: Some(decided_last_commit(block, last_vals)),
        misbehavior: Vec::new(),
        hash: hex_field(&block["block_id"]["hash"]),
        height: h.height,
        time: h.time,
        next_validators_hash: h.next_validators_hash.clone(),
        proposer_address: h.proposer_address.clone(),
        header: Some(h),
    }
}

/// One block's KV changes of one store.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Diff {
    /// Keys set, with their values.
    pub set: Kv,
    /// Keys deleted.
    pub delete: BTreeSet<Vec<u8>>,
}

/// The oracle's KV diffs, by height and store.
pub fn oracle_diffs() -> BTreeMap<i64, BTreeMap<String, Diff>> {
    let mut out: BTreeMap<i64, BTreeMap<String, Diff>> = BTreeMap::new();
    for d in gz_lines(&format!("{ORACLE}/kv-diffs.jsonl.gz")) {
        let diff = Diff {
            set: d["set"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| (hex_field(&p[0]), hex_field(&p[1])))
                .collect(),
            delete: d["delete"]
                .as_array()
                .unwrap()
                .iter()
                .map(hex_field)
                .collect(),
        };
        out.entry(d["height"].as_i64().unwrap())
            .or_default()
            .insert(d["store"].as_str().unwrap().to_owned(), diff);
    }
    out
}

/// The oracle's per-block results (`tx-results.jsonl.gz`), by height.
pub fn oracle_results() -> BTreeMap<i64, serde_json::Value> {
    gz_lines(&format!("{ORACLE}/tx-results.jsonl.gz"))
        .map(|d| (d["height"].as_i64().unwrap(), d))
        .collect()
}

/// The oracle's store roots and app hashes (`store-roots.json`).
pub fn oracle_roots() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(format!("{ORACLE}/store-roots.json")).unwrap()).unwrap()
}

/// Events from the oracle's JSON.
pub fn events(v: &serde_json::Value) -> Vec<Event> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|e| Event {
            r#type: e["type"].as_str().unwrap().to_owned(),
            attributes: e["attributes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| EventAttribute {
                    key: a["key"].as_str().unwrap().to_owned(),
                    value: a["value"].as_str().unwrap().to_owned(),
                    index: a["index"].as_bool().unwrap(),
                })
                .collect(),
        })
        .collect()
}

/// The oracle's EndBlock validator updates as `(address, power)`.
pub fn oracle_updates(result: &serde_json::Value) -> Vec<(Vec<u8>, i64)> {
    result["validator_updates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| (hex_field(&u["address"]), u["power"].as_i64().unwrap()))
        .collect()
}

/// Every event of a block in `FinalizeBlock` order: BeginBlock's, each
/// transaction's, EndBlock's.
pub fn oracle_block_events(result: &serde_json::Value) -> Vec<Event> {
    let mut out = events(&result["begin_block_events"]);
    for tx in result["txs"].as_array().unwrap() {
        out.extend(events(&tx["events"]));
    }
    out.extend(events(&result["end_block_events"]));
    out
}

/// The first difference between two event lists, for a readable failure.
pub fn first_event_difference(got: &[Event], want: &[Event]) -> Option<String> {
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        if g != w {
            return Some(format!("event {i}: rs {g:?}\ngo {w:?}"));
        }
    }
    (got.len() != want.len()).then(|| format!("event count rs {} go {}", got.len(), want.len()))
}
