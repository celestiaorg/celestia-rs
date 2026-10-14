//! Storage-backed multistore: restart, crash consistency, pruning and
//! snapshot export/restore.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::sync::Arc;

use kvdb::{FaultyBackend, KvBackend, MemBackend, RedbBackend, RedbOptions};
use multistore::{CommitId, PruningOptions, Store, StoreKind, StoreOptions};

const STORES: &[&str] = &["acc", "bank", "empty", "staking"];

fn redb(path: &Path) -> Arc<dyn KvBackend> {
    Arc::new(RedbBackend::open(path.join("state.redb"), RedbOptions::default()).unwrap())
}

fn mount(mut store: Store) -> Store {
    for name in STORES {
        store.mount_store(*name, StoreKind::Iavl).unwrap();
    }
    store.load_latest_version().unwrap();
    store
}

fn open(db: Arc<dyn KvBackend>, pruning: PruningOptions) -> Store {
    mount(Store::open(
        db,
        StoreOptions {
            pruning,
            node_cache_size: 64,
        },
    ))
}

/// Deterministic writes for `height`.
fn block(store: &mut Store, height: i64) {
    for i in 0..20 {
        let x = (height * 31 + i * 17) % 97;
        let name = ["acc", "bank", "staking"][usize::try_from(x).unwrap() % 3];
        let s = store.get_store_mut(name).unwrap();
        let key = format!("key/{:02}", (x * 7 + height) % 50).into_bytes();
        if x % 5 == 0 {
            s.delete(&key).unwrap();
        } else {
            s.set(key, format!("{height}:{i}").into_bytes()).unwrap();
        }
    }
}

fn reference(heights: i64) -> Vec<CommitId> {
    let mut mem = mount(Store::new());
    (1..=heights)
        .map(|h| {
            block(&mut mem, h);
            mem.commit().unwrap()
        })
        .collect()
}

#[test]
fn restart_reproduces_last_commit_and_continues_in_lockstep() {
    let expected = reference(30);
    let dir = tempfile::tempdir().unwrap();
    {
        let mut store = open(redb(dir.path()), PruningOptions::nothing());
        for h in 1..=15 {
            block(&mut store, h);
            assert_eq!(
                store.commit().unwrap(),
                expected[usize::try_from(h - 1).unwrap()]
            );
        }
        store.set_metadata(b"chain_id", b"test-chain".to_vec());
        block(&mut store, 16);
        assert_eq!(store.commit().unwrap(), expected[15]);
    }
    let mut store = open(redb(dir.path()), PruningOptions::nothing());
    assert_eq!(store.version(), 16);
    assert_eq!(store.last_commit_id().unwrap(), expected[15]);
    assert_eq!(
        store.get_metadata(b"chain_id").unwrap(),
        Some(b"test-chain".to_vec())
    );
    assert_eq!(
        store.get_commit_info(3).unwrap().commit_id().unwrap(),
        expected[2]
    );
    for h in 17..=30 {
        block(&mut store, h);
        assert_eq!(
            store.commit().unwrap(),
            expected[usize::try_from(h - 1).unwrap()]
        );
    }
}

#[test]
fn interrupted_commit_leaves_previous_height_intact() {
    let expected = reference(8);
    let dir = tempfile::tempdir().unwrap();
    {
        let faulty = Arc::new(FaultyBackend::new(redb(dir.path())));
        let mut store = open(faulty.clone(), PruningOptions::nothing());
        for h in 1..=5 {
            block(&mut store, h);
            store.commit().unwrap();
        }
        // Height 6: the process "crashes" before the commit transaction
        // becomes durable.
        block(&mut store, 6);
        faulty.fail_writes(true);
        assert!(store.commit().is_err());
    }
    let mut store = open(redb(dir.path()), PruningOptions::nothing());
    assert_eq!(store.version(), 5);
    assert_eq!(store.last_commit_id().unwrap(), expected[4]);
    // celestia-core replays height 6 after the handshake.
    for h in 6..=8 {
        block(&mut store, h);
        assert_eq!(
            store.commit().unwrap(),
            expected[usize::try_from(h - 1).unwrap()]
        );
    }
}

