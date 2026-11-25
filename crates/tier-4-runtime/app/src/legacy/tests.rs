#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::io::Read as _;

use celestia_proto::celestia_core_proto::abci;
use celestia_proto::celestia_core_proto::tendermint::types::{
    BlockParams, ConsensusParams, EvidenceParams, Header as TmHeader, ValidatorParams,
    VersionParams,
};
use flate2::read::GzDecoder;
use sdk::MultiStore;
use sdk::version::Release;

use super::*;

const V1_ORACLE_KV_MODULES: &[u8] = include_bytes!(
    "../../../../../conformance/fixtures/mainnet/v1-oracle/kv-height-0-modules.jsonl.gz"
);

/// Mainnet genesis consensus params (`celestia/genesis.json`).
fn mainnet_consensus_params() -> ConsensusParams {
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
        version: Some(VersionParams { app: 1 }),
        ..ConsensusParams::default()
    }
}

fn v1_store() -> MultiStore {
    let mut ms = MultiStore::new();
    let v1 = Release::for_app_version(1).expect("v1");
    crate::stores::mount_release(&mut ms, v1).expect("mount v1");
    ms
}

fn init_request() -> abci::RequestInitChain {
    abci::RequestInitChain {
        chain_id: "celestia".to_owned(),
        initial_height: 1,
        consensus_params: Some(mainnet_consensus_params()),
        app_state_bytes: b"{}".to_vec(),
        ..abci::RequestInitChain::default()
    }
}

/// The oracle's height-0 `params` entries of subspace `baseapp`.
fn oracle_baseapp_params() -> BTreeMap<Vec<u8>, Vec<u8>> {
    let mut text = String::new();
    GzDecoder::new(V1_ORACLE_KV_MODULES)
        .read_to_string(&mut text)
        .unwrap();
    let dump: serde_json::Value = text
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .find(|d| d["store"] == "params")
        .unwrap();
    dump["kv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|kv| {
            let hex = |i: usize| hex::decode(kv[i].as_str().unwrap()).unwrap();
            (hex(0), hex(1))
        })
        .filter(|(k, _)| k.starts_with(b"baseapp/"))
        .collect()
}

/// InitChain stores the consensus params in subspace `baseapp` exactly as
/// Go v0.46 does on mainnet (amino JSON, no `VersionParams`), does not
/// commit, and returns `sha256("")`.
#[test]
fn init_chain_writes_the_mainnet_baseapp_params() {
    let mut ms = v1_store();
    let mut driver = Driver::new(NoModules);
    let res = driver.init_chain(&mut ms, &init_request()).unwrap();
    assert_eq!(res.app_hash, hash::sha256(&[]).to_vec());
    assert_eq!(ms.version(), 0);

    let written: BTreeMap<Vec<u8>, Vec<u8>> = ms
        .range(
            x_params::STORE_KEY,
            Some(b"baseapp/"),
            Some(b"baseapp0"),
            false,
        )
        .unwrap()
        .into_iter()
        .collect();
    let expected = oracle_baseapp_params();
    assert_eq!(expected.len(), 3);
    assert_eq!(written, expected);
}

/// InitChain rejects a request without consensus params (Go panics).
#[test]
fn init_chain_requires_consensus_params() {
    let mut ms = v1_store();
    let mut driver = Driver::new(NoModules);
    let mut req = init_request();
    req.consensus_params = None;
    assert!(driver.init_chain(&mut ms, &req).is_err());
}

fn finalize_request(height: i64) -> abci::RequestFinalizeBlock {
    abci::RequestFinalizeBlock {
        height,
        hash: vec![7; 32],
        header: Some(TmHeader {
            chain_id: "celestia".to_owned(),
            height,
            app_hash: hash::sha256(&[]).to_vec(),
            ..TmHeader::default()
        }),
        ..abci::RequestFinalizeBlock::default()
    }
}

