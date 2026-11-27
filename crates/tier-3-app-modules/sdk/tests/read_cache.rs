//! Differential test of the block-scoped committed-read cache: a
//! multistore with the cache (verify mode on, so a stale hit panics) and
//! one without get the same random interleaving of reads and writes on
//! both stacks, branches, flushes, commits, store mounts and removals,
//! rollbacks, and reads through historical views. Every read must match.

use sdk::store::{Kind, MultiStore, Stack, StoreError};

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

type Read = Result<Option<Vec<u8>>, String>;

fn get(m: &MultiStore, store: &str, key: &[u8]) -> Read {
    m.get(store, key).map_err(|e| e.to_string())
}

fn stores(cached: bool) -> Result<MultiStore, StoreError> {
    let mut m = MultiStore::new();
    m.set_read_cache(cached, true);
    m.mount("a", Kind::Iavl)?;
    m.mount("b", Kind::Iavl)?;
    m.mount("t", Kind::Transient)?;
    m.load_latest_version()?;
    Ok(m)
}

#[test]
fn cached_multistore_reads_what_the_uncached_one_reads() -> Result<(), StoreError> {
    for seed in 1..=30u64 {
        let mut r = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let mut on = stores(true)?;
        let mut off = stores(false)?;
        let mut names: Vec<String> = vec!["a".into(), "b".into(), "t".into()];
        let mut added = 0;
        for step in 0..800 {
            let ctx = format!("seed {seed} step {step}");
            let name = names[r.below(names.len())].clone();
            let key = vec![b'k', u8::try_from(r.below(4)).unwrap_or(0)];
            match r.below(100) {
                0..=49 => assert_eq!(get(&on, &name, &key), get(&off, &name, &key), "{ctx} get"),
                50..=59 => {
                    let v = r.next().to_be_bytes().to_vec();
                    for m in [&mut on, &mut off] {
                        m.set(&name, key.clone(), v.clone())?;
                    }
                }
                60..=63 => {
                    for m in [&mut on, &mut off] {
                        m.delete(&name, &key)?;
                    }
                }
                64..=65 => {
                    let stack = if r.below(2) == 0 {
                        Stack::Finalize
                    } else {
                        Stack::Check
                    };
                    for m in [&mut on, &mut off] {
                        m.select(stack);
                    }
                }
                66..=67 => {
                    for m in [&mut on, &mut off] {
                        m.branch();
                    }
                }
                68 => {
                    if on.depth() > 1 {
                        for m in [&mut on, &mut off] {
                            m.write_branch();
                        }
                    }
                }
                69 => {
                    if on.depth() > 1 {
                        for m in [&mut on, &mut off] {
                            m.discard_branch();
                        }
                    }
                }
                70..=79 => {
                    for m in [&mut on, &mut off] {
                        m.flush()?;
                    }
                }
                80..=84 => {
                    for m in [&mut on, &mut off] {
                        m.flush()?;
                    }
                    assert_eq!(on.commit()?, off.commit()?, "{ctx} commit");
                    for m in [&mut on, &mut off] {
                        m.reset_check();
                    }
                }
                85..=86 => {
                    // Another reader: a historical query view.
                    let v = off.version();
                    if v > 0 {
                        let at = i64::try_from(r.below(usize::try_from(v).unwrap_or(1)))
                            .unwrap_or(0)
                            + 1;
                        let a = on.view_at(at).map(|m| get(&m, &name, &key));
                        let b = off.view_at(at).map(|m| get(&m, &name, &key));
                        assert_eq!(format!("{a:?}"), format!("{b:?}"), "{ctx} view_at {at}");
                    }
                }
                87..=88 => {
                    // A store added by an upgrade.
                    added += 1;
                    let new = format!("added{added}");
                    let at = off.version() + 1;
                    for m in [&mut on, &mut off] {
                        m.mount_iavl_at(&new, at)?;
                    }
                    names.push(new);
                }
                89..=91 => {
                    // Rollback through the root (uncommitted state discarded).
                    let versions = off.root().available_versions();
                    if versions.len() > 1 {
                        let target = versions[r.below(versions.len() - 1)];
                        for m in [&mut on, &mut off] {
                            m.root_mut().rollback_to_version(target)?;
                            m.reset_check();
                        }
                        names.retain(|n| off.is_mounted(n));
                    }
                }
                _ => {
                    // Iteration does not use the cache but must agree.
                    let it =
                        |m: &MultiStore| format!("{:?}", m.range(&name, None, None, step % 2 == 0));
                    assert_eq!(it(&on), it(&off), "{ctx} range");
                }
            }
        }
        let (hits, misses) = on.read_cache_stats();
        assert!(
            hits > 0 && misses > 0,
            "seed {seed}: {hits} hits, {misses} misses"
        );
    }
    Ok(())
}

/// A key created in block N is visible in block N + 1, and an absent key
/// is cached as absent until a flush writes it.
#[test]
fn flush_makes_new_keys_visible() -> Result<(), StoreError> {
    let mut m = stores(true)?;
    assert_eq!(m.get("a", b"new")?, None);
    assert_eq!(m.get("a", b"new")?, None);
    assert_eq!(m.read_cache_stats(), (1, 1), "the absent key is cached");
    m.set("a", b"new".to_vec(), b"1".to_vec())?;
    m.flush()?;
    m.commit()?;
    assert_eq!(m.get("a", b"new")?, Some(b"1".to_vec()));
    Ok(())
}

/// A value read in a block, then written and flushed, reads back as the
/// new value (the flush changes the committed tree and clears the cache).
#[test]
fn read_write_flush_read() -> Result<(), StoreError> {
    let mut m = stores(true)?;
    m.set("a", b"k".to_vec(), b"old".to_vec())?;
    m.flush()?;
    m.commit()?;
    assert_eq!(m.get("a", b"k")?, Some(b"old".to_vec()));
    m.set("a", b"k".to_vec(), b"new".to_vec())?;
    m.flush()?;
    assert_eq!(m.get("a", b"k")?, Some(b"new".to_vec()));
    // A rollback through the root discards the flushed write.
    m.root_mut().rollback_to_version(1)?;
    assert_eq!(m.get("a", b"k")?, Some(b"old".to_vec()));
    Ok(())
}
