//! Live historical parity check for the current `celestia-rs` executable.
//! Inputs and replaying individual windows are documented in conformance/README.md.

#![allow(clippy::print_stdout)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context as _, anyhow, ensure};
use celestia_rs::{networks, node};
use ctypes::{BLOCK_PART_SIZE_BYTES, Block, BlockId};
use flate2::read::GzDecoder;
use light::{RpcClient, http};
use prost::Message as _;
use serde::Deserialize;
use x_upgrade::UpgradeHandlers as _;

const CHAIN: &str = "celestia";
const VERSIONS: [u64; 9] = [1, 2, 3, 4, 5, 6, 8, 9, 10];
const WINDOW_SIZE: usize = 10;
const BINARY: &str = env!("CARGO_BIN_EXE_celestia-rs");
const FIXTURE: &str = include_str!("../../../../conformance/fixtures/mainnet/replay-windows.json");
const ARCHIVE: &[u8] = include_bytes!("../../../../conformance/fixtures/mainnet/blocks.jsonl.gz");
const V1_ORACLE: &str =
    include_str!("../../../../conformance/fixtures/mainnet/v1-oracle/store-roots.json");
const V1_ORACLE_KV_MODULES: &[u8] = include_bytes!(
    "../../../../conformance/fixtures/mainnet/v1-oracle/kv-height-0-modules.jsonl.gz"
);
const V1_ORACLE_KV_DIFFS: &[u8] =
    include_bytes!("../../../../conformance/fixtures/mainnet/v1-oracle/kv-diffs.jsonl.gz");
const V1_ORACLE_TX_RESULTS: &[u8] =
    include_bytes!("../../../../conformance/fixtures/mainnet/v1-oracle/tx-results.jsonl.gz");
/// Commit info entries of app version 1 (celestia-app v3.13.0, Cosmos SDK
/// v0.46): 17 IAVL stores and the memory store `mem_capability`.
const V1_STORES: [&str; 18] = [
    "acc",
    "authz",
    "bank",
    "blob",
    "capability",
    "distribution",
    "evidence",
    "feegrant",
    "gov",
    "ibc",
    "mem_capability",
    "mint",
    "params",
    "qgb",
    "slashing",
    "staking",
    "transfer",
    "upgrade",
];
/// Stores left out of the committed height-0 KV dump (genesis accounts
/// and balances, about 25 MB compressed).
const V1_BULK_STORES: [&str; 2] = ["acc", "bank"];

#[derive(Deserialize)]
struct Fixtures {
    chain_id: String,
    rpc: String,
    windows: Vec<Window>,
}

#[derive(Deserialize)]
struct Window {
    app_version: u64,
    start_height: i64,
    #[serde(default)]
    pre_state: Option<PreState>,
    blocks: Vec<ReferenceBlock>,
}