/// An empty block on an empty module set commits the 18-entry v1 commit
/// info (17 IAVL stores and `mem_capability` with the empty commit ID)
/// and returns the full v0.46 consensus params, with the in-memory app
/// version standing in for the absent `VersionParams`.
#[test]
fn empty_block_commits_the_v1_commit_info() {
    let mut ms = v1_store();
    let mut driver = Driver::new(NoModules);
    driver.init_chain(&mut ms, &init_request()).unwrap();
    let res = driver
        .finalize_block(&mut ms, &finalize_request(1))
        .unwrap();
    assert!(res.tx_results.is_empty());
    let mut cp = mainnet_consensus_params();
    cp.version = Some(VersionParams { app: 1 });
    assert_eq!(res.consensus_param_updates, Some(cp));
    let id = driver.commit(&mut ms).unwrap();
    assert_eq!(id.version, 1);
    assert_eq!(id.hash.to_vec(), res.app_hash);

    let info = ms.root().last_commit_info().unwrap();
    let names: Vec<&str> = info.store_infos.iter().map(|s| s.name.as_str()).collect();
    let v1 = Release::for_app_version(1).unwrap();
    let mut expected: Vec<&str> = v1.stores.to_vec();
    expected.push("mem_capability");
    expected.sort_unstable();
    assert_eq!(names, expected);
    assert_eq!(info.store_info("mem_capability").unwrap().commit_id, None);
    assert_eq!(app_version(&ms), 1);
}

/// FinalizeBlock needs the full header (v0.46 BeginBlock) at the next
/// height; it rejects anything else with an error.
#[test]
fn finalize_block_rejects_missing_header_and_wrong_height() {
    let mut ms = v1_store();
    let mut driver = Driver::new(NoModules);
    driver.init_chain(&mut ms, &init_request()).unwrap();
    let mut req = finalize_request(1);
    req.header = None;
    assert!(driver.finalize_block(&mut ms, &req).is_err());
    assert!(
        driver
            .finalize_block(&mut ms, &finalize_request(2))
            .is_err()
    );
    assert!(driver.commit(&mut ms).is_err());
}

/// A transaction's result: success carries `TxMsgData` of the message
/// responses; failure carries the error code and no data.
#[test]
fn deliver_tx_results_are_shaped_like_v046() {
    let any = celestia_proto::Any {
        type_url: "/cosmos.bank.v1beta1.MsgSendResponse".to_owned(),
        value: Vec::new(),
    };
    let ok = deliver_tx_result(TxOutcome {
        gas_wanted: 210_000,
        gas_used: 86_351,
        events: Vec::new(),
        result: Ok(TxSuccess {
            msg_responses: vec![any],
            log: String::new(),
        }),
    });
    // TxMsgData { msg_responses: [Any { type_url }] }
    let mut data = vec![0x12, 38, 0x0a, 36];
    data.extend_from_slice(b"/cosmos.bank.v1beta1.MsgSendResponse");
    assert_eq!(
        (ok.code, ok.data, ok.gas_wanted, ok.gas_used),
        (0, data, 210_000, 86_351)
    );
    let failed = deliver_tx_result(TxOutcome {
        gas_wanted: 210_000,
        gas_used: 66_079,
        events: Vec::new(),
        result: Err(sdk::SdkError::new("sdk", 5, "insufficient funds")),
    });
    assert_eq!(
        (failed.code, failed.codespace.as_str(), failed.data.len()),
        (5, "sdk", 0)
    );
}

/// The app version of a v0.46 state: `VersionParams` of subspace
/// `baseapp` once written (v2 and later), else 1 (Go `App.Info`).
#[test]
fn app_version_reads_subspace_baseapp() {
    let mut ms = v1_store();
    let mut driver = Driver::new(NoModules);
    let mut req = init_request();
    req.consensus_params.as_mut().unwrap().version = Some(VersionParams { app: 2 });
    driver.init_chain(&mut ms, &req).unwrap();
    driver
        .finalize_block(&mut ms, &finalize_request(1))
        .unwrap();
    driver.commit(&mut ms).unwrap();
    assert_eq!(app_version(&ms), 2);
    assert_eq!(
        ms.root()
            .get_store(x_params::STORE_KEY)
            .unwrap()
            .get(b"baseapp/VersionParams")
            .unwrap(),
        Some(br#"{"app_version":"2"}"#.to_vec())
    );
}

/// Nothing executes on the legacy driver while app versions 1 to 3 run
/// as v9: the driver stays unreachable from the current app.
#[test]
fn legacy_driver_is_inert_until_v046_runs_as_itself() {
    for v in 0..=13 {
        assert!(!executes(v), "v{v}");
    }
}

/// `App::info` reports the app version of a v0.46 state from subspace
/// `baseapp` (the state has no `x/consensus` store).
#[test]
fn app_info_reports_the_legacy_app_version() {
    let mut ms = v1_store();
    let mut driver = Driver::new(NoModules);
    driver.init_chain(&mut ms, &init_request()).unwrap();
    driver
        .finalize_block(&mut ms, &finalize_request(1))
        .unwrap();
    driver.commit(&mut ms).unwrap();
    let mut app = crate::App::from_multistore(ms);
    app.last_height = 1;
    assert_eq!(app.info().app_version, 1);
}
