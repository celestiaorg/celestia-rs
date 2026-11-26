//! App version 1 blocks against the Go v1 oracle
//! (`conformance/v1-oracle`).
//!
//! `v1_blocks_match_the_oracle` runs InitChain on the canonical mainnet
//! genesis and then mainnet blocks 1 to 10 through the v0.46 driver, and
//! requires for each block the oracle's KV changes for every store, its
//! store roots, the app hash, every transaction result (code, codespace,
//! gas wanted and used, data, log, info, events) and the block's events,
//! validator updates, consensus params and timeouts:
//!
//! ```sh
//! CELESTIA_MAINNET_GENESIS=<genesis.json> cargo test -p app --test v1_block_oracle -- --ignored
//! ```
//!
//! It continues past height 10 with a local block file and oracle output
//! (not committed): `CELESTIA_V1_BLOCKS=<blocks.jsonl[.gz]>` holding
//! heights 12 to N+1, `CELESTIA_V1_ORACLE_EXT=<dir>` from `v1-oracle
//! -from 11 -heights N -kv -out <dir>`, and optionally
//! `CELESTIA_V1_LAST_HEIGHT` to stop earlier.
//!
//! `v1_begin_blockers_match_the_oracle_for_blocks_2_to_10` rebuilds the
//! oracle state before each of blocks 2 to 10 from the full height-0 dump
//! (`CELESTIA_V1_ORACLE_KV`, regenerated with the oracle) and the KV
//! diffs, runs only the block's BeginBlock, and requires the oracle's
//! BeginBlock events and the BeginBlock-only state: the `mint` and
//! `slashing` stores and the block's staking `HistoricalInfo`.
//! Transactions are not executed, so the EndBlock of these blocks is
//! not checked here (the blobstream EndBlocker is, in `x-qgb`).

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::print_stdout)]

mod common;

use std::collections::BTreeMap;

use app::legacy::{Driver, Modules as _, V1Modules};
use celestia_proto::celestia_core_proto::abci::{ResponseFinalizeBlock, ValidatorUpdate};
use celestia_proto::celestia_core_proto::tendermint::crypto::public_key::Sum;
use common::{Diff, Kv};
use sdk::{Context, ExecMode, MultiStore};

/// The app hash after mainnet block 1.
const BLOCK_1_APP_HASH: &str = "F43C8B84340E6803AC86AD934B03E7145E155F7E8E2E7E19412D6ED8A44CBF14";

fn v1_store() -> MultiStore {
    let mut ms = MultiStore::new();
    let v1 = sdk::version::Release::for_app_version(1).unwrap();
    app::stores::mount_release(&mut ms, v1).unwrap();
    ms
}

/// The IAVL stores of app version 1 (every oracle store root but the
/// memory store).
fn iavl_stores() -> Vec<String> {
    let roots = common::oracle_roots();
    roots["heights"][0]["stores"]
        .as_object()
        .unwrap()
        .keys()
        .filter(|s| *s != "mem_capability")
        .cloned()
        .collect()
}

fn contents(ms: &MultiStore, store: &str) -> Kv {
    ms.range(store, None, None, false)
        .unwrap()
        .into_iter()
        .collect()
}

fn diff(before: &Kv, after: &Kv) -> Diff {
    let mut d = Diff::default();
    for (k, v) in after {
        if before.get(k) != Some(v) {
            d.set.insert(k.clone(), v.clone());
        }
    }
    for k in before.keys() {
        if !after.contains_key(k) {
            d.delete.insert(k.clone());
        }
    }
    d
}

fn describe(d: &Diff) -> String {
    let keys: Vec<String> = d.set.keys().map(hex::encode).collect();
    let del: Vec<String> = d.delete.iter().map(hex::encode).collect();
    format!("set {keys:?} delete {del:?}")
}

/// `(pub key type, pub key hex, power)` of each update.
fn updates(v: &[ValidatorUpdate]) -> Vec<(String, String, i64)> {
    v.iter()
        .map(|u| match u.pub_key.as_ref().and_then(|k| k.sum.as_ref()) {
            Some(Sum::Ed25519(k)) => ("ed25519".into(), hex::encode_upper(k), u.power),
            Some(Sum::Secp256k1(k)) => ("secp256k1".into(), hex::encode_upper(k), u.power),
            None => (String::new(), String::new(), u.power),
        })
        .collect()
}

