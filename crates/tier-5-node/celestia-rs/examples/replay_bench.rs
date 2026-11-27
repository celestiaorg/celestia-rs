//! Replay benchmark: executes cached Celestia mainnet blocks through the
//! node's block executor (the same path block sync uses after its checks)
//! and reports ms per block. Every app hash must match the network's (the
//! next header's `app_hash`), so an optimization that changes state fails
//! the run.
//!
//! The blocks and genesis come from the cache of the gated
//! `blocksync_mainnet_genesis` test (`CELESTIA_MAINNET_CACHE`, default
//! `target/tmp/mainnet-cache`; run that test once with
//! `CELESTIA_BLOCKSYNC_HEIGHT=<n>` to fill it).
//!
//! ```sh
//! cargo run --release -p celestia-rs --example replay_bench -- \
//!   [--blocks 1000] [--base target/tmp/replay-base] [--cache DIR] \
//!   [--iavl-cache 781250] [--db-cache-mib 512] [--pruning-interval 1] \
//!   [--discard-abci-responses true] [--blocks-jsonl a.jsonl.gz,b.jsonl] \
//!   [--background-pruning true] [--sync-writes true] \
//!   [--home DIR [--in-place true]] [--blocks-home DIR] [--save-blocks false]
//! ```
//!
//! `--base` holds a home after InitChain (created on first use), so later
//! runs copy it and skip the genesis import. Profile with
//! `perf record -g -- target/release/examples/replay_bench ...`.
//!
//! Large state: `--home` starts from an existing node home at any height
//! `H` (for example a node stopped with `start --halt-height H+1`) and
//! replays `--blocks` blocks from `H + 1`. The home is copied first (next
//! to it), unless `--in-place true`, which advances it and makes the run
//! unrepeatable. Blocks come from `--blocks-jsonl`, then from the block
//! store of `--blocks-home` (a stopped node home past `H + blocks`), then
//! from the `.pb` cache. `--save-blocks true` also stores each block with
//! its seen commit (the next block's last commit) before executing it, as
//! block sync does; this makes the home usable as a `--blocks-home`.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail, ensure};
use celestia_rs::{node, sync};
use ctypes::proto::abci;
use ctypes::{BLOCK_PART_SIZE_BYTES, Block, BlockId};
use nodestore::{BlockExecutor, Handshaker, State};
use prost::Message as _;

/// The node binary's allocator (`celestia-rs` default feature `jemalloc`).
#[cfg(feature = "jemalloc")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

// Independent command-line switches.
#[allow(clippy::struct_excessive_bools)]
struct Args {
    blocks: i64,
    base: PathBuf,
    cache: PathBuf,
    iavl_cache: usize,
    db_cache_mib: usize,
    pruning_interval: u64,
    discard_abci_responses: bool,
    blocks_jsonl: Vec<PathBuf>,
    background_pruning: bool,
    sync_writes: bool,
    home: Option<PathBuf>,
    in_place: bool,
    blocks_home: Option<PathBuf>,
    save_blocks: bool,
}

fn args() -> anyhow::Result<Args> {
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/tmp");
    let mut a = Args {
        blocks: 1000,
        base: target.join("replay-base"),
        cache: std::env::var_os("CELESTIA_MAINNET_CACHE")
            .map_or_else(|| target.join("mainnet-cache"), PathBuf::from),
        // `celestia-rs start` defaults: app.toml `iavl-cache-size`,
        // `--db-cache-mib`.
        iavl_cache: 781_250,
        db_cache_mib: 512,
        pruning_interval: 1,
        // config.toml `storage.discard_abci_responses` (template: true).
        discard_abci_responses: true,
        blocks_jsonl: Vec::new(),
        // `celestia-rs start` prunes in the background.
        background_pruning: true,
        // `celestia-rs start` fsyncs every write (Go parity).
        sync_writes: true,
        home: None,
        in_place: false,
        blocks_home: None,
        save_blocks: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let v = it.next().with_context(|| format!("{k} needs a value"))?;
        match k.as_str() {
            "--blocks" => a.blocks = v.parse()?,
            "--base" => a.base = v.into(),
            "--cache" => a.cache = v.into(),
            "--iavl-cache" => a.iavl_cache = v.parse()?,
            "--db-cache-mib" => a.db_cache_mib = v.parse()?,
            "--pruning-interval" => a.pruning_interval = v.parse()?,
            "--discard-abci-responses" => a.discard_abci_responses = v.parse()?,
            "--background-pruning" => a.background_pruning = v.parse()?,
            "--sync-writes" => a.sync_writes = v.parse()?,
            "--blocks-jsonl" => a.blocks_jsonl.extend(v.split(',').map(PathBuf::from)),
            "--home" => a.home = Some(v.into()),
            "--in-place" => a.in_place = v.parse()?,
            "--blocks-home" => a.blocks_home = Some(v.into()),
            "--save-blocks" => a.save_blocks = v.parse()?,
            _ => bail!("unknown argument {k}"),
        }
    }
    Ok(a)
}

