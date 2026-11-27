//! The `v4` upgrade (cosmos-sdk v0.46 to v0.50) against the Go oracle,
//! module by module: a synthetic chain's state after its last v3 block
//! (`kv-handoff.jsonl.gz`) goes in, a Rust migration runs at the upgrade
//! height, and the stores it owns must hold what the Go store loader and
//! `v4` upgrade handler left (`kv-upgrade.jsonl.gz`: the x/upgrade
//! PreBlocker of the upgrade block alone).
//!
//! Each test runs on both synthetic chains: `fixtures/synthetic/v4` (the
//! v1-v3 chain) and `fixtures/synthetic/v3-rich-v4` (12 validators with
//! missed blocks, 1,080 delegations, merged unbonding entries, a
//! redelegation, a vesting account, gov proposals, authz and fee grants,
//! an IBC client, connection and channel) and
//! `fixtures/synthetic/v3-history-v4` (1,005 historical infos, more than
//! staking's historical info key migration moves at once).

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;

use std::collections::BTreeMap;

use common::Kv;
use common::check::{contents, describe, diff};
use sdk::{Context, ExecMode, Header, MultiStore};

/// The fixture directories of the synthetic chains.
const FIXTURES: [&str; 3] = [
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../conformance/fixtures/synthetic/v4"
    ),
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../conformance/fixtures/synthetic/v3-rich-v4"
    ),
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../conformance/fixtures/synthetic/v3-history-v4"
    ),
];

/// One chain's fixture and its upgrade block (the block after the
/// hand-off height).
#[derive(Clone, Copy)]
struct Fixture {
    dir: &'static str,
    upgrade: i64,
}

fn fixtures() -> impl Iterator<Item = Fixture> {
    FIXTURES.into_iter().map(|dir| {
        let roots: serde_json::Value =
            serde_json::from_slice(&std::fs::read(format!("{dir}/store-roots.json")).unwrap())
                .unwrap();
        Fixture {
            dir,
            upgrade: roots["handoff_height"].as_i64().unwrap() + 1,
        }
    })
}

fn hex_pair(p: &serde_json::Value) -> (Vec<u8>, Vec<u8>) {
    (
        hex::decode(p[0].as_str().unwrap()).unwrap(),
        hex::decode(p[1].as_str().unwrap()).unwrap(),
    )
}

/// The oracle's state at the hand-off height, by store.
fn handoff_state(f: Fixture) -> BTreeMap<String, Kv> {
    common::gz_lines(&format!("{}/kv-handoff.jsonl.gz", f.dir))
        .map(|d| {
            let kv = d["kv"].as_array().unwrap().iter().map(hex_pair).collect();
            (d["store"].as_str().unwrap().to_owned(), kv)
        })
        .collect()
}

/// The oracle's state after the upgrade handler, by store.
fn upgraded_state(f: Fixture) -> BTreeMap<String, Kv> {
    let mut state = handoff_state(f);
    let diffs = common::diffs_in(&format!("{}/kv-upgrade.jsonl.gz", f.dir));
    for (store, d) in &diffs[&f.upgrade] {
        let kv = state.entry(store.clone()).or_default();
        for k in &d.delete {
            kv.remove(k);
        }
        kv.extend(d.set.clone());
    }
    state
}

/// The block time of the upgrade block (`v1-oracle` block times).
fn upgrade_time(f: Fixture) -> pbjson_types::Timestamp {
    let blocks: BTreeMap<i64, serde_json::Value> =
        common::gz_lines(&format!("{}/blocks.jsonl.gz", f.dir))
            .map(|d| (d["height"].as_i64().unwrap(), d["result"].clone()))
            .collect();
    common::timestamp(
        blocks[&f.upgrade]["block"]["header"]["time"]
            .as_str()
            .unwrap(),
    )
}

