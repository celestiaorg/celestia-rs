//! The RPC half of bootstrapping `follow` from a snapshot at an old mainnet
//! height, run against real archive RPC servers without the snapshot.
//!
//! With `--snapshot-height`, `follow` verifies the light-client data at the
//! height (trusted header, validator sets at H and H+1, consensus params,
//! the commit, header H+1's app hash and results hash) before it reads the
//! snapshot. So a run with an empty snapshot directory proves the RPC path
//! and then fails at the missing snapshot.
//!
//! ```sh
//! CELESTIA_ARCHIVE_RPC=https://rpc-a,https://rpc-b \
//!   cargo test -p celestia-rs --test follow_snapshot_bootstrap_mainnet -- --ignored
//! ```
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use celestia_rs::{
    follow::{self, FollowOptions},
    ops::Shutdown,
};

/// Header 120,000 of mainnet (`celestia`), app version 1: the first
/// archive snapshot height.
const HEIGHT: u64 = 120_000;
const HASH: &str = "99F12CEEFE15DDF6A6C806455918410ACE6584B99049D664478D337F4702A3C4";
/// Header 120,001's app hash: the state after block 120,000.
const APP_HASH: &str = "CFD8789CD34681EE1B82928607C792FF3DD66B5E51115F241D6C07B56BE29F09";

#[test]
#[ignore = "requires mainnet archive RPC (CELESTIA_ARCHIVE_RPC)"]
fn verifies_light_client_data_then_stops_at_the_missing_snapshot() {
    let rpc: Vec<String> = std::env::var("CELESTIA_ARCHIVE_RPC")
        .expect("set CELESTIA_ARCHIVE_RPC to archive RPC servers holding height 120000")
        .split(',')
        .map(str::to_owned)
        .collect();
    let home = tempfile::tempdir().unwrap();
    let snapshots = tempfile::tempdir().unwrap();
    let opts = FollowOptions {
        home: home.path().to_path_buf(),
        rpc,
        chain_id: Some("celestia".into()),
        snapshot: Some(snapshots.path().to_path_buf()),
        snapshot_height: Some(HEIGHT),
        trust_hash: Some(hex::decode(HASH).unwrap()),
        trust_period: 200_000 * 3600 * 1_000_000_000,
        stop_height: 0,
        prefetch: 2,
        verify_data: true,
        witness_interval: 0,
        app: app::AppOptions::default(),
        shutdown: Shutdown::new(),
    };
    let err = format!("{:#}", follow::follow(&opts).unwrap_err());
    assert!(
        err.contains(&format!(
            "light-client data at {HEIGHT} verified (app hash {APP_HASH}, app version 1)"
        )),
        "{err}"
    );
    assert!(
        err.contains(&format!("no snapshot at height {HEIGHT}")),
        "{err}"
    );
}
