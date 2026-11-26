//! The v8 to v9 upgrade and the app version 9 blocks of the synthetic
//! chain, executed by celestia-rs as celestia-app v9.0.8 did
//! (`conformance/v9-oracle`, fixture `conformance/fixtures/synthetic/v9`).
//!
//! The test seeds the Go oracle's state after block 56 (whose v8 EndBlocker
//! set app version 9 and scheduled plan `v9` at 57) into the v8 store set,
//! opens the node's [`app::App`] on it, and runs blocks 57 (plan `v9` runs
//! in its PreBlocker; no store is added) to 61 (each BlobTx as its inner
//! transaction, as celestia-core delivers it). For each
//! block it requires the oracle's KV changes of every store, every
//! transaction result (code, codespace, gas wanted and used, data, log,
//! info, events) and the block's events, validator updates, consensus
//! params and timeouts. IAVL roots are not compared: the seeded trees do
//! not have the oracle's shape.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::print_stdout)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use celestia_proto::celestia_core_proto::abci::ResponseFinalizeBlock;
use common::Kv;
use common::check::{check_tx_results, contents, describe, diff};
use sdk::MultiStore;
use sdk::version::Release;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v9"
);
/// The last block of celestia-app v8.0.8: the seeded state.
const HANDOFF: i64 = 56;
/// The last block of the fixture.
const LAST: i64 = 61;

fn kv_of(d: &serde_json::Value) -> Kv {
    d["kv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                hex::decode(p[0].as_str().unwrap()).unwrap(),
                hex::decode(p[1].as_str().unwrap()).unwrap(),
            )
        })
        .collect()
}

/// The oracle's state after block 56, by store.
fn state_at_handoff() -> BTreeMap<String, Kv> {
    common::gz_lines(&format!("{FIXTURE}/kv-handoff.jsonl.gz"))
        .map(|d| (d["store"].as_str().unwrap().to_owned(), kv_of(&d)))
        .collect()
}

/// The IAVL stores of the commit info at `height` (the oracle's store
/// roots; the v9 upgrade adds no store).
fn iavl_stores(height: i64) -> Vec<String> {
    let roots: serde_json::Value =
        serde_json::from_slice(&std::fs::read(format!("{FIXTURE}/store-roots.json")).unwrap())
            .unwrap();
    let h16 = roots["heights"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["height"] == height)
        .unwrap();
    h16["stores"].as_object().unwrap().keys().cloned().collect()
}

/// A database whose latest version is 56, holding `state` in the v8 store
/// set, with chain ID `test`.
fn database_at_upgrade(state: &BTreeMap<String, Kv>) -> Arc<dyn multistore::kvdb::KvBackend> {
    let db: Arc<dyn multistore::kvdb::KvBackend> = Arc::new(multistore::kvdb::MemBackend::new());
    let mut ms = MultiStore::open(db.clone(), multistore::StoreOptions::default());
    app::stores::mount_release(&mut ms, Release::for_app_version(8).unwrap()).unwrap();
    ms.load_latest_version().unwrap();
    for _ in 1..HANDOFF {
        ms.commit().unwrap();
    }
    for (store, kv) in state {
        for (k, v) in kv {
            ms.set(store, k.clone(), v.clone()).unwrap();
        }
    }
    ms.flush().unwrap();
    ms.root_mut().set_metadata(b"chain_id", b"test".to_vec());
    let id = ms.commit().unwrap();
    assert_eq!(id.version, HANDOFF);
    db
}

/// The bonded validator set of `staking`: each last validator power with
/// the validator's consensus address.
fn validators(staking: &Kv) -> common::ValidatorSet {
    let mut set = Vec::new();
    for (k, v) in staking.range(vec![0x11]..vec![0x12]) {
        let op = x_staking::keys::address_from_key(k);
        let power = <pbjson_types::Int64Value as prost::Message>::decode(v.as_slice())
            .unwrap()
            .value;
        let val = x_staking::types::Validator::decode(&staking[&x_staking::keys::validator(&op)])
            .unwrap();
        set.push((val.cons_addr().unwrap(), power));
    }
    common::sort_validators(&mut set);
    set
}

/// The oracle's EndBlock validator updates as `(address, power)`
/// (ed25519 addresses: the first 20 bytes of the key's SHA-256).
fn oracle_updates(result: &serde_json::Value) -> Vec<(Vec<u8>, i64)> {
    result["validator_updates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| {
            assert_eq!(u["pub_key_type"], "ed25519");
            let pk = hex::decode(u["pub_key"].as_str().unwrap()).unwrap();
            (
                hash::sha256(&pk)[..20].to_vec(),
                u["power"].as_i64().unwrap(),
            )
        })
        .collect()
}