/// Commit info at the height before a window, from archive RPC proofs.
#[derive(Deserialize)]
struct PreState {
    height: i64,
    upgrade_plan: Option<UpgradePlan>,
    /// Store name to root hash; empty for an SDK v0.46 memory store.
    stores: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct UpgradePlan {
    name: String,
    height: i64,
}

/// Why a faithful Go checkpoint before each window cannot replay.
/// Versions 9 and 10 have no structural blocker. App v1 starts at genesis
/// and executes on the cosmos-sdk v0.46 driver.
const EXPECTED_BLOCKERS: [(u64, &[&str]); 9] = [
    (1, &[]),
    (2, &["no public pre-state at height 2371494"]),
    (3, &["no public pre-state at height 2993219"]),
    (
        4,
        &[
            "pre-state stores that celestia-rs does not mount: mem_capability",
            "celestia-rs stores absent from the pre-state: circuit, consensus, hyperlane, minfee, warp",
        ],
    ),
    (
        5,
        &[
            "celestia-rs stores absent from the pre-state: zkism",
            "no celestia-rs handler for upgrade plan v5 at 6748822",
        ],
    ),
    (
        6,
        &[
            "celestia-rs stores absent from the pre-state: zkism",
            "no celestia-rs handler for upgrade plan v6 at 8662013",
        ],
    ),
    (
        8,
        &[
            "celestia-rs stores absent from the pre-state: zkism",
            "no celestia-rs handler for upgrade plan v8 at 10960600",
        ],
    ),
    (9, &[]),
    (10, &[]),
];

/// Store roots from the Go v1 oracle (`conformance/v1-oracle`).
#[derive(Deserialize)]
struct V1Oracle {
    chain_id: String,
    genesis_sha256: String,
    init_chain_app_hash: String,
    heights: Vec<OracleHeight>,
    /// Uncompressed SHA-256 of each KV dump file.
    kv: BTreeMap<String, String>,
    /// Uncompressed SHA-256 of `tx-results.jsonl`.
    tx_results_sha256: String,
}

#[derive(Deserialize)]
struct OracleHeight {
    height: i64,
    app_hash: String,
    stores: BTreeMap<String, String>,
    keys: BTreeMap<String, usize>,
    /// `[code, gas_wanted, gas_used]` of each DeliverTx.
    tx_results: Vec<[i64; 3]>,
    /// Hex `data` of each DeliverTx, parallel to `tx_results`.
    tx_data: Vec<String>,
}

/// Hex key to hex value, by store.
type OracleState = BTreeMap<String, BTreeMap<String, String>>;

/// One store of the height-0 KV dump.
#[derive(Deserialize)]
struct OracleStoreDump {
    store: String,
    root: String,
    kv: Vec<[String; 2]>,
}

/// The keys one block set or deleted in one store.
#[derive(Deserialize)]
struct OracleKvDiff {
    height: i64,
    store: String,
    set: Vec<[String; 2]>,
    delete: Vec<String>,
}

/// The ABCI responses of one block, from `tx-results.jsonl`.
#[derive(Deserialize)]
struct OracleBlockResults {
    height: i64,
    txs: Vec<OracleTxResult>,
}

/// One DeliverTx response. The fixture also holds `codespace`, `log`,
/// `info` and `events`, which `LastResultsHash` leaves out.
#[derive(Deserialize)]
struct OracleTxResult {
    index: usize,
    hash: String,
    code: u32,
    gas_wanted: i64,
    gas_used: i64,
    data: String,
}

#[derive(Deserialize)]
struct ReferenceBlock {
    height: i64,
    block_hash: String,
    app_hash: String,
    data_hash: String,
    next_block_hash: String,
    post_app_hash: String,
}

#[derive(Deserialize)]
struct ArchiveBlock {
    height: i64,
    result: serde_json::Value,
}

type ArchivedBlocks = BTreeMap<i64, serde_json::Value>;

#[derive(Deserialize)]
struct ReplayConfig {
    rpc: Vec<String>,
    reference_rpc: Option<String>,
    genesis: Option<PathBuf>,
    #[serde(default)]
    checkpoints: BTreeMap<u64, Checkpoint>,
}

#[derive(Deserialize)]
struct Checkpoint {
    snapshot: PathBuf,
    height: i64,
    trust_hash: String,
}

fn compare_hash(label: &str, height: i64, bytes: &[u8], expected: &str) -> anyhow::Result<()> {
    let got = hex::encode_upper(bytes);
    ensure!(
        got == expected,
        "{label} at {height}: local {got}, reference {expected}"
    );
    Ok(())
}

fn read_fixtures() -> anyhow::Result<Fixtures> {
    let fixtures: Fixtures = serde_json::from_str(FIXTURE)?;
    ensure!(
        fixtures.chain_id == CHAIN,
        "fixtures are for a different chain"
    );
    ensure!(
        fixtures.windows.iter().map(|w| w.app_version).eq(VERSIONS),
        "fixtures must cover every app version deployed on mainnet: {VERSIONS:?}"
    );
    for window in &fixtures.windows {
        ensure!(
            window.start_height > 0
                && window.blocks.len() == WINDOW_SIZE
                && window
                    .blocks
                    .iter()
                    .map(|b| b.height)
                    .eq((window.start_height..).take(WINDOW_SIZE)),
            "app v{} fixture must contain ten contiguous positive heights",
            window.app_version
        );
        for pair in window.blocks.windows(2) {
            ensure!(
                pair[0].next_block_hash == pair[1].block_hash
                    && pair[0].post_app_hash == pair[1].app_hash,
                "fixture successor commitments disagree at height {}",
                pair[0].height
            );
        }
        for block in &window.blocks {
            for hash in [
                &block.block_hash,
                &block.app_hash,
                &block.data_hash,
                &block.next_block_hash,
                &block.post_app_hash,
            ] {
                let mut bytes = [0; 32];
                hex::decode_to_slice(hash, &mut bytes)
                    .with_context(|| format!("invalid fixture hash at {}", block.height))?;
                ensure!(
                    hex::encode_upper(bytes) == *hash,
                    "fixture hash at {} must use uppercase hex",
                    block.height
                );
            }
        }
    }
    Ok(fixtures)
}

fn read_archive() -> anyhow::Result<ArchivedBlocks> {
    let mut archive = ArchivedBlocks::new();
    for line in BufReader::new(GzDecoder::new(ARCHIVE)).lines() {
        let row: ArchiveBlock = serde_json::from_str(&line?)?;
        ensure!(
            archive.insert(row.height, row.result).is_none(),
            "duplicate archive block {}",
            row.height
        );
    }
    ensure!(
        archive.len() == VERSIONS.len() * (WINDOW_SIZE + 1),
        "unexpected number of archive fixture rows"
    );
    Ok(archive)
}

fn archived_block(archive: &ArchivedBlocks, height: i64) -> anyhow::Result<(BlockId, Block)> {
    let result = archive.get(&height).context("missing fixture block")?;
    let id = app::comet::block_id(&result["block_id"]).map_err(anyhow::Error::msg)?;
    let id = BlockId::from_proto(&id)?;
    let block = http::block_from_json(&result["block"])?;
    Ok((id, block))
}

fn verify_reference_id(height: i64, id: &BlockId, block: &mut Block) -> anyhow::Result<()> {
    ensure!(
        block.header.chain_id == CHAIN && block.header.height == height,
        "reference returned the wrong chain or height for {height}"
    );
    let block_hash = block.hash().context("reference block has no hash")?;
    ensure!(
        block_hash == id.hash,
        "reconstructed block hash at {height} is {}, archive reports {}",
        hex::encode_upper(block_hash),
        hex::encode_upper(&id.hash)
    );
    let parts = block.make_part_set(BLOCK_PART_SIZE_BYTES)?;
    ensure!(
        parts.header() == id.part_set_header,
        "reconstructed part set at {height} is {:?}, archive reports {:?}",
        parts.header(),
        id.part_set_header
    );
    Ok(())
}

fn verify_reference_block(
    version: u64,
    expected: &ReferenceBlock,
    id: &BlockId,
    block: &Block,
) -> anyhow::Result<()> {
    let height = expected.height;
    ensure!(
        block.header.version.app == version,
        "wrong app version at {height}, expected v{version}"
    );
    compare_hash("block hash", height, &id.hash, &expected.block_hash)?;
    compare_hash(
        "header app hash",
        height,
        &block.header.app_hash,
        &expected.app_hash,
    )?;
    compare_hash(
        "header data root",
        height,
        &block.header.data_hash,
        &expected.data_hash,
    )?;
    let txs: Vec<_> = block.data.txs.iter().map(|tx| tx.0.clone()).collect();
    let (eds, _) = app::proposal::verified_block_eds(version, &txs, &block.header.data_hash)
        .map_err(|error| anyhow!("rebuild data root at {height}: {error}"))?;
    ensure!(
        u64::try_from(eds.width() / 2)? == block.data.square_size,
        "reconstructed square size differs at {height}"
    );
    Ok(())
}

fn replay_window(
    config: &ReplayConfig,
    window: &Window,
    reference: &RpcClient,
    archive: &ArchivedBlocks,
) -> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    let end = window.blocks.last().context("empty fixture window")?;
    let mut follow = Command::new(BINARY);
    follow
        .arg("--home")
        .arg(home.path())
        .args(["follow", "--chain-id", CHAIN, "--rpc"])
        .arg(config.rpc.join(","))
        .arg("--stop-at-height")
        .arg(end.height.to_string())
        .args([
            "--trust-period-hours",
            "87600",
            "--keep-blocks",
            "12",
            "--pruning-keep-recent",
            "12",
            "--witness-interval",
            "1",
        ]);
    if let Some(checkpoint) = config.checkpoints.get(&window.app_version) {
        ensure!(
            (1..window.start_height).contains(&checkpoint.height),
            "checkpoint height must precede {}",
            window.start_height
        );
        let (id, mut block) = reference.block(checkpoint.height).with_context(|| {
            format!(
                "checkpoint block {} from {}",
                checkpoint.height,
                reference.remote()
            )
        })?;
        verify_reference_id(checkpoint.height, &id, &mut block)?;
        compare_hash(
            "checkpoint trust hash",
            checkpoint.height,
            &id.hash,
            &checkpoint.trust_hash,
        )?;
        follow
            .arg("--snapshot")
            .arg(&checkpoint.snapshot)
            .arg("--snapshot-height")
            .arg(checkpoint.height.to_string())
            .arg("--trust-hash")
            .arg(&checkpoint.trust_hash);
    } else if window.start_height == 1 {
        let genesis = config
            .genesis
            .as_ref()
            .context("missing canonical mainnet genesis")?;
        let bytes =
            std::fs::read(genesis).with_context(|| format!("read {}", genesis.display()))?;
        networks::verify_genesis(&networks::MAINNET, &bytes)?;
        std::fs::create_dir(home.path().join("config"))?;
        std::fs::write(home.path().join("config/genesis.json"), bytes)?;
    } else {
        return Err(anyhow!(
            "missing Go SDK checkpoint before height {}; recorded blockers: {:?}",
            window.start_height,
            replay_blockers(window)?
        ));
    }

