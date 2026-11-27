//! The synthetic chain across the v5 to v6 upgrade, as the Go oracles ran
//! it: `conformance/v5-oracle -heights 11 -next-handoff` (fixture
//! `conformance/fixtures/synthetic/v5-to-v6`, blocks 26 to 36) and
//! `conformance/v6-oracle` (fixture `conformance/fixtures/synthetic/v6`,
//! celestia-app v6.4.10, blocks 37 to 41).
//!
//! In the v5 blocks alice and carol become validators below v6's 10%
//! minimum commission, a solo machine (06-solomachine) opens an IBC client,
//! connection and transfer channel, and its uatom transfer is refused by
//! x/tokenfilter; the validators signal version 6 and the EndBlocker of
//! block 36 schedules plan `v6` at 37. celestia-app v6.4.10 opens the same
//! database, runs the `v6` handler and executes blocks 37 to 41, where the
//! solo machine's second transfer mints a voucher. These tests check that
//! the fixtures continue one chain and pin what Go does at the upgrade.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::collections::BTreeMap;

use celestia_proto::celestia_core_proto::tendermint::types::ConsensusParams;
use celestia_proto::cosmos::staking::v1beta1::{Params, Validator};
use celestia_proto::ibc::applications::interchain_accounts::host::v1::Params as HostParams;
use common::Kv;
use prost::Message as _;

const V5: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v5"
);
const V5_TO_V6: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v5-to-v6"
);
const V6: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v6"
);
const V1_V3: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v1-v3"
);
/// The last block of celestia-app v4.
const V5_HANDOFF: i64 = 25;
/// The last block of celestia-app v5: its EndBlocker schedules plan `v6`.
const HANDOFF: i64 = 36;
/// The upgrade block: plan `v6` runs in its PreBlocker.
const UPGRADE: i64 = 37;
const LAST: i64 = 41;
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
fn v6_fixture_files_match_their_record() {
    check_record(
        V5_TO_V6,
        "conformance/v5-oracle",
        "v5.0.12",
        V5_HANDOFF,
        HANDOFF,
    );
    check_record(V6, "conformance/v6-oracle", "v6.4.10", HANDOFF, LAST);
}

/// `v5-to-v6` runs the blocks of `v5` first.
#[test]
fn v5_to_v6_extends_the_v5_fixture() {
    let (short, long) = (heights(V5), heights(V5_TO_V6));
    for (h, want) in &short {
        assert_eq!(&long[h], want, "{h}");
    }
    let (short, long) = (blocks(V5), blocks(V5_TO_V6));
    for (h, want) in &short {
        assert_eq!(&long[h], want, "{h}");
    }
    assert_eq!(
        read(V5, "kv-handoff.jsonl.gz"),
        read(V5_TO_V6, "kv-handoff.jsonl.gz")
    );
    assert_eq!(state_at(V5, 30), state_at(V5_TO_V6, 30));
}

/// The v6 blocks continue the v5 chain: the hand-off state is the v5
/// oracle's state after block 36, and block 37 points at block 36.
#[test]
fn v6_fixture_continues_the_v5_chain() {
    let v5 = heights(V5_TO_V6);
    let v6 = heights(V6);
    assert_eq!(v6[&HANDOFF]["app_hash"], v5[&HANDOFF]["app_hash"]);
    assert_eq!(v6[&HANDOFF]["stores"], v5[&HANDOFF]["stores"]);
    let mut handoff = kv_lines(V6, "kv-handoff.jsonl.gz");
    handoff.retain(|_, kv| !kv.is_empty());
    assert_eq!(handoff, state_at(V5_TO_V6, HANDOFF));

    let (v5_blocks, v6_blocks) = (blocks(V5_TO_V6), blocks(V6));
    assert_eq!(
        v6_blocks.keys().copied().collect::<Vec<_>>(),
        (UPGRADE..=LAST).collect::<Vec<_>>()
    );
    let header = &v6_blocks[&UPGRADE]["block"]["header"];
    assert_eq!(header["last_block_id"], v5_blocks[&HANDOFF]["block_id"]);
    assert_eq!(header["app_hash"], v5[&HANDOFF]["app_hash"]);
    for h in UPGRADE + 1..=LAST {
        let header = &v6_blocks[&h]["block"]["header"];
        assert_eq!(header["app_hash"], v6[&(h - 1)]["app_hash"], "{h}");
        assert_eq!(
            header["last_block_id"],
            v6_blocks[&(h - 1)]["block_id"],
            "{h}"
        );
    }
    for (h, b) in &v5_blocks {
        assert_eq!(b["block"]["header"]["version"]["app"], "5", "{h}");
    }
    for (h, b) in &v6_blocks {
        assert_eq!(b["block"]["header"]["version"]["app"], "6", "{h}");
    }
}

