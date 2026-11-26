//! The `v4` upgrade (cosmos-sdk v0.46 to v0.50) against the Go oracle,
//! module by module: the synthetic chain's state after its last v3 block
//! (`kv-handoff.jsonl.gz` of `conformance/fixtures/synthetic/v4`) goes
//! in, a Rust migration runs at the upgrade height, and the stores it owns
//! must hold what the Go store loader and `v4` upgrade handler left
//! (`kv-upgrade.jsonl.gz`: the x/upgrade PreBlocker of block 16 alone).

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::collections::BTreeMap;

use common::Kv;
use common::check::{contents, describe, diff};
use sdk::{Context, ExecMode, Header, MultiStore};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic/v4"
);
/// The upgrade block.
const UPGRADE: i64 = 16;

fn hex_pair(p: &serde_json::Value) -> (Vec<u8>, Vec<u8>) {
    (
        hex::decode(p[0].as_str().unwrap()).unwrap(),
        hex::decode(p[1].as_str().unwrap()).unwrap(),
    )
}

/// The oracle's state at the hand-off height, by store.
fn handoff_state() -> BTreeMap<String, Kv> {
    common::gz_lines(&format!("{FIXTURE}/kv-handoff.jsonl.gz"))
        .map(|d| {
            let kv = d["kv"].as_array().unwrap().iter().map(hex_pair).collect();
            (d["store"].as_str().unwrap().to_owned(), kv)
        })
        .collect()
}

/// The oracle's state after the upgrade handler, by store.
fn upgraded_state() -> BTreeMap<String, Kv> {
    let mut state = handoff_state();
    let diffs = common::diffs_in(&format!("{FIXTURE}/kv-upgrade.jsonl.gz"));
    for (store, d) in &diffs[&UPGRADE] {
        let kv = state.entry(store.clone()).or_default();
        for k in &d.delete {
            kv.remove(k);
        }
        kv.extend(d.set.clone());
    }
    state
}

/// The block time of the upgrade block (`v1-oracle` block times).
fn upgrade_time() -> pbjson_types::Timestamp {
    let blocks: BTreeMap<i64, serde_json::Value> =
        common::gz_lines(&format!("{FIXTURE}/blocks.jsonl.gz"))
            .map(|d| (d["height"].as_i64().unwrap(), d["result"].clone()))
            .collect();
    common::timestamp(
        blocks[&UPGRADE]["block"]["header"]["time"]
            .as_str()
            .unwrap(),
    )
}

/// A store with the app version 4 store set holding the hand-off state.
fn handoff() -> MultiStore {
    let mut ms = MultiStore::new();
    let v4 = sdk::version::Release::for_app_version(4).unwrap();
    app::stores::mount_release(&mut ms, v4).unwrap();
    let state = handoff_state();
    let mut ctx = ctx(&mut ms);
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

fn ctx(ms: &mut MultiStore) -> Context<'_> {
    let header = Header {
        chain_id: "test".into(),
        height: UPGRADE,
        time: upgrade_time(),
        ..Header::default()
    };
    Context::new(ms, header, ExecMode::Finalize)
}

/// Runs `migrate` on the hand-off state and requires each of `stores`
/// (and the untouched x/params store) to hold the oracle's result.
fn check(migrate: impl FnOnce(&mut Context<'_>) -> sdk::Result<()>, stores: &[&str]) {
    let mut ms = handoff();
    {
        let mut c = ctx(&mut ms);
        migrate(&mut c).unwrap_or_else(|e| panic!("migration: {}", e.log));
    }
    let want = upgraded_state();
    let mut failures = Vec::new();
    for store in stores.iter().chain(&["params"]) {
        let got = contents(&ms, store);
        let w = want.get(*store).cloned().unwrap_or_default();
        if got != w {
            failures.push(format!("{store}: rs vs go {}", describe(&diff(&w, &got))));
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
    let mut ms = handoff();
    {
        let mut c = ctx(&mut ms);
        app::upgrades::v4::migrate_gov(&mut c).unwrap();
    }
    let got = contents(&ms, "gov");
    let want = upgraded_state()["gov"].clone();
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
        kv.remove(&b"0".to_vec());
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
    let mut ms = handoff();
    {
        let mut c = ctx(&mut ms);
        let plan = x_upgrade::get_upgrade_plan(&mut c)
            .unwrap()
            .expect("plan v4");
        assert_eq!((plan.name.as_str(), plan.height), ("v4", UPGRADE));
        x_upgrade::apply_upgrade(&mut c, &handlers, &plan)
            .unwrap_or_else(|e| panic!("ApplyUpgrade: {}", e.log));
    }
    let want = upgraded_state();
    let v4 = sdk::version::Release::for_app_version(4).unwrap();
    let mut failures = Vec::new();
    for store in v4.stores {
        let got = contents(&ms, store);
        let w = want.get(*store).cloned().unwrap_or_default();
        if got != w {
            failures.push(format!("{store}: rs vs go {}", describe(&diff(&w, &got))));
        }
    }
    let mut extra: Vec<&String> = want
        .keys()
        .filter(|s| !v4.stores.contains(&s.as_str()))
        .collect();
    extra.retain(|s| s.as_str() != "mem_capability");
    assert!(extra.is_empty(), "stores outside the v4 set: {extra:?}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
