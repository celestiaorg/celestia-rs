//! Differential test of the latest-value cache at the multi-store level: a
//! store with the cache (verify mode on, so a stale hit panics) and one
//! without get the same random interleaving of writes, reads, iteration,
//! commits (with pruning), rollbacks, reopens, store additions and
//! removals, and snapshot restores. Every read and commit ID must match.

use std::sync::Arc;

use multistore::kvdb::{KvBackend, MemBackend};
use multistore::{PruningOptions, Store, StoreKind, StoreOptions};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).unwrap_or(0)
    }
}

fn options(value_cache_size: usize) -> StoreOptions {
    StoreOptions {
        pruning: PruningOptions {
            keep_recent: 2,
            keep_every: 0,
            interval: 3,
        },
        node_cache_size: 64,
        value_cache_size,
        value_cache_verify: true,
    }
}

struct Side {
    db: Arc<dyn KvBackend>,
    store: Store,
    cache: usize,
}

impl Side {
    fn open(db: Arc<dyn KvBackend>, cache: usize, stores: &[String]) -> Self {
        let mut store = Store::open(Arc::clone(&db), options(cache));
        for name in stores {
            store.mount_store(name.clone(), StoreKind::Iavl).unwrap();
        }
        store.load_latest_version().unwrap();
        Self { db, store, cache }
    }
    fn reopen(&mut self, stores: &[String]) {
        *self = Self::open(Arc::clone(&self.db), self.cache, stores);
    }
}

type Read = Result<Option<Vec<u8>>, String>;

fn get(s: &Store, store: &str, key: &[u8]) -> Read {
    s.get_store(store)
        .map_err(|e| e.to_string())
        .and_then(|st| st.get(key).map_err(|e| e.to_string()))
}

fn iterate(s: &Store, store: &str, reverse: bool) -> String {
    let r = s
        .get_store(store)
        .map_err(|e| e.to_string())
        .and_then(|st| {
            st.lazy_iter(None, None, reverse)
                .map_err(|e| e.to_string())
                .map(|it| it.map(|e| e.map(|e| (e.key, e.value))).collect::<Vec<_>>())
        });
    format!("{r:?}")
}

