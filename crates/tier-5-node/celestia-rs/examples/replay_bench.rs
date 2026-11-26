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
//!   [--discard-abci-responses true]
//! ```
//!
//! `--base` holds a home after InitChain (created on first use), so later
//! runs copy it and skip the genesis import. Profile with
//! `perf record -g -- target/release/examples/replay_bench ...`.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail, ensure};
use celestia_rs::{node, sync};
use ctypes::{BLOCK_PART_SIZE_BYTES, Block, BlockId};
use nodestore::{BlockExecutor, Handshaker, State};
use prost::Message as _;

/// The node binary's allocator (`celestia-rs` default feature `jemalloc`).
#[cfg(feature = "jemalloc")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

struct Args {
    blocks: i64,
    base: PathBuf,
    cache: PathBuf,
    iavl_cache: usize,
    db_cache_mib: usize,
    pruning_interval: u64,
    discard_abci_responses: bool,
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
            _ => bail!("unknown argument {k}"),
        }
    }
    Ok(a)
}

fn cached_block(cache: &Path, height: i64) -> anyhow::Result<Block> {
    let path = cache.join("blocks").join(format!("{height}.pb"));
    let bytes = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    Ok(Block::from_proto(&ctypes::proto::types::Block::decode(
        bytes.as_slice(),
    )?)?)
}

/// Opens the node at `home` as `celestia-rs start` does (app, stores,
/// evidence pool, handshake: InitChain on a fresh home).
fn open(home: &Path, a: &Args) -> anyhow::Result<(BlockExecutor, State)> {
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
    let app = app::App::open_with(home, opts).map_err(|e| anyhow!("open app: {}", e.log))?;
    let (bs, mut ss) = node::open_stores(home)?;
    ss.discard_abci_responses = a.discard_abci_responses;
    let state = ss.load_from_db_or_genesis(&genesis)?;
    let evpool = sync::open_evidence_pool(home, &ss, &bs, &state)?;
    let mut exec = BlockExecutor::new(
        ss.clone(),
        bs.clone(),
        Box::new(node::AppAdapter(Arc::new(Mutex::new(app)))),
        Arc::new(nodestore::NopMempool),
        evpool.executor_handle(),
    );
    let state = Handshaker::new(ss, state, bs, &genesis, consensus::ticker::system_now())
        .handshake(&mut exec)
        .map_err(|e| anyhow!("handshake: {e}"))?;
    Ok((exec, state))
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

fn main() -> anyhow::Result<()> {
    let a = args()?;
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
        drop(open(&a.base, &a)?);
        println!("init_chain_secs: {:.1}", t.elapsed().as_secs_f64());
    }
    let tmp = tempfile::tempdir_in(a.base.parent().unwrap_or(Path::new(".")))?;
    let home = tmp.path().join("home");
    copy_dir(&a.base, &home)?;
    let (mut exec, mut state) = open(&home, &a)?;
    ensure!(state.last_block_height == 0, "base home is past genesis");

    let mut next = cached_block(&a.cache, 1)?;
    let mut times = Vec::new();
    let mut txs = 0usize;
    let started = Instant::now();
    for h in 1..=a.blocks {
        let mut block = next;
        next = cached_block(&a.cache, h + 1)?;
        txs += block.data.txs.len();
        let t = Instant::now();
        let parts = block.make_part_set(BLOCK_PART_SIZE_BYTES)?;
        let id = BlockId {
            hash: block.hash().unwrap_or_default(),
            part_set_header: parts.header(),
        };
        ensure!(id == next.header.last_block_id, "block id at {h}");
        exec.validate_block(&state, &mut block, consensus::ticker::system_now())
            .map_err(|e| anyhow!("validate {h}: {e}"))?;
        state = exec
            .apply_verified_block(&state, id, &mut block)
            .map_err(|e| anyhow!("execute {h}: {e}"))?;
        times.push(t.elapsed());
        ensure!(
            state.app_hash == next.header.app_hash,
            "app hash after block {h}: ours {}, network {}",
            hex::encode_upper(&state.app_hash),
            hex::encode_upper(&next.header.app_hash)
        );
        if h % 100 == 0 {
            let w = &times[times.len() - 100..];
            let ms = w.iter().sum::<Duration>().as_secs_f64() * 10.0;
            eprintln!("height {h}: {ms:.1} ms/block over the last 100");
        }
    }
    let total = started.elapsed();
    let mut sorted = times.clone();
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
    Ok(())
}