    let primary = RpcClient::new(&config.rpc[0], Duration::from_secs(30));
    let status = primary.status().context("read primary RPC status")?;
    ensure!(
        status.network == CHAIN && status.latest_block_height >= end.height + 2,
        "primary RPC must serve mainnet through height {}",
        end.height + 2
    );
    let status = follow
        .status()
        .context("launch the current celestia-rs executable")?;
    ensure!(status.success(), "celestia-rs follow exited with {status}");
    compare_replay(home.path(), window, archive)
}

fn compare_replay(home: &Path, window: &Window, archive: &ArchivedBlocks) -> anyhow::Result<()> {
    let (blocks, states) = node::open_stores(home)?;
    let app = app::App::open(home).map_err(|error| anyhow!("open replay app: {}", error.log))?;
    let end = window.blocks.last().context("empty fixture window")?;
    let state = states
        .load()?
        .context("replay did not save consensus state")?;
    ensure!(
        blocks.height() == end.height && state.last_block_height == end.height,
        "replay stopped before or beyond the requested height {}",
        end.height
    );

    for expected in &window.blocks {
        let height = expected.height;
        let mut local = blocks
            .load_block(height)?
            .with_context(|| format!("missing local block {height}"))?;
        let (id, canonical) = archived_block(archive, height)?;
        ensure!(
            local.to_proto()?.encode_to_vec() == canonical.to_proto()?.encode_to_vec(),
            "canonical protobuf block bytes differ at {height}"
        );
        verify_reference_id(height, &id, &mut local)?;
        verify_reference_block(window.app_version, expected, &id, &local)?;
        verify_archived_successor(archive, expected, &id)?;

        let committed = app.multistore().root().get_commit_info(height)?.hash()?;
        compare_hash(
            "locally committed app hash",
            height,
            &committed,
            &expected.post_app_hash,
        )?;
        println!(
            "mainnet v{} block {height} matches (block {}, app {}, data {})",
            window.app_version, expected.block_hash, expected.post_app_hash, expected.data_hash
        );
    }

    Ok(())
}

