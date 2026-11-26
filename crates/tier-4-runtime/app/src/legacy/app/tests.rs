#![allow(clippy::expect_used, clippy::unwrap_used)]

use celestia_proto::celestia_core_proto::abci::{self, ValidatorUpdate};
use celestia_proto::celestia_core_proto::tendermint::types::{
    BlockParams, ConsensusParams, EvidenceParams, Header as TmHeader, ValidatorParams,
    VersionParams,
};
use sdk::Context;

use super::super::{Modules, TxOutcome, TxSuccess};
use super::AppModules;
use crate::{App, AppOptions};

/// Writes the height in BeginBlock and each transaction's bytes in
/// DeliverTx, so every block changes the app hash.
#[derive(Debug, Default)]
struct Counter;

impl Modules for Counter {
    fn init_genesis(
        &mut self,
        ctx: &mut Context<'_>,
        app_state: &[u8],
    ) -> sdk::Result<Vec<ValidatorUpdate>> {
        ctx.set("blob", b"genesis".to_vec(), app_state.to_vec())?;
        Ok(Vec::new())
    }

    fn begin_block(&mut self, ctx: &mut Context<'_>, header: &TmHeader) -> sdk::Result<()> {
        ctx.set(
            "blob",
            b"height".to_vec(),
            header.height.to_be_bytes().to_vec(),
        )?;
        ctx.set(
            "qgb",
            b"chain".to_vec(),
            header.chain_id.as_bytes().to_vec(),
        )
    }

    fn deliver_tx(
        &mut self,
        ctx: &mut Context<'_>,
        _: &mut kvstore::GasMeter,
        tx: &[u8],
    ) -> TxOutcome {
        let result = ctx
            .set("bank", tx.to_vec(), b"seen".to_vec())
            .map(|()| TxSuccess::default());
        TxOutcome {
            gas_wanted: 10,
            gas_used: 5,
            events: Vec::new(),
            result,
        }
    }

    fn end_block(&mut self, _: &mut Context<'_>) -> sdk::Result<Vec<ValidatorUpdate>> {
        Ok(Vec::new())
    }
}

fn counter() -> AppModules {
    Box::new(Counter)
}

fn consensus_params(app: u64) -> ConsensusParams {
    ConsensusParams {
        block: Some(BlockParams {
            max_bytes: 1_974_272,
            max_gas: -1,
        }),
        evidence: Some(EvidenceParams {
            max_age_num_blocks: 120_961,
            max_age_duration: Some(pbjson_types::Duration {
                seconds: 1_814_400,
                nanos: 0,
            }),
            max_bytes: 1_048_576,
        }),
        validator: Some(ValidatorParams {
            pub_key_types: vec!["ed25519".to_owned()],
        }),
        version: Some(VersionParams { app }),
        ..ConsensusParams::default()
    }
}

fn init_request(app: u64) -> abci::RequestInitChain {
    abci::RequestInitChain {
        chain_id: "celestia".to_owned(),
        initial_height: 1,
        consensus_params: Some(consensus_params(app)),
        app_state_bytes: b"{}".to_vec(),
        ..abci::RequestInitChain::default()
    }
}

fn finalize_request(height: i64) -> abci::RequestFinalizeBlock {
    abci::RequestFinalizeBlock {
        height,
        hash: vec![7; 32],
        txs: vec![height.to_be_bytes().to_vec(), b"tx".to_vec()],
        time: Some(pbjson_types::Timestamp {
            seconds: 1_698_000_000 + height,
            nanos: 0,
        }),
        header: Some(TmHeader {
            chain_id: "celestia".to_owned(),
            height,
            ..TmHeader::default()
        }),
        ..abci::RequestFinalizeBlock::default()
    }
}

