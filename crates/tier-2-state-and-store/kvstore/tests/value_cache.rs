//! Differential test of the latest-value cache: a store with the cache
//! (in verify mode, so a stale hit also panics) and a plain store receive
//! the same random operations and must answer every read identically.

use std::sync::Arc;

use kvstore::{Store, ValueCache};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn key(r: &mut Rng) -> Vec<u8> {
    // A small key space so reads hit written and deleted keys often.
    vec![b'k', u8::try_from(r.below(40)).unwrap_or(0)]
}

type Read = Result<Option<Vec<u8>>, String>;

fn get(s: &Store, k: &[u8]) -> Read {
    s.get(k).map_err(|e| e.to_string())
}

#[test]
fn cached_store_reads_what_the_plain_store_reads() {
    for seed in 1..=40u64 {
        // Capacity below the key space, so eviction runs too.
        let cache = ValueCache::new(16, true);
        let mut plain = Store::new();
        let mut cached = Store::new();
        cached.set_value_cache(Some(Arc::clone(&cache)));
        let mut r = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        for step in 0..600 {
            let ctx = format!("seed {seed} step {step}");
            match r.below(100) {
                0..=29 => {
                    let k = key(&mut r);
                    assert_eq!(get(&cached, &k), get(&plain, &k), "{ctx} get");
                }
                30..=34 => {
                    let k = key(&mut r);
                    assert_eq!(
                        cached.has(&k).map_err(|e| e.to_string()),
                        plain.has(&k).map_err(|e| e.to_string()),
                        "{ctx} has"
                    );
                }
                35..=59 => {
                    let k = key(&mut r);
                    let v = r.next().to_be_bytes().to_vec();
                    assert_eq!(
                        cached.set(k.clone(), v.clone()).map_err(|e| e.to_string()),
                        plain.set(k, v).map_err(|e| e.to_string()),
                        "{ctx} set"
                    );
                }
                60..=74 => {
                    let k = key(&mut r);
                    assert_eq!(
                        cached.delete(&k).map_err(|e| e.to_string()),
                        plain.delete(&k).map_err(|e| e.to_string()),
                        "{ctx} delete"
                    );
                }
                75..=79 => {
                    let it = |s: &Store| {
                        s.lazy_iter(None, None, step % 2 == 0)
                            .map(|it| it.map(|e| e.map(|e| (e.key, e.value))).collect::<Vec<_>>())
                            .map_err(|e| e.to_string())
                            .map(|v| format!("{v:?}"))
                    };
                    assert_eq!(it(&cached), it(&plain), "{ctx} iterate");
                }
                80..=89 => {
                    let a = cached.save_version().map_err(|e| e.to_string());
                    let b = plain.save_version().map_err(|e| e.to_string());
                    assert_eq!(
                        a.map(|c| c.root_hash),
                        b.map(|c| c.root_hash),
                        "{ctx} commit"
                    );
                }
                90..=94 => {
                    // Back to a random committed version (or the empty tree),
                    // discarding uncommitted writes.
                    let versions = plain.tree().available_versions();
                    let target = if versions.is_empty() {
                        None
                    } else {
                        let i = usize::try_from(r.below(versions.len() as u64)).unwrap_or(0);
                        Some(versions[i])
                    };
                    if let Some(v) = target {
                        assert_eq!(
                            cached.tree_mut().load_version(v).map_err(|e| e.to_string()),
                            plain.tree_mut().load_version(v).map_err(|e| e.to_string()),
                            "{ctx} load_version {v}"
                        );
                    }
                }
                95..=97 => {
                    // A clone diverges from its source.
                    let mut copy = cached.clone();
                    let k = key(&mut r);
                    copy.set(k.clone(), b"clone".to_vec()).unwrap_or_default();
                    assert_eq!(get(&cached, &k), get(&plain, &k), "{ctx} clone isolation");
                    cached = copy;
                    plain.set(k, b"clone".to_vec()).unwrap_or_default();
                }
                _ => {
                    // Detach and reattach the cache.
                    cached.set_value_cache(None);
                    let k = key(&mut r);
                    assert_eq!(get(&cached, &k), get(&plain, &k), "{ctx} detached");
                    cached.set_value_cache(Some(Arc::clone(&cache)));
                }
            }
        }
        let (hits, misses) = cache.stats();
        assert!(
            hits > 0 && misses > 0,
            "seed {seed}: {hits} hits, {misses} misses"
        );
    }
}