/// Cosmos SDK commit info hash (the app hash) of store name to root hash,
/// through the production multistore. An empty root is an SDK v0.46
/// memory store (empty commit ID).
fn commit_info_hash(stores: &BTreeMap<String, String>) -> anyhow::Result<Vec<u8>> {
    let mut infos = Vec::with_capacity(stores.len());
    for (name, root) in stores {
        let commit_id = if root.is_empty() {
            None
        } else {
            let hash = hex::decode(root)?;
            Some(multistore::CommitId {
                version: 1,
                hash: hash.try_into().map_err(|_| anyhow!("root of {name}"))?,
            })
        };
        infos.push(multistore::StoreInfo {
            name: name.clone(),
            commit_id,
        });
    }
    Ok(multistore::commit_info_hash(&infos)?.to_vec())
}

/// Checks the pinned pre-state against the window's first header app hash,
/// then lists what stops celestia-rs from replaying the window from it.
fn replay_blockers(window: &Window) -> anyhow::Result<Vec<String>> {
    let height = window.start_height - 1;
    if height == 0 {
        // Genesis: the window executes from InitChain when its app
        // version runs as itself on the v0.46 driver.
        return Ok(if app::legacy::executes(window.app_version) {
            Vec::new()
        } else {
            vec![format!(
                "app version {} does not execute from genesis",
                window.app_version
            )]
        });
    }
    let Some(pre) = &window.pre_state else {
        return Ok(vec![format!("no public pre-state at height {height}")]);
    };
    ensure!(pre.height == height, "pre-state must be at height {height}");
    compare_hash(
        "pre-state commit info",
        height,
        &commit_info_hash(&pre.stores)?,
        &window.blocks[0].app_hash,
    )?;

    let go: BTreeSet<&str> = pre.stores.keys().map(String::as_str).collect();
    let rust: BTreeSet<&str> = app::stores::IAVL_STORES
        .iter()
        .copied()
        .filter(|name| app::stores::exists_at(name, window.app_version))
        .collect();
    let mut blockers = Vec::new();
    for (label, names) in [
        (
            "pre-state stores that celestia-rs does not mount",
            go.difference(&rust),
        ),
        (
            "celestia-rs stores absent from the pre-state",
            rust.difference(&go),
        ),
    ] {
        let names: Vec<&str> = names.copied().collect();
        if !names.is_empty() {
            blockers.push(format!("{label}: {}", names.join(", ")));
        }
    }
    if let Some(plan) = &pre.upgrade_plan {
        ensure!(
            plan.height == window.start_height,
            "pre-state plan {} is not due at {}",
            plan.name,
            window.start_height
        );
        let handlers = app::upgrades::Handlers::at(window.app_version);
        if !handlers.has_handler(&plan.name) {
            blockers.push(format!(
                "no celestia-rs handler for upgrade plan {} at {}",
                plan.name, plan.height
            ));
        } else if !handlers.is_ported(&plan.name) {
            blockers.push(format!(
                "celestia-rs does not port the upgrade handler of plan {} at {}",
                plan.name, plan.height
            ));
        }
    }
    Ok(blockers)
}

