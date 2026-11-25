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

const V1_ORACLE_ROOTS: &str =
    include_str!("../../../../../conformance/fixtures/mainnet/v1-oracle/store-roots.json");

/// The oracle's height-0 entries of `store` (hex-decoded, sorted).
fn oracle_kv(store: &str) -> BTreeMap<Vec<u8>, Vec<u8>> {
    let mut text = String::new();
    GzDecoder::new(V1_ORACLE_KV_MODULES)
        .read_to_string(&mut text)
        .unwrap();
    let dump: serde_json::Value = text
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .find(|d| d["store"] == store)
        .unwrap();
    dump["kv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|kv| {
            let hex = |i: usize| hex::decode(kv[i].as_str().unwrap()).unwrap();
            (hex(0), hex(1))
        })
        .collect()
}

/// The oracle's height-0 IAVL root of `store`.
fn oracle_root(store: &str) -> Vec<u8> {
    let roots: serde_json::Value = serde_json::from_str(V1_ORACLE_ROOTS).unwrap();
    let h0 = &roots["heights"][0];
    assert_eq!(h0["height"], 0);
    hex::decode(h0["stores"][store].as_str().unwrap()).unwrap()
}

/// The parameter sections of the canonical mainnet genesis (`celestia`,
/// SHA-256 `9727aac9…7793`), copied verbatim.
const MAINNET_GENESIS_PARAMS: &str = r#"{
 "auth": {"params": {"max_memo_characters": "256", "tx_sig_limit": "7",
   "tx_size_cost_per_byte": "10", "sig_verify_cost_ed25519": "590",
   "sig_verify_cost_secp256k1": "1000"}},
 "bank": {"params": {"send_enabled": [], "default_send_enabled": true}},
 "staking": {"params": {"unbonding_time": "1814400s", "max_validators": 100,
   "max_entries": 7, "historical_entries": 10000, "bond_denom": "utia",
   "min_commission_rate": "0.050000000000000000"}},
 "distribution": {"params": {"community_tax": "0.020000000000000000",
   "base_proposer_reward": "0.000000000000000000",
   "bonus_proposer_reward": "0.000000000000000000", "withdraw_addr_enabled": true}},
 "slashing": {"params": {"signed_blocks_window": "5000",
   "min_signed_per_window": "0.750000000000000000", "downtime_jail_duration": "60s",
   "slash_fraction_double_sign": "0.020000000000000000",
   "slash_fraction_downtime": "0.000000000000000000"}},
 "gov": {"deposit_params": {"min_deposit": [{"denom": "utia", "amount": "10000000000"}],
   "max_deposit_period": "604800s"}, "voting_params": {"voting_period": "604800s"},
   "tally_params": {"quorum": "0.334000000000000000", "threshold": "0.500000000000000000",
   "veto_threshold": "0.334000000000000000"}},
 "crisis": {"constant_fee": {"denom": "utia", "amount": "1000"}},
 "ibc": {"client_genesis": {"params": {"allowed_clients": ["06-solomachine", "07-tendermint"]}},
   "connection_genesis": {"params": {"max_expected_time_per_block": "75000000000"}}},
 "transfer": {"params": {"send_enabled": true, "receive_enabled": true}},
 "blob": {"params": {"gas_per_blob_byte": 8, "gov_max_square_size": "64"}},
 "qgb": {"params": {"data_commitment_window": "400"}}
}"#;

