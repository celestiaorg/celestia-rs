//! The probe blocks of each historical release (D's fixtures
//! `conformance/fixtures/synthetic/v{4,5,6,8,9}-probes`, pinned in
//! `synthetic_probes_oracle.rs`), executed by celestia-rs.
//!
//! For each release the test seeds the Go oracle's state after the
//! upgrade block (every key at the hand-off height and the KV changes of
//! the next block) into the release's store set, opens the node's
//! [`app::App`] on it, and runs the remaining blocks: the release's usual
//! blocks and its probe blocks (module rules, governance filters, amino
//! signed warp transactions, and from v6 packet-forward-middleware
//! packets). From v6 the fixture's last three blocks (the ICA host channel)
//! are not run: the interchain account address derives from the header's
//! app hash, which differs with seeded trees. For each block it requires the oracle's KV changes of
//! every store, every transaction result (code, codespace, gas wanted and
//! used, data, log, info, events) and the block's events, validator
//! updates, consensus params and timeouts. IAVL roots are not compared:
//! the seeded trees do not have the oracle's shape.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::print_stdout)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use celestia_proto::celestia_core_proto::abci::ResponseFinalizeBlock;
use common::check::{check_tx_results, contents, describe, diff};
use common::{Diff, Kv};
use sdk::MultiStore;
use sdk::version::Release;

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic"
);

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

/// The oracle's state after the upgrade block, by store.
fn state_after_upgrade(
    fixture: &str,
    upgrade: i64,
    diffs: &BTreeMap<i64, BTreeMap<String, Diff>>,
) -> BTreeMap<String, Kv> {
    let mut state: BTreeMap<String, Kv> =
        common::gz_lines(&format!("{fixture}/kv-handoff.jsonl.gz"))
            .map(|d| (d["store"].as_str().unwrap().to_owned(), kv_of(&d)))
            .collect();
    for (store, d) in &diffs[&upgrade] {
        let kv = state.entry(store.clone()).or_default();
        kv.extend(d.set.clone());
        kv.retain(|k, _| !d.delete.contains(k));
    }
    state
}

/// The fixture's store roots file.
fn roots(fixture: &str) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(format!("{fixture}/store-roots.json")).unwrap()).unwrap()
}

/// The IAVL stores of the commit info at `height` (the oracle's store
/// roots).
fn iavl_stores(fixture: &str, height: i64) -> Vec<String> {
    let roots = roots(fixture);
    let at = roots["heights"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["height"] == height)
        .unwrap();
    at["stores"].as_object().unwrap().keys().cloned().collect()
}

/// A database whose latest version is `upgrade`, holding `state` in the
/// store set of app version `version`, with chain ID `test`.
fn database_at_upgrade(
    version: u64,
    upgrade: i64,
    state: &BTreeMap<String, Kv>,
) -> Arc<dyn multistore::kvdb::KvBackend> {
    let db: Arc<dyn multistore::kvdb::KvBackend> = Arc::new(multistore::kvdb::MemBackend::new());
    let mut ms = MultiStore::open(db.clone(), multistore::StoreOptions::default());
    app::stores::mount_release(&mut ms, Release::for_app_version(version).unwrap()).unwrap();
    ms.load_latest_version().unwrap();
    for _ in 1..upgrade {
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
    assert_eq!(id.version, upgrade);
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

/// Runs the probe fixture of app version `version` through [`app::App`]
/// from the state after its upgrade block.
fn probes_match_the_oracle(version: u64) {
    blocks_match_the_oracle(&format!("{FIXTURES}/v{version}-probes"), version);
}

/// Runs `fixture`, whose blocks after the upgrade run at app version
/// `version`, through [`app::App`] from the state after its upgrade block.
#[allow(clippy::too_many_lines)]
fn blocks_match_the_oracle(fixture: &str, version: u64) {
    let fixture = fixture.to_owned();
    let roots = roots(&fixture);
    let handoff = roots["handoff_height"].as_i64().unwrap();
    let upgrade = handoff + 1;
    let fixture_last = roots["heights"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["height"].as_i64().unwrap())
        .max()
        .unwrap();
    // From v6 the last three blocks open an interchain-account channel and
    // execute through it. ibc-go `GenerateAddress` derives the account from
    // the header's app hash, which here is celestia-rs's own (the seeded
    // trees do not have the oracle's shape), so the account differs by
    // construction; the run from genesis covers those blocks.
    let last = if version >= 6 {
        handoff + 16
    } else {
        fixture_last
    };

    let diffs = common::diffs_in(&format!("{fixture}/kv-diffs.jsonl.gz"));
    let results: BTreeMap<i64, serde_json::Value> =
        common::gz_lines(&format!("{fixture}/tx-results.jsonl.gz"))
            .map(|d| (d["height"].as_i64().unwrap(), d))
            .collect();
    let blocks: BTreeMap<i64, serde_json::Value> =
        common::gz_lines(&format!("{fixture}/blocks.jsonl.gz"))
            .map(|d| (d["height"].as_i64().unwrap(), d["result"].clone()))
            .collect();
    let state = state_after_upgrade(&fixture, upgrade, &diffs);

    // No validator updates in the upgrade block: the last powers after it
    // are the set of the upgrade height and the two after it. Each later
    // EndBlock's updates take effect two heights later.
    let mut vals: BTreeMap<i64, common::ValidatorSet> = BTreeMap::new();
    let base = validators(&state["staking"]);
    for h in upgrade..=upgrade + 2 {
        vals.insert(h, base.clone());
    }
    assert!(oracle_updates(&results[&upgrade]).is_empty());
    for h in upgrade + 1..last {
        let next = common::apply_updates(&vals[&(h + 1)], &oracle_updates(&results[&h]));
        vals.insert(h + 2, next);
    }

    let mut app = app::App::open_backend(
        database_at_upgrade(version, upgrade, &state),
        &app::AppOptions {
            chain_id: Some("test".into()),
            ..app::AppOptions::default()
        },
    )
    .unwrap();
    let info = app.info();
    assert_eq!(
        (info.last_block_height, info.app_version),
        (upgrade, version)
    );

    let stores = iavl_stores(&fixture, last);
    let empty = BTreeMap::new();
    let mut failures = Vec::new();
    for height in upgrade + 1..=last {
        let before: BTreeMap<String, Kv> = stores
            .iter()
            .map(|s| (s.clone(), contents(app.multistore(), s)))
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
        let mut want_stores = iavl_stores(&fixture, height);
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

#[test]
fn v4_probes_match_the_oracle() {
    probes_match_the_oracle(4);
}

#[test]
fn v5_probes_match_the_oracle() {
    probes_match_the_oracle(5);
}

#[test]
fn v6_probes_match_the_oracle() {
    probes_match_the_oracle(6);
}

#[test]
fn v8_probes_match_the_oracle() {
    probes_match_the_oracle(8);
}

#[test]
fn v9_probes_match_the_oracle() {
    probes_match_the_oracle(9);
}

/// celestia-app v4.1.0 has no capability module in its module manager, so
/// `InitMemStore` never runs and every IBC packet message on a channel
/// opened at v3 fails (D's fixture `synthetic/v3-ibc-v4`): MsgTransfer with
/// `channel` code 9, MsgRecvPacket and MsgAcknowledgement with
/// `capability` code 7.
#[test]
fn v4_ibc_packets_fail_without_the_capability_module() {
    blocks_match_the_oracle(&format!("{FIXTURES}/v3-ibc-v4"), 4);
}
