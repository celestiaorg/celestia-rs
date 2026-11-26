//! Restoring Go cosmos-sdk v0.46 state-sync snapshots (format 2) of the
//! oracle chains (`conformance/v1-oracle ... -snapshot-at`, fixtures
//! `conformance/fixtures/synthetic/*-snapshots`).
//!
//! celestia-app v3.13.0's snapshot manager wrote each snapshot right after
//! committing its height, with that height's store set:
//!
//! - the synthetic chain: app version 1 (3), the v1 to v2 upgrade block
//!   after its migrations (5), app version 2 (7), app version 3 (12);
//! - the staking chain: vesting accounts and val3 missing blocks (9), val3
//!   jailed (13), pending unbondings, a redelegation and grants at app
//!   version 3 (27);
//! - the IBC chain: a PFM forward in flight (39), the interchain account
//!   channel just opened (72), app version 3 with a new validator (100),
//!   the reopened account channel (120).
//!
//! A fresh [`app::App`] restores each one through `OfferSnapshot` and
//! `ApplySnapshotChunk`, must report the oracle's height, app version and
//! app hash, and then executes every later block of the chain (across the
//! later upgrades) with the oracle's app hash and transaction results.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stdout,
    clippy::panic
)]

mod common;

use std::path::PathBuf;

use app::legacy::{Driver, V1Modules};
use celestia_proto::celestia_core_proto::abci;
use common::fixture::Fixture;

struct Chain {
    fixture: &'static str,
    snapshots: &'static str,
}

const SYNTHETIC: Chain = Chain {
    fixture: "v1-v3",
    snapshots: "v1-v3-snapshots",
};
const STAKING: Chain = Chain {
    fixture: "staking-v1-v3",
    snapshots: "staking-v1-v3-snapshots",
};
const IBC: Chain = Chain {
    fixture: "ibc-v2",
    snapshots: "ibc-v2-snapshots",
};

impl Chain {
    fn fix(&self) -> Fixture {
        Fixture::synthetic(self.fixture)
    }