/// Blocks from `--blocks-jsonl` files (RPC `/block` results, one
/// `{"height", "result"}` object per line, gzipped or not: the format of
/// `conformance/fixtures/mainnet/blocks.jsonl.gz` and of the Go
/// `v1-oracle -blocks` input), then the block store of `--blocks-home`,
/// then the `.pb` cache.
struct Blocks {
    cache: PathBuf,
    json: std::collections::HashMap<i64, Block>,
    store: Option<nodestore::BlockStore>,
}

impl Blocks {
    fn load(cache: PathBuf, files: &[PathBuf], home: Option<&Path>) -> anyhow::Result<Self> {
        use std::io::{BufRead as _, BufReader, Read};
        let mut json = std::collections::HashMap::new();
        for file in files {
            let raw =
                std::fs::File::open(file).with_context(|| format!("open {}", file.display()))?;
            let reader: Box<dyn Read> = if file.extension().is_some_and(|e| e == "gz") {
                Box::new(flate2::read::GzDecoder::new(raw))
            } else {
                Box::new(raw)
            };
            for line in BufReader::new(reader).lines() {
                let row: serde_json::Value = serde_json::from_str(&line?)?;
                let height = row["height"].as_i64().context("row without height")?;
                let block = light::http::block_from_json(&row["result"]["block"])
                    .map_err(|e| anyhow!("block {height} in {}: {e}", file.display()))?;
                json.insert(height, block);
            }
        }
        let store = home
            .map(|h| node::open_stores(h).map(|(bs, _)| bs))
            .transpose()
            .context("open --blocks-home")?;
        Ok(Self { cache, json, store })
    }

    fn get(&self, height: i64) -> anyhow::Result<Block> {
        if let Some(b) = self.json.get(&height) {
            return Ok(b.clone());
        }
        if let Some(store) = &self.store
            && let Some(b) = store
                .load_block(height)
                .map_err(|e| anyhow!("--blocks-home block {height}: {e}"))?
        {
            return Ok(b);
        }
        cached_block(&self.cache, height)
    }
}

fn cached_block(cache: &Path, height: i64) -> anyhow::Result<Block> {
    let path = cache.join("blocks").join(format!("{height}.pb"));
    let bytes = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    Ok(Block::from_proto(&ctypes::proto::types::Block::decode(
        bytes.as_slice(),
    )?)?)
}

/// Time spent per phase of a block: the block ID (part set and hash),
/// `ValidateBlock`, and `ApplyVerifiedBlock`, of which the app's
/// `FinalizeBlock` and `Commit`.
#[derive(Debug, Default, Clone, Copy)]
struct AbciTime {
    block_id: Duration,
    validate: Duration,
    apply: Duration,
    finalize: Duration,
    commit: Duration,
}

/// [`node::AppAdapter`] timing `FinalizeBlock` and `Commit`.
struct Timed {
    inner: node::AppAdapter,
    time: Arc<Mutex<AbciTime>>,
}

impl Timed {
    fn add(&self, t: Instant, f: impl FnOnce(&mut AbciTime) -> &mut Duration) {
        let mut time = self
            .time
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *f(&mut time) += t.elapsed();
    }
}