/// The acknowledgement that a block's transactions wrote, if any.
fn ack_of(result: &serde_json::Value) -> Option<String> {
    result["txs"].as_array().unwrap().iter().find_map(|tx| {
        tx["events"].as_array().unwrap().iter().find_map(|e| {
            (e["type"] == "write_acknowledgement").then(|| {
                e["attributes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|a| a["key"] == "packet_ack")
                    .unwrap()["value"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
        })
    })
}

/// Every transaction of both fixtures succeeds. The solo machine's uatom
/// transfer gets an error acknowledgement from x/tokenfilter at v5 (block
/// 36) and a success acknowledgement at v6 (block 37), which mints the
/// voucher `ibc/…` of `transfer/channel-0/uatom`.
#[test]
fn tokenfilter_refuses_the_foreign_token_at_v5_only() {
    let lines = |dir: &str| {
        common::gz_lines(&format!("{dir}/tx-results.jsonl.gz"))
            .map(|d| (d["height"].as_i64().unwrap(), d))
            .collect::<BTreeMap<_, _>>()
    };
    let mut results = lines(V5_TO_V6);
    results.extend(lines(V6));
    for (h, r) in &results {
        for tx in r["txs"].as_array().unwrap() {
            assert_eq!(tx["code"], 0, "{h}: {}", tx["log"]);
        }
    }
    assert_eq!(
        ack_of(&results[&HANDOFF]).unwrap(),
        r#"{"error":"ABCI code: 29: error handling packet: see events for details"}"#
    );
    assert_eq!(ack_of(&results[&UPGRADE]).unwrap(), r#"{"result":"AQ=="}"#);
    let denoms = |h| -> Vec<String> {
        state_at(V6, h)["bank"]
            .keys()
            .filter(|k| k.first() == Some(&0x02))
            .filter_map(|k| {
                let len = usize::from(k[1]);
                std::str::from_utf8(&k[2 + len..]).ok().map(str::to_owned)
            })
            .filter(|d| d.starts_with("ibc/"))
            .collect()
    };
    assert!(denoms(HANDOFF).is_empty());
    assert!(!denoms(UPGRADE).is_empty());
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

/// The `v6` handler (`kv-upgrade`): unbonding time 337 hours, a 10%
/// minimum commission with alice (6%, max 20%) and carol (7%, max 8%)
/// raised to it (update time: block 37's time) and the genesis validators
/// (50%, max 100%) left alone; evidence MaxAgeDuration 337 hours and
/// MaxAgeNumBlocks 242,640; ICA host enabled with an allow list; and the
/// module migrations, which add interchainaccounts (missing from the
/// stored version map) and run its InitGenesis: it records its version and
/// binds the icahost port again, a new capability at index 4 (the next
/// index becomes 5). capability itself is in the stored map since v3. No
/// store is added or deleted.
#[test]
fn v6_upgrade_sets_the_v6_params() {
    let upgrade = common::diffs_in(&format!("{V6}/kv-upgrade.jsonl.gz"));
    let stores = &upgrade[&UPGRADE];
    assert_eq!(
        stores.keys().map(String::as_str).collect::<Vec<_>>(),
        ["capability", "consensus", "icahost", "staking", "upgrade"]
    );
    let before = state_at(V5_TO_V6, HANDOFF);
    let mut after = before.clone();
    for (store, d) in stores {
        let kv = after.entry(store.clone()).or_default();
        for k in &d.delete {
            kv.remove(k);
        }
        kv.extend(d.set.clone());
    }

    let params = Params::decode(&after["staking"][&vec![0x51]][..]).unwrap();
    assert_eq!(params.unbonding_time.unwrap().seconds, 337 * 3600);
    assert_eq!(dec(&params.min_commission_rate), DEC / 10);

    let time = blocks(V6)[&UPGRADE]["block"]["header"]["time"].clone();
    let time = common::timestamp(time.as_str().unwrap());
    let (old, new) = (validators(&before), validators(&after));
    assert_eq!(old.len(), 6);
    for (name, rate, max_rate) in [("alice", 6, 20), ("carol", 7, 8)] {
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
        assert_eq!(dec(&c.rate), DEC / 10, "{name}");
        assert_eq!(dec(&c.max_rate), DEC * max_rate.max(10) / 100, "{name}");
        assert_eq!(commission.update_time.unwrap(), time, "{name}");
    }
    for name in ["val0", "val1", "val2", "val3"] {
        assert_eq!(old[name], new[name], "{name}");
    }

    let cp = ConsensusParams::decode(&after["consensus"][b"Consensus".as_slice()][..]).unwrap();
    let evidence = cp.evidence.unwrap();
    assert_eq!(evidence.max_age_num_blocks, 242_640);
    assert_eq!(evidence.max_age_duration.unwrap().seconds, 337 * 3600);

    let host = HostParams::decode(&after["icahost"][b"params".as_slice()][..]).unwrap();
    assert!(host.host_enabled);
    assert!(!host.allow_messages.is_empty());

    let mut cap4 = b"capability_index".to_vec();
    cap4.extend(4u64.to_be_bytes());
    let capability = &stores["capability"].set;
    assert_eq!(
        capability.keys().cloned().collect::<Vec<_>>(),
        [cap4.clone(), b"index".to_vec()]
    );
    assert!(capability[&cap4].windows(13).any(|w| w == b"ports/icahost"));
    assert_eq!(capability[b"index".as_slice()], 5u64.to_be_bytes());
    assert_eq!(
        before["capability"][b"index".as_slice()],
        4u64.to_be_bytes()
    );
    assert!(before["upgrade"].contains_key(&[b"\x02".as_slice(), b"capability"].concat()));
    assert!(after["upgrade"].contains_key(&[b"\x02".as_slice(), b"interchainaccounts"].concat()));

    let v6 = heights(V6);
    let names = |h: i64| {
        v6[&h]["versions"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(names(HANDOFF), names(UPGRADE));
}