#[test]
fn pruning_keeps_recent_and_every_versions_with_exact_hashes() {
    let expected = reference(40);
    let db: Arc<dyn KvBackend> = Arc::new(MemBackend::new());
    let pruning = PruningOptions {
        keep_recent: 3,
        keep_every: 10,
        interval: 4,
    };
    let mut store = open(db.clone(), pruning);
    for h in 1..=40 {
        block(&mut store, h);
        store.commit().unwrap();
    }
    let retained: Vec<i64> = (1..=40).filter(|v| pruning.retains(*v, 40)).collect();
    assert_eq!(retained, vec![10, 20, 30, 37, 38, 39, 40]);
    assert_eq!(store.available_versions(), retained);
    for v in 1..=40 {
        let info = store.get_commit_info(v);
        if retained.contains(&v) {
            assert_eq!(
                info.unwrap().commit_id().unwrap(),
                expected[usize::try_from(v - 1).unwrap()]
            );
            for name in STORES {
                let tree = store
                    .get_store(name)
                    .unwrap()
                    .tree()
                    .get_immutable(v)
                    .unwrap();
                assert_eq!(
                    tree.root_hash(),
                    store
                        .get_commit_info(v)
                        .unwrap()
                        .store_info(name)
                        .unwrap()
                        .commit_id
                        .hash
                );
            }
        } else {
            assert!(info.is_err(), "version {v} should be pruned");
        }
    }
    // Reopen sees the same set of versions.
    let reopened = open(db, pruning);
    assert_eq!(reopened.available_versions(), retained);
    assert_eq!(reopened.last_commit_id().unwrap(), expected[39]);
}

#[test]
fn export_restore_round_trip_reproduces_app_hash() {
    let expected = reference(14);
    let src_dir = tempfile::tempdir().unwrap();
    let mut src = open(redb(src_dir.path()), PruningOptions::nothing());
    for h in 1..=12 {
        block(&mut src, h);
        src.commit().unwrap();
    }
    let dst_dir = tempfile::tempdir().unwrap();
    {
        let mut dst = open(redb(dst_dir.path()), PruningOptions::nothing());
        let mut restorer = dst.restore(10).unwrap();
        for (name, exporter) in src.export(10).unwrap() {
            restorer.start_store(&name).unwrap();
            for node in exporter {
                restorer.add_node(node.unwrap()).unwrap();
            }
        }
        assert_eq!(restorer.finish().unwrap(), expected[9]);
    }
    // Restored state survives a restart and keeps producing Go-equal hashes.
    let mut dst = open(redb(dst_dir.path()), PruningOptions::nothing());
    assert_eq!(dst.last_commit_id().unwrap(), expected[9]);
    for h in 11..=14 {
        block(&mut dst, h);
        assert_eq!(
            dst.commit().unwrap(),
            expected[usize::try_from(h - 1).unwrap()]
        );
    }
    // Restoring into a non-empty store is refused; unknown stores rejected.
    assert!(dst.restore(20).is_err());
    let mut fresh = open(Arc::new(MemBackend::new()), PruningOptions::nothing());
    let mut restorer = fresh.restore(10).unwrap();
    assert!(restorer.start_store("nope").is_err());
    // A restore missing a store does not finish.
    restorer.start_store("acc").unwrap();
    assert!(restorer.finish().is_err());
    assert_eq!(fresh.version(), 0);
}

#[test]
fn store_added_after_restart_starts_at_next_version() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut store = open(redb(dir.path()), PruningOptions::nothing());
        block(&mut store, 1);
        store.commit().unwrap();
    }
    let mut store = Store::open(redb(dir.path()), StoreOptions::default());
    for name in STORES.iter().chain(&["newstore"]) {
        store.mount_store(*name, StoreKind::Iavl).unwrap();
    }
    store.load_latest_version().unwrap();
    store
        .get_store_mut("newstore")
        .unwrap()
        .set(b"k".to_vec(), b"v".to_vec())
        .unwrap();
    let id = store.commit().unwrap();
    assert_eq!(id.version, 2);
    let info = store.last_commit_info().unwrap();
    assert_eq!(info.store_info("newstore").unwrap().commit_id.version, 2);
}
