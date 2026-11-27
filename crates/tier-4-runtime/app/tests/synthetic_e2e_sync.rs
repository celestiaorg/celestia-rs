//! One celestia-rs node syncs the synthetic chain from its genesis through
//! every upgrade (app versions 1, 2, 3, 4, 5, 6, 8, 9, 10 and 11) to its last
//! block, with no seeded state, as the Go oracles ran it: blocks 1 to 15
//! of `conformance/fixtures/synthetic/v1-v3` (cosmos-sdk v0.46, upgrades
//! at 6 and 11), then `v4-to-v5` (16 to 25, upgrade to v4 at 16),
//! `v5-to-v6` (26 to 36), `v6-to-v8` (37 to 46), `v8-to-v9` (47 to 56),
//! `v9-to-v10` (57 to 66), `v10-to-v11` (67 to 76) and `v11` (77 to 81),
//! which continue one chain through the oracles' shared app home.
//!
//! After every block the node must give Go's app hash, transaction
//! results and block response (events, validator updates, consensus
//! params, timeouts); from block 16 also Go's state in every store.
//!
//! - [`one_node_syncs_from_genesis_to_the_last_block`]: one persistent
//!   home, genesis to 81, restarted only where the runtime halts for the
//!   v11 upgrade ([`V11_UPGRADE`]).
//! - [`restarts_around_every_upgrade`]: the node closes and reopens its
//!   home after the block before, the block at and the block after each
//!   upgrade height.
//! - [`state_sync_in_every_era`]: a node that took a snapshot after every
//!   block; for one height of each app version, a fresh home restores that
//!   snapshot (format 2 while the state is cosmos-sdk v0.46, format 3
//!   after) and syncs on to 81.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::print_stdout
)]

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use app::App;
use celestia_proto::celestia_core_proto::abci;
use celestia_proto::celestia_core_proto::abci::ResponseFinalizeBlock;
use common::Kv;
use common::check::{check_response, check_tx_results, contents, describe, diff, update_set};

const BASE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../conformance/fixtures/synthetic"
);
/// The v0.46 fixture and the later fixtures with their blocks.
const LEGACY: (&str, i64, i64) = ("v1-v3", 1, 15);
const SEGMENTS: [(&str, i64, i64); 7] = [
    ("v4-to-v5", 16, 25),
    ("v5-to-v6", 26, 36),
    ("v6-to-v8", 37, 46),
    ("v8-to-v9", 47, 56),
    ("v9-to-v10", 57, 66),
    ("v10-to-v11", 67, 76),
    ("v11", 77, 81),
];
const LAST: i64 = 81;
/// The first block of each app version after 1.
const UPGRADES: [i64; 9] = [6, 11, 16, 26, 37, 47, 57, 67, 77];
/// The first v11 block. The runtime executes v10 until a restart finds
/// `data/upgrade-info.json` naming plan `v11` at the next height (as the
/// Go multiplexer swaps in the v11 binary): a node executing v10 halts at
/// this block with `UPGRADE "v11" NEEDED`, and resumes once reopened.
const V11_UPGRADE: i64 = 77;

fn json(path: &str) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn by_height(path: &str, field: &str) -> BTreeMap<i64, serde_json::Value> {
    common::gz_lines(path)
        .map(|d| {
            let v = if field.is_empty() {
                d.clone()
            } else {
                d[field].clone()
            };
            (d["height"].as_i64().unwrap(), v)
        })
        .collect()
}

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

/// One block of the chain as Go ran it.
struct Block {
    /// The `/block` result.
    raw: serde_json::Value,
    result: serde_json::Value,
    app_hash: String,
    /// The fixture directory (for the store state from block 16).
    dir: Option<String>,
}

/// The whole chain: genesis, the v2 upgrade height, every block, and the
/// validator set of every height.
struct Chain {
    genesis: serde_json::Value,
    upgrade_height_v2: i64,
    blocks: BTreeMap<i64, Block>,
    vals: BTreeMap<i64, common::ValidatorSet>,
    /// Go's state after each block from 16, by store.
    states: BTreeMap<i64, BTreeMap<String, Kv>>,
}

