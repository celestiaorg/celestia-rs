//! `celestia-rs follow`: follows a public network over celestia-core
//! JSON-RPC (HTTP or HTTPS) instead of P2P and checks, block by block, that
//! the Rust state machine computes exactly what the network committed.
//!
//! Per height `h` (the node's block-sync path, `blocksync` `try_sync_one`):
//!
//! 1. fetch `/block?height=…` from the RPC servers, many heights ahead
//!    in parallel; block `h+1` carries the canonical commit of `h`;
//! 2. compare header `h` with the state our own execution produced:
//!    `app_hash` and `last_results_hash` of block `h-1`, the validator
//!    sets and the consensus params;
//! 3. light-client verify header `h` against the previous, already
//!    trusted header (`light::verify_adjacent`) and check that the block's
//!    id (header hash and part set, so every tx) is the one 2/3+ of our
//!    validator set signed (`ctypes::verify_commit`); then `ValidateBlock`
//!    and the app's `ProcessProposal` (the network accepted the block, so
//!    must we);
//! 4. execute it (`ApplyVerifiedBlock`: FinalizeBlock with the txs,
//!    misbehavior and DecidedLastCommit, Commit) through the same
//!    `BlockExecutor` as `celestia-rs start`.
//!
//! The first divergence stops the follower and writes a diagnostic bundle
//! (`<home>/follow-diagnostics/<height>/`): the mismatching fields, the
//! block, our FinalizeBlock results next to the network's
//! `/block_results`, a per-tx comparison and, when the network still has
//! the state, each store's root hash from the network vs ours.
//!
//! The first start bootstraps from a Go SDK state-sync snapshot on disk
//! (`<dir>/<height>/<format>/<chunk>`, as Go writes them to
//! `data/snapshots`), verified like state sync: the light client
//! establishes the trusted header at the snapshot height and the restored
//! app hash must equal header `h+1`'s. Later starts resume from the
//! node's stores (handshake replay covers a crash between the app's
//! Commit and the state save).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use ctypes::proto::abci;
use ctypes::{BLOCK_PART_SIZE_BYTES, Block, BlockId, Commit, Header, SignedHeader};
use light::RpcClient;
use nodestore::{BlockExecutor, BlockStore, GenesisDoc, Handshaker, State, StateStore};
use statesync::{Bootstrapper as _, StateProvider as _};

/// Request timeout per RPC call.
const RPC_TIMEOUT: Duration = Duration::from_secs(30);
/// Light client clock drift tolerance (Go's default, 10 s).
const MAX_CLOCK_DRIFT: i64 = 10 * 1_000_000_000;

/// `follow` options.
#[derive(Debug, Clone)]
pub struct FollowOptions {
    /// Node home.
    pub home: PathBuf,
    /// RPC servers; the first is the primary, the others are fallbacks
    /// and witnesses.
    pub rpc: Vec<String>,
    /// Expected chain ID (checked against the primary's `/status`).
    pub chain_id: Option<String>,
    /// Directory holding an SDK snapshot (`<height>/<format>/<chunk>`) for
    /// the first start.
    pub snapshot: Option<PathBuf>,
    /// Snapshot height to use when `snapshot` holds several.
    pub snapshot_height: Option<u64>,
    /// Trusted header hash at the snapshot height (default: the primary's,
    /// cross-checked with the witnesses).
    pub trust_hash: Option<Vec<u8>>,
    /// Light client trusting period (ns).
    pub trust_period: i64,
    /// Stop after this height (0 = follow forever).
    pub stop_height: i64,
    /// Heights fetched ahead in parallel.
    pub prefetch: usize,
    /// Run `ProcessProposal` on every block.
    pub verify_data: bool,
    /// Compare the header hash with every witness every N heights
    /// (0 = never).
    pub witness_interval: i64,
    /// App options (pruning, caches; block retention).
    pub app: app::AppOptions,
    /// Shutdown signal.
    pub shutdown: crate::ops::Shutdown,
}

/// Where diagnostics of a divergence go.
#[must_use]
pub fn diagnostics_dir(home: &Path) -> PathBuf {
    home.join("follow-diagnostics")
}

// ---- fetching ---------------------------------------------------------------

/// A fetched height.
struct Fetched {
    block: Block,
    /// The block id the server reported.
    block_id: BlockId,
    /// Which server answered.
    server: usize,
}

struct FetchState {
    /// Next height a worker should fetch.
    next: i64,
    /// Lowest height not yet taken by the consumer.
    consumed: i64,
    /// Latest height known to exist on the primary.
    tip: i64,
    done: BTreeMap<i64, Result<Fetched, String>>,
    stop: bool,
}

/// Parallel prefetcher of `/block` (Go has no equivalent: this
/// replaces the block sync pool's peers with RPC servers).
struct Fetcher {
    clients: Vec<RpcClient>,
    window: i64,
    st: Mutex<FetchState>,
    cv: Condvar,
    /// Total bytes of blocks fetched (statistics).
    fetched: AtomicI64,
}

fn is_not_yet(e: &str) -> bool {
    e.contains("must be less than or equal to") || e.contains("current blockchain height")
}

fn is_unavailable(e: &str) -> bool {
    e.contains("is not available") || e.contains("lowest height is")
}