#[test]
fn historical_replay_blockers_match_record() -> anyhow::Result<()> {
    let fixtures = read_fixtures()?;
    ensure!(
        fixtures.windows.len() == EXPECTED_BLOCKERS.len(),
        "blocker record size"
    );
    for (window, (version, expected)) in fixtures.windows.iter().zip(EXPECTED_BLOCKERS) {
        ensure!(window.app_version == version, "blocker record order");
        let blockers = replay_blockers(window)?;
        println!("mainnet v{version}: {blockers:?}");
        ensure!(
            blockers == expected,
            "app v{version} replay blockers changed: {blockers:#?}"
        );
    }
    Ok(())
}

fn gunzip_checked(label: &str, bytes: &[u8], sha256: Option<&String>) -> anyhow::Result<String> {
    let mut text = String::new();
    std::io::Read::read_to_string(&mut GzDecoder::new(bytes), &mut text)?;
    let expected = sha256.with_context(|| format!("no SHA-256 recorded for {label}"))?;
    ensure!(
        hex::encode(hash::sha256(text.as_bytes())) == *expected,
        "{label} does not match its recorded SHA-256"
    );
    Ok(text)
}

/// The Go v1 oracle's store roots rebuild its app hashes, which equal the
/// pinned mainnet app hashes of window 1, and its KV dumps agree with them.
#[test]
fn v1_oracle_store_roots_match_record() -> anyhow::Result<()> {
    let fixtures = read_fixtures()?;
    let window = &fixtures.windows[0];
    ensure!(
        window.app_version == 1 && window.start_height == 1,
        "window 1 must start at genesis"
    );
    let oracle: V1Oracle = serde_json::from_str(V1_ORACLE)?;
    ensure!(oracle.chain_id == CHAIN, "v1 oracle is for another chain");
    ensure!(
        Some(oracle.genesis_sha256.as_str()) == networks::MAINNET.genesis_sha256,
        "v1 oracle ran on a different genesis"
    );
    ensure!(
        oracle.init_chain_app_hash == window.blocks[0].app_hash,
        "v1 oracle InitChain app hash {} differs from header 1",
        oracle.init_chain_app_hash
    );
    ensure!(
        oracle
            .heights
            .iter()
            .map(|h| h.height)
            .eq(0..=i64::try_from(WINDOW_SIZE)?),
        "v1 oracle must cover heights 0 to {WINDOW_SIZE}"
    );
    for h in &oracle.heights {
        ensure!(
            h.stores.keys().map(String::as_str).eq(V1_STORES),
            "v1 oracle stores at {}: {:?}",
            h.height,
            h.stores.keys()
        );
        for (name, root) in &h.stores {
            let len = if name == "mem_capability" { 0 } else { 64 };
            ensure!(
                root.len() == len && root.to_uppercase() == *root,
                "v1 oracle root of {name} at {}",
                h.height
            );
        }
        compare_hash(
            "v1 oracle commit info",
            h.height,
            &commit_info_hash(&h.stores)?,
            &h.app_hash,
        )?;
        if let Some(block) = usize::try_from(h.height - 1)
            .ok()
            .and_then(|i| window.blocks.get(i))
        {
            ensure!(
                h.app_hash == block.post_app_hash,
                "v1 oracle app hash at {} is {}, mainnet {}",
                h.height,
                h.app_hash,
                block.post_app_hash
            );
        }
    }

    let state = read_oracle_height_0(&oracle)?;
    check_oracle_diffs(&oracle, state)
}

