//! Long-running, mainnet-like storage benchmark (ADR 0002 backend choice).
//!
//! Simulates a validator's app-state write mix over many blocks on a
//! persistent backend with keep-recent pruning, and prints a time series of
//! on-disk size vs. live data, write amplification, commit latency and RSS.
//!
//! Workload per block (`txs` transactions):
//! - every tx: sender account (auth, ~110 B) and sender + recipient balances
//!   (bank, ~20 B); 1% of senders are hot (half of all txs), 10% of
//!   recipients are new accounts;
//! - 3% of txs delegate (staking delegation + validator, distribution starting
//!   info, historical rewards period churn), 1% unbond (unbonding entry +
//!   queue entry, removed 1,000 blocks later);
//! - BeginBlock: distribution rewards/commission of 100 validators, fee pool,
//!   minter, fee collector;
//! - IBC: 2 light-client updates (consensus state ~600 B + metadata), states
//!   older than 2,000 blocks expire;
//! - every 200 blocks a 32 KB entry (governance proposal / upgrade plan) that
//!   is deleted 500 blocks later.
//!
//! ```text
//! cargo run --release -p multistore [--features rocksdb] --example store_bench_mainnet -- \
//!     backend=redb blocks=3000 txs=600 accounts=200000 keep_recent=100 sample=100
//! ```
//!
//! Other keys: `interval` (pruning interval, 10), `cache_nodes` (IAVL node
//! cache, 500000), `db_cache_mb` (backend cache, 256), `sync` (fsync every
//! commit, 1), `compact_every` (redb: reopen with `compact()` every N blocks,
//! 0 = never), `dir` (database directory; deleted at the end).

#![allow(
    clippy::print_stdout,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::too_many_lines,
    clippy::many_single_char_names,
    clippy::cast_sign_loss
)]

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use kvdb::{KvBackend, RedbBackend, RedbOptions, Result as KvResult, StorageStats, WriteBatch};
use multistore::{PruningOptions, Store, StoreKind, StoreOptions};

const VALIDATORS: u64 = 100;
const STORES: [&str; 6] = ["acc", "bank", "staking", "distribution", "ibc", "gov"];

/// Counts logical bytes handed to the backend.
#[derive(Debug)]
struct Counting {
    inner: Arc<dyn KvBackend>,
    written: std::sync::atomic::AtomicU64,
}

impl KvBackend for Counting {
    fn get(&self, key: &[u8]) -> KvResult<Option<Vec<u8>>> {
        self.inner.get(key)
    }
    fn range(&self, s: Option<&[u8]>, e: Option<&[u8]>, r: bool) -> KvResult<kvdb::KvIter> {
        self.inner.range(s, e, r)
    }
    fn write(&self, batch: WriteBatch) -> KvResult<()> {
        self.written.fetch_add(
            batch.byte_size() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.inner.write(batch)
    }
    fn sync(&self) -> KvResult<()> {
        self.inner.sync()
    }
    fn disk_size(&self) -> Option<u64> {
        self.inner.disk_size()
    }
    fn stats(&self) -> Option<StorageStats> {
        self.inner.stats()
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
    fn bytes(&mut self, len: usize) -> Vec<u8> {
        let mut v = Vec::with_capacity(len + 8);
        while v.len() < len {
            v.extend_from_slice(&self.next().to_le_bytes());
        }
        v.truncate(len);
        v
    }
}

fn addr(tag: u8, i: u64) -> Vec<u8> {
    let mut a = vec![tag; 20];
    a[..8].copy_from_slice(&hash64(i).to_be_bytes());
    a[8..16].copy_from_slice(&hash64(i ^ 0x5555).to_be_bytes());
    a
}

fn hash64(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn key(prefix: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    let mut k = prefix.to_vec();
    for p in parts {
        k.extend_from_slice(p);
    }
    k
}

fn io_write_bytes() -> u64 {
    std::fs::read_to_string("/proc/self/io")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("write_bytes:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

fn rss_mib(field: &str) -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with(field))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
        })
        .unwrap_or(0.0)
        / 1024.0
}

fn dir_blocks_bytes(path: &Path) -> u64 {
    // Allocated (not apparent) size: redb grows its file sparsely.
    std::process::Command::new("du")
        .arg("-sk")
        .arg(path)
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()
        })
        .unwrap_or(0)
        * 1024
}

const MIB: f64 = 1_048_576.0;

