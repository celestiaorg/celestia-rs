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

/// The v10 app opens immediately after the upgrade commit, before its
/// added stores have a committed version. Exercise that boundary on disk,
/// including the neighboring commits, rather than only a fixture's midpoint.
#[test]
fn restart_at_v9_upgrade_preserves_version_stores_and_parity() -> anyhow::Result<()> {
    for scenario in ["v9-upgrade", "v9-upgrade-boundary"] {
        let exchanges = load_exchanges(scenario)?;
        let switch = exchanges
            .iter()
            .find_map(|ex| match (&ex.request.value, &ex.response.value) {
                (
                    Some(request::Value::FinalizeBlock(req)),
                    Some(response::Value::FinalizeBlock(resp)),
                ) if resp
                    .consensus_param_updates
                    .as_ref()
                    .and_then(|cp| cp.version.as_ref())
                    .is_some_and(|version| version.app == 10) =>
                {
                    Some(req.height)
                }
                _ => None,
            })
            .expect("recorded v9 to v10 upgrade");
        let expected = reference(scenario)?;
        assert_parity(scenario, &expected);
        let hashes = go_app_hashes(&exchanges);
        for backend in multistore::kvdb::BackendKind::ALL {
            for checkpoint in [switch - 1, switch, switch + 1] {
                let case = format!("{scenario} on {backend}, restart at {checkpoint}");
                let home = tempfile::tempdir()?;
                let mut opts = options();
                opts.db.backend = *backend;
                let mut app = App::open_with(home.path(), opts.clone())?;
                let mut before = replay_into(scenario, &mut app, 0, Some(checkpoint))?;
                assert_parity(&case, &before);
                assert_eq!(before.matched_heights.last(), Some(&checkpoint), "{case}");
                let info = app.info();
                drop(app);

                let mut app = App::open_with(home.path(), opts)?;
                assert_eq!(app.info(), info, "{case}: Info changed on reopen");
                assert_eq!(info.last_block_height, checkpoint, "{case}");
                assert_eq!(info.app_version, if checkpoint < switch { 9 } else { 10 });
                let go_hash = &hashes
                    .iter()
                    .find(|(height, _)| *height == checkpoint)
                    .expect("recorded checkpoint")
                    .1;
                assert_eq!(&info.last_block_app_hash, go_hash, "{case}");
                assert_eq!(app.chain_id(), "test", "{case}: chain id lost");
                let committed = app
                    .multistore()
                    .root()
                    .last_commit_info()
                    .expect("committed checkpoint");
                for store in ["fibre", "valaddr"] {
                    assert_eq!(
                        app.multistore().is_mounted(store),
                        checkpoint >= switch,
                        "{case}: {store} must mount immediately after the upgrade commit"
                    );
                    assert_eq!(
                        committed.store_info(store).is_some(),
                        checkpoint > switch,
                        "{case}: {store} must first commit at the next height"
                    );
                }

                let after = replay_into(scenario, &mut app, checkpoint, None)?;
                assert_parity(&case, &after);
                assert_eq!(
                    after.matched_heights.first(),
                    Some(&(checkpoint + 1)),
                    "{case}"
                );
                before.matched_heights.extend(after.matched_heights);
                assert_eq!(before.matched_heights, expected.matched_heights, "{case}");
                assert!(
                    before.init_chain_diffs.is_empty(),
                    "{case}: InitChain differs"
                );
                assert!(
                    before.event_log_diffs.is_empty(),
                    "{case}: pre-restart events differ"
                );
                assert!(
                    after.event_log_diffs.is_empty(),
                    "{case}: post-restart events differ"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn snapshot_restore_resumes_replay_with_full_parity() -> anyhow::Result<()> {
    snapshot_restore_resumes_replay("staking", 0)
}

/// A snapshot of an app-version-9 state (no fibre / valaddr stores) restores
/// into a fresh node when the offer carries the trusted app version 9, as
/// state sync from mainnet (v9.0.8) does. Regression: the restore used to
/// fail at the last chunk with `unknown store: fibre`.
#[test]
fn v9_snapshot_restore_resumes_replay_with_full_parity() -> anyhow::Result<()> {
    snapshot_restore_resumes_replay("v9-staking", 9)
}

fn snapshot_restore_resumes_replay(scenario: &str, app_version: u64) -> anyhow::Result<()> {
    snapshot_restore_after_offers(scenario, app_version, false)
}

/// A rejected v9 offer must not change a later v10 restore, and an accepted
/// but abandoned v9 offer must be replaceable with a v10 offer.
#[test]
fn snapshot_restore_recovers_after_v9_offers() -> anyhow::Result<()> {
    snapshot_restore_after_offers("staking", 10, true)
}

fn snapshot_restore_after_offers(
    scenario: &str,
    app_version: u64,
    prior_v9_offers: bool,
) -> anyhow::Result<()> {
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
    src.wait_for_snapshot();
    let listed = src.list_snapshots().snapshots;
    assert_eq!(listed.len(), 1);
    let snapshot = listed[0].clone();
    assert_eq!(snapshot.height, u64::try_from(at)?);

    // Fresh node state-syncs through the ABCI calls.
    let dst_home = tempfile::tempdir()?;
    let mut dst_opts = options();
    dst_opts.chain_id = Some(src.chain_id().to_owned());
    let mut dst = App::open_with(dst_home.path(), dst_opts)?;
    if prior_v9_offers {
        use celestia_proto::celestia_core_proto::abci::RequestOfferSnapshot;
        let rejected = dst.offer_snapshot(RequestOfferSnapshot {
            snapshot: None,
            app_version: 9,
            ..Default::default()
        });
        assert_eq!(rejected.result, 3, "reject missing snapshot");
        for name in app::stores::ADDED_IN_V10 {
            assert!(dst.multistore().is_mounted(name));
        }
        // Offer validation selects the store set; no chunks have arrived,
        // so the attempt can still be replaced by the trusted v10 offer.
        let abandoned = dst.offer_snapshot(RequestOfferSnapshot {
            snapshot: Some(snapshot.clone()),
            app_version: 9,
            ..Default::default()
        });
        assert_eq!(abandoned.result, 1, "accept first attempt");
        for name in app::stores::ADDED_IN_V10 {
            assert!(!dst.multistore().is_mounted(name));
        }
    }
    let offer = dst.offer_snapshot(
        celestia_proto::celestia_core_proto::abci::RequestOfferSnapshot {
            snapshot: Some(snapshot.clone()),
            app_hash: Vec::new(),
            app_version,
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

/// Go `SnapshotIfApplicable` snapshots in a goroutine: Commit of the
/// snapshot height returns while the snapshot is still being written, the
/// chain keeps committing (and pruning) meanwhile, and the snapshot height
/// is not pruned until the snapshot is done. Regression: Mocha height
/// 1,722,000 blocked consensus for 10 s inside Commit.
#[test]
fn snapshot_is_written_in_background_while_commits_continue() -> anyhow::Result<()> {
    let scenario = "staking";
    let hashes = go_app_hashes(&load_exchanges(scenario)?);
    let last = hashes.last().map_or(0, |(h, _)| *h);
    let at = last / 2;
    let src_home = tempfile::tempdir()?;
    let mut opts = options();
    opts.snapshots = snapshots::SnapshotOptions {
        interval: u64::try_from(at)?,
        keep_recent: 1,
    };
    let mut src = App::open_with(src_home.path(), opts)?;
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    src.snapshot_manager_mut()
        .expect("snapshot manager")
        .set_snapshot_hook(move |height| {
            let _ = started_tx.send(height);
            let _ = release_rx.lock().map(|rx| rx.recv());
        });

    // Every Commit returns although the snapshot of `at` is blocked.
    assert_parity(scenario, &replay_into(scenario, &mut src, 0, None)?);
    assert_eq!(started_rx.recv()?, u64::try_from(at)?);
    assert!(src.list_snapshots().snapshots.is_empty());
    // Pruning (keep_recent 2) ran past `at` but kept it for the snapshot.
    let versions = src.multistore().root().available_versions();
    assert!(versions.contains(&at), "{versions:?}");
    assert!(versions.get(1).is_some_and(|v| *v > at + 1), "{versions:?}");

    release_tx.send(())?;
    src.wait_for_snapshot();
    let listed = src.list_snapshots().snapshots;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].height, u64::try_from(at)?);

    // The snapshot restores to Go's state at `at`.
    let dst_home = tempfile::tempdir()?;
    let mut dst_opts = options();
    dst_opts.chain_id = Some(src.chain_id().to_owned());
    let mut dst = App::open_with(dst_home.path(), dst_opts)?;
    let snapshot = listed[0].clone();
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
    let go_hash = hashes
        .iter()
        .find(|(h, _)| *h == at)
        .map(|(_, x)| x.clone());
    assert_eq!(Some(dst.info().last_block_app_hash), go_hash);
    Ok(())
}