/// Runs blocks `heights` and returns each committed app hash.
fn run(app: &mut App, heights: std::ops::RangeInclusive<i64>) -> Vec<Vec<u8>> {
    heights
        .map(|h| {
            let res = app.finalize_block(finalize_request(h)).unwrap();
            assert_eq!(res.tx_results.len(), 2);
            assert!(
                res.tx_results
                    .iter()
                    .all(|r| r.code == 0 && r.gas_used == 5)
            );
            app.commit().unwrap();
            let info = app.info();
            assert_eq!(info.last_block_height, h);
            assert_eq!(info.last_block_app_hash, res.app_hash);
            res.app_hash
        })
        .collect()
}

fn store_names(app: &App) -> Vec<String> {
    let info = app.multistore().root().last_commit_info().unwrap();
    info.store_infos.iter().map(|s| s.name.clone()).collect()
}

fn v1_commit_info_names() -> Vec<String> {
    let v1 = sdk::version::Release::for_app_version(1).unwrap();
    let mut names: Vec<String> = v1
        .stores
        .iter()
        .chain(v1.commit_memory_stores)
        .map(|s| (*s).to_owned())
        .collect();
    names.sort_unstable();
    names
}

/// A genesis at app version 1 mounts the 17 v1 stores and
/// `mem_capability` in place of the v10 stores, and InitChain,
/// FinalizeBlock and Commit run on the v0.46 driver.
#[test]
fn v1_genesis_runs_on_the_v046_driver() {
    let mut app = App::new();
    app.set_legacy_modules(counter);
    assert!(!app.is_sdk_v046_state());
    let init = app.init_chain(init_request(1)).unwrap();
    assert_eq!(init.app_hash, hash::sha256(&[]).to_vec());
    assert!(app.is_sdk_v046_state());
    // Info between InitChain and the first commit already reports v1.
    assert_eq!(app.info().app_version, 1);
    assert_eq!(
        app.info().timeout_info,
        Some(super::super::timeouts(1)),
        "v1 timeouts before block 1"
    );
    let hashes = run(&mut app, 1..=2);
    assert_ne!(hashes[0], hashes[1]);
    assert_eq!(store_names(&app), v1_commit_info_names());
    assert!(!app.multistore().is_mounted("consensus"));
    assert_eq!(app.info().app_version, 1);
    assert_eq!(
        app.info().timeout_info,
        Some(super::super::timeouts(1)),
        "v1 timeouts"
    );
    // The chain ID written at InitChain is persisted with the first commit.
    assert_eq!(app.chain_id(), "celestia");
}

/// A v1 state reopens with the v1 store set (from its latest commit
/// info), its driver takes the chain ID from the next header, and the
/// app hashes equal those of an uninterrupted run; a rollback reopens the
/// driver the same way.
#[test]
fn v1_state_reopens_and_continues_with_the_same_app_hashes() {
    let straight = tempfile::tempdir().unwrap();
    let mut app = App::open_with(straight.path(), AppOptions::default()).unwrap();
    app.set_legacy_modules(counter);
    app.init_chain(init_request(1)).unwrap();
    let want = run(&mut app, 1..=6);
    drop(app);

    let home = tempfile::tempdir().unwrap();
    let mut app = App::open_with(home.path(), AppOptions::default()).unwrap();
    app.set_legacy_modules(counter);
    app.init_chain(init_request(1)).unwrap();
    let mut got = run(&mut app, 1..=3);
    drop(app);

    let mut app = App::open_with(home.path(), AppOptions::default()).unwrap();
    assert!(app.is_sdk_v046_state());
    assert_eq!(app.info().last_block_height, 3);
    assert_eq!(app.info().app_version, 1);
    assert_eq!(app.chain_id(), "celestia");
    assert_eq!(store_names(&app), v1_commit_info_names());
    app.set_legacy_modules(counter);
    got.extend(run(&mut app, 4..=5));

    app.rollback_to(4).unwrap();
    assert_eq!(app.info().last_block_height, 4);
    got.pop();
    got.extend(run(&mut app, 5..=6));
    assert_eq!(got, want);
}

/// Genesis at app versions 2 and 3 (cosmos-sdk v0.46, not ported) fails
/// with a clear error instead of running as v9.
#[test]
fn v2_and_v3_genesis_is_not_supported() {
    for v in [2, 3] {
        let mut app = App::new();
        let e = app.init_chain(init_request(v)).unwrap_err();
        assert!(e.log.contains("not supported"), "v{v}: {}", e.log);
        assert!(
            e.log.contains(&format!("app version {v}")),
            "v{v}: {}",
            e.log
        );
    }
}