struct Config {
    backend: String,
    blocks: u64,
    txs: u64,
    accounts: u64,
    pruning: PruningOptions,
    cache_nodes: usize,
    db_cache: usize,
    sync: bool,
    sample: u64,
    compact_every: u64,
    dir: PathBuf,
}

fn open_backend(cfg: &Config, compact: bool) -> anyhow::Result<Arc<dyn KvBackend>> {
    Ok(match cfg.backend.as_str() {
        "redb" => Arc::new(RedbBackend::open(
            cfg.dir.join("state.redb"),
            RedbOptions {
                cache_size: cfg.db_cache,
                sync_writes: cfg.sync,
                compact_on_open: compact,
            },
        )?),
        "mem" => Arc::new(kvdb::MemBackend::new()),
        #[cfg(feature = "rocksdb")]
        "rocksdb" => {
            let db = kvdb::RocksBackend::open(
                cfg.dir.join("state.rocksdb"),
                kvdb::RocksOptions {
                    block_cache_size: cfg.db_cache,
                    sync_writes: cfg.sync,
                    ..kvdb::RocksOptions::default()
                },
            )?;
            if compact {
                db.compact()?;
            }
            Arc::new(db)
        }
        other => anyhow::bail!("unknown backend {other} (rocksdb needs --features rocksdb)"),
    })
}

fn open_store(cfg: &Config, db: Arc<dyn KvBackend>) -> anyhow::Result<Store> {
    let mut store = Store::open(
        db,
        StoreOptions {
            pruning: cfg.pruning,
            node_cache_size: cfg.cache_nodes,
        },
    );
    for name in STORES {
        store.mount_store(name, StoreKind::Iavl)?;
    }
    store.load_latest_version()?;
    Ok(store)
}

fn percentile(sorted: &[Duration], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i].as_secs_f64() * 1e3
}

