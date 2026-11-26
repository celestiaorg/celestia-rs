//! App version 4 blocks of the synthetic chain, executed by celestia-rs as
//! celestia-app v4.1.0 did (`conformance/v4-oracle`, fixture
//! `conformance/fixtures/synthetic/v4`).
//!
//! The test seeds the Go oracle's state after the upgrade block 16 (every
//! key at height 15 and the KV changes of block 16) into a v4 store set,
//! opens the node's [`app::App`] on it, and runs blocks 17 to 20. For each
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
use common::check::{check_tx_results, contents, describe, diff};
use common::{Diff, Kv};
use sdk::MultiStore;
use sdk::version::Release;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v4"
);
/// The upgrade block: plan `v4` runs in its PreBlocker.
const UPGRADE: i64 = 16;
/// The last block of the fixture.
const LAST: i64 = 20;

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

/// The oracle's state after block 16, by store.
fn state_after_upgrade(diffs: &BTreeMap<i64, BTreeMap<String, Diff>>) -> BTreeMap<String, Kv> {
    let mut state: BTreeMap<String, Kv> =
        common::gz_lines(&format!("{FIXTURE}/kv-handoff.jsonl.gz"))
            .map(|d| (d["store"].as_str().unwrap().to_owned(), kv_of(&d)))
            .collect();
    for (store, d) in &diffs[&UPGRADE] {
        let kv = state.entry(store.clone()).or_default();
        kv.extend(d.set.clone());
        kv.retain(|k, _| !d.delete.contains(k));
    }
    state
}

/// The IAVL stores of the v4 commit info (the oracle's store roots at
/// block 16).
fn iavl_stores() -> Vec<String> {
    let roots: serde_json::Value =
        serde_json::from_slice(&std::fs::read(format!("{FIXTURE}/store-roots.json")).unwrap())
            .unwrap();
    let h16 = roots["heights"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["height"] == UPGRADE)
        .unwrap();
    h16["stores"].as_object().unwrap().keys().cloned().collect()
}

/// A database whose latest version is 16, holding `state` in the v4 store
/// set, with chain ID `test`.
fn database_at_upgrade(state: &BTreeMap<String, Kv>) -> Arc<dyn multistore::kvdb::KvBackend> {
    let db: Arc<dyn multistore::kvdb::KvBackend> = Arc::new(multistore::kvdb::MemBackend::new());
    let mut ms = MultiStore::open(db.clone(), multistore::StoreOptions::default());
    app::stores::mount_release(&mut ms, Release::for_app_version(4).unwrap()).unwrap();
    ms.load_latest_version().unwrap();
    for _ in 1..UPGRADE {
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
    assert_eq!(id.version, UPGRADE);
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

/// The v4 oracle's EndBlock validator updates as `(address, power)`
/// (ed25519 addresses: the first 20 bytes of the key's SHA-256).
fn oracle_updates(result: &serde_json::Value) -> Vec<(Vec<u8>, i64)> {
    result["validator_updates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| {
            assert_eq!(u["pub_key_type"], "ed25519");
            let pk = hex::decode(u["pub_key"].as_str().unwrap()).unwrap();
            (hash::sha256(&pk)[..20].to_vec(), u["power"].as_i64().unwrap())
        })
        .collect()
}

/// The block's events, validator updates, consensus params and timeouts
/// against the v4 oracle's record of the block.
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
    failures
}

/// celestia-rs executes app version 4 as celestia-app v4.1.0: blocks 17
/// to 20 make the oracle's KV changes, transaction results and block
/// responses.
#[test]
fn v4_blocks_match_the_oracle() {
    let diffs = common::diffs_in(&format!("{FIXTURE}/kv-diffs.jsonl.gz"));
    let results: BTreeMap<i64, serde_json::Value> =
        common::gz_lines(&format!("{FIXTURE}/tx-results.jsonl.gz"))
            .map(|d| (d["height"].as_i64().unwrap(), d))
            .collect();
    let blocks: BTreeMap<i64, serde_json::Value> =
        common::gz_lines(&format!("{FIXTURE}/blocks.jsonl.gz"))
            .map(|d| (d["height"].as_i64().unwrap(), d["result"].clone()))
            .collect();
    let state = state_after_upgrade(&diffs);

    // No validator updates in blocks 14 to 16 (the v1-v3 and v4
    // oracles' records): the last powers after block 16 are the set of
    // heights 16 to 18. Each later EndBlock's updates take effect two
    // heights later.
    let mut vals: BTreeMap<i64, common::ValidatorSet> = BTreeMap::new();
    let base = validators(&state["staking"]);
    for h in UPGRADE..=UPGRADE + 2 {
        vals.insert(h, base.clone());
    }
    assert!(oracle_updates(&results[&UPGRADE]).is_empty());
    for h in UPGRADE + 1..LAST {
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
    assert_eq!((info.last_block_height, info.app_version), (UPGRADE, 4));

    let stores = iavl_stores();
    let empty = BTreeMap::new();
    let mut failures = Vec::new();
    for height in UPGRADE + 1..=LAST {
        let before: BTreeMap<String, Kv> = stores
            .iter()
            .map(|s| (s.clone(), contents(app.multistore(), s)))
            .collect();
        let res = match app.finalize_block(common::finalize_request(
            &blocks[&height],
            &vals[&(height - 1)],
        )) {
            Ok(res) => res,
            Err(e) => {
                failures.push(format!("height {height}: {}", e.log));
                break;
            }
        };
        app.commit().unwrap();
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
            let got = diff(&before[s], &contents(app.multistore(), s));
            let w = want.get(s).cloned().unwrap_or_default();
            if got != w {
                changed.insert(s.clone());
                failures.push(format!(
                    "height {height} store {s}:\n  rs {}\n  go {}",
                    describe(&got),
                    describe(&w)
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