/// The block's events, validator updates, consensus params and timeouts
/// against the oracle's record of the block.
fn check_response(res: &ResponseFinalizeBlock, result: &serde_json::Value) -> Vec<String> {
    let mut failures = Vec::new();
    if let Some(d) = common::first_event_difference(&res.events, &common::events(&result["events"]))
    {
        failures.push(format!("events: {d}"));
    }
    let got_updates: Vec<(Vec<u8>, i64)> = common::check::update_set(&res.validator_updates);
    if got_updates != oracle_updates(result) {
        failures.push(format!("validator updates {got_updates:?}"));
    }
    let cp = res.consensus_param_updates.clone().unwrap_or_default();
    let (block, evidence, validator, version) = (
        cp.block.unwrap_or_default(),
        cp.evidence.unwrap_or_default(),
        cp.validator.unwrap_or_default(),
        cp.version.unwrap_or_default(),
    );
    let age = evidence.max_age_duration.unwrap_or_default();
    let got_cp = serde_json::json!({
        "block": {"max_bytes": block.max_bytes.to_string(), "max_gas": block.max_gas.to_string()},
        "evidence": {
            "max_age_num_blocks": evidence.max_age_num_blocks.to_string(),
            "max_age_duration": (age.seconds * 1_000_000_000 + i64::from(age.nanos)).to_string(),
            "max_bytes": evidence.max_bytes.to_string(),
        },
        "validator": {"pub_key_types": validator.pub_key_types},
        "version": {"app": version.app.to_string()},
    });
    if got_cp != result["consensus_param_updates"] {
        failures.push(format!("consensus params {got_cp}"));
    }
    let timeouts = res.timeout_info.unwrap_or_default();
    let ns = |d: Option<pbjson_types::Duration>| {
        d.map(|d| d.seconds * 1_000_000_000 + i64::from(d.nanos))
    };
    let got_t = serde_json::json!({
        "timeout_propose_ns": ns(timeouts.timeout_propose),
        "timeout_commit_ns": ns(timeouts.timeout_commit),
    });
    if got_t != result["timeouts"] {
        failures.push(format!("timeouts {got_t}"));
    }
    // celestia-app v9.0.8 records every TimeoutInfo field.
    let got_info = serde_json::json!({
        "timeout_propose": ns(timeouts.timeout_propose),
        "timeout_propose_delta": ns(timeouts.timeout_propose_delta),
        "timeout_prevote": ns(timeouts.timeout_prevote),
        "timeout_prevote_delta": ns(timeouts.timeout_prevote_delta),
        "timeout_precommit": ns(timeouts.timeout_precommit),
        "timeout_precommit_delta": ns(timeouts.timeout_precommit_delta),
        "timeout_commit": ns(timeouts.timeout_commit),
        "delayed_precommit_timeout": ns(timeouts.delayed_precommit_timeout),
    });
    if got_info != result["timeout_info"] {
        failures.push(format!("timeout info {got_info}"));
    }
    failures
}

/// `bytes` with each occurrence of `from` replaced by `to` (same length).
fn replace(bytes: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    assert_eq!(from.len(), to.len());
    let mut out = bytes.to_vec();
    let mut i = 0;
    while i + from.len() <= out.len() {
        if &out[i..i + from.len()] == from {
            out[i..i + from.len()].copy_from_slice(to);
            i += from.len();
        } else {
            i += 1;
        }
    }
    out
}