impl nodestore::AppConn for Timed {
    fn info(&mut self) -> nodestore::Result<abci::ResponseInfo> {
        self.inner.info()
    }
    fn init_chain(
        &mut self,
        req: abci::RequestInitChain,
    ) -> nodestore::Result<abci::ResponseInitChain> {
        self.inner.init_chain(req)
    }
    fn prepare_proposal(
        &mut self,
        req: abci::RequestPrepareProposal,
    ) -> nodestore::Result<abci::ResponsePrepareProposal> {
        self.inner.prepare_proposal(req)
    }
    fn process_proposal(
        &mut self,
        req: abci::RequestProcessProposal,
    ) -> nodestore::Result<abci::ResponseProcessProposal> {
        self.inner.process_proposal(req)
    }
    fn finalize_block(
        &mut self,
        req: abci::RequestFinalizeBlock,
    ) -> nodestore::Result<abci::ResponseFinalizeBlock> {
        let t = Instant::now();
        let r = self.inner.finalize_block(req);
        self.add(t, |a| &mut a.finalize);
        r
    }
    fn commit(&mut self) -> nodestore::Result<abci::ResponseCommit> {
        let t = Instant::now();
        let r = self.inner.commit();
        self.add(t, |a| &mut a.commit);
        r
    }
}

/// Counters read between blocks.
struct Probes {
    abci: Arc<Mutex<AbciTime>>,
    /// IAVL node cache: hits, misses, disk reads, disk bytes, reads made
    /// to resolve a child reference, and of those, records later decoded.
    nodes: Box<dyn Fn() -> [u64; 6]>,
}