/// Height-0 KV pairs of every store except the bulk ones, by store.
fn read_oracle_height_0(oracle: &V1Oracle) -> anyhow::Result<OracleState> {
    let h0 = &oracle.heights[0];
    let modules = gunzip_checked(
        "kv-height-0-modules.jsonl",
        V1_ORACLE_KV_MODULES,
        oracle.kv.get("kv-height-0-modules.jsonl"),
    )?;
    let mut state = OracleState::new();
    for line in modules.lines() {
        let dump: OracleStoreDump = serde_json::from_str(line)?;
        ensure!(
            h0.stores.get(&dump.store) == Some(&dump.root),
            "height-0 dump root of {}",
            dump.store
        );
        ensure!(
            dump.kv.windows(2).all(|p| p[0][0] < p[1][0]),
            "height-0 dump of {} is not sorted",
            dump.store
        );
        ensure!(
            h0.keys.get(&dump.store) == Some(&dump.kv.len()),
            "height-0 key count of {}",
            dump.store
        );
        let kv: BTreeMap<String, String> = dump.kv.into_iter().map(|[k, v]| (k, v)).collect();
        state.insert(dump.store, kv);
    }
    let dumped: Vec<&str> = V1_STORES
        .into_iter()
        .filter(|s| *s != "mem_capability" && !V1_BULK_STORES.contains(s))
        .collect();
    ensure!(
        state.keys().map(String::as_str).eq(dumped.iter().copied()),
        "height-0 dump stores: {:?}",
        state.keys()
    );
    Ok(state)
}

/// Applies each block's KV diff to the height-0 dump: a store whose keys
/// change must change its root, a store with no diff at a height keeps
/// its root, and the dumped stores keep the recorded key counts.
fn check_oracle_diffs(oracle: &V1Oracle, mut state: OracleState) -> anyhow::Result<()> {
    let diffs = gunzip_checked(
        "kv-diffs.jsonl",
        V1_ORACLE_KV_DIFFS,
        oracle.kv.get("kv-diffs.jsonl"),
    )?;
    let mut changed: BTreeMap<i64, Vec<OracleKvDiff>> = BTreeMap::new();
    for line in diffs.lines() {
        let diff: OracleKvDiff = serde_json::from_str(line)?;
        ensure!(
            !diff.set.is_empty() || !diff.delete.is_empty(),
            "empty diff for {} at {}",
            diff.store,
            diff.height
        );
        changed.entry(diff.height).or_default().push(diff);
    }
    for pair in oracle.heights.windows(2) {
        let (before, after) = (&pair[0], &pair[1]);
        let mut touched = BTreeSet::new();
        for diff in changed.remove(&after.height).unwrap_or_default() {
            ensure!(
                touched.insert(diff.store.clone()),
                "duplicate diff for {} at {}",
                diff.store,
                after.height
            );
            if let Some(kv) = state.get_mut(&diff.store) {
                for [k, v] in diff.set {
                    kv.insert(k, v);
                }
                for k in diff.delete {
                    ensure!(kv.remove(&k).is_some(), "deleted key absent: {k}");
                }
            }
        }
        for name in V1_STORES {
            let same_root = before.stores[name] == after.stores[name];
            ensure!(
                !(touched.contains(name) && same_root),
                "{name} changed keys but kept its root at {}",
                after.height
            );
            ensure!(
                touched.contains(name) || same_root,
                "{name} root changed at {} without a KV change",
                after.height
            );
        }
        for (name, kv) in &state {
            ensure!(
                after.keys.get(name) == Some(&kv.len()),
                "{name} key count at {}",
                after.height
            );
        }
    }
    ensure!(changed.is_empty(), "diffs for unknown heights");
    Ok(())
}

/// Messages of mainnet blocks 2 to 10 (v1). Each has an empty
/// `Msg...Response` in cosmos-sdk v0.46 and ibc-go v6, so a successful
/// tx's v0.46 `TxMsgData` follows from its message types alone.
const V1_EMPTY_RESPONSE_MSGS: [&str; 7] = [
    "/cosmos.bank.v1beta1.MsgSend",
    "/cosmos.bank.v1beta1.MsgMultiSend",
    "/cosmos.staking.v1beta1.MsgDelegate",
    "/cosmos.staking.v1beta1.MsgCreateValidator",
    "/ibc.core.client.v1.MsgCreateClient",
    "/ibc.core.client.v1.MsgUpdateClient",
    "/ibc.core.connection.v1.MsgConnectionOpenTry",
];