/// celestia-rs executes the v8 to v9 upgrade and app version 9 as
/// celestia-app v9.0.8: blocks 57 to 61 make the oracle's KV changes,
/// transaction results and block responses.
#[test]
#[allow(clippy::too_many_lines)]
fn v9_blocks_match_the_oracle() {
    let diffs = common::diffs_in(&format!("{FIXTURE}/kv-diffs.jsonl.gz"));
    let results: BTreeMap<i64, serde_json::Value> =
        common::gz_lines(&format!("{FIXTURE}/tx-results.jsonl.gz"))
            .map(|d| (d["height"].as_i64().unwrap(), d))
            .collect();
    let blocks: BTreeMap<i64, serde_json::Value> =
        common::gz_lines(&format!("{FIXTURE}/blocks.jsonl.gz"))
            .map(|d| (d["height"].as_i64().unwrap(), d["result"].clone()))
            .collect();
    let state = state_at_handoff();

    // No validator updates in blocks 55 and 56 (the v8-to-v9 oracle's
    // record): the last powers after block 56 are the set of heights 56 to
    // 58. Each later EndBlock's updates take effect two heights later.
    let mut vals: BTreeMap<i64, common::ValidatorSet> = BTreeMap::new();
    let base = validators(&state["staking"]);
    for h in HANDOFF..=HANDOFF + 2 {
        vals.insert(h, base.clone());
    }
    let v8_to_v9: BTreeMap<i64, serde_json::Value> = common::gz_lines(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../conformance/fixtures/synthetic/v8-to-v9/tx-results.jsonl.gz"
    ))
    .map(|d| (d["height"].as_i64().unwrap(), d))
    .collect();
    for h in HANDOFF - 1..=HANDOFF {
        assert!(oracle_updates(&v8_to_v9[&h]).is_empty(), "{h}");
    }
    for h in HANDOFF + 1..LAST {
        let next = common::apply_updates(&vals[&(h + 1)], &oracle_updates(&results[&h]));
        vals.insert(h + 2, next);
    }

    let mut app = app::App::open_backend(
        database_at_upgrade(&state),
        &app::AppOptions {
            chain_id: Some("test".into()),
            ..app::AppOptions::default()
        },
    )
    .unwrap();
    let info = app.info();
    assert_eq!((info.last_block_height, info.app_version), (HANDOFF, 9));

    let stores = iavl_stores(LAST);
    let empty = BTreeMap::new();
    let mut failures = Vec::new();
    for height in HANDOFF + 1..=LAST {
        let before: BTreeMap<String, Kv> = stores
            .iter()
            .map(|s| {
                let kv = if app.multistore().is_mounted(s) {
                    contents(app.multistore(), s)
                } else {
                    Kv::new()
                };
                (s.clone(), kv)
            })
            .collect();
        // The seeded trees do not have the oracle's shape, so the app hash
        // celestia-rs committed differs from the one the block header
        // commits to; staking `HistoricalInfo` stores it.
        let rs_app_hash = app.info().last_block_app_hash;
        let mut req = common::finalize_request(&blocks[&height], &vals[&(height - 1)]);
        // celestia-core passes each BlobTx's inner transaction to
        // FinalizeBlock, as the oracle did.
        for tx in &mut req.txs {
            if let Some(blob_tx) = txdecode::unmarshal_blob_tx_v9(tx) {
                *tx = blob_tx.unwrap().tx;
            }
        }
        let go_app_hash = req.header.as_ref().unwrap().app_hash.clone();
        let res = match app.finalize_block(req) {
            Ok(res) => res,
            Err(e) => {
                failures.push(format!("height {height}: {}", e.log));
                break;
            }
        };
        app.commit().unwrap();
        let committed: Vec<String> = app
            .multistore()
            .root()
            .last_commit_info()
            .unwrap()
            .store_infos
            .iter()
            .map(|s| s.name.clone())
            .collect();
        let mut want_stores = iavl_stores(height);
        want_stores.sort();
        let mut got_stores = committed;
        got_stores.sort();
        if got_stores != want_stores {
            failures.push(format!("height {height} commit info stores {got_stores:?}"));
        }
        let result = &results[&height];
        for f in check_tx_results(&res, result) {
            failures.push(format!("height {height} {f}"));
        }
        for f in check_response(&res, result) {
            failures.push(format!("height {height} {f}"));
        }
        let want = diffs.get(&height).unwrap_or(&empty);
        let mut changed = BTreeSet::new();
        for s in &stores {
            let mut got = diff(&before[s], &contents(app.multistore(), s));
            if s == "staking" {
                for (k, v) in &mut got.set {
                    if k.first() == Some(&0x50) {
                        *v = replace(v, &rs_app_hash, &go_app_hash);
                    }
                }
            }
            let w = want.get(s).cloned().unwrap_or_default();
            if got != w {
                changed.insert(s.clone());
                let values: Vec<String> = got
                    .set
                    .iter()
                    .filter(|(k, v)| w.set.get(*k).is_some_and(|wv| wv != *v))
                    .map(|(k, v)| {
                        format!(
                            "    {}: rs {} go {}",
                            hex::encode(k),
                            hex::encode(v),
                            hex::encode(&w.set[k])
                        )
                    })
                    .collect();
                failures.push(format!(
                    "height {height} store {s}:\n  rs {}\n  go {}\n{}",
                    describe(&got),
                    describe(&w),
                    values.join("\n")
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