    /// The Go snapshot directory (`<height>/<format>/<chunk>`).
    fn go_snapshots(&self) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../conformance/fixtures/synthetic")
            .join(self.snapshots)
    }

    /// The snapshot's chunks, in order.
    fn chunks(&self, height: u64) -> Vec<Vec<u8>> {
        let dir = self
            .go_snapshots()
            .join(height.to_string())
            .join(snapshots::FORMAT_V046.to_string());
        let mut out = Vec::new();
        while let Ok(bz) = std::fs::read(dir.join(out.len().to_string())) {
            out.push(bz);
        }
        assert!(!out.is_empty(), "no chunks in {}", dir.display());
        out
    }

    /// The app version of the consensus params after block `height`.
    fn app_version_after(&self, height: i64) -> u64 {
        let mut v = 1;
        for (h, r) in self.fix().results() {
            if h <= height
                && let Some(av) = r["consensus_param_updates"]["version"]["app_version"].as_u64()
            {
                v = av;
            }
        }
        v
    }

    fn restore(&self, height: u64) -> (tempfile::TempDir, app::App) {
        let fix = self.fix();
        let home = tempfile::tempdir().unwrap();
        let mut a = app::App::open(home.path()).unwrap();
        a.set_v2_upgrade_height(fix.roots()["upgrade_height_v2"].as_i64().unwrap());
        let chunks = self.chunks(height);
        let mut total = hash::Sha256Hasher::new();
        let mut chunk_hashes = Vec::new();
        for c in &chunks {
            total.update(c);
            chunk_hashes.push(hash::sha256(c).to_vec());
        }
        let snap = snapshots::Snapshot {
            height,
            format: snapshots::FORMAT_V046,
            chunks: u32::try_from(chunks.len()).unwrap(),
            hash: total.finalize().to_vec(),
            chunk_hashes,
        };
        let h = i64::try_from(height).unwrap();
        let want_hash = hex::decode(
            fix.roots()["heights"][usize::try_from(height).unwrap()]["app_hash"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        let offer = a.offer_snapshot(abci::RequestOfferSnapshot {
            snapshot: Some(abci::Snapshot {
                height,
                format: snap.format,
                chunks: snap.chunks,
                hash: snap.hash.clone(),
                metadata: snap.metadata_bytes(),
            }),
            app_hash: want_hash.clone(),
            app_version: self.app_version_after(h),
        });
        assert_eq!(
            offer.result,
            abci::response_offer_snapshot::Result::Accept as i32,
            "{} offer of snapshot {height}",
            self.fixture
        );
        for (i, chunk) in chunks.into_iter().enumerate() {
            let r = a.apply_snapshot_chunk(abci::RequestApplySnapshotChunk {
                index: u32::try_from(i).unwrap(),
                chunk,
                sender: String::new(),
            });
            assert_eq!(
                r.result,
                abci::response_apply_snapshot_chunk::Result::Accept as i32,
                "{} chunk {i} of snapshot {height}",
                self.fixture
            );
        }
        let info = a.info();
        assert_eq!(info.last_block_height, h, "restored height");
        assert_eq!(
            info.app_version,
            self.app_version_after(h),
            "restored app version"
        );
        assert_eq!(
            info.last_block_app_hash, want_hash,
            "{} restored app hash at {height}",
            self.fixture
        );
        assert!(a.is_sdk_v046_state());
        (home, a)
    }

    /// The genesis validator updates, from a throwaway InitChain.
    fn genesis_validators(&self) -> Vec<abci::ValidatorUpdate> {
        let mut ms = sdk::MultiStore::new();
        app::stores::mount_release(&mut ms, sdk::version::Release::for_app_version(1).unwrap())
            .unwrap();
        Driver::new(V1Modules)
            .init_chain(&mut ms, &common::init_request(&self.fix().genesis()))
            .unwrap()
            .validators
    }

    fn restore_and_replay(&self, height: u64) {
        let f = self.fix();
        let (_home, mut a) = self.restore(height);
        let roots = f.roots();
        let heights = roots["heights"].as_array().unwrap();
        let results = f.results();
        let blocks = f.blocks();
        let vals = f.validator_sets(&self.genesis_validators());
        let last = f.last_height();
        for h in i64::try_from(height).unwrap() + 1..=last {
            let res = a
                .finalize_block(common::finalize_request(&blocks[&h], &vals[&(h - 1)]))
                .unwrap_or_else(|e| {
                    panic!(
                        "{} height {h} after snapshot {height}: {}",
                        self.fixture, e.log
                    )
                });
            a.commit().unwrap();
            let want = heights[usize::try_from(h).unwrap()]["app_hash"]
                .as_str()
                .unwrap();
            assert_eq!(
                hex::encode_upper(&res.app_hash),
                want,
                "{} app hash at {h} after snapshot {height}",
                self.fixture
            );
            let failures = common::check::check_tx_results(&res, &results[&h]);
            assert!(
                failures.is_empty(),
                "{} height {h}: {}",
                self.fixture,
                failures.join("\n")
            );
        }
        println!(
            "{}: snapshot {height} restored, blocks {} to {last} match",
            self.fixture,
            height + 1
        );
        assert_eq!(a.info().app_version, self.app_version_after(last));
    }
}

impl Chain {
    /// Replays the chain from genesis with a snapshot after every commit
    /// and requires, at each of `heights` (Go's snapshot heights), a format
    /// 2 snapshot with the same items as Go's, in the same order; then
    /// restores that Rust snapshot into a fresh app.
    fn export_like_go(&self, heights: &[u64]) {
        let f = self.fix();
        let home = tempfile::tempdir().unwrap();
        let mut a = app::App::open_with(
            home.path(),
            app::AppOptions {
                snapshots: snapshots::SnapshotOptions {
                    interval: 1,
                    keep_recent: 0,
                },
                ..app::AppOptions::default()
            },
        )
        .unwrap();
        a.set_v2_upgrade_height(f.roots()["upgrade_height_v2"].as_i64().unwrap());
        a.init_chain(common::init_request(&f.genesis())).unwrap();
        let blocks = f.blocks();
        let vals = f.validator_sets(&self.genesis_validators());
        let last = i64::try_from(*heights.iter().max().unwrap()).unwrap();
        for h in 1..=last {
            let prev = vals.get(&(h - 1)).cloned().unwrap_or_default();
            a.finalize_block(common::finalize_request(&blocks[&h], &prev))
                .unwrap_or_else(|e| panic!("{} height {h}: {}", self.fixture, e.log));
            a.commit().unwrap();
            a.wait_for_snapshot();
        }
        let listed = a.list_snapshots().snapshots;
        let rust = home.path().join("data/snapshots");
        for &h in heights {
            let s = listed
                .iter()
                .find(|s| s.height == h)
                .unwrap_or_else(|| panic!("{}: no Rust snapshot at {h}", self.fixture));
            assert_eq!(
                s.format,
                snapshots::FORMAT_V046,
                "{} snapshot {h}",
                self.fixture
            );
            let c = snapshots::compare_dirs(&rust, &self.go_snapshots(), h, snapshots::FORMAT_V046)
                .unwrap();
            assert!(
                c.first_difference.is_none(),
                "{} snapshot {h}: {}",
                self.fixture,
                c.first_difference.unwrap()
            );
            assert!(c.items > 0);
            println!(
                "{} snapshot {h}: {} items equal to Go's (chunk bytes identical: {})",
                self.fixture, c.items, c.chunk_bytes_identical
            );
            self.restore_rust(&rust, h, &listed);
        }
    }

    /// Restores a Rust format 2 snapshot into a fresh app.
    fn restore_rust(&self, dir: &std::path::Path, height: u64, listed: &[abci::Snapshot]) {
        let s = listed.iter().find(|s| s.height == height).unwrap().clone();
        let fix = self.fix();
        let want = hex::decode(
            fix.roots()["heights"][usize::try_from(height).unwrap()]["app_hash"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        let home = tempfile::tempdir().unwrap();
        let mut b = app::App::open(home.path()).unwrap();
        b.set_v2_upgrade_height(fix.roots()["upgrade_height_v2"].as_i64().unwrap());
        let chunks = s.chunks;
        let offer = b.offer_snapshot(abci::RequestOfferSnapshot {
            snapshot: Some(s),
            app_hash: want.clone(),
            app_version: self.app_version_after(i64::try_from(height).unwrap()),
        });
        assert_eq!(
            offer.result,
            abci::response_offer_snapshot::Result::Accept as i32
        );
        for i in 0..chunks {
            let chunk = std::fs::read(
                dir.join(height.to_string())
                    .join(snapshots::FORMAT_V046.to_string())
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
                abci::response_apply_snapshot_chunk::Result::Accept as i32
            );
        }
        assert_eq!(
            b.info().last_block_app_hash,
            want,
            "{} Rust snapshot {height}",
            self.fixture
        );
    }
}

#[test]
fn exports_synthetic_chain_snapshots_like_go() {
    SYNTHETIC.export_like_go(&[3, 5, 7, 12]);
}

#[test]
fn exports_staking_chain_snapshots_like_go() {
    STAKING.export_like_go(&[9, 13, 27]);
}

#[test]
fn exports_ibc_chain_snapshots_like_go() {
    IBC.export_like_go(&[39, 72, 100, 120]);
}

#[test]
fn restores_an_app_version_1_snapshot() {
    SYNTHETIC.restore_and_replay(3);
}

#[test]
fn restores_the_v2_upgrade_block_snapshot() {
    SYNTHETIC.restore_and_replay(5);
}

#[test]
fn restores_an_app_version_2_snapshot() {
    SYNTHETIC.restore_and_replay(7);
}

#[test]
fn restores_an_app_version_3_snapshot() {
    SYNTHETIC.restore_and_replay(12);
}

#[test]
fn restores_staking_chain_snapshots() {
    for h in [9, 13, 27] {
        STAKING.restore_and_replay(h);
    }
}

#[test]
fn restores_ibc_chain_snapshots() {
    for h in [39, 72, 100, 120] {
        IBC.restore_and_replay(h);
    }
}

/// A v0.46 app version restores only from format 2, and only into a
/// fresh app.
#[test]
fn refuses_other_formats_and_held_states() {
    use abci::response_offer_snapshot::Result as R;
    let offer = |a: &mut app::App, format: u32| {
        let chunks = SYNTHETIC.chunks(3);
        let snap = snapshots::Snapshot {
            height: 3,
            format,
            chunks: 1,
            hash: hash::sha256(&chunks[0]).to_vec(),
            chunk_hashes: vec![hash::sha256(&chunks[0]).to_vec()],
        };
        a.offer_snapshot(abci::RequestOfferSnapshot {
            snapshot: Some(abci::Snapshot {
                height: 3,
                format,
                chunks: 1,
                hash: snap.hash.clone(),
                metadata: snap.metadata_bytes(),
            }),
            app_hash: Vec::new(),
            app_version: 1,
        })
        .result
    };
    let home = tempfile::tempdir().unwrap();
    let mut fresh = app::App::open(home.path()).unwrap();
    assert_eq!(
        offer(&mut fresh, snapshots::CURRENT_FORMAT),
        R::Reject as i32
    );
    let (_home, mut restored) = SYNTHETIC.restore(3);
    assert_eq!(
        offer(&mut restored, snapshots::FORMAT_V046),
        R::Reject as i32
    );
}

/// The out-of-block memory store rebuild (after a restore or a reopen)
/// leaves InitChain and block 1 unchanged: InitGenesis already rebuilds the
/// capability memory store, so every validator's first block after
/// InitChain finds it initialized and its BeginBlocker charges only the
/// `mem_initialized` read, as on every later block. A fresh home (height 0)
/// never runs the out-of-block rebuild.
#[test]
fn init_chain_leaves_the_memory_store_initialized_for_block_1() {
    use sdk::{Context, ExecMode, Header};
    let mut ms = sdk::MultiStore::new();
    app::stores::mount_release(&mut ms, sdk::version::Release::for_app_version(1).unwrap())
        .unwrap();
    let mut driver = Driver::new(V1Modules);
    driver
        .init_chain(&mut ms, &common::init_request(&SYNTHETIC.fix().genesis()))
        .unwrap();
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    ms.select(sdk::store::Stack::Finalize);
    let gas = |ms: &mut sdk::MultiStore, rebuild: bool| {
        let mut ctx = Context::new(ms, Header::default(), ExecMode::Finalize);
        if rebuild {
            x_capability::init_mem_store(&mut ctx).unwrap();
        } else {
            assert!(x_capability::is_initialized(&mut ctx).unwrap());
        }
        ctx.gas.consumed_to_limit()
    };
    let read = gas(&mut ms, false);
    assert!(read > 0, "the mem_initialized read costs gas at v0.46");
    assert_eq!(
        gas(&mut ms, true),
        read,
        "block 1 does not rebuild the memory store"
    );
}