/// A v0.46 state whose app version became 4 stops at the next block with
/// a clear error.
#[test]
fn v046_state_at_app_version_4_does_not_execute() {
    let mut app = App::new();
    app.set_legacy_modules(counter);
    app.init_chain(init_request(1)).unwrap();
    run(&mut app, 1..=1);
    let mut ctx = Context::new(&mut app.ms, sdk::Header::default(), sdk::ExecMode::Finalize);
    super::super::consensus_params::set_initial_app_version(&mut ctx, 4).unwrap();
    drop(ctx);
    app.ms.flush().unwrap();
    app.ms.commit().unwrap();
    app.load_committed_state().unwrap();
    assert_eq!(app.info().app_version, 4);
    let e = app.finalize_block(finalize_request(3)).unwrap_err();
    assert!(
        e.log.contains("block execution at app version 4"),
        "{}",
        e.log
    );
}

/// CheckTx, PrepareProposal, ProcessProposal, queries and snapshot
/// offers fail on a v0.46 state rather than run v0.50 code on it.
#[test]
fn v046_state_rejects_mempool_proposals_queries_and_snapshots() {
    use abci::response_offer_snapshot::Result as R;
    let home = tempfile::tempdir().unwrap();
    let mut app = App::open_with(home.path(), AppOptions::default()).unwrap();
    app.set_legacy_modules(counter);
    app.init_chain(init_request(1)).unwrap();
    run(&mut app, 1..=2);

    let check = app
        .check_tx(&abci::RequestCheckTx {
            tx: b"tx".to_vec(),
            ..abci::RequestCheckTx::default()
        })
        .unwrap();
    assert_ne!(check.code, 0);
    assert!(
        check.log.contains("CheckTx at app version 1"),
        "{}",
        check.log
    );

    let e = app
        .prepare_proposal(&abci::RequestPrepareProposal {
            height: 3,
            ..abci::RequestPrepareProposal::default()
        })
        .unwrap_err();
    assert!(
        e.log.contains("PrepareProposal at app version 1"),
        "{}",
        e.log
    );
    let e = app
        .process_proposal(&abci::RequestProcessProposal {
            height: 3,
            ..abci::RequestProcessProposal::default()
        })
        .unwrap_err();
    assert!(
        e.log.contains("ProcessProposal at app version 1"),
        "{}",
        e.log
    );

    for path in [
        "/store/bank/key",
        "/cosmos.bank.v1beta1.Query/Balance",
        "/app/version",
    ] {
        let q = app.query(&abci::RequestQuery {
            path: path.to_owned(),
            ..abci::RequestQuery::default()
        });
        assert_ne!(q.code, 0, "{path}");
        assert!(
            q.log.contains("queries at app version 1"),
            "{path}: {}",
            q.log
        );
    }
    assert!(app.create_query_context(0, false).is_err());
    assert!(app.export_app_state(None).is_err());

    // A real snapshot (of any committed multistore) is accepted at app
    // version 10 and rejected at app versions 1 to 3.
    let mut ms = sdk::MultiStore::new();
    for (name, kind) in crate::stores::all() {
        ms.mount(name, kind).unwrap();
    }
    ms.set("bank", b"k".to_vec(), b"v".to_vec()).unwrap();
    ms.flush().unwrap();
    ms.commit().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let manager = snapshots::Manager::new(
        dir.path().to_owned(),
        snapshots::SnapshotOptions {
            interval: 1,
            keep_recent: 0,
        },
    )
    .unwrap();
    let s = manager.create(ms.root(), 1).unwrap();
    for (app_version, want) in [
        (10, R::Accept),
        (1, R::Reject),
        (2, R::Reject),
        (3, R::Reject),
    ] {
        let fresh_home = tempfile::tempdir().unwrap();
        let mut fresh = App::open_with(fresh_home.path(), AppOptions::default()).unwrap();
        let offer = fresh.offer_snapshot(abci::RequestOfferSnapshot {
            snapshot: Some(abci::Snapshot {
                height: s.height,
                format: s.format,
                chunks: s.chunks,
                hash: s.hash.clone(),
                metadata: s.metadata_bytes(),
            }),
            app_hash: vec![2; 32],
            app_version,
        });
        assert_eq!(offer.result, want as i32, "v{app_version}");
    }
}

