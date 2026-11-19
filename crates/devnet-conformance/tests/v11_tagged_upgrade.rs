//! Replay the signed v10 upgrade workload through the tagged v11 boundary.

use app::{App, AppOptions, NodeOptions};
use celestia_proto::celestia_core_proto::abci::{request, response};
use devnet_conformance::{load_exchanges, replay::replay_into};

const SIGNAL_HEIGHT: i64 = 99;
const UPGRADE_HEIGHT: i64 = SIGNAL_HEIGHT + 1;

#[test]
fn v11_tagged_upgrade() -> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    let options = AppOptions {
        node: NodeOptions {
            home: Some(home.path().to_path_buf()),
            ..NodeOptions::default()
        },
        ..AppOptions::default()
    };
    let mut app = App::open_with(home.path(), options.clone())?;
    let report = replay_into("upgrade", &mut app, 0, None)?;
    assert_eq!(
        report.matched_heights,
        (1..=SIGNAL_HEIGHT).collect::<Vec<_>>()
    );
    assert_eq!(report.halted_at, Some(UPGRADE_HEIGHT));
    assert!(
        report.errors.is_empty() && report.first_mismatch.is_none(),
        "{report:?}"
    );
    let before = app.info();
    assert_eq!(before.last_block_height, SIGNAL_HEIGHT);
    assert_eq!(before.app_version, 11);
    // The running v10 executor halts at the recorded next block and writes
    // the actual upgrade plan consumed by the new process.
    let disk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(home.path().join("data/upgrade-info.json"))?)?;
    assert_eq!(disk["name"], "v11");
    assert_eq!(disk["height"], UPGRADE_HEIGHT);
    drop(app);

    let app = App::open_with(home.path(), options.clone())?;
    assert_eq!(app.info(), before);
    assert_tagged_boundary(app, home.path(), options)
}

#[test]
fn embedded_v11_upgrade_without_restart() -> anyhow::Result<()> {
    assert_embedded_upgrade(false)
}

#[test]
fn embedded_v11_upgrade_reopens_at_boundary() -> anyhow::Result<()> {
    assert_embedded_upgrade(true)
}

fn assert_embedded_upgrade(reopen: bool) -> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    let options = AppOptions {
        node: NodeOptions {
            home: Some(home.path().to_path_buf()),
            activate_v11_in_process: true,
            ..NodeOptions::default()
        },
        ..AppOptions::default()
    };
    let mut app = App::open_with(home.path(), options.clone())?;
    let report = replay_into("upgrade", &mut app, 0, Some(SIGNAL_HEIGHT))?;
    assert_eq!(
        report.matched_heights,
        (1..=SIGNAL_HEIGHT).collect::<Vec<_>>()
    );
    assert!(
        report.errors.is_empty() && report.first_mismatch.is_none(),
        "{report:?}"
    );
    assert_eq!(app.info().app_version, 11);
    if reopen {
        drop(app);
        app = App::open_with(home.path(), options.clone())?;
    }
    assert_tagged_boundary(app, home.path(), options)?;
    assert!(!home.path().join("data/upgrade-info.json").exists());
    Ok(())
}

fn assert_tagged_boundary(
    mut app: App,
    home: &std::path::Path,
    options: AppOptions,
) -> anyhow::Result<()> {
    let exchanges = load_exchanges("upgrade/v11-tagged-upgrade")?;
    let [prepare, process, finalize, commit] = exchanges.as_slice() else {
        anyhow::bail!("expected four upgrade ABCI exchanges");
    };
    let (Some(request::Value::PrepareProposal(req)), Some(response::Value::PrepareProposal(go))) =
        (&prepare.request.value, &prepare.response.value)
    else {
        anyhow::bail!("missing upgrade PrepareProposal exchange");
    };
    assert_eq!(req.height, UPGRADE_HEIGHT);
    assert_eq!(&app.prepare_proposal(req)?, go);
    let (Some(request::Value::ProcessProposal(req)), Some(response::Value::ProcessProposal(go))) =
        (&process.request.value, &process.response.value)
    else {
        anyhow::bail!("missing upgrade ProcessProposal exchange");
    };
    assert_eq!(req.height, UPGRADE_HEIGHT);
    assert_eq!(&app.process_proposal(req)?, go);
    let (Some(request::Value::FinalizeBlock(req)), Some(response::Value::FinalizeBlock(go))) =
        (&finalize.request.value, &finalize.response.value)
    else {
        anyhow::bail!("missing upgrade FinalizeBlock exchange");
    };
    assert_eq!(req.height, UPGRADE_HEIGHT);
    assert_eq!(&app.finalize_block(req.clone())?, go);
    let (Some(request::Value::Commit(_)), Some(response::Value::Commit(go_commit))) =
        (&commit.request.value, &commit.response.value)
    else {
        anyhow::bail!("missing upgrade Commit exchange");
    };
    assert_eq!(&app.commit()?, go_commit);
    let after = app.info();
    assert_eq!(after.app_version, 11);
    assert_eq!(after.last_block_height, UPGRADE_HEIGHT);
    assert_eq!(after.last_block_app_hash, go.app_hash);
    drop(app);
    assert_eq!(App::open_with(home, options)?.info(), after);
    Ok(())
}
