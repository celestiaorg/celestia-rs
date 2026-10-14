//! ADR 0002 acceptance: every devnet scenario replays with full parity on
//! every persistent backend (RocksDB, redb), and a replay interrupted halfway
//! (process restart) resumes from disk on the default backend and still
//! matches Go at every height.

use abci_server::record::Exchange;
use app::{App, AppOptions};
use celestia_proto::celestia_core_proto::abci::{request, response};
use devnet_conformance::load_exchanges;
use devnet_conformance::replay::{Report, replay_into};

/// Every recorded scenario (new ones are picked up automatically).
fn scenarios() -> Vec<String> {
    let dir = devnet_conformance::scenario_dir("");
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|e| e.path().join("abci.rec").exists())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// The in-memory replay is the reference: a storage-backed replay must
/// reach exactly the same heights with the same outcome.
fn reference(scenario: &str) -> anyhow::Result<Report> {
    devnet_conformance::replay::replay(scenario)
}

fn options() -> AppOptions {
    let mut options = AppOptions::default();
    // Small cache and aggressive pruning exercise the lazy-loading and
    // orphan paths; fsync is skipped to keep the test fast.
    options.store.node_cache_size = 2_000;
    options.store.pruning = multistore::PruningOptions {
        keep_recent: 2,
        keep_every: 0,
        interval: 3,
    };
    options.db.sync_writes = false;
    options
}

fn assert_parity(scenario: &str, r: &Report) {
    assert!(r.errors.is_empty(), "{scenario}: {:?}", r.errors);
    assert_eq!(
        r.first_mismatch, None,
        "{scenario}: stores {:#?}",
        r.store_diffs
    );
}

/// Go's app hash after committing each height (FinalizeBlock responses).
fn go_app_hashes(exchanges: &[Exchange]) -> Vec<(i64, Vec<u8>)> {
    exchanges
        .iter()
        .filter_map(|ex| match (&ex.request.value, &ex.response.value) {
            (Some(request::Value::FinalizeBlock(r)), Some(response::Value::FinalizeBlock(go))) => {
                Some((r.height, go.app_hash.clone()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn all_scenarios_replay_on_every_backend() -> anyhow::Result<()> {
    let scenarios = scenarios();
    assert!(scenarios.len() >= 4, "{scenarios:?}");
    for scenario in &scenarios {
        let expected = reference(scenario)?;
        for backend in multistore::kvdb::BackendKind::ALL {
            let home = tempfile::tempdir()?;
            let mut opts = options();
            opts.db.backend = *backend;
            let mut app = App::open_with(home.path(), opts)?;
            let r = replay_into(scenario, &mut app, 0, None)?;
            assert_eq!(
                r.matched_heights, expected.matched_heights,
                "{scenario} on {backend}"
            );
            assert_eq!(
                r.first_mismatch, expected.first_mismatch,
                "{scenario} on {backend}"
            );
            assert_eq!(r.errors, expected.errors, "{scenario} on {backend}");
            assert!(
                !r.matched_heights.is_empty(),
                "{scenario}: nothing replayed"
            );
        }
    }
    Ok(())
}

#[test]
fn restart_halfway_resumes_from_disk_with_full_parity() -> anyhow::Result<()> {
    for scenario in &scenarios() {
        let expected = reference(scenario)?;
        if expected.first_mismatch.is_some() || !expected.errors.is_empty() {
            // Not at parity yet in memory; covered by the equivalence test.
            continue;
        }
        let hashes = go_app_hashes(&load_exchanges(scenario)?);
        let last = hashes.last().map_or(0, |(h, _)| *h);
        let half = last / 2;
        assert!(half > 0, "{scenario}: too short");
        let home = tempfile::tempdir()?;
        {
            let mut app = App::open_with(home.path(), options())?;
            let r = replay_into(scenario, &mut app, 0, Some(half))?;
            assert_parity(scenario, &r);
            assert_eq!(r.matched_heights.last(), Some(&half), "{scenario}");
        }
        // Restart: the handshake must report the last committed height and
        // Go's app hash for it, so celestia-core replays only the rest.
        let mut app = App::open_with(home.path(), options())?;
        let info = app.info();
        assert_eq!(info.last_block_height, half, "{scenario}");
        let go_hash = hashes
            .iter()
            .find(|(h, _)| *h == half)
            .map(|(_, hash)| hash.clone());
        assert_eq!(Some(info.last_block_app_hash), go_hash, "{scenario}");
        assert!(!app.chain_id().is_empty(), "{scenario}: chain id lost");
        let r = replay_into(scenario, &mut app, half, None)?;
        assert_parity(scenario, &r);
        assert_eq!(r.matched_heights.first(), Some(&(half + 1)), "{scenario}");
        assert_eq!(r.matched_heights.last(), Some(&last), "{scenario}");
    }
    Ok(())
}

#[test]
fn snapshot_restore_resumes_replay_with_full_parity() -> anyhow::Result<()> {
    let scenario = "staking";
    let hashes = go_app_hashes(&load_exchanges(scenario)?);
    let last = hashes.last().map_or(0, |(h, _)| *h);
    let at = last / 2;
    // Source node takes a snapshot at `at`.
    let src_home = tempfile::tempdir()?;
    let mut opts = options();
    opts.snapshots = snapshots::SnapshotOptions {
        interval: u64::try_from(at)?,
        keep_recent: 1,
    };
    let mut src = App::open_with(src_home.path(), opts)?;
    assert_parity(scenario, &replay_into(scenario, &mut src, 0, Some(at))?);
    let listed = src.list_snapshots().snapshots;
    assert_eq!(listed.len(), 1);
    let snapshot = listed[0].clone();
    assert_eq!(snapshot.height, u64::try_from(at)?);

    // Fresh node state-syncs through the ABCI calls.
    let dst_home = tempfile::tempdir()?;
    let mut dst_opts = options();
    dst_opts.chain_id = Some(src.chain_id().to_owned());
    let mut dst = App::open_with(dst_home.path(), dst_opts)?;
    let offer = dst.offer_snapshot(
        celestia_proto::celestia_core_proto::abci::RequestOfferSnapshot {
            snapshot: Some(snapshot.clone()),
            app_hash: Vec::new(),
            app_version: 0,
        },
    );
    assert_eq!(offer.result, 1, "accept");
    for index in 0..snapshot.chunks {
        let chunk = src
            .load_snapshot_chunk(
                &celestia_proto::celestia_core_proto::abci::RequestLoadSnapshotChunk {
                    height: snapshot.height,
                    format: snapshot.format,
                    chunk: index,
                },
            )
            .chunk;
        let applied = dst.apply_snapshot_chunk(
            celestia_proto::celestia_core_proto::abci::RequestApplySnapshotChunk {
                index,
                chunk,
                sender: "src".into(),
            },
        );
        assert_eq!(applied.result, 1, "chunk {index} accepted");
    }
    let info = dst.info();
    assert_eq!(info.last_block_height, at);
    assert_eq!(info.last_block_app_hash, src.info().last_block_app_hash);
    drop(dst);
    // ... and survives a restart, then follows the chain to the end.
    let mut dst = App::open_with(dst_home.path(), options())?;
    assert_eq!(dst.info().last_block_height, at);
    let r = replay_into(scenario, &mut dst, at, None)?;
    assert_parity(scenario, &r);
    assert_eq!(r.matched_heights.last(), Some(&last));
    Ok(())
}