impl Probes {
    fn add(&self, f: impl FnOnce(&mut AbciTime)) {
        f(&mut self
            .abci
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner));
    }

    fn sample(&self) -> Sample {
        Sample {
            abci: *self
                .abci
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            nodes: (self.nodes)(),
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Sample {
    abci: AbciTime,
    nodes: [u64; 6],
}

impl Sample {
    /// What happened between `earlier` and `self`, per block over `n`.
    #[allow(clippy::cast_precision_loss)]
    fn report(&self, earlier: &Self, n: usize) -> String {
        let n = n.max(1) as f64;
        let d = |i: usize| self.nodes[i].saturating_sub(earlier.nodes[i]) as f64;
        let ms = |a: Duration, b: Duration| a.saturating_sub(b).as_secs_f64() * 1e3 / n;
        let (now, then) = (&self.abci, &earlier.abci);
        let finalize = ms(now.finalize, then.finalize);
        let commit = ms(now.commit, then.commit);
        let apply = ms(now.apply, then.apply);
        let lookups = d(0) + d(1);
        format!(
            "block id {:.1} ms, validate {:.1} ms, apply {:.1} ms (finalize {:.1}, commit {:.1}, \
             rest {:.1}); node cache hit {:.1}% of {:.0}/block, disk reads {:.0}/block, \
             child-ref reads {:.0}/block, encoded decodes {:.0}/block, {:.0} KiB/block",
            ms(now.block_id, then.block_id),
            ms(now.validate, then.validate),
            apply,
            finalize,
            commit,
            apply - finalize - commit,
            if lookups > 0.0 {
                100.0 * d(0) / lookups
            } else {
                0.0
            },
            lookups / n,
            d(2) / n,
            d(4) / n,
            d(5) / n,
            d(3) / 1024.0 / n,
        )
    }
}

/// Opens the node at `home` as `celestia-rs start` does (app, stores,
/// evidence pool, handshake: InitChain on a fresh home).
fn open(home: &Path, a: &Args) -> anyhow::Result<(BlockExecutor, State, Probes)> {
    let genesis = node::load_genesis(home)?;
    let mut opts = app::AppOptions {
        chain_id: Some(genesis.chain_id.clone()),
        genesis_app_version: Some(genesis.consensus_params.version.app),
        ..app::AppOptions::default()
    };
    opts.node.home = Some(home.to_path_buf());
    opts.store.node_cache_size = a.iavl_cache;
    opts.db.cache_size = a.db_cache_mib << 20;
    opts.store.pruning.interval = a.pruning_interval;
    opts.store.background_pruning = a.background_pruning;
    opts.db.sync_writes = a.sync_writes;
    let app = app::App::open_with(home, opts).map_err(|e| anyhow!("open app: {}", e.log))?;
    let (bs, mut ss) = node::open_stores_with_options(
        home,
        &multistore::kvdb::DbOptions {
            cache_size: 64 << 20,
            sync_writes: a.sync_writes,
            ..multistore::kvdb::DbOptions::default()
        },
    )?;
    ss.discard_abci_responses = a.discard_abci_responses;
    let state = ss.load_from_db_or_genesis(&genesis)?;
    let evpool = sync::open_evidence_pool(home, &ss, &bs, &state)?;
    let cache = Arc::clone(app.multistore().root().node_cache());
    let probes = Probes {
        abci: Arc::default(),
        nodes: Box::new(move || {
            let s = cache.stats();
            [
                s.hits,
                s.misses,
                s.disk_reads,
                s.disk_bytes,
                s.ref_reads,
                s.ref_decodes,
            ]
        }),
    };
    let mut exec = BlockExecutor::new(
        ss.clone(),
        bs.clone(),
        Box::new(Timed {
            inner: node::AppAdapter(Arc::new(Mutex::new(app))),
            time: Arc::clone(&probes.abci),
        }),
        Arc::new(nodestore::NopMempool),
        evpool.executor_handle(),
    );
    let state = Handshaker::new(ss, state, bs, &genesis, consensus::ticker::system_now())
        .handshake(&mut exec)
        .map_err(|e| anyhow!("handshake: {e}"))?;
    Ok((exec, state, probes))
}

fn copy_dir(from: &Path, to: &Path) -> anyhow::Result<()> {
    let status = std::process::Command::new("cp")
        .arg("-a")
        .arg(from)
        .arg(to)
        .status()?;
    ensure!(
        status.success(),
        "cp -a {} {}",
        from.display(),
        to.display()
    );
    Ok(())
}

fn percentile(sorted: &[Duration], p: f64) -> f64 {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted.get(i).map_or(0.0, |d| d.as_secs_f64() * 1e3)
}

/// A copy of the `--base` home after InitChain (created on first use).
fn base_copy(a: &Args) -> anyhow::Result<tempfile::TempDir> {
    if !a.base.join("config/genesis.json").exists() {
        let genesis = a.cache.join("genesis.json");
        celestia_rs::networks::verify_genesis(
            &celestia_rs::networks::MAINNET,
            &std::fs::read(&genesis).with_context(|| format!("read {}", genesis.display()))?,
        )?;
        std::fs::create_dir_all(a.base.join("config"))?;
        std::fs::create_dir_all(a.base.join("data"))?;
        std::fs::copy(&genesis, a.base.join("config/genesis.json"))?;
        let t = Instant::now();
        drop(open(&a.base, a)?);
        println!("init_chain_secs: {:.1}", t.elapsed().as_secs_f64());
    }
    let tmp = tempfile::tempdir_in(a.base.parent().unwrap_or(Path::new(".")))?;
    copy_dir(&a.base, &tmp.path().join("home"))?;
    Ok(tmp)
}

/// The run's totals; `app_hash_mismatches: 0` (a mismatch stops the run).
fn summary(times: &[Duration], txs: usize, total: Duration, digest: &[u8]) {
    let mut sorted = times.to_vec();
    sorted.sort();
    #[allow(clippy::cast_precision_loss)]
    let n = times.len() as f64;
    println!("blocks: {}", times.len());
    println!("txs: {txs}");
    println!(
        "exec_ms_per_block: {:.1}",
        times.iter().sum::<Duration>().as_secs_f64() * 1e3 / n
    );
    println!("p50_ms: {:.1}", percentile(&sorted, 0.5));
    println!("p90_ms: {:.1}", percentile(&sorted, 0.9));
    println!("max_ms: {:.1}", percentile(&sorted, 1.0));
    println!("wall_secs: {:.1}", total.as_secs_f64());
    println!("app_hash_mismatches: 0");
    println!("responses_digest: {}", hex::encode(digest));
}

fn main() -> anyhow::Result<()> {
    let a = args()?;
    // Holds the copied home until the run ends.
    let tmp;
    let home = if let Some(src) = &a.home {
        if a.in_place {
            src.clone()
        } else {
            tmp = tempfile::tempdir_in(src.parent().unwrap_or(Path::new(".")))?;
            let home = tmp.path().join("home");
            let t = Instant::now();
            copy_dir(src, &home)?;
            println!("copy_home_secs: {:.1}", t.elapsed().as_secs_f64());
            home
        }
    } else {
        tmp = base_copy(&a)?;
        tmp.path().join("home")
    };
    let (mut exec, mut state, probes) = open(&home, &a)?;
    let first = state.last_block_height + 1;
    if a.home.is_none() {
        ensure!(first == 1, "base home is past genesis");
    }
    println!("first_height: {first}");

    let blocks = Blocks::load(a.cache.clone(), &a.blocks_jsonl, a.blocks_home.as_deref())?;
    let mut next = blocks.get(first)?;
    let mut times = Vec::new();
    let mut txs = 0usize;
    // Chained SHA-256 of every FinalizeBlock response (tx results with gas
    // and events, block events, updates): equal digests mean identical
    // results. Gas and codes are also checked against the network through
    // each next header's last_results_hash in validate_block.
    let mut digest = [0u8; 32];
    let started = Instant::now();
    let first_sample = probes.sample();
    let mut window = first_sample;
    for h in first..first + a.blocks {
        let mut block = next;
        next = blocks.get(h + 1)?;
        txs += block.data.txs.len();
        let t = Instant::now();
        let parts = block.make_part_set(BLOCK_PART_SIZE_BYTES)?;
        let id = BlockId {
            hash: block.hash().unwrap_or_default(),
            part_set_header: parts.header(),
        };
        ensure!(id == next.header.last_block_id, "block id at {h}");
        let t_validate = Instant::now();
        exec.validate_block(&state, &mut block, consensus::ticker::system_now())
            .map_err(|e| anyhow!("validate {h}: {e}"))?;
        let validated = t_validate.elapsed();
        probes.add(|p| {
            p.block_id += t_validate - t;
            p.validate += validated;
        });
        if a.save_blocks {
            let commit = next
                .last_commit
                .as_ref()
                .context("next block without last commit")?;
            exec.block_store
                .save_block(&mut block, &parts, commit)
                .map_err(|e| anyhow!("save block {h}: {e}"))?;
        }
        let t_apply = Instant::now();
        state = exec
            .apply_verified_block(&state, id, &mut block)
            .map_err(|e| anyhow!("execute {h}: {e}"))?;
        let applied = t_apply.elapsed();
        probes.add(|p| p.apply += applied);
        times.push(t.elapsed());
        let resp = exec
            .state_store
            .load_last_finalize_block_response(h)
            .map_err(|e| anyhow!("response {h}: {e}"))?;
        let mut buf = digest.to_vec();
        buf.extend_from_slice(&resp.encode_to_vec());
        digest = hash::sha256(&buf);
        ensure!(
            state.app_hash == next.header.app_hash,
            "app hash after block {h}: ours {}, network {}",
            hex::encode_upper(&state.app_hash),
            hex::encode_upper(&next.header.app_hash)
        );
        if times.len() % 100 == 0 {
            let w = &times[times.len() - 100..];
            let ms = w.iter().sum::<Duration>().as_secs_f64() * 10.0;
            let now = probes.sample();
            eprintln!(
                "height {h}: {ms:.1} ms/block over the last 100 ({})",
                now.report(&window, 100)
            );
            window = now;
        }
    }
    summary(&times, txs, started.elapsed(), &digest);
    // RocksDB's own counters over the run (the databases were opened for
    // it): WAL syncs, stalls, flush and compaction volume per level.
    for line in multistore::kvdb::rocksdb_report() {
        println!("{line}");
    }
    println!(
        "per_block: {}",
        probes.sample().report(&first_sample, times.len())
    );
    Ok(())
}