/// A node configured to take state-sync snapshots writes format 2
/// (cosmos-sdk v0.46) snapshots of a v1 state, as Go v0.46 does.
#[test]
fn v046_state_takes_format_2_snapshots() {
    let home = tempfile::tempdir().unwrap();
    let options = AppOptions {
        snapshots: snapshots::SnapshotOptions {
            interval: 1,
            keep_recent: 2,
        },
        ..AppOptions::default()
    };
    let mut app = App::open_with(home.path(), options).unwrap();
    app.set_legacy_modules(counter);
    app.init_chain(init_request(1)).unwrap();
    for h in 1..=3 {
        run(&mut app, h..=h);
        app.wait_for_snapshot();
    }
    let listed: Vec<_> = app
        .list_snapshots()
        .snapshots
        .into_iter()
        .map(|s| (s.height, s.format))
        .collect();
    assert_eq!(
        listed,
        [(3, snapshots::FORMAT_V046), (2, snapshots::FORMAT_V046)]
    );
}

/// A snapshot height whose snapshot was refused because the previous one
/// was still being written (Go `Manager.Create`'s conflict) is taken by
/// `finish_snapshots`, which a finite replay calls before it exits.
#[test]
fn finish_snapshots_takes_the_snapshot_a_conflict_refused() {
    let home = tempfile::tempdir().unwrap();
    let options = AppOptions {
        snapshots: snapshots::SnapshotOptions {
            interval: 1,
            keep_recent: 0,
        },
        ..AppOptions::default()
    };
    let mut app = App::open_with(home.path(), options).unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    app.snapshot_manager_mut()
        .unwrap()
        .set_snapshot_hook(move |height| {
            let _ = started_tx.send(height);
            if height == 2 {
                let _ = release_rx.lock().map(|rx| rx.recv());
            }
        });
    app.set_legacy_modules(counter);
    app.init_chain(init_request(1)).unwrap();
    run(&mut app, 1..=1);
    app.wait_for_snapshot();
    run(&mut app, 2..=2);
    assert_eq!(started_rx.recv().unwrap(), 1);
    assert_eq!(started_rx.recv().unwrap(), 2);
    // Snapshot 2 is blocked, so the commit of 3 cannot start one.
    run(&mut app, 3..=3);
    release_tx.send(()).unwrap();
    app.finish_snapshots();
    let heights: Vec<_> = app
        .list_snapshots()
        .snapshots
        .into_iter()
        .map(|s| (s.height, s.format))
        .collect();
    assert_eq!(
        heights,
        [
            (3, snapshots::FORMAT_V046),
            (2, snapshots::FORMAT_V046),
            (1, snapshots::FORMAT_V046)
        ]
    );
}

/// [`Counter`], whose transaction `b"unported"` reaches a not-ported
/// behavior after BeginBlock wrote.
#[derive(Debug, Default)]
struct AbortOn;

impl Modules for AbortOn {
    fn init_genesis(
        &mut self,
        ctx: &mut Context<'_>,
        app_state: &[u8],
    ) -> sdk::Result<Vec<ValidatorUpdate>> {
        Counter.init_genesis(ctx, app_state)
    }

    fn begin_block(&mut self, ctx: &mut Context<'_>, header: &TmHeader) -> sdk::Result<()> {
        Counter.begin_block(ctx, header)
    }

