//! A Go oracle fixture of a chain built by `conformance/v1-oracle`
//! (`fixtures/synthetic/*`): its genesis, blocks and the oracle's record,
//! and the replay of its blocks through the cosmos-sdk v0.46 driver.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stdout,
    clippy::panic,
    dead_code
)]

use std::collections::BTreeMap;

use app::legacy::{Driver, V1Modules};
use sdk::MultiStore;

use super::check::{check_response, check_tx_results, contents, describe, diff, update_set};
use super::{Diff, Kv};

/// A fixture directory.
pub struct Fixture {
    /// The directory.
    pub dir: String,
}

impl Fixture {
    /// `conformance/fixtures/synthetic/<name>`.
    pub fn synthetic(name: &str) -> Self {
        Self {
            dir: format!(
                "{}/../../../conformance/fixtures/synthetic/{name}",
                env!("CARGO_MANIFEST_DIR")
            ),
        }
    }

    /// A file's bytes.
    pub fn read(&self, name: &str) -> Vec<u8> {
        std::fs::read(format!("{}/{name}", self.dir)).unwrap()
    }

    /// A gzipped file, uncompressed.
    pub fn gunzip(&self, name: &str) -> Vec<u8> {
        let mut out = Vec::new();
        std::io::Read::read_to_end(
            &mut flate2::read::GzDecoder::new(&self.read(name)[..]),
            &mut out,
        )
        .unwrap();
        out
    }

    /// `store-roots.json`.
    pub fn roots(&self) -> serde_json::Value {
        serde_json::from_slice(&self.read("store-roots.json")).unwrap()
    }

    /// The genesis.
    pub fn genesis(&self) -> serde_json::Value {
        serde_json::from_slice(&self.read("genesis.json")).unwrap()
    }

    /// The last recorded height.
    pub fn last_height(&self) -> i64 {
        self.roots()["heights"].as_array().unwrap().last().unwrap()["height"]
            .as_i64()
            .unwrap()
    }

    /// `tx-results.jsonl.gz` by height.
    pub fn results(&self) -> BTreeMap<i64, serde_json::Value> {
        super::gz_lines(&format!("{}/tx-results.jsonl.gz", self.dir))
            .map(|d| (d["height"].as_i64().unwrap(), d))
            .collect()
    }

    /// `kv-diffs.jsonl.gz` by height and store.
    pub fn diffs(&self) -> BTreeMap<i64, BTreeMap<String, Diff>> {
        super::diffs_in(&format!("{}/kv-diffs.jsonl.gz", self.dir))
    }

    /// `blocks.jsonl.gz` by height (the RPC `block` results).
    pub fn blocks(&self) -> BTreeMap<i64, serde_json::Value> {
        super::gz_lines(&format!("{}/blocks.jsonl.gz", self.dir))
            .map(|d| (d["height"].as_i64().unwrap(), d["result"].clone()))
            .collect()
    }

