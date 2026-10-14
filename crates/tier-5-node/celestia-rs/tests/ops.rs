//! Node operations on a single-validator chain: `rollback` (one height,
//! like `celestia-appd rollback`), `halt-height` (Go `checkHalt`: the node
//! stops before executing the block), graceful shutdown, and a signalled
//! app-version upgrade without a handler (the node halts exactly at the
//! upgrade height with `data/upgrade-info.json`, like Go).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::many_single_char_names
)]

use std::path::Path;
use std::time::Duration;

use celestia_proto::Any;
use celestia_rs::node::{self, StartOptions};
use celestia_rs::{genesis, init, keys, rollback, txs};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/home");

fn copy_fixture(home: &Path) {
    let cfg = home.join("config");
    std::fs::create_dir_all(&cfg).unwrap();
    for f in ["genesis.json", "priv_validator_key.json"] {
        std::fs::copy(Path::new(FIXTURE).join("config").join(f), cfg.join(f)).unwrap();
    }
}

fn opts(home: &Path, stop: i64) -> StartOptions {
    StartOptions::new(
        home.to_path_buf(),
        stop,
        consensus::ConsensusConfig::default(),
    )
}

fn heights(home: &Path) -> (i64, i64, i64) {
    let (bs, ss) = node::open_stores(home).unwrap();
    let state = ss.load().unwrap().unwrap();
    let app = app::App::open(home).unwrap();
    (
        bs.height(),
        state.last_block_height,
        app.info().last_block_height,
    )
}

#[test]
fn rollback_one_height_and_resume() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    copy_fixture(home);
    node::start(&opts(home, 4)).unwrap();
    assert_eq!(heights(home), (4, 4, 4));
    let mut block4 = {
        let (bs, _) = node::open_stores(home).unwrap();
        bs.load_block(4).unwrap().unwrap()
    };
    let (h, hash) = rollback::rollback(home, false, app::AppOptions::default()).unwrap();
    assert_eq!(h, 3);
    assert_eq!(hash, block4.header.app_hash, "app hash after height 3");
    // State and app at 3, the block stays (Go: removeBlock = false).
    assert_eq!(heights(home), (4, 3, 3));
    // Restart: the handshake re-executes block 4 from the block store and
    // consensus continues; the re-executed block is the same block.
    node::start(&opts(home, 6)).unwrap();
    assert_eq!(heights(home), (6, 6, 6));
    let (bs, _) = node::open_stores(home).unwrap();
    assert_eq!(
        bs.load_block(4).unwrap().unwrap().hash().unwrap(),
        block4.hash().unwrap()
    );
    drop(bs);
    // --hard also deletes the last block.
    let (h, _) = rollback::rollback(home, true, app::AppOptions::default()).unwrap();
    assert_eq!(h, 5);
    assert_eq!(heights(home), (5, 5, 5));
    // A store one block ahead of the state (an interrupted commit) only
    // loses that block.
    let (bs, _) = node::open_stores(home).unwrap();
    assert_eq!(bs.height(), 5);
}

#[test]
fn halt_height_stops_before_the_block() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    copy_fixture(home);
    let mut o = opts(home, 0);
    o.app.node.halt_height = 3;
    // Returns (clean halt) instead of running forever.
    node::start(&o).unwrap();
    let (store, state, app) = heights(home);
    assert_eq!((state, app), (2, 2), "block 3 is not executed");
    assert!(store == 3 || store == 2, "block store at {store}");
    // Without the halt the node replays block 3 and continues.
    node::start(&opts(home, 5)).unwrap();
    assert_eq!(heights(home), (5, 5, 5));
}

#[test]
fn shutdown_stops_the_node() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    copy_fixture(home);
    let o = opts(home, 0);
    let shutdown = o.shutdown.clone();
    let h = home.to_path_buf();
    let t = std::thread::spawn(move || node::start(&o));
    // Let it commit a few blocks, then stop it like SIGTERM does.
    let wal = h.join("data/cs.wal/wal");
    for _ in 0..600 {
        if wal.exists()
            && consensus::wal::decode_all(&std::fs::read(&wal).unwrap())
                .0
                .iter()
                .filter(|m| matches!(m.msg, consensus::wal::WalMessage::EndHeight(_)))
                .count()
                >= 3
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    shutdown.trigger();
    t.join().unwrap().unwrap();
    let (store, state, app) = heights(home);
    assert!(state >= 2);
    assert_eq!(state, app);
    assert!(store == state || store == state + 1);
    // The WAL decodes completely after the shutdown.
    let bz = std::fs::read(&wal).unwrap();
    let (_, n, err) = consensus::wal::decode_all(&bz);
    assert!(err.is_none());
    assert_eq!(n, bz.len());
    // And the node restarts from there.
    node::start(&opts(home, state + 2)).unwrap();
}