    fn deliver_tx(
        &mut self,
        ctx: &mut Context<'_>,
        gas: &mut kvstore::GasMeter,
        tx: &[u8],
    ) -> TxOutcome {
        if tx == b"unported" {
            return TxOutcome {
                gas_wanted: 1,
                gas_used: 1,
                events: Vec::new(),
                result: Err(super::super::not_ported(sdk::error::sdk::unknown_request(
                    "the handler of X",
                ))),
            };
        }
        Counter.deliver_tx(ctx, gas, tx)
    }

    fn end_block(&mut self, ctx: &mut Context<'_>) -> sdk::Result<Vec<ValidatorUpdate>> {
        Counter.end_block(ctx)
    }
}

fn abort_on() -> AppModules {
    Box::new(AbortOn)
}

/// A node that stops between FinalizeBlock and Commit on a persistent v1
/// state reopens at the last commit, finalizes the same block again and
/// reaches the app hashes of an uninterrupted run.
#[test]
fn v1_crash_between_finalize_and_commit_replays_the_block() {
    let straight = tempfile::tempdir().unwrap();
    let mut app = App::open_with(straight.path(), AppOptions::default()).unwrap();
    app.set_legacy_modules(counter);
    app.init_chain(init_request(1)).unwrap();
    let want = run(&mut app, 1..=4);
    drop(app);

    let home = tempfile::tempdir().unwrap();
    let mut app = App::open_with(home.path(), AppOptions::default()).unwrap();
    app.set_legacy_modules(counter);
    app.init_chain(init_request(1)).unwrap();
    let mut got = run(&mut app, 1..=2);
    let uncommitted = app.finalize_block(finalize_request(3)).unwrap();
    assert_eq!(uncommitted.app_hash, want[2]);
    drop(app);

    let mut app = App::open_with(home.path(), AppOptions::default()).unwrap();
    app.set_legacy_modules(counter);
    assert_eq!(app.info().last_block_height, 2);
    assert_eq!(app.info().last_block_app_hash, want[1]);
    got.extend(run(&mut app, 3..=4));
    assert_eq!(got, want);
}

/// A block that reaches a not-ported behavior aborts through [`App`]:
/// nothing is committed, the same block aborts again in place and after
/// a reopen with the same error, and the committed state is unchanged.
#[test]
fn v1_abort_keeps_the_committed_state_and_aborts_again() {
    let home = tempfile::tempdir().unwrap();
    let mut app = App::open_with(home.path(), AppOptions::default()).unwrap();
    app.set_legacy_modules(abort_on);
    app.init_chain(init_request(1)).unwrap();
    let hashes = run(&mut app, 1..=2);
    let mut req = finalize_request(3);
    // A transaction whose write only the aborted block makes.
    req.txs.push(b"leak".to_vec());
    req.txs.push(b"unported".to_vec());
    let first = app.finalize_block(req.clone()).unwrap_err();
    assert!(
        first.log.contains("block 3 not executed: tx 3"),
        "{}",
        first.log
    );
    assert!(app.commit().is_err(), "nothing to commit");
    let again = app.finalize_block(req.clone()).unwrap_err();
    assert_eq!(again.log, first.log);
    assert_eq!(app.info().last_block_height, 2);
    assert_eq!(app.info().last_block_app_hash, hashes[1]);
    // The aborted blocks left no writes behind: block 3 without the
    // unported transaction matches the same block after a reopen.
    let in_place = app.finalize_block(finalize_request(3)).unwrap();
    drop(app);

    let mut app = App::open_with(home.path(), AppOptions::default()).unwrap();
    app.set_legacy_modules(abort_on);
    assert_eq!(app.info().last_block_height, 2);
    assert_eq!(app.info().last_block_app_hash, hashes[1]);
    let after = app.finalize_block(req).unwrap_err();
    assert_eq!(after.log, first.log);
    drop(app);
    let mut app = App::open_with(home.path(), AppOptions::default()).unwrap();
    app.set_legacy_modules(abort_on);
    let clean = app.finalize_block(finalize_request(3)).unwrap();
    assert_eq!(in_place.app_hash, clean.app_hash);
    app.commit().unwrap();
    assert_eq!(app.ms.get("bank", b"leak").unwrap(), None);
}

