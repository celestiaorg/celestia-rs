//! Replay the v12 upgrade against a database committed by upstream v11.
use app::{App, AppOptions, NodeOptions};
use celestia_proto::celestia_core_proto::abci::{request, response};
use devnet_conformance::load_exchanges;

const SIGNAL_HEIGHT: i64 = 5;
const UPGRADE_HEIGHT: i64 = SIGNAL_HEIGHT + 1;

#[test]
fn v12_upgrade_without_restart() -> anyhow::Result<()> {
    replay_boundary(false)
}

#[test]
fn v12_upgrade_reopens_at_boundary() -> anyhow::Result<()> {
    replay_boundary(true)
}

fn replay_boundary(reopen: bool) -> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    let options = AppOptions {
        node: NodeOptions {
            home: Some(home.path().to_path_buf()),
            activate_bundled_upgrades: true,
            ..NodeOptions::default()
        },
        ..AppOptions::default()
    };
    let mut app = App::open_with(home.path(), options.clone())?;
    let mut verified_heights = Vec::new();
    for exchange in load_exchanges("upgrade/v12-main-upgrade")? {
        match (exchange.request.value, exchange.response.value) {
            (Some(request::Value::InitChain(req)), Some(response::Value::InitChain(go))) => {
                let rust = app.init_chain(req)?;
                assert_eq!(rust.validators, go.validators);
                assert_eq!(rust.app_hash, go.app_hash);
            }
            (
                Some(request::Value::FinalizeBlock(req)),
                Some(response::Value::FinalizeBlock(go)),
            ) => {
                let height = req.height;
                let rust = app.finalize_block(req)?;
                assert_eq!(rust.app_hash, go.app_hash, "height {height}");
                assert_eq!(rust.timeout_info, go.timeout_info, "height {height}");
                if height >= UPGRADE_HEIGHT {
                    assert_eq!(rust, go, "height {height}");
                }
                verified_heights.push(height);
            }
            (Some(request::Value::Commit(_)), Some(response::Value::Commit(go))) => {
                assert_eq!(app.commit()?, go);
                if reopen && app.info().last_block_height == SIGNAL_HEIGHT {
                    drop(app);
                    app = App::open_with(home.path(), options.clone())?;
                }
            }
            exchange => anyhow::bail!("unexpected ABCI exchange {exchange:?}"),
        }
    }
    assert_eq!(verified_heights, (1..=7).collect::<Vec<_>>());
    assert!(!home.path().join("data/upgrade-info.json").exists());
    let expected = app.info();
    assert_eq!(expected.app_version, 12);
    drop(app);
    assert_eq!(App::open_with(home.path(), options)?.info(), expected);
    Ok(())
}