/// Writes every v1 subspace from the mainnet genesis values, in v1
/// InitGenesis order (`app/modules.go` `SetOrderInitGenesis`), after the
/// consensus params of InitChain.
fn write_mainnet_genesis_params(ctx: &mut sdk::Context<'_>) -> sdk::Result<()> {
    let g: serde_json::Value = serde_json::from_str(MAINNET_GENESIS_PARAMS).unwrap();
    consensus_params::store(ctx, &mainnet_consensus_params())?;
    params::set_auth(ctx, &g["auth"]["params"])?;
    params::set_bank(ctx, &g["bank"]["params"])?;
    params::set_distribution(ctx, &g["distribution"]["params"])?;
    params::set_staking(ctx, &g["staking"]["params"])?;
    params::set_slashing(ctx, &g["slashing"]["params"])?;
    let gov = &g["gov"];
    x_gov::set_v046_params(
        ctx,
        &serde_json::from_value(gov["deposit_params"].clone()).unwrap(),
        &serde_json::from_value(gov["voting_params"].clone()).unwrap(),
        &serde_json::from_value(gov["tally_params"].clone()).unwrap(),
    )?;
    params::set_crisis(ctx, &g["crisis"]["constant_fee"])?;
    params::set_ibc_client(ctx, &g["ibc"]["client_genesis"]["params"])?;
    params::set_ibc_connection(ctx, &g["ibc"]["connection_genesis"]["params"])?;
    params::set_transfer(ctx, &g["transfer"]["params"])?;
    params::set_blob(ctx, &g["blob"]["params"])?;
    params::set_qgb(ctx, &g["qgb"]["params"])
}

/// From the mainnet genesis values, the v1 subspaces hold exactly the
/// oracle's 36 height-0 `params` entries, and the committed `params`
/// root is the oracle's. cosmos-sdk v0.46 writes the genesis state to
/// IAVL from the sorted cache (`cachekv.Store.Write`), so the tree is
/// built in key order, as the multistore's flush does.
#[test]
fn genesis_params_match_the_v1_oracle() {
    let mut ms = v1_store();
    let mut ctx = sdk::Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
    ctx.consensus_params.version = Some(VersionParams { app: 1 });
    write_mainnet_genesis_params(&mut ctx).unwrap();
    drop(ctx);
    ms.flush().unwrap();
    ms.commit().unwrap();

    let written: BTreeMap<Vec<u8>, Vec<u8>> = ms
        .range(x_params::STORE_KEY, None, None, false)
        .unwrap()
        .into_iter()
        .collect();
    let expected = oracle_kv("params");
    assert_eq!(expected.len(), 36);
    let show = |m: &BTreeMap<Vec<u8>, Vec<u8>>| -> Vec<String> {
        m.iter()
            .map(|(k, v)| {
                format!(
                    "{}={}",
                    String::from_utf8_lossy(k),
                    String::from_utf8_lossy(v)
                )
            })
            .collect()
    };
    assert_eq!(show(&written), show(&expected));
    let info = ms.root().last_commit_info().unwrap();
    let root = info.store_info("params").unwrap().commit_id.unwrap().hash;
    assert_eq!(root.to_vec(), oracle_root("params"));
}

/// Each v1 key table registers every key once.
#[test]
fn v1_key_tables_have_unique_keys() {
    for s in params::V1_SUBSPACES {
        let keys: std::collections::BTreeSet<&str> = s.table.iter().map(|p| p.key).collect();
        assert_eq!(keys.len(), s.table.len(), "{}", s.name);
    }
}