#[test]
fn cached_multistore_reads_what_the_plain_multistore_reads() {
    for seed in 1..=12u64 {
        let mut r = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let mut stores: Vec<String> = vec!["acc".into(), "bank".into()];
        let mut added = 0;
        // No rollback to before a store removal (a store upgrade's deleted
        // stores are never mounted again).
        let mut min_rollback = 0;
        // Hits and misses of every cache the run creates.
        let mut totals = (0, 0);
        let mut count = |s: &Store| {
            let (h, m) = s.value_cache().map_or((0, 0), |c| c.stats());
            totals = (totals.0 + h, totals.1 + m);
        };
        let mut plain = Side::open(Arc::new(MemBackend::new()), 0, &stores);
        let mut cached = Side::open(Arc::new(MemBackend::new()), 24, &stores);
        for step in 0..500 {
            let ctx = format!("seed {seed} step {step}");
            let name = stores[r.below(stores.len())].clone();
            let key = vec![b'k', u8::try_from(r.below(8)).unwrap_or(0)];
            match r.below(100) {
                0..=29 => assert_eq!(
                    get(&cached.store, &name, &key),
                    get(&plain.store, &name, &key),
                    "{ctx} get {name}"
                ),
                30..=54 => {
                    let v = r.next().to_be_bytes().to_vec();
                    for s in [&mut plain, &mut cached] {
                        s.store
                            .get_store_mut(&name)
                            .unwrap()
                            .set(key.clone(), v.clone())
                            .unwrap();
                    }
                }
                55..=64 => {
                    for s in [&mut plain, &mut cached] {
                        s.store.get_store_mut(&name).unwrap().delete(&key).unwrap();
                    }
                }
                65..=67 => {
                    let rev = step % 2 == 0;
                    assert_eq!(
                        iterate(&cached.store, &name, rev),
                        iterate(&plain.store, &name, rev),
                        "{ctx} iterate {name}"
                    );
                }
                68..=81 => assert_eq!(
                    cached.store.commit().map_err(|e| e.to_string()),
                    plain.store.commit().map_err(|e| e.to_string()),
                    "{ctx} commit"
                ),
                82..=86 => {
                    let versions = plain.store.available_versions();
                    let versions: Vec<_> =
                        versions.into_iter().filter(|v| *v > min_rollback).collect();
                    if versions.len() > 1 {
                        let target = versions[r.below(versions.len() - 1)];
                        assert_eq!(
                            cached
                                .store
                                .rollback_to_version(target)
                                .map_err(|e| e.to_string()),
                            plain
                                .store
                                .rollback_to_version(target)
                                .map_err(|e| e.to_string()),
                            "{ctx} rollback to {target}"
                        );
                        // Stores added after the target are gone from it.
                        stores.retain(|s| plain.store.get_store(s).is_ok());
                    }
                }
                87..=90 => {
                    count(&cached.store);
                    plain.reopen(&stores);
                    cached.reopen(&stores);
                }
                91..=93 => {
                    added += 1;
                    let new = format!("added{added}");
                    let at = plain.store.version() + 1;
                    for s in [&mut plain, &mut cached] {
                        s.store
                            .mount_store_at(new.clone(), StoreKind::Iavl, at)
                            .unwrap();
                    }
                    stores.push(new);
                }
                94..=95 => {
                    if let Some(i) = stores.iter().position(|s| s.starts_with("added")) {
                        let gone = stores.remove(i);
                        for s in [&mut plain, &mut cached] {
                            s.store.remove_store(&gone).unwrap();
                        }
                        // The upgrade block commits without it.
                        assert_eq!(
                            cached.store.commit().map_err(|e| e.to_string()),
                            plain.store.commit().map_err(|e| e.to_string()),
                            "{ctx} commit after removing {gone}"
                        );
                        min_rollback = plain.store.version();
                    }
                }
                _ => {
                    // Snapshot restore of the latest version into fresh
                    // stores; the cached side reads before restoring.
                    let version = plain.store.version();
                    // (Not while a store added since holds no version yet.)
                    if version == 0 || plain.store.export(version).is_err() {
                        continue;
                    }
                    let mut fresh = Vec::new();
                    for cache in [0, 24] {
                        let mut side = Side::open(Arc::new(MemBackend::new()), cache, &stores);
                        for n in &stores {
                            let _ = get(&side.store, n, &key);
                        }
                        let mut restorer = side.store.restore(version).unwrap();
                        for (n, exporter) in plain.store.export(version).unwrap() {
                            restorer.start_store(&n).unwrap();
                            for node in exporter {
                                restorer.add_node(node.unwrap()).unwrap();
                            }
                        }
                        restorer.finish().unwrap();
                        fresh.push(side);
                    }
                    count(&cached.store);
                    cached = fresh.pop().unwrap();
                    plain = fresh.pop().unwrap();
                    assert_eq!(
                        cached.store.last_commit_id().unwrap(),
                        plain.store.last_commit_id().unwrap(),
                        "{ctx} restore"
                    );
                }
            }
        }
        count(&cached.store);
        let (hits, misses) = totals;
        assert!(
            hits > 0 && misses > 0,
            "seed {seed}: {hits} hits, {misses} misses"
        );
    }
}

/// Commits and pruning keep the cache warm; a rollback or reopen starts
/// it over.
#[test]
fn commits_and_pruning_keep_cached_values() {
    let stores = vec!["acc".to_owned()];
    let mut side = Side::open(Arc::new(MemBackend::new()), 16, &stores);
    let stats = |s: &Side| s.store.value_cache().unwrap().stats();
    let k = b"k".to_vec();
    side.store
        .get_store_mut("acc")
        .unwrap()
        .set(k.clone(), b"v".to_vec())
        .unwrap();
    assert_eq!(get(&side.store, "acc", &k), Ok(Some(b"v".to_vec())));
    assert_eq!(stats(&side), (1, 0), "a write is cached");
    for _ in 0..7 {
        side.store.commit().unwrap();
    }
    assert!(side.store.available_versions().len() < 7, "pruning ran");
    assert_eq!(get(&side.store, "acc", &k), Ok(Some(b"v".to_vec())));
    assert_eq!(stats(&side), (2, 0), "commits and pruning keep it");
    side.store.rollback_to_version(6).unwrap();
    assert_eq!(get(&side.store, "acc", &k), Ok(Some(b"v".to_vec())));
    assert_eq!(stats(&side), (2, 1), "a rollback starts over");
}