impl Fetcher {
    fn new(clients: Vec<RpcClient>, start: i64, window: usize) -> Arc<Self> {
        Arc::new(Self {
            clients,
            window: i64::try_from(window.max(1)).unwrap_or(16),
            st: Mutex::new(FetchState {
                next: start,
                consumed: start,
                tip: 0,
                done: BTreeMap::new(),
                stop: false,
            }),
            cv: Condvar::new(),
            fetched: AtomicI64::new(0),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FetchState> {
        self.st
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn stop(&self) {
        self.lock().stop = true;
        self.cv.notify_all();
    }

    /// One height from the first server that has it (primary first), with
    /// retries on transport errors.
    fn fetch(&self, height: i64) -> Result<Fetched, String> {
        let mut last = String::new();
        for attempt in 0..8u32 {
            for (i, c) in self.clients.iter().enumerate() {
                match c.block(height) {
                    Ok((block_id, block)) => {
                        return Ok(Fetched {
                            block,
                            block_id,
                            server: i,
                        });
                    }
                    Err(e) => {
                        tracing::debug!(height, server = c.remote(), error = %e, "fetch failed");
                        last = format!("{}: {e}", c.remote());
                    }
                }
            }
            if self.lock().stop {
                break;
            }
            std::thread::sleep(Duration::from_millis(500 * u64::from(attempt + 1)));
        }
        Err(last)
    }

    fn refresh_tip(&self) {
        match self.clients[0].status() {
            Ok(s) => {
                let mut st = self.lock();
                if s.latest_block_height > st.tip {
                    st.tip = s.latest_block_height;
                    self.cv.notify_all();
                }
            }
            Err(e) => tracing::warn!(error = %e, "status of the primary failed"),
        }
    }

    fn worker(self: &Arc<Self>) {
        loop {
            let height = {
                let mut st = self.lock();
                loop {
                    if st.stop {
                        return;
                    }
                    if st.next < st.consumed + self.window && st.next <= st.tip {
                        break;
                    }
                    st = self
                        .cv
                        .wait_timeout(st, Duration::from_millis(500))
                        .map_or_else(|e| e.into_inner().0, |(g, _)| g);
                }
                st.next += 1;
                st.next - 1
            };
            let r = self.fetch(height);
            if let Ok(f) = &r {
                self.fetched.fetch_add(
                    i64::try_from(f.block.data.txs.iter().map(|t| t.0.len()).sum::<usize>())
                        .unwrap_or(0),
                    Ordering::Relaxed,
                );
            }
            let mut st = self.lock();
            st.done.insert(height, r);
            self.cv.notify_all();
        }
    }

    /// Tip poller: keeps `tip` current while the consumer is near it.
    fn tip_poller(self: &Arc<Self>) {
        loop {
            {
                let st = self.lock();
                if st.stop {
                    return;
                }
            }
            self.refresh_tip();
            let near = {
                let st = self.lock();
                st.next > st.tip
            };
            std::thread::sleep(if near {
                Duration::from_millis(1000)
            } else {
                Duration::from_secs(10)
            });
        }
    }

    /// Waits for `height` (taking it out of the buffer).
    fn take(
        &self,
        height: i64,
        shutdown: &crate::ops::Shutdown,
    ) -> Option<Result<Fetched, String>> {
        let mut st = self.lock();
        st.consumed = st.consumed.max(height);
        self.cv.notify_all();
        loop {
            if let Some(r) = st.done.remove(&height) {
                st.consumed = height + 1;
                self.cv.notify_all();
                return Some(r);
            }
            if st.stop || shutdown.is_triggered() {
                return None;
            }
            st = self
                .cv
                .wait_timeout(st, Duration::from_millis(500))
                .map_or_else(|e| e.into_inner().0, |(g, _)| g);
        }
    }

    fn tip(&self) -> i64 {
        self.lock().tip
    }
}

// ---- bootstrap --------------------------------------------------------------

/// A Go SDK snapshot on disk.
struct LocalSnapshot {
    snapshot: snapshots::Snapshot,
    dir: PathBuf,
}

fn read_snapshot(root: &Path, want: Option<u64>) -> anyhow::Result<LocalSnapshot> {
    let mut heights: Vec<u64> = std::fs::read_dir(root)
        .with_context(|| format!("read {}", root.display()))?
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
        .collect();
    heights.sort_unstable();
    let height = match want {
        Some(h) if heights.contains(&h) => h,
        Some(h) => bail!("no snapshot at height {h} in {}", root.display()),
        None => *heights
            .last()
            .ok_or_else(|| anyhow!("no snapshot in {}", root.display()))?,
    };
    let format = snapshots::CURRENT_FORMAT;
    let dir = root.join(height.to_string()).join(format.to_string());
    let mut chunks: Vec<u32> = std::fs::read_dir(&dir)
        .with_context(|| format!("read {}", dir.display()))?
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
        .collect();
    chunks.sort_unstable();
    if chunks.is_empty() || chunks.iter().enumerate().any(|(i, c)| *c as usize != i) {
        bail!("snapshot {} has missing chunks: {chunks:?}", dir.display());
    }
    let mut total = hash::Sha256Hasher::new();
    let mut chunk_hashes = Vec::with_capacity(chunks.len());
    for c in &chunks {
        let bz = std::fs::read(dir.join(c.to_string()))?;
        total.update(&bz);
        chunk_hashes.push(hash::sha256(&bz).to_vec());
    }
    Ok(LocalSnapshot {
        snapshot: snapshots::Snapshot {
            height,
            format,
            chunks: u32::try_from(chunks.len())?,
            hash: total.finalize().to_vec(),
            chunk_hashes,
        },
        dir,
    })
}

#[allow(clippy::too_many_lines)]
/// Restores `snap` into the app (OfferSnapshot + ApplySnapshotChunk, the
/// state-sync syncer's calls) and bootstraps the node stores with the
/// light-client-verified state at the snapshot height.
fn bootstrap(
    opts: &FollowOptions,
    chain_id: &str,
    app: &Arc<Mutex<app::App>>,
    ss: &StateStore,
    bs: &BlockStore,
) -> anyhow::Result<State> {
    let root = opts.snapshot.as_ref().ok_or_else(|| {
        anyhow!("the home has no state: pass --snapshot <dir> for the first start")
    })?;
    let snap = read_snapshot(root, opts.snapshot_height)?;
    let height = i64::try_from(snap.snapshot.height)?;
    tracing::info!(
        height,
        chunks = snap.snapshot.chunks,
        hash = %hex::encode_upper(&snap.snapshot.hash),
        "bootstrapping from snapshot"
    );
    let trust_hash = if let Some(h) = &opts.trust_hash {
        h.clone()
    } else {
        let c = RpcClient::new(&opts.rpc[0], RPC_TIMEOUT);
        let (sh, _) = c
            .commit(Some(height))
            .map_err(|e| anyhow!("trusted header {height}: {e}"))?;
        let h = sh
            .header
            .as_ref()
            .and_then(Header::hash)
            .ok_or_else(|| anyhow!("no header at {height}"))?;
        tracing::warn!(
            height,
            hash = %hex::encode_upper(&h),
            "no --trust-hash: trusting the primary's header at the snapshot height"
        );
        h
    };
    // Witnesses must still have the snapshot height; the light client
    // needs one, so a lone primary witnesses itself.
    let mut servers = vec![opts.rpc[0].clone()];
    for r in &opts.rpc[1..] {
        match RpcClient::new(r, RPC_TIMEOUT).status() {
            Ok(s) if s.earliest_block_height <= height => servers.push(r.clone()),
            Ok(s) => tracing::info!(
                server = %r,
                earliest = s.earliest_block_height,
                "not a bootstrap witness: pruned below the snapshot height"
            ),
            Err(e) => tracing::warn!(server = %r, error = %e, "not a bootstrap witness"),
        }
    }
    if servers.len() < 2 {
        servers.push(servers[0].clone());
    }
    let sp = statesync::LightClientStateProvider::new(
        chain_id,
        ctypes::Consensus::default(),
        1,
        &servers,
        &light::TrustOptions {
            period: opts.trust_period,
            height,
            hash: trust_hash,
        },
    )
    .map_err(|e| anyhow!("light client: {e}"))?;
    let h = snap.snapshot.height;
    let app_hash = sp
        .app_hash(h)
        .map_err(|e| anyhow!("app hash at {h}: {e}"))?;
    let state = sp.state(h).map_err(|e| anyhow!("state at {h}: {e}"))?;
    let commit = sp.commit(h).map_err(|e| anyhow!("commit at {h}: {e}"))?;

    let mut a = app
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let offer = a.offer_snapshot(abci::RequestOfferSnapshot {
        snapshot: Some(abci::Snapshot {
            height: snap.snapshot.height,
            format: snap.snapshot.format,
            chunks: snap.snapshot.chunks,
            hash: snap.snapshot.hash.clone(),
            metadata: snap.snapshot.metadata_bytes(),
        }),
        app_hash: app_hash.clone(),
        app_version: state.consensus_params.version.app,
    });
    if offer.result != abci::response_offer_snapshot::Result::Accept as i32 {
        bail!(
            "app refused the snapshot (OfferSnapshot result {})",
            offer.result
        );
    }
    let started = Instant::now();
    for i in 0..snap.snapshot.chunks {
        let chunk = std::fs::read(snap.dir.join(i.to_string()))?;
        let r = a.apply_snapshot_chunk(abci::RequestApplySnapshotChunk {
            index: i,
            chunk,
            sender: String::new(),
        });
        if r.result != abci::response_apply_snapshot_chunk::Result::Accept as i32 {
            bail!("applying snapshot chunk {i} failed (result {})", r.result);
        }
        tracing::info!(
            chunk = i,
            of = snap.snapshot.chunks,
            "applied snapshot chunk"
        );
    }
    // verifyApp.
    let info = a.info();
    drop(a);
    if info.last_block_height != height {
        bail!(
            "restored app is at height {}, expected {height}",
            info.last_block_height
        );
    }
    if info.app_version != state.consensus_params.version.app {
        bail!(
            "app version mismatch. Expected: {}, got: {}",
            state.consensus_params.version.app,
            info.app_version
        );
    }
    if info.last_block_app_hash != app_hash {
        bail!(
            "restored app hash {} does not match the network's {} (header {})",
            hex::encode_upper(&info.last_block_app_hash),
            hex::encode_upper(&app_hash),
            height + 1
        );
    }
    tracing::info!(
        height,
        app_hash = %hex::encode_upper(&app_hash),
        app_version = info.app_version,
        secs = started.elapsed().as_secs(),
        "snapshot restored; app hash matches the network"
    );
    statesync::NodeStores {
        state_store: ss.clone(),
        block_store: bs.clone(),
    }
    .bootstrap(&state, &commit)
    .map_err(|e| anyhow!("{e}"))?;
    Ok(state)
}

// ---- verification -------------------------------------------------------------

/// A header field the network committed that differs from ours.
#[derive(Debug, Clone)]
struct FieldMismatch {
    name: &'static str,
    ours: String,
    network: String,
}

/// Compares header `h` (`state.last_block_height + 1`) with the state our
/// execution of block `h-1` produced.
fn compare_header(state: &State, header: &Header) -> Vec<FieldMismatch> {
    let mut out = Vec::new();
    let mut check = |name, ours: &[u8], net: &[u8]| {
        if ours != net {
            out.push(FieldMismatch {
                name,
                ours: hex::encode_upper(ours),
                network: hex::encode_upper(net),
            });
        }
    };
    check("app_hash", &state.app_hash, &header.app_hash);
    check(
        "last_results_hash",
        &state.last_results_hash,
        &header.last_results_hash,
    );
    check(
        "validators_hash",
        &state.validators.hash(),
        &header.validators_hash,
    );
    check(
        "next_validators_hash",
        &state.next_validators.hash(),
        &header.next_validators_hash,
    );
    check(
        "consensus_hash",
        &state.consensus_params.hash(),
        &header.consensus_hash,
    );
    check(
        "last_block_id",
        &state.last_block_id.hash,
        &header.last_block_id.hash,
    );
    if state.version.app != header.version.app {
        out.push(FieldMismatch {
            name: "version.app",
            ours: state.version.app.to_string(),
            network: header.version.app.to_string(),
        });
    }
    out
}

fn signed_header(header: &Header, commit: &Commit) -> SignedHeader {
    SignedHeader {
        header: Some(header.clone()),
        commit: Some(commit.clone()),
    }
}

// ---- diagnostics --------------------------------------------------------------

fn write_json(path: &Path, v: &serde_json::Value) {
    let s = serde_json::to_string_pretty(v).unwrap_or_default();
    if let Err(e) = std::fs::write(path, s) {
        tracing::error!(path = %path.display(), error = %e, "write diagnostics");
    }
}

/// Per-store root hashes of the network at `height` from ICS-23 proofs of
/// `/store/<name>/key` queries (when the RPC server still has that state).
fn network_store_hashes(
    client: &RpcClient,
    names: &[String],
    height: i64,
) -> BTreeMap<String, String> {
    use prost::Message as _;
    let mut out = BTreeMap::new();
    for name in names {
        let path = format!(
            "/abci_query?path=%22/store/{name}/key%22&data=0x00&height={height}&prove=true"
        );
        let r = match client.get_json(&path) {
            Ok(r) => r,
            Err(e) => {
                out.insert(name.clone(), format!("error: {e}"));
                continue;
            }
        };
        let ops = r["response"]["proofOps"]["ops"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let hash = ops
            .iter()
            .find(|op| op["type"].as_str() == Some("ics23:simple"))
            .and_then(|op| op["data"].as_str())
            .and_then(|d| {
                use base64::Engine as _;
                base64::engine::general_purpose::STANDARD.decode(d).ok()
            })
            .and_then(|bz| ics23::CommitmentProof::decode(bz.as_slice()).ok())
            .and_then(|p| match p.proof {
                Some(ics23::commitment_proof::Proof::Exist(e)) => Some(hex::encode_upper(e.value)),
                _ => None,
            });
        out.insert(
            name.clone(),
            hash.unwrap_or_else(|| format!("unavailable: {}", r["response"]["log"])),
        );
    }
    out
}

/// Writes the divergence bundle for block `height` (whose execution
/// produced the mismatching state) and returns its directory.
#[allow(clippy::too_many_arguments)]
fn write_diagnostics(
    home: &Path,
    client: &RpcClient,
    app: &Arc<Mutex<app::App>>,
    ss: &StateStore,
    height: i64,
    reason: &str,
    mismatches: &[FieldMismatch],
) -> PathBuf {
    let dir = diagnostics_dir(home).join(height.to_string());
    let _ = std::fs::create_dir_all(&dir);
    let fields: Vec<serde_json::Value> = mismatches
        .iter()
        .map(|m| serde_json::json!({"field": m.name, "ours": m.ours, "network": m.network}))
        .collect();
    // Our results.
    let ours = ss.load_finalize_block_response(height).ok().flatten();
    if let Some(r) = &ours {
        let mut s = String::new();
        rpc::encode::block_results(height, r).write(&mut s);
        let _ = std::fs::write(dir.join("our_block_results.json"), s);
    }
    // The network's.
    let net = client
        .get_json(&format!("/block_results?height={height}"))
        .ok();
    if let Some(n) = &net {
        write_json(&dir.join("network_block_results.json"), n);
    }
    if let Ok(b) = client.get_json(&format!("/block?height={height}")) {
        write_json(&dir.join("block.json"), &b);
    }
    // Per-tx comparison of the consensus-relevant fields.
    let mut txs = Vec::new();
    if let (Some(o), Some(n)) = (&ours, &net) {
        let nr = n["txs_results"].as_array().cloned().unwrap_or_default();
        for (i, r) in o.tx_results.iter().enumerate() {
            let x = nr.get(i).cloned().unwrap_or_default();
            let ncode = x["code"].as_u64().unwrap_or(0);
            let nwanted = x["gas_wanted"].as_str().unwrap_or("0").to_owned();
            let nused = x["gas_used"].as_str().unwrap_or("0").to_owned();
            let ndata = x["data"].as_str().unwrap_or("").to_owned();
            let odata = {
                use base64::Engine as _;
                base64::engine::general_purpose::STANDARD.encode(&r.data)
            };
            let same = u64::from(r.code) == ncode
                && r.gas_wanted.to_string() == nwanted
                && r.gas_used.to_string() == nused
                && odata == ndata;
            txs.push(serde_json::json!({
                "index": i,
                "match": same,
                "ours": {"code": r.code, "codespace": r.codespace, "gas_wanted": r.gas_wanted.to_string(), "gas_used": r.gas_used.to_string(), "log": r.log},
                "network": {"code": ncode, "codespace": x["codespace"], "gas_wanted": nwanted, "gas_used": nused, "log": x["log"]},
            }));
        }
    }
    // Store hashes: ours from the commit info, the network's from proofs at
    // `height` (the version committed by block `height`).
    let ours_stores: BTreeMap<String, String> = {
        let a = app
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        a.multistore()
            .root()
            .get_commit_info(height)
            .map(|ci| {
                ci.store_infos
                    .iter()
                    .map(|s| (s.name.clone(), hex::encode_upper(s.commit_id.hash)))
                    .collect()
            })
            .unwrap_or_default()
    };
    let names: Vec<String> = ours_stores.keys().cloned().collect();
    let net_stores = network_store_hashes(client, &names, height);
    let stores: Vec<serde_json::Value> = names
        .iter()
        .map(|n| {
            let o = ours_stores.get(n).cloned().unwrap_or_default();
            let w = net_stores.get(n).cloned().unwrap_or_default();
            serde_json::json!({"store": n, "match": o == w, "ours": o, "network": w})
        })
        .collect();
    write_json(
        &dir.join("summary.json"),
        &serde_json::json!({
            "height": height,
            "reason": reason,
            "header_fields": fields,
            "tx_results": txs,
            "stores": stores,
        }),
    );
    dir
}

// ---- the loop -----------------------------------------------------------------

/// `follow --light-only`: header verification without execution (a
/// network whose app version this binary cannot execute yet). The light
/// client (`light::Client` over the RPC servers, the others as witnesses)
/// starts from the trusted header at `trust_height` and verifies every
/// height up to `stop_height` (0: the tip at start) sequentially, then
/// jumps to the current tip with skipping verification.
#[allow(clippy::too_many_lines)]
pub fn light_only(
    rpc: &[String],
    chain_id: Option<&str>,
    trust_height: i64,
    trust_hash: Option<Vec<u8>>,
    trust_period: i64,
    stop_height: i64,
    shutdown: &crate::ops::Shutdown,
) -> anyhow::Result<FollowStats> {
    let primary = RpcClient::new(&rpc[0], RPC_TIMEOUT);
    let status = primary.status().map_err(|e| anyhow!("status: {e}"))?;
    if let Some(want) = chain_id
        && want != status.network
    {
        bail!(
            "{} serves chain {:?}, expected {want:?}",
            rpc[0],
            status.network
        );
    }
    let trust_height = if trust_height > 0 {
        trust_height
    } else {
        status.latest_block_height - 1000
    };
    let hash = if let Some(h) = trust_hash {
        h
    } else {
        let (sh, _) = primary
            .commit(Some(trust_height))
            .map_err(|e| anyhow!("header {trust_height}: {e}"))?;
        sh.header
            .as_ref()
            .and_then(Header::hash)
            .unwrap_or_default()
    };
    let provider = |r: &String| -> Arc<dyn light::Provider> {
        Arc::new(light::HttpProvider::with_client(
            status.network.clone(),
            RpcClient::new(r, RPC_TIMEOUT),
        ))
    };
    let witnesses = if rpc.len() > 1 {
        rpc[1..].iter().map(provider).collect()
    } else {
        vec![provider(&rpc[0])]
    };
    let client = |height: i64, hash: Vec<u8>, mode: light::Mode| {
        light::Client::new(
            status.network.clone(),
            &light::TrustOptions {
                period: trust_period,
                height,
                hash,
            },
            provider(&rpc[0]),
            witnesses.clone(),
            Box::new(light::DbStore::in_memory()),
            light::Options {
                mode,
                ..light::Options::default()
            },
        )
        .map_err(|e| anyhow!("light client: {e}"))
    };
    let mut lc = client(trust_height, hash.clone(), light::Mode::Sequential)?;
    let mut last_hash = hash;
    let end = if stop_height > 0 {
        stop_height
    } else {
        status.latest_block_height
    };
    let started = Instant::now();
    let mut totals = FollowStats {
        first: trust_height + 1,
        last: trust_height,
        ..FollowStats::default()
    };
    for h in trust_height + 1..=end {
        if shutdown.is_triggered() {
            break;
        }
        let lb = lc
            .verify_light_block_at_height(h, light::verifier::now())
            .map_err(|e| anyhow!("light client verification of {h}: {e}"))?;
        totals.last = h;
        totals.blocks += 1;
        last_hash = light::lightblock_hash(&lb);
        if totals.blocks.is_multiple_of(100) || h == end {
            #[allow(clippy::cast_precision_loss)]
            let rate = totals.blocks as f64 / started.elapsed().as_secs_f64().max(1e-9);
            tracing::info!(
                height = h,
                hash = %hex::encode_upper(light::lightblock_hash(&lb)),
                headers_per_sec = format!("{rate:.2}"),
                "verified header"
            );
        }
    }
    if !shutdown.is_triggered() {
        let tip = primary
            .status()
            .map_err(|e| anyhow!("status: {e}"))?
            .latest_block_height;
        let mut lc = client(totals.last, last_hash, light::Mode::Skipping)?;
        let lb = lc
            .verify_light_block_at_height(tip, light::verifier::now())
            .map_err(|e| anyhow!("skipping verification of {tip}: {e}"))?;
        tracing::info!(
            height = tip,
            hash = %hex::encode_upper(light::lightblock_hash(&lb)),
            "verified the tip (skipping verification)"
        );
    }
    Ok(totals)
}

/// Statistics of a run.
#[derive(Debug, Default, Clone, Copy)]
pub struct FollowStats {
    /// First height executed.
    pub first: i64,
    /// Last height executed.
    pub last: i64,
    /// Blocks executed.
    pub blocks: u64,
    /// Transactions executed.
    pub txs: u64,
}

/// `celestia-rs follow`.
#[allow(clippy::too_many_lines)]
pub fn follow(opts: &FollowOptions) -> anyhow::Result<FollowStats> {
    if opts.rpc.is_empty() {
        bail!("at least one --rpc server is required");
    }
    let clients: Vec<RpcClient> = opts
        .rpc
        .iter()
        .map(|r| RpcClient::new(r, RPC_TIMEOUT))
        .collect();
    let primary_status = clients[0]
        .status()
        .map_err(|e| anyhow!("status of {}: {e}", opts.rpc[0]))?;
    let chain_id = primary_status.network.clone();
    if let Some(want) = &opts.chain_id
        && *want != chain_id
    {
        bail!(
            "{} serves chain {chain_id:?}, expected {want:?}",
            opts.rpc[0]
        );
    }
    tracing::info!(
        chain_id,
        primary = %opts.rpc[0],
        tip = primary_status.latest_block_height,
        earliest = primary_status.earliest_block_height,
        "following over RPC"
    );
    let home = &opts.home;
    std::fs::create_dir_all(home.join("data"))?;
    let mut app_opts = opts.app.clone();
    app_opts.chain_id = Some(chain_id.clone());
    app_opts.node.home = Some(home.clone());
    let app =
        app::App::open_with(home, app_opts).map_err(|e| anyhow!("open app state: {}", e.log))?;
    let app = Arc::new(Mutex::new(app));
    let (bs, ss) = crate::node::open_stores(home)?;
    let state = match ss.load().map_err(|e| anyhow!("load state: {e}"))? {
        Some(s) if s.last_block_height > 0 => s,
        _ => bootstrap(opts, &chain_id, &app, &ss, &bs)?,
    };
    if state.chain_id != chain_id {
        bail!("the home follows {:?}, not {chain_id:?}", state.chain_id);
    }
    let mut exec = BlockExecutor::new(
        ss.clone(),
        bs.clone(),
        Box::new(crate::node::AppAdapter(app.clone())),
        Arc::new(nodestore::NopMempool),
        Box::new(nodestore::EmptyEvidencePool),
    );
    // Handshake: replays a block committed by the app but not yet recorded
    // in the state (crash between Commit and the state save).
    let gen_doc = GenesisDoc {
        genesis_time: ctypes::Time::default(),
        chain_id: chain_id.clone(),
        initial_height: state.initial_height,
        consensus_params: state.consensus_params.clone(),
        validators: Vec::new(),
        app_hash: Vec::new(),
        app_state: Vec::new(),
    };
    let mut state = Handshaker::new(
        ss.clone(),
        state,
        bs.clone(),
        &gen_doc,
        consensus::ticker::system_now(),
    )
    .handshake(&mut exec)
    .map_err(|e| anyhow!("handshake: {e}"))?;
    tracing::info!(
        height = state.last_block_height,
        app_hash = %hex::encode_upper(&state.app_hash),
        "resuming"
    );

    // The trusted header at the current height (its hash must be the one
    // our state recorded); its canonical commit comes with block h0+1.
    let h0 = state.last_block_height;
    let (_, first) = fetch_one(&clients, h0)?;
    if first.header.hash().unwrap_or_default() != state.last_block_id.hash {
        bail!(
            "header {h0} from the RPC server ({}) is not the one in our state ({})",
            hex::encode_upper(first.header.hash().unwrap_or_default()),
            hex::encode_upper(&state.last_block_id.hash)
        );
    }
    let mut trusted_header = first.header;

    let fetcher = Fetcher::new(clients.clone(), h0 + 1, opts.prefetch);
    fetcher.refresh_tip();
    let mut threads = Vec::new();
    for _ in 0..opts.prefetch.max(1) {
        let f = Arc::clone(&fetcher);
        threads.push(std::thread::spawn(move || f.worker()));
    }
    {
        let f = Arc::clone(&fetcher);
        threads.push(std::thread::spawn(move || f.tip_poller()));
    }
    let stopped = Arc::new(AtomicBool::new(false));
    {
        let f = Arc::clone(&fetcher);
        let stopped = Arc::clone(&stopped);
        opts.shutdown.on_trigger(move || {
            stopped.store(true, Ordering::SeqCst);
            f.stop();
        });
    }

    let mut totals = FollowStats {
        first: h0 + 1,
        last: h0,
        ..FollowStats::default()
    };
    let started = Instant::now();
    let mut window_start = (Instant::now(), 0u64, 0u64);
    let mut waited = Duration::ZERO;
    let mut exec_time = Duration::ZERO;
    let take = |h: i64| -> anyhow::Result<Option<Fetched>> {
        match fetcher.take(h, &opts.shutdown) {
            None => Ok(None),
            Some(Ok(f)) => {
                if f.block.header.height != h {
                    bail!(
                        "server {} answered height {} for {h}",
                        opts.rpc[f.server],
                        f.block.header.height
                    );
                }
                Ok(Some(f))
            }
            Some(Err(e)) => Err(anyhow!("fetching block {h} failed: {e}")),
        }
    };
    let result: anyhow::Result<()> = (|| {
        let t0 = Instant::now();
        let Some(mut cur) = take(h0 + 1)? else {
            return Ok(());
        };
        // Block h0+1's last commit is the canonical commit of h0.
        let mut trusted_commit = cur
            .block
            .last_commit
            .clone()
            .ok_or_else(|| anyhow!("block {} has no last commit", h0 + 1))?;
        waited += t0.elapsed();
        loop {
            let height = state.last_block_height + 1;
            if opts.shutdown.is_triggered() {
                return Ok(());
            }
            if opts.stop_height > 0 && height > opts.stop_height {
                tracing::info!(height = state.last_block_height, "stop height reached");
                return Ok(());
            }
            // Block h+1 carries the canonical commit of h.
            let t0 = Instant::now();
            let Some(next) = take(height + 1)? else {
                return Ok(());
            };
            waited += t0.elapsed();
            let t1 = Instant::now();
            let commit = next
                .block
                .last_commit
                .clone()
                .ok_or_else(|| anyhow!("block {} has no last commit", height + 1))?;
            let header = cur.block.header.clone();
            // Our execution of block h-1 vs header h.
            let mismatches = compare_header(&state, &header);
            if !mismatches.is_empty() {
                let dir = write_diagnostics(
                    home,
                    &clients[0],
                    &app,
                    &ss,
                    height - 1,
                    &format!(
                        "header {height} disagrees with our execution of block {}",
                        height - 1
                    ),
                    &mismatches,
                );
                for m in &mismatches {
                    tracing::error!(field = m.name, ours = %m.ours, network = %m.network, "MISMATCH");
                }
                bail!(
                    "state after block {} diverges from the network ({}); diagnostics in {}",
                    height - 1,
                    mismatches
                        .iter()
                        .map(|m| m.name)
                        .collect::<Vec<_>>()
                        .join(", "),
                    dir.display()
                );
            }
            // Light client: header h is signed by 2/3+ of the validator set
            // header h-1 committed to (ours, whose hash header h carries).
            light::verify_adjacent(
                &signed_header(&trusted_header, &trusted_commit),
                &signed_header(&header, &commit),
                &state.validators,
                opts.trust_period,
                light::verifier::now(),
                MAX_CLOCK_DRIFT,
            )
            .map_err(|e| anyhow!("light client verification of header {height}: {e}"))?;
            if opts.witness_interval > 0 && height % opts.witness_interval == 0 {
                check_witnesses(&clients, cur.server, height, &header)?;
            }
            // The block id (header hash + part set: every tx, the evidence
            // and the last commit) must be the one the commit signs.
            let mut block = cur.block;
            let parts = block
                .make_part_set(BLOCK_PART_SIZE_BYTES)
                .map_err(|e| anyhow!("part set of block {height}: {e}"))?;
            let block_id = BlockId {
                hash: block.hash().unwrap_or_default(),
                part_set_header: parts.header(),
            };
            if block_id != cur.block_id || block_id != next.block.header.last_block_id {
                bail!(
                    "block {height} rebuilt from {} does not have the network's block id",
                    opts.rpc[cur.server]
                );
            }
            ctypes::verify_commit(&chain_id, &state.validators, &block_id, height, &commit)
                .map_err(|e| anyhow!("commit of block {height}: {e}"))?;
            exec.validate_block(&state, &mut block, consensus::ticker::system_now())
                .map_err(|e| anyhow!("block {height} is invalid against our state: {e}"))?;
            if opts.verify_data {
                let ok = exec
                    .process_proposal(&mut block, &state)
                    .map_err(|e| anyhow!("ProcessProposal of block {height}: {e}"))?;
                if !ok {
                    let dir = write_diagnostics(
                        home,
                        &clients[0],
                        &app,
                        &ss,
                        height,
                        "ProcessProposal rejected a block the network committed",
                        &[],
                    );
                    bail!(
                        "ProcessProposal rejected block {height} (committed by the network); diagnostics in {}",
                        dir.display()
                    );
                }
            }
            bs.save_block(&mut block, &parts, &commit)
                .map_err(|e| anyhow!("save block {height}: {e}"))?;
            let ntx = block.data.txs.len();
            state = exec
                .apply_verified_block(&state, block_id, &mut block)
                .map_err(|e| anyhow!("executing block {height}: {e}"))?;
            exec_time += t1.elapsed();
            trusted_header = header;
            trusted_commit = commit;
            cur = next;
            totals.last = height;
            totals.blocks += 1;
            totals.txs += ntx as u64;
            if totals.blocks.is_multiple_of(100) {
                let dt = window_start.0.elapsed().as_secs_f64().max(1e-9);
                #[allow(clippy::cast_precision_loss)]
                let rate = (totals.blocks - window_start.1) as f64 / dt;
                #[allow(clippy::cast_precision_loss)]
                let tx_rate = (totals.txs - window_start.2) as f64 / dt;
                tracing::info!(
                    height,
                    tip = fetcher.tip(),
                    blocks_per_sec = format!("{rate:.2}"),
                    txs_per_sec = format!("{tx_rate:.1}"),
                    exec_secs = exec_time.as_secs(),
                    wait_secs = waited.as_secs(),
                    tx_mib = fetcher.fetched.load(Ordering::Relaxed) >> 20,
                    app_hash = %hex::encode_upper(&state.app_hash),
                    "followed"
                );
                window_start = (Instant::now(), totals.blocks, totals.txs);
            } else if height + 1 >= fetcher.tip() {
                tracing::info!(
                    height,
                    txs = ntx,
                    app_hash = %hex::encode_upper(&state.app_hash),
                    "at tip"
                );
            }
        }
    })();
    fetcher.stop();
    for t in threads {
        let _ = t.join();
    }
    tracing::info!(
        first = totals.first,
        last = totals.last,
        blocks = totals.blocks,
        txs = totals.txs,
        secs = started.elapsed().as_secs(),
        "follow finished"
    );
    if stopped.load(Ordering::SeqCst) {
        return Ok(totals);
    }
    result.map(|()| totals)
}

/// Block `height` from the first server that has it.
fn fetch_one(clients: &[RpcClient], height: i64) -> anyhow::Result<(BlockId, Block)> {
    let mut last = String::new();
    for c in clients {
        match c.block(height) {
            Ok(b) => return Ok(b),
            Err(e) => last = format!("{}: {e}", c.remote()),
        }
    }
    Err(anyhow!("block {height}: {last}"))
}

/// Compares header `height` with every witness that still has it.
fn check_witnesses(
    clients: &[RpcClient],
    server: usize,
    height: i64,
    header: &Header,
) -> anyhow::Result<()> {
    let want = header.hash().unwrap_or_default();
    for (i, c) in clients.iter().enumerate() {
        if i == server {
            continue;
        }
        match c.commit(Some(height)) {
            Ok((sh, _)) => {
                let got = sh
                    .header
                    .as_ref()
                    .and_then(Header::hash)
                    .unwrap_or_default();
                if got != want {
                    bail!(
                        "witness {} has header {} at height {height}, the primary {}: conflicting headers",
                        c.remote(),
                        hex::encode_upper(&got),
                        hex::encode_upper(&want)
                    );
                }
            }
            Err(e) if is_unavailable(&e.0) || is_not_yet(&e.0) => {}
            Err(e) => tracing::debug!(witness = c.remote(), error = %e, "witness unavailable"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_snapshot_hashes() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let fmt = dir.path().join("1500").join("3");
        std::fs::create_dir_all(&fmt).unwrap_or_else(|e| panic!("{e}"));
        std::fs::create_dir_all(dir.path().join("900").join("3").join("0"))
            .unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(fmt.join("0"), b"abc").unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(fmt.join("1"), b"de").unwrap_or_else(|e| panic!("{e}"));
        let s = read_snapshot(dir.path(), None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(s.snapshot.height, 1500);
        assert_eq!(s.snapshot.chunks, 2);
        assert_eq!(s.snapshot.chunk_hashes[0], hash::sha256(b"abc").to_vec());
        assert_eq!(s.snapshot.hash, hash::sha256(b"abcde").to_vec());
        assert!(read_snapshot(dir.path(), Some(7)).is_err());
        std::fs::remove_file(fmt.join("0")).unwrap_or_else(|e| panic!("{e}"));
        assert!(read_snapshot(dir.path(), Some(1500)).is_err());
    }

    /// A block with duplicate-vote evidence survives the RPC JSON (our
    /// Go-identical `/block` encoder) and the follower's decoder with the
    /// same evidence hash and block hash.
    #[test]
    fn block_json_round_trip_with_evidence() {
        use ctypes::evidence::DuplicateVoteEvidence;
        let vote = |hash: u8| ctypes::Vote {
            vote_type: 2,
            height: 9,
            round: 1,
            block_id: BlockId {
                hash: vec![hash; 32],
                part_set_header: ctypes::PartSetHeader {
                    total: 1,
                    hash: vec![hash; 32],
                },
            },
            timestamp: ctypes::Time {
                seconds: 1_700_000_000,
                nanos: 5,
            },
            validator_address: vec![7; 20],
            validator_index: 3,
            signature: vec![1; 64],
            extension: Vec::new(),
            extension_signature: Vec::new(),
        };
        let ev = ctypes::Evidence::DuplicateVote(DuplicateVoteEvidence {
            vote_a: Some(vote(1)),
            vote_b: Some(vote(2)),
            total_voting_power: 100,
            validator_power: 10,
            timestamp: ctypes::Time {
                seconds: 1_700_000_000,
                nanos: 0,
            },
        });
        let evidence = ctypes::EvidenceData { evidence: vec![ev] };
        let header = Header {
            chain_id: "c".into(),
            height: 10,
            evidence_hash: evidence.hash(),
            data_hash: vec![3; 32],
            ..Header::default()
        };
        let mut block = Block::new(
            header.clone(),
            ctypes::Data::new(vec![ctypes::Tx(vec![1, 2])], 1, Some(vec![3; 32])),
            evidence,
            Some(Commit::default()),
        );
        let mut s = String::new();
        rpc::encode::block(&block).write(&mut s);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap_or_default();
        let mut back = light::http::block_from_json(&v).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back.evidence, block.evidence);
        assert_eq!(back.hash(), block.hash());
        assert_eq!(back.data.txs, block.data.txs);
    }

    #[test]
    fn header_mismatches_are_named() {
        let state = statesync::empty_state("c", ctypes::Consensus::default(), 1);
        let header = Header {
            app_hash: state.app_hash.clone(),
            last_results_hash: state.last_results_hash.clone(),
            validators_hash: state.validators.hash(),
            next_validators_hash: state.next_validators.hash(),
            consensus_hash: state.consensus_params.hash(),
            ..Header::default()
        };
        assert!(compare_header(&state, &header).is_empty());
        let bad = Header {
            app_hash: vec![1],
            last_results_hash: vec![2],
            ..header
        };
        let names: Vec<_> = compare_header(&state, &bad)
            .iter()
            .map(|m| m.name)
            .collect();
        assert_eq!(names, ["app_hash", "last_results_hash"]);
    }
}