/// The oracle's DeliverTx results of blocks 1 to 10, shaped by the
/// legacy driver, hash to the `LastResultsHash` of the next mainnet
/// header (Tendermint v0.34: code, data, gas wanted, gas used).
#[test]
fn v1_oracle_tx_results_rebuild_last_results_hash() -> anyhow::Result<()> {
    use celestia_proto::cosmos::tx::v1beta1::{TxBody, TxRaw};

    let oracle: V1Oracle = serde_json::from_str(V1_ORACLE)?;
    let archive = read_archive()?;
    for h in oracle.heights.iter().filter(|h| h.height >= 1) {
        let (_, block) = archived_block(&archive, h.height)?;
        let (_, next) = archived_block(&archive, h.height + 1)?;
        ensure!(
            block.data.txs.len() == h.tx_results.len(),
            "v1 oracle tx results at {}",
            h.height
        );
        let mut results = Vec::with_capacity(h.tx_results.len());
        for (tx, [code, gas_wanted, gas_used]) in block.data.txs.iter().zip(&h.tx_results) {
            let result = if *code == 0 {
                let body = TxBody::decode(TxRaw::decode(tx.0.as_slice())?.body_bytes.as_slice())?;
                let mut msg_responses = Vec::new();
                for msg in &body.messages {
                    ensure!(
                        V1_EMPTY_RESPONSE_MSGS.contains(&msg.type_url.as_str()),
                        "unexpected v1 message {}",
                        msg.type_url
                    );
                    msg_responses.push(celestia_proto::Any {
                        type_url: format!("{}Response", msg.type_url),
                        value: Vec::new(),
                    });
                }
                Ok(app::legacy::TxSuccess {
                    msg_responses,
                    log: String::new(),
                })
            } else {
                Err(sdk::SdkError::new("sdk", u32::try_from(*code)?, ""))
            };
            results.push(app::legacy::deliver_tx_result(app::legacy::TxOutcome {
                gas_wanted: u64::try_from(*gas_wanted)?,
                gas_used: u64::try_from(*gas_used)?,
                events: Vec::new(),
                result,
            }));
        }
        compare_hash(
            "v1 last results hash",
            h.height,
            &ctypes::results_hash(&results),
            &hex::encode_upper(&next.header.last_results_hash),
        )?;
    }
    Ok(())
}

/// The Go v1 oracle's DeliverTx results hash to the `last_results_hash`
/// of each next mainnet header, and agree with `store-roots.json`.
#[test]
fn v1_oracle_tx_results_match_last_results_hash() -> anyhow::Result<()> {
    let oracle: V1Oracle = serde_json::from_str(V1_ORACLE)?;
    let text = gunzip_checked(
        "tx-results.jsonl",
        V1_ORACLE_TX_RESULTS,
        Some(&oracle.tx_results_sha256),
    )?;
    let archive = read_archive()?;
    let mut heights = Vec::new();
    for line in text.lines() {
        let block: OracleBlockResults = serde_json::from_str(line)?;
        let height = block.height;
        heights.push(height);
        let (id, mut archived) = archived_block(&archive, height)?;
        verify_reference_id(height, &id, &mut archived)?;
        let (next_id, mut next) = archived_block(&archive, height + 1)?;
        verify_reference_id(height + 1, &next_id, &mut next)?;
        ensure!(
            block.txs.len() == archived.data.txs.len(),
            "v1 oracle has {} tx results at {height}, block has {} txs",
            block.txs.len(),
            archived.data.txs.len()
        );
        let recorded = oracle
            .heights
            .iter()
            .find(|h| h.height == height)
            .with_context(|| format!("no store roots at {height}"))?;
        ensure!(
            recorded.tx_results.len() == block.txs.len()
                && recorded.tx_data.len() == block.txs.len(),
            "store-roots tx results at {height}"
        );
        let mut results = Vec::new();
        for (i, (tx, archived_tx)) in block.txs.iter().zip(&archived.data.txs).enumerate() {
            ensure!(tx.index == i, "tx index {} at {height}:{i}", tx.index);
            compare_hash("tx hash", height, &hash::sha256(&archived_tx.0), &tx.hash)?;
            ensure!(
                recorded.tx_results[i] == [i64::from(tx.code), tx.gas_wanted, tx.gas_used]
                    && recorded.tx_data[i] == tx.data,
                "tx {height}:{i} differs from store-roots.json"
            );
            results.push(ctypes::proto::abci::ExecTxResult {
                code: tx.code,
                data: hex::decode(&tx.data)?,
                gas_wanted: tx.gas_wanted,
                gas_used: tx.gas_used,
                ..Default::default()
            });
        }
        compare_hash(
            "v1 oracle results hash",
            height,
            &ctypes::results_hash(&results),
            &hex::encode_upper(&next.header.last_results_hash),
        )?;
    }
    ensure!(
        heights.iter().copied().eq(1..=i64::try_from(WINDOW_SIZE)?),
        "v1 oracle tx results must cover heights 1 to {WINDOW_SIZE}: {heights:?}"
    );
    Ok(())
}

