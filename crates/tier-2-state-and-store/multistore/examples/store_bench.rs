//! Synthetic load on a redb-backed multistore: `blocks` commits of `writes`
//! random 32-byte keys with 100-byte values each, pruning to the latest 100
//! versions (or `keep_recent`). Prints throughput, peak RSS, cache stats and on-disk size.
//!
//! ```text
//! cargo run -p multistore --example store_bench -- [blocks] [writes] [cache_nodes] [keep_recent]
//! ```

#![allow(clippy::print_stdout, clippy::cast_precision_loss)]

use std::sync::Arc;
use std::time::Instant;

use kvdb::{KvBackend, RedbBackend, RedbOptions};
use multistore::{PruningOptions, Store, StoreKind, StoreOptions};

fn peak_rss_mib() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
        })
        .unwrap_or(0.0)
        / 1024.0
}

fn main() -> anyhow::Result<()> {
    let args: Vec<u64> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let blocks = args.first().copied().unwrap_or(200);
    let writes = args.get(1).copied().unwrap_or(5_000);
    let cache = usize::try_from(args.get(2).copied().unwrap_or(100_000))?;
    let pruning = PruningOptions {
        keep_recent: args.get(3).copied().unwrap_or(100),
        keep_every: 0,
        interval: 10,
    };
    let dir = std::env::temp_dir().join(format!("store-bench-{}", std::process::id()));
    let db: Arc<dyn KvBackend> = Arc::new(RedbBackend::open(
        dir.join("state.redb"),
        RedbOptions::default(),
    )?);
    let mut store = Store::open(
        Arc::clone(&db),
        StoreOptions {
            pruning,
            node_cache_size: cache,
        },
    );
    store.mount_store("bank", StoreKind::Iavl)?;
    store.load_latest_version()?;
    let mut x = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let start = Instant::now();
    let mut commit_ms = 0u128;
    for _ in 0..blocks {
        let s = store.get_store_mut("bank")?;
        for _ in 0..writes {
            let k = next();
            let mut key = [0u8; 32];
            key[..8].copy_from_slice(&k.to_be_bytes());
            key[8..16].copy_from_slice(&next().to_be_bytes());
            s.set(key.to_vec(), vec![u8::try_from(k % 251)?; 100])?;
        }
        let t = Instant::now();
        store.commit()?;
        commit_ms += t.elapsed().as_millis();
    }
    let total = start.elapsed();
    let entries = store.get_store("bank")?.len();
    let disk = db.disk_size().unwrap_or(0);
    let (mut live_keys, mut live_bytes) = (0u64, 0u64);
    for item in db.range(None, None, false)? {
        let (k, v) = item?;
        live_keys += 1;
        live_bytes += (k.len() + v.len()) as u64;
    }
    let stats = store.node_cache().stats();
    // Cold point reads through a fresh cache.
    drop(store);
    let mut reopened = Store::open(
        db,
        StoreOptions {
            pruning,
            node_cache_size: cache,
        },
    );
    reopened.mount_store("bank", StoreKind::Iavl)?;
    let t = Instant::now();
    reopened.load_latest_version()?;
    let load_ms = t.elapsed().as_millis();
    let t = Instant::now();
    let tree = reopened.get_store("bank")?;
    let mut found = 0;
    for _ in 0..10_000 {
        let mut key = [0u8; 32];
        key[..8].copy_from_slice(&next().to_be_bytes());
        found += usize::from(tree.get(&key)?.is_some());
    }
    let read_us = t.elapsed().as_micros() / 10_000;
    println!(
        "blocks={blocks} writes/block={writes} entries={entries} total_s={:.1} avg_commit_ms={} \
         peak_rss_mib={:.0} cache_nodes={}/{} hits={} misses={} disk_mib={:.1} live_keys={live_keys} live_mib={:.1} live_bytes/entry={:.0} \
         reopen_ms={load_ms} cold_get_us={read_us} ({found} hits)",
        total.as_secs_f64(),
        commit_ms / u128::from(blocks.max(1)),
        peak_rss_mib(),
        stats.len,
        stats.capacity,
        stats.hits,
        stats.misses,
        disk as f64 / 1_048_576.0,
        live_bytes as f64 / 1_048_576.0,
        live_bytes as f64 / entries.max(1) as f64,
    );
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}
