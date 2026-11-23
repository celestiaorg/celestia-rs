//! Live historical parity check for the current `celestia-rs` executable.
//! Inputs and replaying individual windows are documented in conformance/README.md.

#![allow(clippy::print_stdout)]

use std::collections::BTreeMap;
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

const CHAIN: &str = "celestia";
const VERSIONS: [u64; 9] = [1, 2, 3, 4, 5, 6, 8, 9, 10];
const WINDOW_SIZE: usize = 10;
const BINARY: &str = env!("CARGO_BIN_EXE_celestia-rs");
const FIXTURE: &str = include_str!("../../../../conformance/fixtures/mainnet/replay-windows.json");
const ARCHIVE: &[u8] = include_bytes!("../../../../conformance/fixtures/mainnet/blocks.jsonl.gz");

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
    blocks: Vec<ReferenceBlock>,
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
            "missing Go SDK checkpoint before height {}",
            window.start_height
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