#[test]
fn mainnet_fixture_covers_every_deployed_version() -> anyhow::Result<()> {
    read_fixtures()?;
    Ok(())
}

#[test]
fn historical_mainnet_block_reconstruction() -> anyhow::Result<()> {
    let fixtures = read_fixtures()?;
    let archive = read_archive()?;
    let mut failures = Vec::new();
    for window in fixtures.windows {
        for expected in window.blocks {
            let result = archived_block(&archive, expected.height).and_then(|(id, mut block)| {
                verify_reference_id(expected.height, &id, &mut block)?;
                verify_reference_block(window.app_version, &expected, &id, &block)?;
                verify_archived_successor(&archive, &expected, &id)
            });
            match result {
                Ok(()) => println!(
                    "mainnet v{} block {}: reconstructed block ID and data root match",
                    window.app_version, expected.height
                ),
                Err(error) => failures.push(format!(
                    "app v{} height {}: {error:#}",
                    window.app_version, expected.height
                )),
            }
        }
    }
    ensure!(
        failures.is_empty(),
        "{} archival blocks failed reconstruction:\n{}",
        failures.len(),
        failures.join("\n")
    );
    Ok(())
}

fn verify_archived_successor(
    archive: &ArchivedBlocks,
    expected: &ReferenceBlock,
    id: &BlockId,
) -> anyhow::Result<()> {
    let height = expected.height;
    let next_height = height + 1;
    let next = archive
        .get(&next_height)
        .context("missing fixture successor")?;
    let header = http::header_from_json(&next["block"]["header"])?;
    ensure!(
        header.height == next_height && header.chain_id == CHAIN && header.last_block_id == *id,
        "fixture successor does not commit to block {height}"
    );
    let hash = header.hash().context("fixture successor has no hash")?;
    compare_hash(
        "successor block hash",
        next_height,
        &hash,
        &expected.next_block_hash,
    )?;
    compare_hash(
        "successor app hash",
        next_height,
        &header.app_hash,
        &expected.post_app_hash,
    )
}

#[test]
#[ignore = "requires archival mainnet RPC and historical application checkpoints"]
fn historical_mainnet_replay() -> anyhow::Result<()> {
    let config_path = std::env::var_os("CELESTIA_MAINNET_REPLAY_CONFIG")
        .context("set CELESTIA_MAINNET_REPLAY_CONFIG to the replay inputs JSON file")?;
    let config: ReplayConfig = serde_json::from_slice(&std::fs::read(config_path)?)?;
    ensure!(!config.rpc.is_empty(), "configure at least one archive RPC");
    let fixtures = read_fixtures()?;
    let archive = read_archive()?;
    let only = std::env::var("CELESTIA_MAINNET_REPLAY_VERSION")
        .ok()
        .map(|value| value.parse::<u64>())
        .transpose()?;
    if let Some(version) = only {
        ensure!(
            VERSIONS.contains(&version),
            "app v{version} is absent from mainnet history"
        );
        println!("scope: mainnet app v{version} only");
    }
    let reference = RpcClient::new(
        config.reference_rpc.as_deref().unwrap_or(&fixtures.rpc),
        Duration::from_secs(30),
    );
    let binary_hash =
        hash::sha256(&std::fs::read(BINARY).context("read current celestia-rs executable")?);
    println!(
        "celestia-rs executable SHA-256 {}",
        hex::encode(binary_hash)
    );
    let mut failures = Vec::new();
    for window in &fixtures.windows {
        if only.is_some_and(|version| version != window.app_version) {
            continue;
        }
        if let Err(error) = replay_window(&config, window, &reference, &archive) {
            failures.push(format!("app v{}: {error:#}", window.app_version));
        }
    }
    ensure!(
        failures.is_empty(),
        "mainnet replay failed:\n{}",
        failures.join("\n")
    );
    Ok(())
}
