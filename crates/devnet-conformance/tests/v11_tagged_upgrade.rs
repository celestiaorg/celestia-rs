//! Replay the signed v10 upgrade workload through the v11 restart.

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

    let mut app = App::open_with(home.path(), options.clone())?;
    assert_eq!(app.info(), before);
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
    let ours = app.finalize_block(req.clone())?;
    assert_eq!(ours.app_hash, go.app_hash);
    assert_eq!(ours.consensus_param_updates, go.consensus_param_updates);
    assert_eq!(ours.timeout_info, go.timeout_info);
    assert_eq!(ours.tx_results, go.tx_results);
    assert_eq!(ours.validator_updates, go.validator_updates);
    assert!(matches!(
        commit.request.value,
        Some(request::Value::Commit(_))
    ));
    assert!(matches!(
        commit.response.value,
        Some(response::Value::Commit(_))
    ));
    app.commit()?;
    let after = app.info();
    assert_eq!(after.app_version, 11);
    assert_eq!(after.last_block_height, UPGRADE_HEIGHT);
    assert_eq!(after.last_block_app_hash, go.app_hash);
    drop(app);
    assert_eq!(App::open_with(home.path(), options)?.info(), after);
    Ok(())
}