fn main() -> anyhow::Result<()> {
    let mut kv = BTreeMap::new();
    for arg in std::env::args().skip(1) {
        if let Some((k, v)) = arg.split_once('=') {
            kv.insert(k.to_string(), v.to_string());
        }
    }
    let num = |k: &str, d: u64| -> u64 { kv.get(k).and_then(|v| v.parse().ok()).unwrap_or(d) };
    let cfg = Config {
        backend: kv.get("backend").cloned().unwrap_or_else(|| "redb".into()),
        blocks: num("blocks", 3000),
        txs: num("txs", 600),
        accounts: num("accounts", 200_000),
        pruning: PruningOptions {
            keep_recent: num("keep_recent", 100),
            keep_every: 0,
            interval: num("interval", 10),
        },
        cache_nodes: usize::try_from(num("cache_nodes", 500_000))?,
        db_cache: usize::try_from(num("db_cache_mb", 256))? << 20,
        sync: num("sync", 1) == 1,
        sample: num("sample", 100).max(1),
        compact_every: num("compact_every", 0),
        dir: kv.get("dir").map_or_else(
            || std::env::temp_dir().join(format!("store-bench-{}", std::process::id())),
            PathBuf::from,
        ),
    };
    let _ = std::fs::remove_dir_all(&cfg.dir);
    let counting = Arc::new(Counting {
        inner: open_backend(&cfg, false)?,
        written: 0.into(),
    });
    let mut store = open_store(&cfg, counting.clone())?;
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let io0 = io_write_bytes();

    // Genesis.
    let t = Instant::now();
    let mut accounts = cfg.accounts;
    {
        for i in 0..accounts {
            let a = addr(1, i);
            store
                .get_store_mut("acc")?
                .set(key(&[1], &[&a]), rng.bytes(110))?;
            store
                .get_store_mut("bank")?
                .set(key(&[2, 20], &[&a, b"utia"]), rng.bytes(20))?;
        }
        for v in 0..VALIDATORS {
            let val = addr(2, v);
            store
                .get_store_mut("staking")?
                .set(key(&[0x21], &[&val]), rng.bytes(400))?;
            let d = store.get_store_mut("distribution")?;
            for tag in [2u8, 6, 7, 8] {
                d.set(key(&[tag], &[&val]), rng.bytes(150))?;
            }
        }
        for i in 0..accounts / 10 {
            let del = addr(1, i);
            let val = addr(2, i % VALIDATORS);
            store
                .get_store_mut("staking")?
                .set(key(&[0x31], &[&del, &val]), rng.bytes(80))?;
            store
                .get_store_mut("distribution")?
                .set(key(&[4], &[&val, &del]), rng.bytes(50))?;
        }
    }
    store.commit()?;
    println!(
        "# backend={} txs/block={} genesis_accounts={} keep_recent={} interval={} sync={} genesis_s={:.1}",
        cfg.backend,
        cfg.txs,
        cfg.accounts,
        cfg.pruning.keep_recent,
        cfg.pruning.interval,
        cfg.sync,
        t.elapsed().as_secs_f64()
    );
    println!(
        "block\tfile_mib\tdu_mib\talloc_mib\tlive_mib\tlive_keys\tfile/live\tlogical_w_mib\tdisk_w_mib\twa\tcommit_p50_ms\tcommit_p99_ms\tcommit_max_ms\trss_mib\tcache_mib\twall_s"
    );

    let mut unbond_queue: VecDeque<(u64, Vec<u8>, Vec<u8>)> = VecDeque::new();
    let mut ibc_queue: VecDeque<(u64, Vec<u8>)> = VecDeque::new();
    let mut gov_queue: VecDeque<(u64, Vec<u8>)> = VecDeque::new();
    let mut periods = vec![1u64; usize::try_from(VALIDATORS)?];
    let mut commits: Vec<Duration> = Vec::new();
    let mut all_commits: Vec<Duration> = Vec::new();
    let hot = (cfg.accounts / 100).max(1);
    let start = Instant::now();
    let mut compact_total = Duration::ZERO;
    let mut store_opt = Some(store);
    let mut counting = counting;

    for height in 2..=cfg.blocks + 1 {
        let store = store_opt.as_mut().ok_or_else(|| anyhow::anyhow!("store"))?;
        for _ in 0..cfg.txs {
            let sender = if rng.chance(50) {
                rng.below(hot)
            } else {
                rng.below(accounts)
            };
            let recipient = if rng.chance(10) {
                accounts += 1;
                let a = addr(1, accounts - 1);
                store
                    .get_store_mut("acc")?
                    .set(key(&[1], &[&a]), rng.bytes(110))?;
                accounts - 1
            } else {
                rng.below(accounts)
            };
            let s = addr(1, sender);
            store
                .get_store_mut("acc")?
                .set(key(&[1], &[&s]), rng.bytes(110))?;
            let bank = store.get_store_mut("bank")?;
            bank.set(key(&[2, 20], &[&s, b"utia"]), rng.bytes(20))?;
            bank.set(
                key(&[2, 20], &[&addr(1, recipient), b"utia"]),
                rng.bytes(20),
            )?;
            if rng.chance(3) {
                let v = rng.below(VALIDATORS);
                let val = addr(2, v);
                store
                    .get_store_mut("staking")?
                    .set(key(&[0x31], &[&s, &val]), rng.bytes(80))?;
                store
                    .get_store_mut("staking")?
                    .set(key(&[0x21], &[&val]), rng.bytes(400))?;
                let d = store.get_store_mut("distribution")?;
                d.set(key(&[4], &[&val, &s]), rng.bytes(50))?;
                let p = &mut periods[usize::try_from(v)?];
                d.delete(&key(&[5], &[&val, &p.to_be_bytes()]))?;
                *p += 1;
                d.set(key(&[5], &[&val, &p.to_be_bytes()]), rng.bytes(120))?;
                d.set(key(&[6], &[&val]), rng.bytes(120))?;
            } else if rng.chance(1) {
                let val = addr(2, rng.below(VALIDATORS));
                let ubd = key(&[0x32], &[&s, &val]);
                let queue = key(&[0x41], &[&height.to_be_bytes(), &s]);
                let st = store.get_store_mut("staking")?;
                st.set(ubd.clone(), rng.bytes(150))?;
                st.set(queue.clone(), rng.bytes(60))?;
                unbond_queue.push_back((height + 1000, ubd, queue));
            }
        }
        // BeginBlock / EndBlock.
        {
            let d = store.get_store_mut("distribution")?;
            for v in 0..VALIDATORS {
                let val = addr(2, v);
                for tag in [2u8, 6, 7] {
                    d.set(key(&[tag], &[&val]), rng.bytes(150))?;
                }
            }
            d.set(b"\x00feepool".to_vec(), rng.bytes(100))?;
            store
                .get_store_mut("bank")?
                .set(key(&[2, 20], &[&[0xfe; 20], b"utia"]), rng.bytes(20))?;
            store
                .get_store_mut("acc")?
                .set(b"\x00minter".to_vec(), rng.bytes(50))?;
            while unbond_queue.front().is_some_and(|(h, _, _)| *h <= height) {
                if let Some((_, ubd, queue)) = unbond_queue.pop_front() {
                    let st = store.get_store_mut("staking")?;
                    st.delete(&ubd)?;
                    st.delete(&queue)?;
                }
            }
            let ibc = store.get_store_mut("ibc")?;
            for client in 0..2u64 {
                let cs = format!("clients/07-tendermint-{client}/consensusStates/1-{height}");
                ibc.set(cs.clone().into_bytes(), rng.bytes(600))?;
                ibc.set(
                    format!("clients/07-tendermint-{client}/clientState").into_bytes(),
                    rng.bytes(300),
                )?;
                let meta = format!("clients/07-tendermint-{client}/processedTime/1-{height}");
                ibc.set(meta.clone().into_bytes(), rng.bytes(8))?;
                ibc_queue.push_back((height + 2000, cs.into_bytes()));
                ibc_queue.push_back((height + 2000, meta.into_bytes()));
            }
            while ibc_queue.front().is_some_and(|(h, _)| *h <= height) {
                if let Some((_, k)) = ibc_queue.pop_front() {
                    ibc.delete(&k)?;
                }
            }
            let gov = store.get_store_mut("gov")?;
            if height % 200 == 0 {
                let k = key(b"\x00proposal", &[&height.to_be_bytes()]);
                gov.set(k.clone(), rng.bytes(32 << 10))?;
                gov_queue.push_back((height + 500, k));
            }
            while gov_queue.front().is_some_and(|(h, _)| *h <= height) {
                if let Some((_, k)) = gov_queue.pop_front() {
                    gov.delete(&k)?;
                }
            }
        }
        let t = Instant::now();
        store.commit()?;
        let elapsed = t.elapsed();
        commits.push(elapsed);
        all_commits.push(elapsed);

        if cfg.compact_every > 0 && height % cfg.compact_every == 0 {
            let t = Instant::now();
            drop(store_opt.take());
            let written = counting.written.load(std::sync::atomic::Ordering::Relaxed);
            drop(counting);
            counting = Arc::new(Counting {
                inner: open_backend(&cfg, true)?,
                written: written.into(),
            });
            store_opt = Some(open_store(&cfg, counting.clone())?);
            let took = t.elapsed();
            compact_total += took;
            println!("# compacted at {height} in {:.2}s", took.as_secs_f64());
        }

        if (height - 1) % cfg.sample == 0 || height == cfg.blocks + 1 {
            let (mut live_keys, mut live_bytes) = (0u64, 0u64);
            for item in counting.range(None, None, false)? {
                let (k, v) = item?;
                live_keys += 1;
                live_bytes += (k.len() + v.len()) as u64;
            }
            let stats = counting.stats().unwrap_or_default();
            let logical = counting.written.load(std::sync::atomic::Ordering::Relaxed);
            let disk_w = io_write_bytes() - io0;
            commits.sort_unstable();
            println!(
                "{}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{}\t{:.2}\t{:.0}\t{:.0}\t{:.2}\t{:.1}\t{:.1}\t{:.1}\t{:.0}\t{:.0}\t{:.0}",
                height,
                stats.file_bytes as f64 / MIB,
                dir_blocks_bytes(&cfg.dir) as f64 / MIB,
                stats.allocated_bytes as f64 / MIB,
                live_bytes as f64 / MIB,
                live_keys,
                stats.file_bytes as f64 / live_bytes.max(1) as f64,
                logical as f64 / MIB,
                disk_w as f64 / MIB,
                disk_w as f64 / logical.max(1) as f64,
                percentile(&commits, 0.5),
                percentile(&commits, 0.99),
                percentile(&commits, 1.0),
                rss_mib("VmRSS:"),
                stats.cache_bytes as f64 / MIB,
                start.elapsed().as_secs_f64(),
            );
            commits.clear();
        }
    }
    all_commits.sort_unstable();
    println!(
        "# done: blocks={} accounts={} commit_p50_ms={:.1} p99_ms={:.1} max_ms={:.1} peak_rss_mib={:.0} compaction_s={:.1} total_s={:.0}",
        cfg.blocks,
        accounts,
        percentile(&all_commits, 0.5),
        percentile(&all_commits, 0.99),
        percentile(&all_commits, 1.0),
        rss_mib("VmHWM:"),
        compact_total.as_secs_f64(),
        start.elapsed().as_secs_f64()
    );
    drop(store_opt);
    drop(counting);
    let _ = std::fs::remove_dir_all(&cfg.dir);
    Ok(())
}