impl Chain {
    fn load() -> Self {
        let legacy = format!("{BASE}/{}", LEGACY.0);
        let roots = json(&format!("{legacy}/store-roots.json"));
        let mut blocks = BTreeMap::new();
        let results = by_height(&format!("{legacy}/tx-results.jsonl.gz"), "");
        for (h, block) in by_height(&format!("{legacy}/blocks.jsonl.gz"), "result") {
            let app_hash = roots["heights"][usize::try_from(h).unwrap()]["app_hash"]
                .as_str()
                .unwrap()
                .to_owned();
            blocks.insert(
                h,
                Block {
                    raw: block,
                    result: results[&h].clone(),
                    app_hash,
                    dir: None,
                },
            );
        }
        let mut states = BTreeMap::new();
        for (name, first, last) in SEGMENTS {
            let dir = format!("{BASE}/{name}");
            let roots = json(&format!("{dir}/store-roots.json"));
            let results = by_height(&format!("{dir}/tx-results.jsonl.gz"), "");
            let fixture_blocks = by_height(&format!("{dir}/blocks.jsonl.gz"), "result");
            let mut state: BTreeMap<String, Kv> =
                common::gz_lines(&format!("{dir}/kv-handoff.jsonl.gz"))
                    .map(|d| (d["store"].as_str().unwrap().to_owned(), kv_of(&d)))
                    .collect();
            let diffs = common::diffs_in(&format!("{dir}/kv-diffs.jsonl.gz"));
            for h in first..=last {
                for (store, d) in diffs.get(&h).cloned().unwrap_or_default() {
                    let kv = state.entry(store).or_default();
                    for k in &d.delete {
                        kv.remove(k);
                    }
                    kv.extend(d.set);
                }
                let mut s = state.clone();
                s.retain(|_, kv| !kv.is_empty());
                states.insert(h, s);
                let app_hash = roots["heights"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|r| r["height"] == h)
                    .unwrap()["app_hash"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                blocks.insert(
                    h,
                    Block {
                        raw: fixture_blocks[&h].clone(),
                        result: results[&h].clone(),
                        app_hash,
                        dir: Some(dir.clone()),
                    },
                );
            }
        }
        assert_eq!(
            blocks.keys().copied().collect::<Vec<_>>(),
            (1..=LAST).collect::<Vec<_>>()
        );
        let genesis = json(&format!("{legacy}/genesis.json"));
        Self {
            upgrade_height_v2: roots["upgrade_height_v2"].as_i64().unwrap(),
            genesis,
            blocks,
            vals: BTreeMap::new(),
            states,
        }
    }

    /// The validator set of every height, from InitChain's validators and
    /// each block's updates (which take effect two blocks later).
    fn with_validators(mut self, init: &[abci::ValidatorUpdate]) -> Self {
        let mut set = update_set(init);
        common::sort_validators(&mut set);
        self.vals.insert(1, set.clone());
        self.vals.insert(2, set);
        for h in 1..=LAST {
            let next = common::apply_updates(
                &self.vals[&(h + 1)],
                &common::oracle_updates(&self.blocks[&h].result),
            );
            self.vals.insert(h + 2, next);
        }
        self
    }

    /// The app version in effect after block `height` commits (the next
    /// block's header version).
    fn app_version_after(&self, height: i64) -> u64 {
        let next = self
            .blocks
            .get(&(height + 1))
            .unwrap_or(&self.blocks[&height]);
        next.raw["block"]["header"]["version"]["app"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap()
    }

    fn app_version_of(&self, height: i64) -> u64 {
        self.blocks[&height].raw["block"]["header"]["version"]["app"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap()
    }

    /// The FinalizeBlock request of block `height`: from app version 4,
    /// celestia-core v0.38 passes each BlobTx's inner transaction.
    fn request(&self, height: i64) -> abci::RequestFinalizeBlock {
        let last = if height == 1 {
            Vec::new()
        } else {
            self.vals[&(height - 1)].clone()
        };
        let mut req = common::finalize_request(&self.blocks[&height].raw, &last);
        if self.app_version_of(height) >= 4 {
            for tx in &mut req.txs {
                if let Some(blob_tx) = txdecode::unmarshal_blob_tx_v9(tx) {
                    *tx = blob_tx.unwrap().tx;
                }
            }
        }
        req
    }

    /// Executes and commits block `height` on `app` and requires Go's
    /// results, block response, state (from 16) and app hash. A node that
    /// halts for the v11 upgrade is reopened with `reopen` (the operator's
    /// restart) and runs the block again. Returns the node.
    #[must_use]
    fn step(&self, mut app: App, height: i64, label: &str, reopen: &dyn Fn() -> App) -> App {
        let res = match app.finalize_block(self.request(height)) {
            Ok(res) => res,
            Err(e) if height == V11_UPGRADE && app::is_upgrade_needed(&e.log) => {
                drop(app);
                app = reopen();
                app.finalize_block(self.request(height))
                    .unwrap_or_else(|e| panic!("{label} height {height} after restart: {}", e.log))
            }
            Err(e) => panic!("{label} height {height}: {}", e.log),
        };
        app.commit().unwrap();
        let b = &self.blocks[&height];
        let mut failures = check_tx_results(&res, &b.result);
        if b.dir.is_some() {
            failures.extend(check_response_v050(&res, &b.result));
        } else {
            failures.extend(check_response(&res, &b.result));
        }
        if b.dir.is_some() {
            for (store, want) in &self.states[&height] {
                let got = contents(app.multistore(), store);
                if got != *want {
                    failures.push(format!("{store}: rs vs go {}", describe(&diff(want, &got))));
                }
            }
        }
        let got = hex::encode_upper(app.info().last_block_app_hash);
        if got != b.app_hash {
            failures.push(format!("app hash {got} vs {}", b.app_hash));
        }
        assert!(
            failures.is_empty(),
            "{label} height {height}:\n{}",
            failures.join("\n")
        );
        app
    }
}

/// The block's events, validator updates (as `(address, power)`: ed25519
/// addresses are the first 20 bytes of the key's SHA-256), consensus
/// params and timeouts against a v0.50 oracle's record of the block.
fn check_response_v050(res: &ResponseFinalizeBlock, result: &serde_json::Value) -> Vec<String> {
    let mut failures = Vec::new();
    if let Some(d) = common::first_event_difference(&res.events, &common::events(&result["events"]))
    {
        failures.push(format!("events: {d}"));
    }
    let want_updates: Vec<(Vec<u8>, i64)> = result["validator_updates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| {
            let pk = hex::decode(u["pub_key"].as_str().unwrap()).unwrap();
            (
                hash::sha256(&pk)[..20].to_vec(),
                u["power"].as_i64().unwrap(),
            )
        })
        .collect();
    let got_updates = common::check::update_set(&res.validator_updates);
    if got_updates != want_updates {
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

/// Opens the node at `home`. The node home is where `x/upgrade` writes
/// `data/upgrade-info.json` and where the reopened node reads it.
fn open(home: &Path, chain: &Chain, mut options: app::AppOptions) -> App {
    options.node.home = Some(home.to_path_buf());
    let mut a = App::open_with(home, options).unwrap();
    a.set_v2_upgrade_height(chain.upgrade_height_v2);
    a
}

/// A fresh home at genesis, and the chain with its validator sets.
fn genesis_node(options: app::AppOptions) -> (tempfile::TempDir, App, Chain) {
    let chain = Chain::load();
    let home = tempfile::tempdir().unwrap();
    let mut a = open(home.path(), &chain, options);
    let init = a.init_chain(common::init_request(&chain.genesis)).unwrap();
    let chain = chain.with_validators(&init.validators);
    (home, a, chain)
}

#[test]
fn one_node_syncs_from_genesis_to_the_last_block() {
    let (home, mut a, chain) = genesis_node(app::AppOptions::default());
    let restarts = std::cell::Cell::new(0);
    let reopen = || {
        restarts.set(restarts.get() + 1);
        open(home.path(), &chain, app::AppOptions::default())
    };
    for h in 1..=LAST {
        a = chain.step(a, h, "sync", &reopen);
    }
    // The node executed v10 until it halted at the v11 upgrade block.
    assert_eq!(restarts.get(), 1, "restarts for the v11 upgrade");
    assert_eq!(a.info().last_block_height, LAST);
}

#[test]
fn restarts_around_every_upgrade() {
    let (home, mut a, chain) = genesis_node(app::AppOptions::default());
    let restart_after: Vec<i64> = UPGRADES.iter().flat_map(|u| [u - 1, *u, u + 1]).collect();
    let reopen = || open(home.path(), &chain, app::AppOptions::default());
    for h in 1..=LAST {
        a = chain.step(a, h, "restarts", &reopen);
        if restart_after.contains(&h) {
            drop(a);
            a = open(home.path(), &chain, app::AppOptions::default());
            let info = a.info();
            assert_eq!(info.last_block_height, h, "reopened after {h}");
            assert_eq!(
                hex::encode_upper(&info.last_block_app_hash),
                chain.blocks[&h].app_hash,
                "reopened after {h}"
            );
        }
    }
}

/// The snapshot heights: one per app version, inside each era.
const RESTORE_AT: [i64; 10] = [3, 8, 13, 20, 30, 40, 50, 59, 70, 79];

#[test]
fn state_sync_in_every_era() {
    let options = app::AppOptions {
        snapshots: snapshots::SnapshotOptions {
            interval: 1,
            keep_recent: 0,
        },
        ..app::AppOptions::default()
    };
    let (home, mut a, chain) = genesis_node(options.clone());
    let reopen = || open(home.path(), &chain, options.clone());
    for h in 1..=LAST {
        a = chain.step(a, h, "snapshotting node", &reopen);
        a.wait_for_snapshot();
    }
    let listed = a.list_snapshots().snapshots;
    let dir = home.path().join("data/snapshots");
    for h in RESTORE_AT {
        let height = u64::try_from(h).unwrap();
        let s = listed
            .iter()
            .find(|s| s.height == height)
            .unwrap_or_else(|| panic!("no snapshot at {h}"))
            .clone();
        let want_format = if chain.app_version_after(h) <= 3 {
            snapshots::FORMAT_V046
        } else {
            3
        };
        assert_eq!(s.format, want_format, "snapshot {h} format");
        let fresh = tempfile::tempdir().unwrap();
        let restored = app::AppOptions {
            chain_id: Some("test".to_owned()),
            genesis_app_version: Some(1),
            ..app::AppOptions::default()
        };
        let mut b = open(fresh.path(), &chain, restored.clone());
        let want_hash = hex::decode(&chain.blocks[&h].app_hash).unwrap();
        let offer = b.offer_snapshot(abci::RequestOfferSnapshot {
            snapshot: Some(s.clone()),
            app_hash: want_hash.clone(),
            app_version: chain.app_version_after(h),
        });
        assert_eq!(
            offer.result,
            abci::response_offer_snapshot::Result::Accept as i32,
            "offer of snapshot {h}"
        );
        for i in 0..s.chunks {
            let chunk = std::fs::read(
                dir.join(height.to_string())
                    .join(s.format.to_string())
                    .join(i.to_string()),
            )
            .unwrap();
            let r = b.apply_snapshot_chunk(abci::RequestApplySnapshotChunk {
                index: i,
                chunk,
                sender: String::new(),
            });
            assert_eq!(
                r.result,
                abci::response_apply_snapshot_chunk::Result::Accept as i32,
                "snapshot {h} chunk {i}"
            );
        }
        assert_eq!(b.info().last_block_height, h, "restored {h}");
        assert_eq!(b.info().last_block_app_hash, want_hash, "restored {h}");
        for next in h + 1..=LAST {
            b = chain.step(b, next, &format!("restored at {h}"), &|| {
                open(fresh.path(), &chain, restored.clone())
            });
        }
        println!("restored at {h} (format {}) and synced to {LAST}", s.format);
    }
}