/// A fresh single-validator chain `test` (upgrade delay 3 blocks) whose
/// validator signals v11 and calls try-upgrade at height 1.
#[test]
fn signalled_upgrade_halts_like_go() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    init::init(&init::InitOptions {
        home: home.to_path_buf(),
        moniker: "v".into(),
        chain_id: "test".into(),
        network: None,
        overwrite: true,
        recover_mnemonic: None,
        download_genesis: false,
    })
    .unwrap();
    // No P2P / RPC in this test.
    std::fs::remove_file(home.join("config/config.toml")).unwrap();
    let mut kr = keyring::Keyring::new(home, keyring::Backend::Test);
    let key = kr
        .add_mnemonic(
            "val",
            &keyring::hd::new_mnemonic(),
            "",
            keyring::hd::DEFAULT_HD_PATH,
        )
        .unwrap();
    let g = home.join("config/genesis.json");
    genesis::add_genesis_account(
        &g,
        &key.bech32(keys::ACC_PREFIX),
        "1000000000000utia",
        false,
    )
    .unwrap();
    std::fs::write(home.join("config/config.toml"), init::config_toml("v")).unwrap();
    genesis::gentx(
        &genesis::GentxOptions {
            home: home.to_path_buf(),
            amount: "100000000utia".into(),
            ip: "127.0.0.1".into(),
            ..genesis::GentxOptions::default()
        },
        &key,
    )
    .unwrap();
    genesis::collect_gentxs(home, None).unwrap();
    std::fs::remove_file(home.join("config/config.toml")).unwrap();
    let inbox = home.join("data/tx-inbox");
    std::fs::create_dir_all(&inbox).unwrap();
    let msgs = [
        Any::pack(&celestia_proto::celestia::signal::v1::MsgSignalVersion {
            validator_address: key.bech32(keys::VAL_PREFIX),
            version: 11,
        }),
        Any::pack(&celestia_proto::celestia::signal::v1::MsgTryUpgrade {
            signer: key.bech32(keys::ACC_PREFIX),
        }),
    ];
    for (i, m) in msgs.into_iter().enumerate() {
        let tx = txs::sign(
            &txs::UnsignedTx {
                msgs: vec![m],
                memo: String::new(),
                timeout_height: 0,
                fee: txs::FeeOptions::default().fee().unwrap(),
                chain_id: "test".into(),
                account_number: 0,
                sequence: 1 + u64::try_from(i).unwrap(),
            },
            &key,
        )
        .unwrap();
        std::fs::write(inbox.join(format!("{i:04}.tx")), tx.raw).unwrap();
    }
    let o = opts(home, 30);
    let shutdown = o.shutdown.clone();
    let info = home.join("data/upgrade-info.json");
    let watch = {
        let info = info.clone();
        std::thread::spawn(move || {
            for _ in 0..1200 {
                if info.exists() {
                    // Like Go, the node idles (RPC up) until stopped.
                    std::thread::sleep(Duration::from_millis(300));
                    shutdown.trigger();
                    return true;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            shutdown.trigger();
            false
        })
    };
    node::start(&o).unwrap();
    assert!(watch.join().unwrap(), "no upgrade-info.json");
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&info).unwrap()).unwrap();
    assert_eq!(v["name"], "v11");
    assert_eq!(v["time"], "0001-01-01T00:00:00Z");
    let halt = v["height"].as_i64().unwrap();
    let (_, state, app_h) = heights(home);
    assert_eq!(app_h, halt - 1, "the upgrade block is not executed");
    assert_eq!(state, halt - 1);
    // The app version is 11 at the halt (bumped by the signal EndBlocker).
    let (_, ss) = node::open_stores(home).unwrap();
    assert_eq!(ss.load().unwrap().unwrap().consensus_params.version.app, 11);
    drop(ss);
    // The same binary halts again on restart without executing the block:
    // like Go, replaying the saved upgrade block in the handshake fails
    // (or, when the block was not saved, consensus halts again).
    let o = opts(home, 0);
    let s = o.shutdown.clone();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        s.trigger();
    });
    match node::start(&o) {
        Ok(()) => {}
        Err(e) => assert!(format!("{e:#}").contains("UPGRADE \"v11\" NEEDED"), "{e:#}"),
    }
    t.join().unwrap();
    assert_eq!(heights(home).2, halt - 1);
}