fn oracle_updates(result: &serde_json::Value) -> Vec<(String, String, i64)> {
    result["validator_updates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| {
            (
                u["pub_key_type"].as_str().unwrap().to_owned(),
                u["pub_key"].as_str().unwrap().to_owned(),
                u["power"].as_i64().unwrap(),
            )
        })
        .collect()
}

/// The block's events, validator updates, consensus params and timeouts
/// against the oracle's record of the block.
fn check_response(res: &ResponseFinalizeBlock, result: &serde_json::Value) -> Vec<String> {
    let mut failures = Vec::new();
    if let Some(d) =
        common::first_event_difference(&res.events, &common::oracle_block_events(result))
    {
        failures.push(format!("events: {d}"));
    }
    if updates(&res.validator_updates) != oracle_updates(result) {
        failures.push(format!(
            "validator updates {:?}",
            updates(&res.validator_updates)
        ));
    }
    let cp = res.consensus_param_updates.clone().unwrap_or_default();
    let want_cp = &result["consensus_param_updates"];
    let (block, evidence, validator, version) = (
        cp.block.unwrap_or_default(),
        cp.evidence.unwrap_or_default(),
        cp.validator.unwrap_or_default(),
        cp.version.unwrap_or_default(),
    );
    let age = evidence.max_age_duration.unwrap_or_default();
    let got_cp = serde_json::json!({
        "block": {"max_bytes": block.max_bytes, "max_gas": block.max_gas},
        "evidence": {
            "max_age_num_blocks": evidence.max_age_num_blocks,
            "max_age_duration_ns": age.seconds * 1_000_000_000 + i64::from(age.nanos),
            "max_bytes": evidence.max_bytes,
        },
        "validator": {"pub_key_types": validator.pub_key_types},
        "version": {"app_version": version.app},
    });
    if got_cp != *want_cp {
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

/// Each transaction result against the oracle's: every `ExecTxResult`
/// field the v0.34 `ResponseDeliverTx` carries.
fn check_tx_results(res: &ResponseFinalizeBlock, result: &serde_json::Value) -> Vec<String> {
    let want = result["txs"].as_array().unwrap();
    let mut failures = Vec::new();
    if res.tx_results.len() != want.len() {
        failures.push(format!(
            "tx count rs {} go {}",
            res.tx_results.len(),
            want.len()
        ));
    }
    for (i, (got, w)) in res.tx_results.iter().zip(want).enumerate() {
        let mut fields = Vec::new();
        let mut field = |name: &str, g: String, w: String| {
            if g != w {
                fields.push(format!("  {name}: rs {g}\n  {name}: go {w}"));
            }
        };
        field("code", got.code.to_string(), w["code"].to_string());
        field(
            "codespace",
            got.codespace.clone(),
            w["codespace"].as_str().unwrap().to_owned(),
        );
        field(
            "gas_wanted",
            got.gas_wanted.to_string(),
            w["gas_wanted"].to_string(),
        );
        field(
            "gas_used",
            got.gas_used.to_string(),
            w["gas_used"].to_string(),
        );
        field(
            "data",
            hex::encode(&got.data),
            w["data"].as_str().unwrap().to_owned(),
        );
        // The oracle's JSON holds each invalid UTF-8 byte of a Go string
        // as U+FFFD (a log can quote store keys).
        field(
            "log",
            gostr::lossy(&got.log).into_owned(),
            w["log"].as_str().unwrap().to_owned(),
        );
        field(
            "info",
            gostr::lossy(&got.info).into_owned(),
            w["info"].as_str().unwrap().to_owned(),
        );
        if let Some(d) = common::first_event_difference(&got.events, &common::events(&w["events"]))
        {
            fields.push(format!("  events: {d}"));
        }
        if !fields.is_empty() {
            failures.push(format!("tx {i}:\n{}", fields.join("\n")));
        }
    }
    failures
}

/// The validator set an EndBlock's updates lead to, as `(address,
/// power)`.
fn update_set(v: &[ValidatorUpdate]) -> Vec<(Vec<u8>, i64)> {
    v.iter()
        .map(|u| match u.pub_key.as_ref().and_then(|k| k.sum.as_ref()) {
            Some(Sum::Ed25519(k)) => (hash::sha256(k)[..20].to_vec(), u.power),
            _ => (Vec::new(), u.power),
        })
        .collect()
}

#[test]
#[ignore = "needs the mainnet genesis: CELESTIA_MAINNET_GENESIS=<genesis.json>"]
#[allow(clippy::too_many_lines)]
fn v1_blocks_match_the_oracle() {
    let genesis = common::mainnet_genesis();
    let mut oracle = common::OracleStream::open();
    let last_height = oracle.last_to_run();
    let mut blocks = common::block_stream(last_height);
    let updates = common::OracleStream::updates(last_height);
    let roots = oracle.roots.clone();
    let stores = iavl_stores();

    let mut ms = v1_store();
    let mut driver = Driver::new(V1Modules);
    let init = driver
        .init_chain(&mut ms, &common::init_request(&genesis))
        .unwrap();
    drop(genesis);
    assert_eq!(init.app_hash, hash::sha256(&[]).to_vec());
    assert_eq!(roots[&1]["app_hash"], BLOCK_1_APP_HASH);

    // Validator sets by height, from the oracle's EndBlock updates (the
    // requests' inputs): the genesis set signs heights 1 and 2, and each
    // EndBlock(h)'s updates take effect at h + 2.
    let mut vals: BTreeMap<i64, common::ValidatorSet> = BTreeMap::new();
    let mut genesis_set = update_set(&init.validators);
    common::sort_validators(&mut genesis_set);
    vals.insert(1, genesis_set.clone());
    vals.insert(2, genesis_set);
    for h in 1..last_height {
        let next = common::apply_updates(&vals[&(h + 1)], &updates[&h]);
        vals.insert(h + 2, next);
    }

    let mut before: BTreeMap<String, Kv> = stores
        .iter()
        .map(|s| (s.clone(), contents(&ms, s)))
        .collect();
    println!("height app-hash roots kv txs events updates");
    for height in 1..=last_height {
        let last = if height == 1 {
            Vec::new()
        } else {
            vals[&(height - 1)].clone()
        };
        let (h, block) = blocks.next().unwrap();
        assert_eq!(h, height);
        let req = common::finalize_request(&block, &last);
        drop(block);
        let want_diffs = oracle.diffs(height);
        let result = oracle.result(height);
        let res = driver
            .finalize_block(&mut ms, &req)
            .unwrap_or_else(|e| panic!("height {height}: {}", e.log));
        let id = driver.commit(&mut ms).unwrap();

        let mut failures = Vec::new();
        let hr = &roots[&height];
        assert_eq!(hr["height"], height);
        let info = ms.root().last_commit_info().unwrap().clone();
        let (mut kv_ok, mut roots_ok) = (true, true);
        for s in &stores {
            let after = contents(&ms, s);
            let got = diff(&before[s], &after);
            let want = want_diffs.get(s).cloned().unwrap_or_default();
            if got != want {
                kv_ok = false;
                let only = |a: &Diff, b: &Diff| Diff {
                    set: a
                        .set
                        .iter()
                        .filter(|(k, v)| b.set.get(*k) != Some(v))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    delete: a.delete.difference(&b.delete).cloned().collect(),
                };
                failures.push(format!(
                    "{s}: kv diff\n  rs only {}\n  go only {}",
                    describe(&only(&got, &want)),
                    describe(&only(&want, &got))
                ));
                for (k, v) in &got.set {
                    if let Some(w) = want.set.get(k)
                        && w != v
                    {
                        failures.push(format!(
                            "  {s} {}: rs {} go {}",
                            hex::encode(k),
                            hex::encode(v),
                            hex::encode(w)
                        ));
                    }
                }
            }
            before.insert(s.clone(), after);
            let root = info
                .store_info(s)
                .and_then(|si| si.commit_id.as_ref())
                .map(|c| hex::encode_upper(c.hash))
                .unwrap_or_default();
            if root != hr["stores"][s].as_str().unwrap() {
                roots_ok = false;
                failures.push(format!("{s}: root {root}"));
            }
        }
        let app_hash = hex::encode_upper(id.hash);
        let app_ok = app_hash == hr["app_hash"].as_str().unwrap()
            && hex::encode_upper(&res.app_hash) == app_hash;
        if !app_ok {
            failures.push(format!("app hash {app_hash}, want {}", hr["app_hash"]));
        }
        let tx_failures = check_tx_results(&res, &result);
        let tx_ok = tx_failures.is_empty();
        failures.extend(tx_failures);
        let response = check_response(&res, &result);
        let events_ok = !response.iter().any(|f| f.starts_with("events"));
        let updates_ok = !response.iter().any(|f| f.starts_with("validator updates"));
        failures.extend(response);
        let ok = |b: bool| if b { "match" } else { "DIFF" };
        println!(
            "{height:>6} {} {} {} {} ({} txs) {} {}",
            ok(app_ok),
            ok(roots_ok),
            ok(kv_ok),
            ok(tx_ok),
            res.tx_results.len(),
            ok(events_ok),
            ok(updates_ok)
        );
        // Later blocks run on this block's state: stop at the first miss.
        assert!(
            failures.is_empty(),
            "height {height}:\n{}",
            failures.join("\n")
        );
    }
}

/// The node's [`app::App`] (not the bare driver) runs InitChain on the
/// mainnet genesis and blocks 1 to 10 on a persistent home, closing and
/// reopening it after block 5: every app hash and every transaction's
/// code and gas equal the oracle's.
#[test]
#[ignore = "needs the mainnet genesis: CELESTIA_MAINNET_GENESIS=<genesis.json>"]
fn v1_app_runs_blocks_1_to_10_across_a_reopen() {
    let genesis = common::mainnet_genesis();
    let archive = common::archive();
    let results = common::oracle_results();
    let roots = common::oracle_roots();
    let home = tempfile::tempdir().unwrap();
    let mut app = app::App::open(home.path()).unwrap();
    let init = app.init_chain(common::init_request(&genesis)).unwrap();
    drop(genesis);
    assert!(app.is_sdk_v046_state());
    assert_eq!(init.app_hash, hash::sha256(&[]).to_vec());

    let mut vals: BTreeMap<i64, common::ValidatorSet> = BTreeMap::new();
    let mut genesis_set = update_set(&init.validators);
    common::sort_validators(&mut genesis_set);
    vals.insert(1, genesis_set.clone());
    vals.insert(2, genesis_set);
    for h in 1..=9 {
        let next = common::apply_updates(&vals[&(h + 1)], &common::oracle_updates(&results[&h]));
        vals.insert(h + 2, next);
    }
    for height in 1..=10i64 {
        if height == 6 {
            drop(app);
            app = app::App::open(home.path()).unwrap();
            assert!(app.is_sdk_v046_state());
            assert_eq!(app.info().last_block_height, 5);
            assert_eq!(app.info().app_version, 1);
        }
        let last = if height == 1 {
            Vec::new()
        } else {
            vals[&(height - 1)].clone()
        };
        let req = common::finalize_request(&archive[&height], &last);
        let res = app.finalize_block(req).unwrap();
        app.commit().unwrap();
        let want = &roots["heights"][usize::try_from(height).unwrap()];
        assert_eq!(
            hex::encode_upper(&res.app_hash),
            want["app_hash"].as_str().unwrap(),
            "app hash at {height}"
        );
        assert_eq!(app.info().last_block_app_hash, res.app_hash);
        let want_txs = results[&height]["txs"].as_array().unwrap();
        assert_eq!(res.tx_results.len(), want_txs.len());
        for (i, (got, w)) in res.tx_results.iter().zip(want_txs).enumerate() {
            assert_eq!(
                (i64::from(got.code), got.gas_wanted, got.gas_used),
                (
                    w["code"].as_i64().unwrap(),
                    w["gas_wanted"].as_i64().unwrap(),
                    w["gas_used"].as_i64().unwrap()
                ),
                "tx {height}:{i}"
            );
        }
        println!(
            "app block {height} matches {}",
            hex::encode_upper(&res.app_hash)
        );
    }
}

/// The oracle state at height 0 from the full dump, every IAVL store.
fn full_height_0(path: &str) -> BTreeMap<String, Kv> {
    common::gz_lines(path)
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

fn apply(ms: &mut MultiStore, diffs: &BTreeMap<String, Diff>) {
    for (store, d) in diffs {
        for (k, v) in &d.set {
            ms.set(store, k.clone(), v.clone()).unwrap();
        }
        for k in &d.delete {
            ms.delete(store, k).unwrap();
        }
    }
}

/// The genesis validator set from the oracle's staking state: each last
/// validator power with the validator's consensus address.
fn genesis_validators(staking: &Kv) -> common::ValidatorSet {
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

#[test]
#[ignore = "needs CELESTIA_V1_ORACLE_KV=<kv-height-0.jsonl.gz> from the oracle"]
fn v1_begin_blockers_match_the_oracle_for_blocks_2_to_10() {
    // Runs the blockers outside the driver: hold its v0.46 scope.
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let path = std::env::var("CELESTIA_V1_ORACLE_KV")
        .expect("set CELESTIA_V1_ORACLE_KV to the oracle's kv-height-0.jsonl.gz");
    let archive = common::archive();
    let diffs = common::oracle_diffs();
    let results = common::oracle_results();
    let state0 = full_height_0(&path);

    // Validator sets by height: genesis for heights 1 and 2, then each
    // EndBlock's updates take effect two heights later.
    let mut vals: BTreeMap<i64, common::ValidatorSet> = BTreeMap::new();
    let genesis = genesis_validators(&state0["staking"]);
    assert_eq!(genesis.len(), 75);
    vals.insert(1, genesis.clone());
    vals.insert(2, genesis);
    for h in 1..=9 {
        let next = common::apply_updates(&vals[&(h + 1)], &common::oracle_updates(&results[&h]));
        vals.insert(h + 2, next);
    }

    let mut ms = v1_store();
    for (store, kv) in &state0 {
        for (k, v) in kv {
            ms.set(store, k.clone(), v.clone()).unwrap();
        }
    }
    let mut oracle: BTreeMap<String, Kv> = ["mint", "slashing"]
        .iter()
        .map(|s| ((*s).to_owned(), state0[*s].clone()))
        .collect();
    drop(state0);
    let empty = BTreeMap::new();
    let advance = |ms: &mut MultiStore, oracle: &mut BTreeMap<String, Kv>, h: i64| {
        let d = diffs.get(&h).unwrap_or(&empty);
        apply(ms, d);
        for (s, kv) in oracle.iter_mut() {
            if let Some(d) = d.get(s) {
                kv.extend(d.set.clone());
                kv.retain(|k, _| !d.delete.contains(k));
            }
        }
    };
    advance(&mut ms, &mut oracle, 1);
    let mut failures = Vec::new();
    for height in 2..=10i64 {
        let block = &archive[&height];
        let req = common::finalize_request(block, &vals[&(height - 1)]);
        let tm_header = req.header.clone().unwrap();
        let header = sdk::Header {
            chain_id: "celestia".into(),
            height,
            time: req.time.unwrap(),
            proposer_address: req.proposer_address.clone(),
            hash: req.hash.clone(),
            app_hash: tm_header.app_hash.clone(),
            next_validators_hash: req.next_validators_hash.clone(),
        };
        let mut ctx = Context::new(&mut ms, header, ExecMode::Finalize);
        ctx.consensus_params = app::legacy::consensus_params::load(&mut ctx, 1).unwrap();
        ctx.vote_infos = req.decided_last_commit.clone().unwrap().votes;
        ctx.multistore().branch();
        V1Modules.begin_block(&mut ctx, &tm_header).unwrap();
        let got_events = ctx.events.take();
        let result = &results[&height];
        if let Some(d) = common::first_event_difference(
            &got_events,
            &common::events(&result["begin_block_events"]),
        ) {
            failures.push(format!("height {height} events: {d}"));
        }
        drop(ctx);
        // The stores only BeginBlock writes at these heights.
        let block_diffs = diffs.get(&height).cloned().unwrap_or_default();
        for (s, before) in &oracle {
            let mut want = before.clone();
            if let Some(d) = block_diffs.get(s) {
                want.extend(d.set.clone());
                want.retain(|k, _| !d.delete.contains(k));
            }
            let got = contents(&ms, s);
            if got != want {
                let d = diff(&want, &got);
                failures.push(format!("height {height} {s}: rs vs go {}", describe(&d)));
            }
        }
        let hi_key = x_staking::keys::historical_info_v046(height);
        let got_hi = ms.get(x_staking::STORE_KEY, &hi_key).unwrap();
        let want_hi = block_diffs
            .get(x_staking::STORE_KEY)
            .and_then(|d| d.set.get(&hi_key))
            .cloned();
        if got_hi != want_hi || want_hi.is_none() {
            failures.push(format!("height {height}: HistoricalInfo"));
        }
        ms.discard_branch();
        advance(&mut ms, &mut oracle, height);
        println!("height {height}: BeginBlock checked");
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