fn params_ctx(f: impl FnOnce(&mut sdk::Context<'_>)) {
    let mut ms = v1_store();
    let mut ctx = sdk::Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
    f(&mut ctx);
}

type Setter = fn(&mut sdk::Context<'_>, &serde_json::Value) -> sdk::Result<()>;

/// Genesis params that Go rejects, by setter.
fn invalid_param_cases() -> Vec<(&'static str, Setter, serde_json::Value)> {
    use serde_json::json;
    vec![
        ("auth zero", params::set_auth, json!({})),
        (
            "auth unknown field",
            params::set_auth,
            json!({"max_memo_characters": "1",
            "tx_sig_limit": "1", "tx_size_cost_per_byte": "1", "sig_verify_cost_ed25519": "1",
            "sig_verify_cost_secp256k1": "1", "extra": 1}),
        ),
        (
            "auth not a number",
            params::set_auth,
            json!({"max_memo_characters": "x"}),
        ),
        (
            "auth negative",
            params::set_auth,
            json!({"max_memo_characters": "-1"}),
        ),
        (
            "auth float",
            params::set_auth,
            json!({"max_memo_characters": 1.5}),
        ),
        (
            "bank duplicate",
            params::set_bank,
            json!({"send_enabled": [
            {"denom": "utia", "enabled": true}, {"denom": "utia"}]}),
        ),
        (
            "bank bad denom",
            params::set_bank,
            json!({"send_enabled": [{"denom": "u"}]}),
        ),
        (
            "staking no dec",
            params::set_staking,
            json!({"unbonding_time": "1s",
            "max_validators": 1, "max_entries": 1, "bond_denom": "utia"}),
        ),
        (
            "staking bad duration",
            params::set_staking,
            json!({"unbonding_time": "1 s"}),
        ),
        (
            "staking overflow",
            params::set_staking,
            json!({"unbonding_time": "1s",
            "max_validators": 4_294_967_296_u64}),
        ),
        (
            "distribution > 1",
            params::set_distribution,
            json!({"community_tax": "1.1",
            "base_proposer_reward": "0", "bonus_proposer_reward": "0"}),
        ),
        (
            "slashing negative",
            params::set_slashing,
            json!({"signed_blocks_window": "1",
            "min_signed_per_window": "-0.1", "downtime_jail_duration": "1s",
            "slash_fraction_double_sign": "0", "slash_fraction_downtime": "0"}),
        ),
        (
            "crisis nil amount",
            params::set_crisis,
            json!({"denom": "utia"}),
        ),
        (
            "ibc blank client",
            params::set_ibc_client,
            json!({"allowed_clients": [" "]}),
        ),
        (
            "transfer type",
            params::set_transfer,
            json!({"send_enabled": "true"}),
        ),
        (
            "blob not a power of two",
            params::set_blob,
            json!({"gas_per_blob_byte": 8,
            "gov_max_square_size": "48"}),
        ),
        ("qgb nil", params::set_qgb, serde_json::Value::Null),
        (
            "qgb small",
            params::set_qgb,
            json!({"data_commitment_window": "99"}),
        ),
        (
            "qgb large",
            params::set_qgb,
            json!({"data_commitment_window": "1001"}),
        ),
        ("not an object", params::set_transfer, json!([])),
    ]
}

/// Malformed or invalid genesis params are errors (Go: a jsonpb error or
/// a `SetParamSet` panic), never a Rust panic.
#[test]
fn invalid_genesis_params_are_errors() {
    let cases = invalid_param_cases();
    for (name, set, value) in &cases {
        params_ctx(|ctx| assert!(set(ctx, value).is_err(), "{name}"));
    }
}

/// jsonpb's decoding rules: JSON names, quoted 32-bit integers, `null` as
/// the zero value, and a nil list written as amino `null`.
#[test]
fn genesis_params_decode_like_jsonpb() {
    use serde_json::json;
    params_ctx(|ctx| {
        params::set_blob(
            ctx,
            &json!({"gasPerBlobByte": "8", "gov_max_square_size": 64}),
        )
        .unwrap();
        params::set_bank(ctx, &json!({"send_enabled": null})).unwrap();
        params::set_ibc_client(ctx, &json!(null)).unwrap();
        params::set_transfer(ctx, &json!(null)).unwrap();
        params::set_crisis(ctx, &json!({"denom": "u", "amount": "-5"})).unwrap();
        let get = |ctx: &mut sdk::Context<'_>, s: &str, k: &[u8]| {
            String::from_utf8(x_params::get_raw(ctx, s, k).unwrap().unwrap()).unwrap()
        };
        assert_eq!(get(ctx, "blob", b"GasPerBlobByte"), "8");
        assert_eq!(get(ctx, "blob", b"GovMaxSquareSize"), "\"64\"");
        assert_eq!(get(ctx, "bank", b"SendEnabled"), "null");
        assert_eq!(get(ctx, "bank", b"DefaultSendEnabled"), "false");
        assert_eq!(get(ctx, "ibc", b"AllowedClients"), "null");
        assert_eq!(get(ctx, "transfer", b"SendEnabled"), "false");
        // crisis uses `Set`: no validation.
        assert_eq!(
            get(ctx, "crisis", b"ConstantFee"),
            r#"{"denom":"u","amount":"-5"}"#
        );
    });
}