    /// The oracle's state at height 0, by store.
    pub fn height_0(&self) -> BTreeMap<String, Kv> {
        super::gz_lines(&format!("{}/kv-height-0.jsonl.gz", self.dir))
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

    /// Validator sets by height: the genesis set signs heights 1 and 2,
    /// and each EndBlock(h)'s updates take effect at h + 2.
    pub fn validator_sets(
        &self,
        genesis: &[celestia_proto::celestia_core_proto::abci::ValidatorUpdate],
    ) -> BTreeMap<i64, super::ValidatorSet> {
        let results = self.results();
        let mut vals = BTreeMap::new();
        let mut genesis_set = update_set(genesis);
        super::sort_validators(&mut genesis_set);
        vals.insert(1, genesis_set.clone());
        vals.insert(2, genesis_set);
        for h in 1..self.last_height() {
            let next = super::apply_updates(&vals[&(h + 1)], &super::oracle_updates(&results[&h]));
            vals.insert(h + 2, next);
        }
        vals
    }

    /// Runs InitChain and blocks 1 to `through` through the v0.46 driver
    /// and requires, for each block, the oracle's KV changes of every
    /// store, its store roots, the app hash, every transaction result and
    /// the block's events, validator updates, consensus params and
    /// timeouts.
    #[allow(clippy::too_many_lines)]
    pub fn run_blocks(&self, through: i64) {
        let genesis = self.genesis();
        let r = self.roots();
        let heights = r["heights"].as_array().unwrap();
        let results = self.results();
        let mut want_diffs = self.diffs();
        let blocks = self.blocks();

        let mut ms = v1_store();
        let mut driver = Driver::new(V1Modules);
        driver.set_upgrade_height_v2(r["upgrade_height_v2"].as_i64().unwrap());
        let init = driver
            .init_chain(&mut ms, &super::init_request(&genesis))
            .unwrap();
        assert_eq!(init.app_hash, hash::sha256(&[]).to_vec());
        assert_eq!(r["init_chain_app_hash"], hex::encode_upper(&init.app_hash));

        // InitChain writes the oracle's height-0 state (Go commits it alone as
        // height 0; block 1 runs on it uncommitted).
        let state0 = self.height_0();
        let stores0 = iavl_stores(&heights[0]);
        assert_eq!(
            stores0,
            state0.keys().cloned().collect::<Vec<_>>(),
            "height 0 stores"
        );
        for (s, want) in &state0 {
            let got = contents(&ms, s);
            assert!(
                got == *want,
                "height 0 {s}: rs vs go {}",
                describe(&diff(want, &got))
            );
        }

        let vals = self.validator_sets(&init.validators);

        let mut before = state0;
        for height in 1..=through {
            let last = if height == 1 {
                Vec::new()
            } else {
                vals[&(height - 1)].clone()
            };
            let req = super::finalize_request(&blocks[&height], &last);
            let res = driver
                .finalize_block(&mut ms, &req)
                .unwrap_or_else(|e| panic!("height {height}: {}", e.log));
            let id = driver.commit(&mut ms).unwrap();

            let want_roots = &heights[usize::try_from(height).unwrap()];
            let want_diff = want_diffs.remove(&height).unwrap_or_default();
            let stores = iavl_stores(want_roots);
            let info = ms.root().last_commit_info().unwrap().clone();
            let mut failures = Vec::new();
            let mut names: Vec<&str> = info.store_infos.iter().map(|s| s.name.as_str()).collect();
            names.sort_unstable();
            let want_names: Vec<&str> = want_roots["stores"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            if names != want_names {
                failures.push(format!("commit info stores {names:?}"));
            }
            let mut after = BTreeMap::new();
            for s in &stores {
                let now = if ms.is_mounted(s) {
                    contents(&ms, s)
                } else {
                    Kv::new()
                };
                let got = diff(before.get(s).unwrap_or(&Kv::new()), &now);
                let want = want_diff.get(s).cloned().unwrap_or_default();
                if got != want {
                    failures.push(format!(
                        "{s}: kv diff\n  rs {}\n  go {}",
                        describe(&got),
                        describe(&want)
                    ));
                }
                let root = info
                    .store_info(s)
                    .and_then(|si| si.commit_id.as_ref())
                    .map(|c| hex::encode_upper(c.hash))
                    .unwrap_or_default();
                if root != want_roots["stores"][s].as_str().unwrap() {
                    failures.push(format!("{s}: root {root}"));
                }
                after.insert(s.clone(), now);
            }
            before = after;
            let app_hash = hex::encode_upper(id.hash);
            if app_hash != want_roots["app_hash"].as_str().unwrap()
                || hex::encode_upper(&res.app_hash) != app_hash
            {
                failures.push(format!(
                    "app hash {app_hash}, want {}",
                    want_roots["app_hash"]
                ));
            }
            failures.extend(check_tx_results(&res, &results[&height]));
            failures.extend(check_response(&res, &results[&height]));
            assert!(
                failures.is_empty(),
                "height {height}:\n{}",
                failures.join("\n")
            );
            println!(
                "{} block {height} matches {app_hash}",
                self.dir.rsplit('/').next().unwrap_or_default()
            );
        }
    }
}

fn v1_store() -> MultiStore {
    let mut ms = MultiStore::new();
    let v1 = sdk::version::Release::for_app_version(1).unwrap();
    app::stores::mount_release(&mut ms, v1).unwrap();
    ms
}

/// The IAVL stores of a height's commit info.
fn iavl_stores(roots: &serde_json::Value) -> Vec<String> {
    roots["stores"]
        .as_object()
        .unwrap()
        .keys()
        .filter(|s| *s != "mem_capability")
        .cloned()
        .collect()
}
