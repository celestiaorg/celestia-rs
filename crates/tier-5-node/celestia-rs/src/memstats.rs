//! Memory diagnostics on the Prometheus endpoint (`docs/benchmarks/memory.md`).
//!
//! Always available (sampled at scrape time, no cost otherwise):
//!
//! - `memstats_kvdb_cache_bytes{db="…"}` / `memstats_kvdb_data_bytes{db="…"}`:
//!   RocksDB block cache + memtables and SST + memtable bytes of every node
//!   database (app state, block store, state store, evidence, indexers);
//! - `memstats_process_threads`, `memstats_process_open_fds` (Linux).
//!
//! The binary runs on jemalloc by default (feature `jemalloc`; unprefixed
//! on Linux, so RocksDB's C++ allocations use it too). With
//! `--features memstats` it also exports
//! `memstats_heap_{allocated,active,resident,mapped,retained,metadata}_bytes`
//! from jemalloc's statistics and builds jemalloc's heap profiler:
//! `MALLOC_CONF=prof:true,lg_prof_interval:34,prof_prefix:/dir/jeprof`
//! writes a sampled heap profile after every 16 GiB allocated, readable
//! with `jeprof`.

use std::sync::{Arc, Mutex, Weak};

use kvdb::KvBackend;

use crate::metrics::Metrics;

type DbList = Vec<(String, Weak<dyn KvBackend>)>;

static DBS: Mutex<DbList> = Mutex::new(Vec::new());

type Gauge = Arc<dyn Fn() -> f64 + Send + Sync>;

static GAUGES: Mutex<Vec<(String, Gauge)>> = Mutex::new(Vec::new());

/// Registers a structure-size gauge (`memstats_<name>`, e.g. the IAVL node
/// cache length) exported by [`register_probes`]; a later registration
/// under the same name replaces it, so a node restarted in-process does
/// not keep the old gauge (and what its closure holds) alive.
pub fn register_gauge(name: &str, f: impl Fn() -> f64 + Send + Sync + 'static) {
    let name = format!("memstats_{name}");
    let mut gauges = GAUGES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    gauges.retain(|(n, _)| *n != name);
    gauges.push((name, Arc::new(f)));
}

/// Records an opened node database for the `memstats_kvdb_*` gauges (weakly:
/// a dropped database disappears from the metrics; a database reopened
/// under the same name replaces the old entry).
pub fn register_db(name: &str, db: &Arc<dyn KvBackend>) {
    let mut dbs = DBS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    dbs.retain(|(n, w)| w.strong_count() > 0 && n != name);
    dbs.push((name.to_owned(), Arc::downgrade(db)));
}

fn db_stats() -> Vec<(String, kvdb::StorageStats)> {
    let dbs = DBS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    dbs.iter()
        .filter_map(|(n, w)| Some((n.clone(), w.upgrade()?.stats()?)))
        .collect()
}

/// Registers the memory probes on `m`.
#[allow(clippy::cast_precision_loss)]
pub fn register_probes(m: &Metrics) {
    let names: Vec<String> = DBS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .map(|(n, _)| n.clone())
        .collect();
    for name in names {
        let labels = format!("db=\"{name}\"");
        let n1 = name.clone();
        m.probe_labeled("memstats_kvdb_cache_bytes", &labels, move || {
            db_stats()
                .iter()
                .filter(|(n, _)| *n == n1)
                .map(|(_, s)| s.cache_bytes as f64)
                .sum()
        });
        m.probe_labeled("memstats_kvdb_data_bytes", &labels, move || {
            db_stats()
                .iter()
                .filter(|(n, _)| *n == name)
                .map(|(_, s)| s.allocated_bytes as f64)
                .sum()
        });
    }
    let gauges = GAUGES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    for (name, f) in gauges {
        m.probe(&name, move || f());
    }
    m.probe("memstats_process_threads", || {
        proc_status("Threads:").unwrap_or(0.0)
    });
    m.probe("memstats_process_open_fds", || {
        std::fs::read_dir("/proc/self/fd").map_or(0.0, |d| d.count() as f64)
    });
    #[cfg(feature = "memstats")]
    heap::register(m);
}

fn proc_status(key: &str) -> Option<f64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    s.lines()
        .find_map(|l| l.strip_prefix(key))
        .and_then(|v| v.split_whitespace().next()?.parse().ok())
}

#[cfg(feature = "memstats")]
mod heap {
    use tikv_jemalloc_ctl::{epoch, stats};

    use crate::metrics::Metrics;

    macro_rules! stat {
        ($m:expr, $name:literal, $mib:path) => {
            $m.probe($name, || {
                let _ = epoch::advance();
                #[allow(clippy::cast_precision_loss)]
                $mib().map_or(0.0, |v| v as f64)
            });
        };
    }

    pub(super) fn register(m: &Metrics) {
        stat!(m, "memstats_heap_allocated_bytes", stats::allocated::read);
        stat!(m, "memstats_heap_active_bytes", stats::active::read);
        stat!(m, "memstats_heap_resident_bytes", stats::resident::read);
        stat!(m, "memstats_heap_mapped_bytes", stats::mapped::read);
        stat!(m, "memstats_heap_retained_bytes", stats::retained::read);
        stat!(m, "memstats_heap_metadata_bytes", stats::metadata::read);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db_entries(name: &str) -> Vec<bool> {
        DBS.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, w)| w.strong_count() > 0)
            .collect()
    }

    /// The registries stay bounded across in-process node restarts: one
    /// entry per name, and a dropped database is held only weakly.
    #[test]
    fn registries_are_bounded() {
        let mut keep = None;
        for _ in 0..100 {
            let db: Arc<dyn KvBackend> = Arc::new(kvdb::MemBackend::new());
            register_db("memstats-test", &db);
            register_gauge("test_gauge", || 1.0);
            keep = Some(db);
        }
        assert_eq!(db_entries("memstats-test"), vec![true]);
        let gauges = GAUGES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|(n, _)| n == "memstats_test_gauge")
            .count();
        assert_eq!(gauges, 1);
        drop(keep);
        assert_eq!(db_entries("memstats-test"), vec![false]);
        let other: Arc<dyn KvBackend> = Arc::new(kvdb::MemBackend::new());
        register_db("memstats-test-other", &other);
        assert!(db_entries("memstats-test").is_empty());
    }

    #[test]
    fn probes_render() {
        let db: Arc<dyn KvBackend> = Arc::new(kvdb::MemBackend::new());
        register_db("memstats-render", &db);
        let m = Metrics::new("cometbft", "test");
        register_probes(&m);
        let out = m.render();
        assert!(out.contains("cometbft_memstats_process_threads"));
        assert!(out.contains("cometbft_memstats_process_open_fds"));
    }
}