/// A store with the app version 4 store set holding the hand-off state.
fn handoff(f: Fixture) -> MultiStore {
    let mut ms = MultiStore::new();
    let v4 = sdk::version::Release::for_app_version(4).unwrap();
    app::stores::mount_release(&mut ms, v4).unwrap();
    let state = handoff_state(f);
    let mut ctx = ctx(f, &mut ms);
    for (store, kv) in state {
        if store == "mem_capability" {
            continue;
        }
        for (k, v) in kv {
            ctx.set(&store, k, v).unwrap();
        }
    }
    drop(ctx);
    ms
}

fn ctx(f: Fixture, ms: &mut MultiStore) -> Context<'_> {
    let header = Header {
        chain_id: "test".into(),
        height: f.upgrade,
        time: upgrade_time(f),
        ..Header::default()
    };
    Context::new(ms, header, ExecMode::Finalize)
}

/// Runs `migrate` on each chain's hand-off state and requires each of
/// `stores` (and the untouched x/params store) to hold the oracle's
/// result.
fn check(migrate: impl Fn(&mut Context<'_>) -> sdk::Result<()>, stores: &[&str]) {
    let mut failures = Vec::new();
    for f in fixtures() {
        let mut ms = handoff(f);
        {
            let mut c = ctx(f, &mut ms);
            migrate(&mut c).unwrap_or_else(|e| panic!("{}: migration: {}", f.dir, e.log));
        }
        let want = upgraded_state(f);
        for store in stores.iter().chain(&["params"]) {
            let got = contents(&ms, store);
            let w = want.get(*store).cloned().unwrap_or_default();
            if got != w {
                failures.push(format!(
                    "{}: {store}: rs vs go {}",
                    f.dir,
                    describe(&diff(&w, &got))
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// auth 3 to 5: params out of the subspace `auth` (key 0x00), the global
/// account number from `globalAccountNumber` to the sequence at 0x02.
#[test]
fn v4_migrates_auth() {
    check(app::upgrades::v4::migrate_auth, &["acc"]);
}

/// bank 3 to 4: `MigrateSendEnabledParams` and params out of the subspace
/// `bank` (key 0x05, `default_send_enabled` only).
#[test]
fn v4_migrates_bank() {
    check(app::upgrades::v4::migrate_bank, &["bank"]);
}

/// distribution 2 to 4: params out of the subspace with the proposer
/// rewards reset to zero (0x09); 3 to 4 is a no-op.
#[test]
fn v4_migrates_distribution() {
    check(app::upgrades::v4::migrate_distribution, &["distribution"]);
}

/// slashing 2 to 4: params out of the subspace (0x00); 3 to 4 removes
/// nothing (the fork's missed-block cleanup builds a short address) and,
/// with at most 10 validators, records no continuation key.
#[test]
fn v4_migrates_slashing() {
    check(app::upgrades::v4::migrate_slashing, &["slashing"]);
}

/// transfer 2 to 5: total escrow per denom, params out of the subspace
/// `transfer`, bank metadata for denom traces (bank migrates first, as in
/// `RunMigrations`).
#[test]
fn v4_migrates_transfer() {
    check(
        |ctx| {
            app::upgrades::v4::migrate_bank(ctx)?;
            app::upgrades::v4::migrate_transfer(ctx)
        },
        &["transfer", "bank"],
    );
}

/// blob 2 to 3: `MigrateParams`, subspace to store.
#[test]
fn v4_migrates_blob() {
    check(app::upgrades::v4::migrate_blob, &["blob"]);
}

/// staking 3 to 5: params (0x51), merged unbonding entries, the
/// delegations-by-validator index (0x71) and binary historical info keys.
#[test]
fn v4_migrates_staking() {
    check(app::upgrades::v4::migrate_staking, &["staking"]);
}

/// gov 3 to 5: voting-period index, params from the subspace and the
/// defaults, the default constitution. The `v4` handler then raises the
/// expedited min deposit ([`v4_upgrade_matches_go_in_every_store`]).
#[test]
fn v4_migrates_gov() {
    for f in fixtures() {
        check_gov(f);
    }
}

fn check_gov(f: Fixture) {
    let mut ms = handoff(f);
    {
        let mut c = ctx(f, &mut ms);
        app::upgrades::v4::migrate_gov(&mut c).unwrap();
    }
    let got = contents(&ms, "gov");
    let want = upgraded_state(f)["gov"].clone();
    // Everything but the params matches; the params differ only in the
    // expedited min deposit, which the handler sets after the migrations.
    let params = |kv: &Kv| {
        <celestia_proto::cosmos::gov::v1::Params as prost::Message>::decode(&kv[&b"0".to_vec()][..])
            .unwrap()
    };
    let (mut g, w) = (params(&got), params(&want));
    assert_eq!(g.expedited_min_deposit[0].denom, "stake");
    g.expedited_min_deposit.clone_from(&w.expedited_min_deposit);
    assert_eq!(g, w);
    let without_params = |kv: &Kv| {
        let mut kv = kv.clone();
        kv.remove(b"0".as_slice());
        kv
    };
    assert_eq!(without_params(&got), without_params(&want));
}

/// ibc core 2 to 6: the localhost client and sentinel connection, client,
/// connection and channel params.
#[test]
fn v4_migrates_ibc() {
    check(app::upgrades::v4::migrate_ibc, &["ibc"]);
}

/// The whole x/upgrade `ApplyUpgrade` of plan `v4` with the handlers of
/// celestia-app v4.1.0: every store (the v4 store set, which adds circuit,
/// consensus, hyperlane, minfee and warp) holds what Go's PreBlocker left,
/// including the consensus params at app version 4, the circuit disable
/// list, the default genesis of the modules the v3 version map lacks, the
/// module version map and the done marker of the plan.
#[test]
fn v4_upgrade_matches_go_in_every_store() {
    use x_upgrade::UpgradeHandlers as _;
    let handlers = app::upgrades::Handlers::of_release(4);
    assert!(handlers.has_handler("v4"));
    let mut failures = Vec::new();
    for f in fixtures() {
        let mut ms = handoff(f);
        {
            let mut c = ctx(f, &mut ms);
            let plan = x_upgrade::get_upgrade_plan(&mut c)
                .unwrap()
                .expect("plan v4");
            assert_eq!((plan.name.as_str(), plan.height), ("v4", f.upgrade));
            x_upgrade::apply_upgrade(&mut c, &handlers, &plan)
                .unwrap_or_else(|e| panic!("{}: ApplyUpgrade: {}", f.dir, e.log));
        }
        let want = upgraded_state(f);
        let v4 = sdk::version::Release::for_app_version(4).unwrap();
        for store in v4.stores {
            let got = contents(&ms, store);
            let w = want.get(*store).cloned().unwrap_or_default();
            if got != w {
                failures.push(format!(
                    "{}: {store}: rs vs go {}",
                    f.dir,
                    describe(&diff(&w, &got))
                ));
            }
        }
        let mut extra: Vec<&String> = want
            .keys()
            .filter(|s| !v4.stores.contains(&s.as_str()))
            .collect();
        extra.retain(|s| s.as_str() != "mem_capability");
        assert!(
            extra.is_empty(),
            "{}: stores outside the v4 set: {extra:?}",
            f.dir
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The rich chain reaches the cases that only appear with more state: the
/// slashing continuation key after 10 validators, the staking one after
/// 1,000 delegations, and gov's voting-period index.
#[test]
fn v4_rich_chain_reaches_the_continuations() {
    let f = fixtures().nth(1).unwrap();
    let diffs = common::diffs_in(&format!("{}/kv-upgrade.jsonl.gz", f.dir));
    let d = &diffs[&f.upgrade];
    assert!(d["slashing"].set.contains_key([0xFF].as_slice()));
    assert!(d["staking"].set.contains_key([0x81].as_slice()));
    assert!(d["gov"].set.keys().any(|k| k[0] == 0x04));
}

/// The BeginBlockers of the upgrade block finish what the migrations left
/// for later: staking indexes the remaining delegations and deletes its
/// cursor, and slashing walks the remaining signing infos and deletes its
/// cursor, as in Go's state after the upgrade block (rich chain).
#[test]
fn v4_begin_blockers_finish_the_continuations() {
    use x_upgrade::UpgradeHandlers as _;
    let f = fixtures().nth(1).unwrap();
    let handlers = app::upgrades::Handlers::of_release(4);
    assert!(handlers.has_handler("v4"));
    let mut ms = handoff(f);
    {
        let mut c = ctx(f, &mut ms);
        let plan = x_upgrade::get_upgrade_plan(&mut c).unwrap().unwrap();
        x_upgrade::apply_upgrade(&mut c, &handlers, &plan).unwrap();
        assert!(c.get("staking", &[0x81]).unwrap().is_some());
        assert!(c.get("slashing", &[0xFF]).unwrap().is_some());
        let cursor = c.get("staking", &[0x81]).unwrap().unwrap();
        x_staking::migrations::index_delegations_by_validator(&mut c, Some(&cursor), 10_000)
            .unwrap();
        // No votes: only the cursor walk of the slashing BeginBlocker.
        x_slashing::begin_block(&mut c).unwrap();
    }
    // Go's state after the upgrade block.
    let mut want = handoff_state(f);
    for (store, d) in &common::diffs_in(&format!("{}/kv-diffs.jsonl.gz", f.dir))[&f.upgrade] {
        let kv = want.entry(store.clone()).or_default();
        for k in &d.delete {
            kv.remove(k);
        }
        kv.extend(d.set.clone());
    }
    let pick = |kv: &Kv, prefixes: &[u8]| -> Kv {
        kv.iter()
            .filter(|(k, _)| prefixes.contains(&k[0]))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    };
    let got = pick(&contents(&ms, "staking"), &[0x71, 0x81]);
    let w = pick(&want["staking"], &[0x71, 0x81]);
    assert!(got.len() > 1_080, "index entries: {}", got.len());
    assert!(got == w, "staking: rs vs go {}", describe(&diff(&w, &got)));
    let got = pick(&contents(&ms, "slashing"), &[0xFF]);
    let w = pick(&want["slashing"], &[0xFF]);
    assert!(
        got.is_empty() && got == w,
        "slashing: rs vs go {}",
        describe(&diff(&w, &got))
    );
}

/// The history chain: staking's v5 migration moves 1,000 of the 1,005
/// v0.46 historical info keys and records the next one at 0x82; the
/// BeginBlocker of the upgrade block moves the rest and deletes the
/// cursor, as in Go's state after the upgrade block (the BeginBlocker's
/// own historical info of the upgrade height aside).
#[test]
fn v4_history_chain_finishes_the_historical_info_cursor() {
    use x_upgrade::UpgradeHandlers as _;
    let f = fixtures().nth(2).unwrap();
    let handlers = app::upgrades::Handlers::of_release(4);
    assert!(handlers.has_handler("v4"));
    let mut ms = handoff(f);
    {
        let mut c = ctx(f, &mut ms);
        let plan = x_upgrade::get_upgrade_plan(&mut c).unwrap().unwrap();
        x_upgrade::apply_upgrade(&mut c, &handlers, &plan).unwrap();
        let cursor = c
            .get("staking", &[0x82])
            .unwrap()
            .expect("historical info cursor after the migration");
        x_staking::migrations::migrate_historical_info_keys(&mut c, Some(&cursor), 1_000).unwrap();
    }
    let mut want = handoff_state(f);
    for (store, d) in &common::diffs_in(&format!("{}/kv-diffs.jsonl.gz", f.dir))[&f.upgrade] {
        let kv = want.entry(store.clone()).or_default();
        for k in &d.delete {
            kv.remove(k);
        }
        kv.extend(d.set.clone());
    }
    let own = [&[0x50][..], &f.upgrade.to_be_bytes()].concat();
    let pick = |kv: &Kv| -> Kv {
        kv.iter()
            .filter(|(k, _)| (k[0] == 0x50 || k[0] == 0x82) && **k != own)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    };
    let got = pick(&contents(&ms, "staking"));
    let w = pick(&want["staking"]);
    assert!(got.len() > 1_000, "historical infos: {}", got.len());
    assert!(got == w, "staking: rs vs go {}", describe(&diff(&w, &got)));
}