/// A database with a cosmos-sdk v0.50 store set whose app version is 1
/// (written when app version 1 ran as v9) does not open: app version 1
/// now selects the v0.46 release, whose modules and handlers differ.
#[test]
fn v050_state_at_app_version_1_does_not_open() {
    use prost::Message as _;
    let home = tempfile::tempdir().unwrap();
    let mut app = App::open_with(home.path(), AppOptions::default()).unwrap();
    assert!(!app.is_sdk_v046_state());
    app.ms
        .set(
            x_consensus::STORE_KEY,
            x_consensus::PARAMS_KEY.to_vec(),
            consensus_params(1).encode_to_vec(),
        )
        .unwrap();
    app.ms.flush().unwrap();
    app.ms.commit().unwrap();
    drop(app);
    let Err(e) = App::open_with(home.path(), AppOptions::default()) else {
        panic!("opened a v0.50 state at app version 1");
    };
    assert!(e.log.contains("app version 1"), "{}", e.log);
    assert!(e.log.contains("cosmos-sdk v0.50 store set"), "{}", e.log);
}

/// Fails InitGenesis after writing.
#[derive(Debug, Default)]
struct FailGenesis;

impl Modules for FailGenesis {
    fn init_genesis(
        &mut self,
        ctx: &mut Context<'_>,
        _: &[u8],
    ) -> sdk::Result<Vec<ValidatorUpdate>> {
        ctx.set("blob", b"partial".to_vec(), b"x".to_vec())?;
        Err(sdk::error::sdk::logic("bad genesis"))
    }

    fn begin_block(&mut self, _: &mut Context<'_>, _: &TmHeader) -> sdk::Result<()> {
        Ok(())
    }

    fn deliver_tx(
        &mut self,
        ctx: &mut Context<'_>,
        gas: &mut kvstore::GasMeter,
        tx: &[u8],
    ) -> TxOutcome {
        Counter.deliver_tx(ctx, gas, tx)
    }

    fn end_block(&mut self, _: &mut Context<'_>) -> sdk::Result<Vec<ValidatorUpdate>> {
        Ok(Vec::new())
    }
}

fn fail_genesis() -> AppModules {
    Box::new(FailGenesis)
}

/// A failed v1 InitChain leaves the app as it was: the v0.50 stores
/// mounted, no driver and none of the genesis writes, so a later
/// InitChain runs as on a fresh app.
#[test]
fn failed_v1_init_chain_restores_the_app() {
    let mut want = App::new();
    want.set_legacy_modules(counter);
    want.init_chain(init_request(1)).unwrap();
    let want = run(&mut want, 1..=1);

    let mut app = App::new();
    app.set_legacy_modules(fail_genesis);
    let e = app.init_chain(init_request(1)).unwrap_err();
    assert!(e.log.contains("bad genesis"), "{}", e.log);
    assert!(!app.is_sdk_v046_state());
    assert!(app.multistore().is_mounted("consensus"));
    assert!(!app.multistore().is_mounted("qgb"));
    assert_eq!(app.chain_id(), "");
    app.set_legacy_modules(counter);
    app.init_chain(init_request(1)).unwrap();
    assert_eq!(run(&mut app, 1..=1), want);
}

/// A reopened v1 state checks the block header's chain ID against the
/// stored one.
#[test]
fn reopened_v1_state_rejects_another_chain_id() {
    let home = tempfile::tempdir().unwrap();
    let mut app = App::open_with(home.path(), AppOptions::default()).unwrap();
    app.set_legacy_modules(counter);
    app.init_chain(init_request(1)).unwrap();
    run(&mut app, 1..=1);
    drop(app);
    let mut app = App::open_with(home.path(), AppOptions::default()).unwrap();
    app.set_legacy_modules(counter);
    let mut req = finalize_request(2);
    req.header.as_mut().unwrap().chain_id = "mocha-4".to_owned();
    let e = app.finalize_block(req).unwrap_err();
    assert!(e.log.contains("invalid chain ID"), "{}", e.log);
    run(&mut app, 2..=2);
}
