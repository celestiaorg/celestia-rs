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

/// The stores of app versions 2 and 3.
fn v2_store() -> MultiStore {
    let mut ms = MultiStore::new();
    let v2 = Release::for_app_version(2).expect("v2");
    crate::stores::mount_release(&mut ms, v2).expect("mount v2");
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

/// Modules that record the chain ID BeginBlock sees.
#[derive(Default)]
struct RecordChainId(Vec<String>);

impl Modules for RecordChainId {
    fn init_genesis(&mut self, _: &mut Context<'_>, _: &[u8]) -> Result<Vec<ValidatorUpdate>> {
        Ok(Vec::new())
    }

    fn begin_block(&mut self, _: &mut Context<'_>, header: &TmHeader) -> Result<()> {
        self.0.push(header.chain_id.clone());
        Ok(())
    }

    fn deliver_tx(
        &mut self,
        ctx: &mut Context<'_>,
        block_gas: &mut kvstore::GasMeter,
        tx: &[u8],
    ) -> TxOutcome {
        NoModules.deliver_tx(ctx, block_gas, tx)
    }

    fn end_block(&mut self, _: &mut Context<'_>) -> Result<Vec<ValidatorUpdate>> {
        Ok(Vec::new())
    }
}

/// A driver opened on committed state gets no InitChain. It takes the
/// chain ID from the first block header, and rejects a header of another
/// chain.
#[test]
fn reopened_driver_takes_the_chain_id_from_the_header() {
    let mut ms = v1_store();
    let mut driver = Driver::new(NoModules);
    driver.init_chain(&mut ms, &init_request()).unwrap();
    driver
        .finalize_block(&mut ms, &finalize_request(1))
        .unwrap();
    driver.commit(&mut ms).unwrap();

    let mut reopened = Driver::new(RecordChainId::default());
    reopened
        .finalize_block(&mut ms, &finalize_request(2))
        .unwrap();
    reopened.commit(&mut ms).unwrap();
    assert_eq!(reopened.modules.0, ["celestia"]);

    let mut other = finalize_request(3);
    other.header.as_mut().unwrap().chain_id = "mocha-4".to_owned();
    assert!(reopened.finalize_block(&mut ms, &other).is_err());
    let mut empty = finalize_request(3);
    empty.header.as_mut().unwrap().chain_id = String::new();
    assert!(
        Driver::new(NoModules)
            .finalize_block(&mut ms, &empty)
            .is_err()
    );
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
    let mut ms = v2_store();
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

/// App version 1 runs as itself on the legacy driver; app versions 2
/// and 3 (still run as v9) and every v0.50 version do not.
#[test]
fn only_app_version_1_executes_on_the_legacy_driver() {
    for v in 0..=13 {
        assert_eq!(executes(v), v == 1, "v{v}");
        assert_eq!(is_v046(v), (1..=3).contains(&v), "v{v}");
    }
}

/// The v0.46 release of a commit info is the one whose store set it is
/// exactly; a v0.50 commit info has none.
#[test]
fn release_of_recognizes_v046_commit_infos() {
    for v in 1..=12 {
        let release = Release::for_app_version(v).unwrap();
        let mut ms = MultiStore::new();
        crate::stores::mount_release(&mut ms, release).unwrap();
        ms.commit().unwrap();
        let info = ms.root().last_commit_info().unwrap();
        let got = release_of(info).map(|r| r.app_version);
        let want = match v {
            1 => Some(1),
            2 | 3 => Some(2),
            _ => None,
        };
        assert_eq!(got, want, "v{v}");
    }
    let mut ms = MultiStore::new();
    for (name, kind) in crate::stores::all() {
        ms.mount(name, kind).unwrap();
    }
    ms.commit().unwrap();
    assert!(release_of(ms.root().last_commit_info().unwrap()).is_none());
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

/// InitChain's genesis validator check: a non-empty request must list the
/// validators InitGenesis returns.
#[test]
fn genesis_validators_must_match_the_request() {
    use celestia_proto::celestia_core_proto::tendermint::crypto::{PublicKey, public_key::Sum};
    let v = |k: u8, power: i64| abci::ValidatorUpdate {
        pub_key: Some(PublicKey {
            sum: Some(Sum::Ed25519(vec![k; 32])),
        }),
        power,
    };
    assert!(check_genesis_validators(&[], &[v(1, 1)]).is_ok());
    assert!(check_genesis_validators(&[v(2, 1), v(1, 1)], &[v(1, 1), v(2, 1)]).is_ok());
    assert!(check_genesis_validators(&[v(1, 1)], &[v(1, 2)]).is_err());
    assert!(check_genesis_validators(&[v(1, 1)], &[v(1, 1), v(2, 1)]).is_err());
}

/// The blobstream hook stores the default EVM address of a new validator
/// at app version 1, rejects a second validator with the same address,
/// and does nothing where `qgb` is not mounted.
#[test]
fn qgb_hook_writes_default_evm_addresses_at_v1() {
    use x_staking::StakingHooks as _;
    let val = [7u8; 20];
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let mut ms = v1_store();
    let mut ctx = sdk::Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
    ctx.consensus_params.version = Some(VersionParams { app: 1 });
    x_qgb::Hooks
        .after_validator_created(&mut ctx, &val)
        .unwrap();
    assert_eq!(
        ctx.get(x_qgb::STORE_KEY, &[b"EVMAddress".as_slice(), &val].concat())
            .unwrap(),
        Some(val.to_vec())
    );
    let e = x_qgb::Hooks
        .after_validator_created(&mut ctx, &val)
        .unwrap_err();
    assert_eq!((e.codespace, e.code), ("qgb", 37));
    ctx.consensus_params.version = Some(VersionParams { app: 2 });
    assert!(
        x_qgb::Hooks
            .after_validator_created(&mut ctx, &[8; 20])
            .is_ok()
    );
    drop(ctx);

    let mut ms = MultiStore::new();
    ms.mount("staking", sdk::Kind::Iavl).unwrap();
    let mut ctx = sdk::Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
    ctx.consensus_params.version = Some(VersionParams { app: 1 });
    assert!(x_qgb::Hooks.after_validator_created(&mut ctx, &val).is_ok());
}

/// The v1 InitChainer rejects what ibc-go v6 or the module manager
/// reject: a v8 channel params field, genesis light clients, unknown
/// fields, and a genesis with no validators.
#[test]
fn v1_init_chainer_rejects_invalid_genesis() {
    use serde_json::json;
    let ibc = |channel: serde_json::Value, client: serde_json::Value| {
        json!({"ibc": {"client_genesis": client, "connection_genesis": {},
            "channel_genesis": channel}})
    };
    let cases = [
        ibc(json!({"params": {}}), json!({})),
        ibc(
            json!({}),
            json!({"clients": [{"client_id": "07-tendermint-0"}]}),
        ),
        ibc(json!({}), json!({"bogus": 1})),
        json!({"mint": {"bond_denom": "utia", "x": 1}}),
        json!({"qgb": {"params": null}}),
        json!({"crisis": {"constant_fee": {"denom": "utia"}}}),
        json!({}),
    ];
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    for (i, state) in cases.iter().enumerate() {
        let mut ms = v1_store();
        let mut ctx = sdk::Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
        ctx.consensus_params.version = Some(VersionParams { app: 1 });
        let bytes = serde_json::to_vec(state).unwrap();
        assert!(genesis::init_chainer(&mut ctx, &bytes).is_err(), "case {i}");
    }
}

/// At app versions 1 to 3 the shared modules use the cosmos-sdk v0.46
/// layout: params from their subspace, the `globalAccountNumber`
/// `UInt64Value`, the bank `0x00` index sentinel, and no staking
/// delegations-by-validator index or stored validator updates.
#[test]
fn shared_modules_use_the_v046_layout_at_v1() {
    let g: serde_json::Value = serde_json::from_str(MAINNET_GENESIS_PARAMS).unwrap();
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    params_ctx(|ctx| {
        ctx.consensus_params.version = Some(VersionParams { app: 1 });
        params::set_auth(ctx, &g["auth"]["params"]).unwrap();
        params::set_staking(ctx, &g["staking"]["params"]).unwrap();
        assert_eq!(x_auth::get_params(ctx).unwrap().tx_sig_limit, 7);
        assert_eq!(
            x_staking::store::get_params(ctx).unwrap().max_validators,
            100
        );
        assert_eq!(x_staking::store::bond_denom(ctx).unwrap(), "utia");
        let p = x_staking::store::get_params(ctx).unwrap();
        x_staking::store::set_params(ctx, &p).unwrap();
        assert_eq!(ctx.get(x_staking::STORE_KEY, &[0x51]).unwrap(), None);

        assert_eq!(x_auth::next_account_number(ctx).unwrap(), 0);
        assert_eq!(x_auth::next_account_number(ctx).unwrap(), 1);
        assert_eq!(
            ctx.get(x_auth::STORE_KEY, b"globalAccountNumber").unwrap(),
            Some(vec![0x08, 0x02])
        );

        let addr = [3u8; 20];
        let coin = sdk::coins::Coin::new("utia", 5);
        x_bank::set_balance(ctx, &addr, &coin).unwrap();
        let mut index = vec![0x03];
        index.extend_from_slice(b"utia\x00\x14");
        index.extend_from_slice(&addr);
        assert_eq!(ctx.get(x_bank::STORE_KEY, &index).unwrap(), Some(vec![0]));

        x_staking::store::set_validator_updates(ctx, &[]).unwrap();
        assert_eq!(ctx.get(x_staking::STORE_KEY, &[0x61]).unwrap(), None);
    });
}

/// A v1 context at `height` with the mainnet params, and two bonded
/// validators with 50 % and no commission, 100 power each (the setup of
/// cosmos-sdk v0.46 `TestAllocateTokensToManyValidators`).
fn v046_two_validators(ms: &mut MultiStore, height: i64) -> (sdk::Context<'_>, [Vec<u8>; 2]) {
    use x_staking::types::{BondStatus, Validator};
    let header = sdk::Header {
        chain_id: "celestia".into(),
        height,
        ..sdk::Header::default()
    };
    let mut ctx = sdk::Context::new(ms, header, sdk::ExecMode::Finalize);
    ctx.consensus_params.version = Some(VersionParams { app: 1 });
    write_mainnet_genesis_params(&mut ctx).unwrap();
    let mut cons = [Vec::new(), Vec::new()];
    for (i, rate) in [(1u8, 5), (2, 0)] {
        let pk = celestia_proto::Any::pack(&celestia_proto::cosmos::crypto::ed25519::PubKey {
            key: vec![i; 32],
        });
        let op = x_staking::addr::val_to_string(&[i; 20]).unwrap();
        let mut v = Validator::new(
            op,
            pk,
            celestia_proto::cosmos::staking::v1beta1::Description::default(),
        );
        v.status = BondStatus::Bonded;
        v.tokens = math::Int::from_u64(100_000_000);
        v.delegator_shares = math::LegacyDec::new(100_000_000);
        v.commission.rate = math::LegacyDec::new_with_prec(rate, 1).unwrap();
        x_staking::store::set_validator(&mut ctx, &v).unwrap();
        x_staking::store::set_validator_by_cons_addr(&mut ctx, &v).unwrap();
        x_staking::store::set_validator_by_power_index(&mut ctx, &v).unwrap();
        x_staking::store::set_last_validator_power(&mut ctx, &[i; 20], 100).unwrap();
        x_distribution::keeper::initialize_validator(&mut ctx, &[i; 20]).unwrap();
        x_slashing::add_pubkey(&mut ctx, &v.pubkey().unwrap()).unwrap();
        cons[usize::from(i - 1)] = v.cons_addr().unwrap();
    }
    (ctx, cons)
}

fn vote(cons: &[u8], signed: bool) -> abci::VoteInfo {
    abci::VoteInfo {
        validator: Some(abci::Validator {
            address: cons.to_vec(),
            power: 100,
        }),
        block_id_flag: if signed { 2 } else { 1 },
    }
}

/// cosmos-sdk v0.46 distribution pays the previous proposer
/// `base + bonus * signed fraction` before the voters' shares: the
/// expectations of v0.46 `TestAllocateTokensToManyValidators` (community
/// tax 2 %, base 1 %, bonus 4 %, 100 utia of fees, both signed, the second
/// validator proposed).
#[test]
fn v046_distribution_pays_the_proposer_reward() {
    use x_distribution::store as ds;
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let mut ms = v1_store();
    let (mut ctx, cons) = v046_two_validators(&mut ms, 2);
    params::set_distribution(
        &mut ctx,
        &serde_json::json!({"community_tax": "0.02", "base_proposer_reward": "0.01",
            "bonus_proposer_reward": "0.04", "withdraw_addr_enabled": true}),
    )
    .unwrap();
    x_distribution::store::set_fee_pool(&mut ctx, &sdk::dec_coins::DecCoins::new()).unwrap();
    x_distribution::store::set_previous_proposer(&mut ctx, &cons[1]).unwrap();
    let fc = x_auth::get_module_account(&mut ctx, x_auth::FEE_COLLECTOR)
        .unwrap()
        .unwrap();
    let fc = x_auth::Account::Module(fc).address().unwrap();
    x_bank::set_balance(&mut ctx, &fc, &sdk::coins::Coin::new("utia", 100)).unwrap();
    ctx.vote_infos = vec![vote(&cons[0], true), vote(&cons[1], true)];
    ctx.header.proposer_address = cons[0].clone();
    x_distribution::begin_block(&mut ctx).unwrap();

    let dec = |v: &sdk::dec_coins::DecCoins| v.to_string();
    let s = |n: &str| format!("{n}utia");
    let (a, b) = ([1u8; 20], [2u8; 20]);
    assert_eq!(
        dec(&ds::get_outstanding(&mut ctx, &a).unwrap()),
        s("46.500000000000000000")
    );
    assert_eq!(
        dec(&ds::get_outstanding(&mut ctx, &b).unwrap()),
        s("51.500000000000000000")
    );
    assert_eq!(
        dec(&ds::get_fee_pool(&mut ctx).unwrap()),
        s("2.000000000000000000")
    );
    assert_eq!(
        dec(&ds::get_commission(&mut ctx, &a).unwrap()),
        s("23.250000000000000000")
    );
    assert!(ds::get_commission(&mut ctx, &b).unwrap().is_empty());
    assert_eq!(
        dec(&ds::get_current(&mut ctx, &a).unwrap().rewards),
        s("23.250000000000000000")
    );
    assert_eq!(
        dec(&ds::get_current(&mut ctx, &b).unwrap().rewards),
        s("51.500000000000000000")
    );
    let reward = ctx
        .events
        .events()
        .iter()
        .find(|e| e.r#type == "proposer_reward")
        .unwrap();
    assert_eq!(reward.attributes[0].value, s("5.000000000000000000"));
    assert_eq!(ds::get_previous_proposer(&mut ctx).unwrap(), cons[0]);
}

/// cosmos-sdk v0.46 slashing stores one `BoolValue` key per missed index
/// (an empty value once signed again), counts a jailed validator, and
/// jails for downtime.
#[test]
fn v046_slashing_tracks_missed_blocks_per_index() {
    crate::modules::install_staking_hooks();
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let mut ms = v1_store();
    let (mut ctx, cons) = v046_two_validators(&mut ms, 1);
    params::set_slashing(
        &mut ctx,
        &serde_json::json!({"signed_blocks_window": "2", "min_signed_per_window": "0.5",
            "downtime_jail_duration": "60s", "slash_fraction_double_sign": "0.02",
            "slash_fraction_downtime": "0"}),
    )
    .unwrap();
    for c in &cons {
        let info = x_slashing::SigningInfo {
            address: x_staking::addr::cons_to_string(c).unwrap(),
            start_height: 0,
            index_offset: 0,
            jailed_until: sdk::time::unix_epoch(),
            tombstoned: false,
            missed_blocks_counter: 0,
        };
        x_slashing::set_signing_info(&mut ctx, c, &info).unwrap();
    }
    let key = |i| x_slashing::v046_missed_block_key(&cons[0], i);
    let counter = |ctx: &mut sdk::Context<'_>| {
        x_slashing::get_signing_info(ctx, &cons[0])
            .unwrap()
            .unwrap()
            .missed_blocks_counter
    };
    // Height 1, index 0: missed.
    ctx.vote_infos = vec![vote(&cons[0], false), vote(&cons[1], true)];
    x_slashing::begin_block(&mut ctx).unwrap();
    assert_eq!(
        ctx.get(x_slashing::STORE_KEY, &key(0)).unwrap(),
        Some(vec![0x08, 0x01])
    );
    assert_eq!(ctx.get(x_slashing::STORE_KEY, &key(1)).unwrap(), None);
    assert_eq!(counter(&mut ctx), 1);
    let liveness = ctx.events.take();
    assert_eq!(liveness.len(), 1);
    assert_eq!(liveness[0].r#type, "liveness");
    // Height 2, index 1: signed, by a jailed validator (still counted).
    let mut v = x_staking::store::validator(&mut ctx, &[1; 20]).unwrap();
    v.jailed = true;
    x_staking::store::set_validator(&mut ctx, &v).unwrap();
    ctx.header.height = 2;
    ctx.vote_infos = vec![vote(&cons[0], true), vote(&cons[1], true)];
    x_slashing::begin_block(&mut ctx).unwrap();
    assert_eq!(ctx.get(x_slashing::STORE_KEY, &key(1)).unwrap(), None);
    // Height 3, index 0 again: signed, so the bit is cleared.
    ctx.header.height = 3;
    x_slashing::begin_block(&mut ctx).unwrap();
    assert_eq!(
        ctx.get(x_slashing::STORE_KEY, &key(0)).unwrap(),
        Some(Vec::new())
    );
    assert_eq!(counter(&mut ctx), 0);
    // Past the first window, two misses in a row exceed the one allowed:
    // v0.46 slashes (by a zero fraction, which still records a
    // distribution slash event), jails and resets the window.
    v.jailed = false;
    x_staking::store::set_validator(&mut ctx, &v).unwrap();
    ctx.events.take();
    for h in [4, 5] {
        ctx.header.height = h;
        ctx.vote_infos = vec![vote(&cons[0], false)];
        x_slashing::begin_block(&mut ctx).unwrap();
    }
    let v = x_staking::store::validator(&mut ctx, &[1; 20]).unwrap();
    assert!(v.jailed);
    assert_eq!(v.tokens, math::Int::from_u64(100_000_000));
    assert_eq!(
        ctx.get(
            x_staking::STORE_KEY,
            &x_staking::keys::power_index(100, &[1; 20])
        )
        .unwrap(),
        None
    );
    let info = x_slashing::get_signing_info(&mut ctx, &cons[0])
        .unwrap()
        .unwrap();
    assert_eq!((info.missed_blocks_counter, info.index_offset), (0, 0));
    assert_eq!(info.jailed_until.seconds, ctx.header.time.seconds + 60);
    for i in 0..2 {
        assert_eq!(ctx.get(x_slashing::STORE_KEY, &key(i)).unwrap(), None);
    }
    let slash_events =
        x_distribution::store::slash_events_between(&mut ctx, &[1; 20], 0, 100).unwrap();
    assert_eq!(slash_events.len(), 1, "{slash_events:?}");
    assert_eq!(slash_events[0].0, 5);
    assert!(slash_events[0].2.is_zero());
    let events = ctx.events.take();
    let slash = events.iter().find(|e| e.r#type == "slash").unwrap();
    let burned = slash
        .attributes
        .iter()
        .find(|a| a.key == "burned_coins")
        .unwrap();
    assert_eq!(burned.value, "0");
    // Jailed, the validator is no longer slashed.
    ctx.header.height = 6;
    x_slashing::begin_block(&mut ctx).unwrap();
}

/// cosmos-sdk v0.46 `CancelUnbondingDelegation`: delegates part of an
/// unbonding entry back (from the not-bonded pool to the bonded pool of a
/// bonded validator), shrinking the entry, and removes the unbonding
/// delegation with its last entry. A missing entry or a jailed validator
/// fails.
#[test]
fn v046_cancel_unbonding_delegation() {
    use celestia_proto::cosmos::base::v1beta1::Coin;
    use celestia_proto::cosmos::staking::v1beta1::MsgCancelUnbondingDelegation;
    crate::modules::install_staking_hooks();
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let mut ms = v1_store();
    let (mut ctx, _) = v046_two_validators(&mut ms, 10);
    x_distribution::store::set_fee_pool(&mut ctx, &sdk::dec_coins::DecCoins::new()).unwrap();
    ctx.header.time = pbjson_types::Timestamp {
        seconds: 1_000,
        nanos: 0,
    };
    let del = [9u8; 20];
    let val = [1u8; 20];
    x_staking::delegation::set_ubd_entry(
        &mut ctx,
        &del,
        &val,
        3,
        pbjson_types::Timestamp {
            seconds: 2_000,
            nanos: 0,
        },
        &math::Int::from_u64(100),
    )
    .unwrap();
    let pool = |name: &str| x_auth::module_address(name);
    x_bank::set_balance(
        &mut ctx,
        &pool(x_staking::NOT_BONDED_POOL),
        &sdk::coins::Coin::new("utia", 100),
    )
    .unwrap();
    let cancel = |amount: &str, height: i64| {
        celestia_proto::Any::pack(&MsgCancelUnbondingDelegation {
            delegator_address: x_auth::bech32(&del).unwrap(),
            validator_address: x_staking::addr::val_to_string(&val).unwrap(),
            amount: Some(Coin {
                denom: "utia".into(),
                amount: amount.into(),
            }),
            creation_height: height,
        })
    };
    msgs::validate_basic(&cancel("40", 3)).unwrap();
    let e = msgs::validate_basic(&cancel("40", 0)).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 18));
    let e = msgs::route(&mut ctx, &cancel("40", 4)).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 38), "{}", e.log);
    let e = msgs::route(&mut ctx, &cancel("101", 3)).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 18), "{}", e.log);

    msgs::route(&mut ctx, &cancel("40", 3)).unwrap();
    let ubd = x_staking::store::get_ubd(&mut ctx, &del, &val)
        .unwrap()
        .unwrap();
    assert_eq!(ubd.entries.len(), 1);
    assert_eq!(ubd.entries[0].balance, math::Int::from_u64(60));
    assert_eq!(ubd.entries[0].initial_balance, math::Int::from_u64(60));
    let d = x_staking::store::get_delegation(&mut ctx, &del, &val)
        .unwrap()
        .unwrap();
    assert_eq!(d.shares, math::LegacyDec::new(40));
    let bal = |ctx: &mut sdk::Context<'_>, name: &str| {
        x_bank::get_balance(ctx, &pool(name), "utia")
            .unwrap()
            .amount
    };
    assert_eq!(
        bal(&mut ctx, x_staking::NOT_BONDED_POOL),
        math::Int::from_u64(60)
    );
    assert_eq!(
        bal(&mut ctx, x_staking::BONDED_POOL),
        math::Int::from_u64(40)
    );
    let ev = ctx.events.take();
    assert!(ev.iter().any(|e| e.r#type == "cancel_unbonding_delegation"));
    // Only the pool transfer's `message` (bank's sender): the msg server
    // emits no `message` event of its own.
    assert!(
        !ev.iter()
            .any(|e| e.r#type == "message" && e.attributes.iter().any(|a| a.key == "module"))
    );

    msgs::route(&mut ctx, &cancel("60", 3)).unwrap();
    assert!(
        x_staking::store::get_ubd(&mut ctx, &del, &val)
            .unwrap()
            .is_none()
    );

    let mut v = x_staking::store::validator(&mut ctx, &val).unwrap();
    v.jailed = true;
    x_staking::store::set_validator(&mut ctx, &v).unwrap();
    let e = msgs::route(&mut ctx, &cancel("1", 3)).unwrap_err();
    assert_eq!((e.codespace, e.code), ("staking", 7), "{}", e.log);
}

/// cosmos-sdk v0.46 `Unjail`: needs the validator's self-delegation at
/// its minimum, a jailed validator, and the jail time to have passed;
/// then the validator is back in the power index.
#[test]
fn v046_unjail() {
    use celestia_proto::cosmos::slashing::v1beta1::MsgUnjail;
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let mut ms = v1_store();
    let (mut ctx, cons) = v046_two_validators(&mut ms, 10);
    let val = [1u8; 20];
    let unjail = celestia_proto::Any::pack(&MsgUnjail {
        validator_addr: x_staking::addr::val_to_string(&val).unwrap(),
    });
    msgs::validate_basic(&unjail).unwrap();
    let e = msgs::route(&mut ctx, &unjail).unwrap_err();
    assert_eq!((e.codespace, e.code), ("slashing", 6), "{}", e.log);
    x_staking::store::set_delegation(
        &mut ctx,
        &x_staking::types::Delegation {
            delegator_address: x_auth::bech32(&val).unwrap(),
            validator_address: x_staking::addr::val_to_string(&val).unwrap(),
            shares: math::LegacyDec::new(100_000_000),
        },
    )
    .unwrap();
    let e = msgs::route(&mut ctx, &unjail).unwrap_err();
    assert_eq!((e.codespace, e.code), ("slashing", 5), "{}", e.log);
    x_staking::jail(&mut ctx, &cons[0]).unwrap();
    let mut info = x_slashing::SigningInfo {
        address: x_staking::addr::cons_to_string(&cons[0]).unwrap(),
        start_height: 0,
        index_offset: 0,
        jailed_until: pbjson_types::Timestamp {
            seconds: 60,
            nanos: 0,
        },
        tombstoned: false,
        missed_blocks_counter: 0,
    };
    x_slashing::set_signing_info(&mut ctx, &cons[0], &info).unwrap();
    ctx.header.time = pbjson_types::Timestamp {
        seconds: 59,
        nanos: 0,
    };
    let e = msgs::route(&mut ctx, &unjail).unwrap_err();
    assert_eq!((e.codespace, e.code), ("slashing", 4), "{}", e.log);
    info.tombstoned = true;
    x_slashing::set_signing_info(&mut ctx, &cons[0], &info).unwrap();
    ctx.header.time.seconds = 60;
    let e = msgs::route(&mut ctx, &unjail).unwrap_err();
    assert_eq!((e.codespace, e.code), ("slashing", 4), "{}", e.log);
    info.tombstoned = false;
    x_slashing::set_signing_info(&mut ctx, &cons[0], &info).unwrap();
    msgs::route(&mut ctx, &unjail).unwrap();
    assert!(!x_staking::store::validator(&mut ctx, &val).unwrap().jailed);
    assert!(
        ctx.get(
            x_staking::STORE_KEY,
            &x_staking::keys::power_index(100, &val)
        )
        .unwrap()
        .is_some()
    );
    let ev = ctx.events.take();
    let m = ev.iter().find(|e| e.r#type == "message").unwrap();
    assert_eq!(m.attributes[0].value, "slashing");
}

/// cosmos-sdk v0.46 staking stores `HistoricalInfo` under the decimal
/// height with the given header, and begins unbonding a validator
/// without an unbonding ID.
#[test]
fn v046_staking_layout() {
    use x_staking::types::BondStatus;
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let mut ms = v1_store();
    let (mut ctx, _) = v046_two_validators(&mut ms, 7);
    let header = TmHeader {
        chain_id: "celestia".into(),
        height: 7,
        data_hash: vec![9; 32],
        ..TmHeader::default()
    };
    x_staking::abci::begin_block_v046(&mut ctx, &header).unwrap();
    let hi = ctx.get(x_staking::STORE_KEY, b"P7").unwrap().unwrap();
    let hi = <celestia_proto::cosmos::staking::v1beta1::HistoricalInfo as prost::Message>::decode(
        hi.as_slice(),
    )
    .unwrap();
    assert_eq!(hi.header, Some(header));
    assert_eq!(hi.valset.len(), 2);

    let v = x_staking::store::validator(&mut ctx, &[1; 20]).unwrap();
    let v = x_staking::validator::begin_unbonding_validator(&mut ctx, v).unwrap();
    assert_eq!(v.status, BondStatus::Unbonding);
    assert!(v.unbonding_ids.is_empty());
    let stored = x_staking::store::validator(&mut ctx, &[1; 20]).unwrap();
    assert!(stored.unbonding_ids.is_empty());
    for prefix in [0x37u8, 0x38, 0x39] {
        let n = ctx
            .multistore()
            .range(
                x_staking::STORE_KEY,
                Some(&[prefix]),
                Some(&[prefix + 1]),
                false,
            )
            .unwrap()
            .len();
        assert_eq!(n, 0, "prefix {prefix:#x}");
    }
}

/// Outside the legacy driver's scope, the raw app version 1 (as the main
/// app sets it) does not turn the blobstream hook on: it neither reads
/// nor writes `qgb` and consumes no gas.
#[test]
fn qgb_hook_is_inert_without_the_legacy_scope() {
    use x_staking::StakingHooks as _;
    sdk::version::set_app_version(1);
    let mut ms = v1_store();
    let mut ctx = sdk::Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
    ctx.consensus_params.version = Some(VersionParams { app: 1 });
    for _ in 0..2 {
        x_qgb::Hooks
            .after_validator_created(&mut ctx, &[7; 20])
            .unwrap();
    }
    assert_eq!(ctx.gas.consumed(), 0);
    assert_eq!(
        ctx.get(
            x_qgb::STORE_KEY,
            &[b"EVMAddress".as_slice(), &[7; 20]].concat()
        )
        .unwrap(),
        None
    );
    drop(ctx);
    sdk::version::set_app_version(0);
}

/// Without the legacy scope, shared modules keep the v0.50 layout at app
/// versions 1 to 3 (the main app runs them as v9), and the v1 ante
/// handler refuses to run.
#[test]
fn shared_modules_keep_the_v050_layout_without_the_legacy_scope() {
    for v in 1..=3 {
        sdk::version::set_app_version(v);
        params_ctx(|ctx| {
            ctx.consensus_params.version = Some(VersionParams { app: v });
            assert!(!ctx.is_sdk_v046());
            let p = x_staking::store::get_params(ctx).unwrap();
            x_staking::store::set_params(ctx, &p).unwrap();
            assert!(ctx.get(x_staking::STORE_KEY, &[0x51]).unwrap().is_some());
            assert_eq!(x_auth::next_account_number(ctx).unwrap(), 0);
            assert_eq!(
                ctx.get(x_auth::STORE_KEY, b"globalAccountNumber").unwrap(),
                None
            );
            let addr = [3u8; 20];
            x_bank::set_balance(ctx, &addr, &sdk::coins::Coin::new("utia", 5)).unwrap();
            let mut index = vec![0x03];
            index.extend_from_slice(b"utia\x00\x14");
            index.extend_from_slice(&addr);
            assert_eq!(ctx.get(x_bank::STORE_KEY, &index).unwrap(), Some(vec![]));
            x_staking::store::set_validator_updates(ctx, &[]).unwrap();
            assert!(ctx.get(x_staking::STORE_KEY, &[0x61]).unwrap().is_some());
            let (e, _) = crate::txpipe::ante_v046(ctx, &sdk::tx::Tx::default(), false).unwrap_err();
            assert!(e.log.contains("only on the legacy driver"), "{}", e.log);
        });
    }
    sdk::version::set_app_version(0);
}

/// Records whether cosmos-sdk v0.46 semantics are active in each phase and
/// fails the phase named by `fail`.
#[derive(Default)]
struct Probe {
    seen: Vec<(&'static str, bool, u64)>,
    fail: Option<&'static str>,
}

impl Probe {
    fn phase(&mut self, ctx: &sdk::Context<'_>, name: &'static str) -> Result<()> {
        self.seen
            .push((name, ctx.is_sdk_v046(), sdk::version::app_version()));
        if self.fail == Some(name) {
            return Err(sdk::error::sdk::logic(name));
        }
        Ok(())
    }
}

impl Modules for Probe {
    fn init_genesis(&mut self, ctx: &mut Context<'_>, _: &[u8]) -> Result<Vec<ValidatorUpdate>> {
        self.phase(ctx, "init_genesis").map(|()| Vec::new())
    }

    fn begin_block(&mut self, ctx: &mut Context<'_>, _: &TmHeader) -> Result<()> {
        self.phase(ctx, "begin_block")
    }

    fn deliver_tx(
        &mut self,
        ctx: &mut Context<'_>,
        _: &mut kvstore::GasMeter,
        _: &[u8],
    ) -> TxOutcome {
        let result = self.phase(ctx, "deliver_tx").map(|()| TxSuccess::default());
        TxOutcome {
            gas_wanted: 0,
            gas_used: 0,
            events: Vec::new(),
            result,
        }
    }

    fn end_block(&mut self, ctx: &mut Context<'_>) -> Result<Vec<ValidatorUpdate>> {
        self.phase(ctx, "end_block").map(|()| Vec::new())
    }
}

/// The driver turns cosmos-sdk v0.46 semantics on for its modules at the
/// block's app version, and restores the thread's previous state when each
/// call returns, on success and on error.
#[test]
fn driver_scopes_sdk_v046_semantics_to_its_calls() {
    let restored = || {
        assert_eq!(sdk::version::app_version(), 7);
        assert!(!sdk::version::sdk_v046_active());
        assert!(!sdk::version::is_sdk_v046(1));
    };
    sdk::version::set_app_version(7);

    let mut ms = v2_store();
    let mut driver = Driver::new(Probe::default());
    let mut req = init_request();
    req.consensus_params.as_mut().unwrap().version = Some(VersionParams { app: 2 });
    driver.init_chain(&mut ms, &req).unwrap();
    restored();
    let mut block = finalize_request(1);
    block.txs = vec![vec![1]];
    driver.finalize_block(&mut ms, &block).unwrap();
    restored();
    driver.commit(&mut ms).unwrap();
    restored();
    assert!(driver.commit(&mut ms).is_err());
    restored();
    let seen: Vec<_> = driver
        .modules
        .seen
        .iter()
        .map(|(_, on, v)| (*on, *v))
        .collect();
    assert_eq!(seen, vec![(true, 2); 4]);

    for fail in ["init_genesis", "begin_block", "end_block"] {
        let mut ms = v1_store();
        let mut driver = Driver::new(Probe {
            fail: Some(fail),
            ..Probe::default()
        });
        let res = driver.init_chain(&mut ms, &init_request());
        restored();
        if fail == "init_genesis" {
            assert!(res.is_err());
            continue;
        }
        res.unwrap();
        assert!(
            driver
                .finalize_block(&mut ms, &finalize_request(1))
                .is_err()
        );
        restored();
    }
    sdk::version::set_app_version(0);
}

/// Mainnet block `height`'s transactions (`conformance/fixtures/mainnet/
/// blocks.jsonl.gz`).
fn mainnet_txs(height: i64) -> Vec<Vec<u8>> {
    use base64::Engine as _;
    use std::io::BufRead as _;
    let f = std::fs::File::open(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../conformance/fixtures/mainnet/blocks.jsonl.gz"
    ))
    .unwrap();
    for line in std::io::BufReader::new(GzDecoder::new(f)).lines() {
        let d: serde_json::Value = serde_json::from_str(&line.unwrap()).unwrap();
        if d["height"] == height {
            return d["result"]["block"]["data"]["txs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| {
                    base64::engine::general_purpose::STANDARD
                        .decode(t.as_str().unwrap())
                        .unwrap()
                })
                .collect();
        }
    }
    panic!("no block {height}");
}

/// The Go oracle's result of transaction `index` of block `height`
/// (`tx-results.jsonl.gz`).
fn oracle_tx_result(height: i64, index: usize) -> serde_json::Value {
    let mut text = String::new();
    GzDecoder::new(
        &include_bytes!(
            "../../../../../conformance/fixtures/mainnet/v1-oracle/tx-results.jsonl.gz"
        )[..],
    )
    .read_to_string(&mut text)
    .unwrap();
    text.lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .find(|d| d["height"] == height)
        .unwrap()["txs"][index]
        .clone()
}

/// `outcome` shaped as `DeliverTx` returns it, against the oracle's
/// record: every field of the result and its events.
fn assert_oracle_tx(outcome: TxOutcome, want: &serde_json::Value) {
    let got = deliver_tx_result(outcome);
    let events: Vec<serde_json::Value> = got
        .events
        .iter()
        .map(|e| {
            serde_json::json!({"type": e.r#type, "attributes": e.attributes.iter().map(|a| {
                serde_json::json!({"key": a.key, "value": a.value, "index": a.index})
            }).collect::<Vec<_>>()})
        })
        .collect();
    let got = serde_json::json!({
        "index": want["index"], "hash": want["hash"],
        "code": got.code, "codespace": got.codespace,
        "gas_wanted": got.gas_wanted, "gas_used": got.gas_used,
        "data": hex::encode(&got.data), "log": got.log, "info": got.info,
        "events": events,
    });
    assert_eq!(&got, want);
}

/// The ante handler, the bank `MsgSend` server and `runTx`'s gas, events,
/// data and log reproduce mainnet transactions 2:0 (a `MsgSend` that sets
/// the sender's public key) and 3:0 (the same sender's next `MsgSend`,
/// failing with insufficient funds after its fee is paid) exactly as the
/// Go oracle ran them, on the state those transactions read (taken from
/// the oracle's store trace).
#[test]
fn v1_msg_send_matches_the_oracle() {
    let h = |s: &str| hex::decode(s).unwrap();
    let sender = "7d0a39bed044eb9f0c2a202a76075038804a2fe5";
    let recipient = "4e7d04e898379e78298986470deda5ad4ff79670";
    let fee_collector = "f1829676db577682e944fc3493d451b67ff3e29f";
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let mut ms = v1_store();
    let txs2 = mainnet_txs(2);
    let txs3 = mainnet_txs(3);
    {
        let mut ctx = sdk::Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
        ctx.consensus_params.version = Some(VersionParams { app: 1 });
        write_mainnet_genesis_params(&mut ctx).unwrap();
    }
    let acc = |a: &str| [vec![0x01], h(a)].concat();
    let balance = |a: &str| [vec![0x02, 0x14], h(a), b"utia".to_vec()].concat();
    let index = |a: &str| [b"\x03utia\x00\x14".to_vec(), h(a)].concat();
    ms.set(
        x_auth::STORE_KEY,
        acc(sender),
        h("0a202f636f736d6f732e617574682e763162657461312e426173654163636f756e7412340a2f63656c657374696131303539726e306b73676e3465377270327971343876703673387a717935746c39303261786e6e18e406"),
    )
    .unwrap();
    ms.set(
        x_auth::STORE_KEY,
        acc(fee_collector),
        h("0a222f636f736d6f732e617574682e763162657461312e4d6f64756c654163636f756e7412460a350a2f63656c6573746961313778706676616b6d32616d67393632796c73366638347a336b656c6c3863356c706e6a73337318c5e00b120d6665655f636f6c6c6563746f72"),
    )
    .unwrap();
    // The recipient exists (only `HasAccount` reads it).
    let recipient_acc = x_auth::Account::Base(celestia_proto::cosmos::auth::v1beta1::BaseAccount {
        address: x_auth::bech32(&h(recipient)).unwrap(),
        pub_key: None,
        account_number: 1,
        sequence: 0,
    });
    ms.set(
        x_auth::STORE_KEY,
        acc(recipient),
        prost::Message::encode_to_vec(&recipient_acc.to_any()),
    )
    .unwrap();
    for a in [sender, recipient] {
        ms.set(x_bank::STORE_KEY, balance(a), b"9030060000".to_vec())
            .unwrap();
        ms.set(x_bank::STORE_KEY, index(a), vec![0]).unwrap();
    }

    let run = |ms: &mut MultiStore, height: i64, tx: &[u8]| {
        let header = sdk::Header {
            chain_id: "celestia".into(),
            height,
            ..sdk::Header::default()
        };
        let mut ctx = sdk::Context::new(ms, header, sdk::ExecMode::Finalize);
        ctx.consensus_params = consensus_params::load(&mut ctx, 1).unwrap();
        let mut block_gas = tx::block_gas_meter(&ctx);
        tx::deliver_tx(&mut ctx, &mut block_gas, tx)
    };
    assert_oracle_tx(run(&mut ms, 2, &txs2[0]), &oracle_tx_result(2, 0));
    // Block 3 starts after the distribution BeginBlocker emptied the fee
    // collector.
    ms.delete(x_bank::STORE_KEY, &balance(fee_collector))
        .unwrap();
    ms.delete(x_bank::STORE_KEY, &index(fee_collector)).unwrap();
    ms.set(x_bank::STORE_KEY, balance(sender), b"7500000".to_vec())
        .unwrap();
    let failed = run(&mut ms, 3, &txs3[0]);
    assert_oracle_tx(failed, &oracle_tx_result(3, 0));
    // The fee stays paid; the send is rolled back.
    assert_eq!(
        ms.get(x_bank::STORE_KEY, &balance(sender)).unwrap(),
        Some(b"5000000".to_vec())
    );
    assert_eq!(
        ms.get(x_bank::STORE_KEY, &balance(recipient)).unwrap(),
        Some(b"18050120000".to_vec())
    );
}

/// The `ABCIMessageLogs` JSON: events flattened by type (types sorted,
/// attributes of a type concatenated), empty keys and values omitted and
/// strings escaped as Go's `encoding/json` does.
#[test]
fn message_logs_are_go_amino_json() {
    let ev = |t: &str, attrs: &[(&str, &str)]| sdk::event::event(t, attrs.iter().copied());
    let logs = vec![
        (
            0,
            vec![
                ev("message", &[("action", "/a.Msg")]),
                ev("transfer", &[("amount", "1utia")]),
                ev("message", &[("module", "bank")]),
                ev("coin_spent", &[("note", "<&>\u{2028}"), ("empty", "")]),
            ],
        ),
        (1, vec![ev("x", &[])]),
    ];
    assert_eq!(
        tx::message_logs(&logs),
        concat!(
            r#"[{"msg_index":0,"events":[{"type":"coin_spent","attributes":"#,
            "[{\"key\":\"note\",\"value\":\"\\u003c\\u0026\\u003e\\u2028\"},{\"key\":\"empty\"}]},",
            r#"{"type":"message","attributes":[{"key":"action","value":"/a.Msg"},"#,
            r#"{"key":"module","value":"bank"}]},"#,
            r#"{"type":"transfer","attributes":[{"key":"amount","value":"1utia"}]}]},"#,
            r#"{"msg_index":1,"events":[{"type":"x","attributes":null}]}]"#
        )
    );
}

/// `MsgVersioningGateKeeper`'s accepted messages: blobstream at app
/// version 1 only, x/signal from app version 2, nothing it does not know.
#[test]
fn gatekeeper_accepts_the_messages_of_each_app_version() {
    let qgb = "/celestia.qgb.v1.MsgRegisterEVMAddress";
    let signal = "/celestia.signal.v1.MsgSignalVersion";
    let send = "/cosmos.bank.v1beta1.MsgSend";
    assert_eq!(tx::gatekeeper_accepts(1, send), Some(true));
    assert_eq!(tx::gatekeeper_accepts(1, qgb), Some(true));
    assert_eq!(tx::gatekeeper_accepts(1, signal), Some(false));
    for v in [2, 3] {
        assert_eq!(tx::gatekeeper_accepts(v, qgb), Some(false));
        assert_eq!(tx::gatekeeper_accepts(v, signal), Some(true));
        assert_eq!(tx::gatekeeper_accepts(v, send), Some(true));
    }
    assert_eq!(
        tx::gatekeeper_accepts(1, "/cosmos.bank.v1beta1.MsgUpdateParams"),
        Some(false)
    );
    assert_eq!(tx::gatekeeper_accepts(4, send), None);
}

/// `ValidateBasic` of a `MsgMultiSend` whose input and output denoms
/// differ panics in Go (`Coin.IsEqual`): `runTx` recovers it as
/// `ErrPanic`. No messages, bad addresses and non-positive coins fail
/// with their own errors.
#[test]
fn v046_validate_basic_fails_like_go() {
    use celestia_proto::cosmos::bank::v1beta1 as bank;
    use celestia_proto::cosmos::base::v1beta1::Coin;
    let addr = x_auth::bech32(&[1; 20]).unwrap();
    let coin = |d: &str, a: &str| Coin {
        denom: d.into(),
        amount: a.into(),
    };
    let multi = bank::MsgMultiSend {
        inputs: vec![bank::Input {
            address: addr.clone(),
            coins: vec![coin("utia", "1")],
        }],
        outputs: vec![bank::Output {
            address: addr.clone(),
            coins: vec![coin("uatom", "1")],
        }],
    };
    let e =
        sdk::error::recover(|| msgs::validate_basic_tx_msgs(&[celestia_proto::Any::pack(&multi)]))
            .unwrap_err();
    assert_eq!((e.codespace, e.code), ("undefined", 111_222));
    let e = msgs::validate_basic_tx_msgs(&[]).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 18));
    let send = |from: &str, amount: Vec<Coin>| {
        celestia_proto::Any::pack(&bank::MsgSend {
            from_address: from.into(),
            to_address: addr.clone(),
            amount,
        })
    };
    let e = msgs::validate_basic(&send("cosmos1xyz", vec![coin("utia", "1")])).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 7));
    let e = msgs::validate_basic(&send(&addr, vec![coin("utia", "0")])).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 10));
    msgs::validate_basic(&send(&addr, vec![coin("utia", "1")])).unwrap();
}

/// `runTx` before the ante handler reports the block context's meter: an
/// undecodable transaction uses the gas of every consensus-params read
/// (`getContextForTx`) so far and wants none. A block whose gas meter is
/// out of gas fails the next transaction without gas info.
#[test]
fn v046_run_tx_gas_before_the_ante_handler() {
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let mut ms = v1_store();
    let mut ctx = sdk::Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
    ctx.consensus_params = mainnet_consensus_params();
    consensus_params::store(&mut ctx, &mainnet_consensus_params()).unwrap();
    ctx.gas = kvstore::GasMeter::infinite();
    let mut block_gas = tx::block_gas_meter(&ctx);
    assert_eq!(block_gas.limit(), None);
    let first = tx::deliver_tx(&mut ctx, &mut block_gas, b"\xff");
    let per_tx = ctx.gas.consumed();
    assert!(per_tx > 0);
    assert_eq!((first.gas_wanted, first.gas_used), (0, per_tx));
    assert_eq!(first.result.unwrap_err().code, 2);
    let second = tx::deliver_tx(&mut ctx, &mut block_gas, b"\xff");
    assert_eq!(second.gas_used, 2 * per_tx);
    assert_eq!(block_gas.consumed(), 3 * per_tx);

    let mut cp = mainnet_consensus_params();
    cp.block.as_mut().unwrap().max_gas = 10;
    ctx.consensus_params = cp;
    let mut block_gas = tx::block_gas_meter(&ctx);
    assert_eq!(block_gas.limit(), Some(10));
    let over = tx::deliver_tx(&mut ctx, &mut block_gas, b"\xff");
    let e = over.result.unwrap_err();
    assert_eq!(e.code, 11, "{}", e.log);
    assert!(e.log.starts_with("out of gas in location: block gas meter"));
    let none = tx::deliver_tx(&mut ctx, &mut block_gas, b"\xff");
    assert_eq!((none.gas_wanted, none.gas_used), (0, 0));
    assert!(
        none.result
            .unwrap_err()
            .log
            .contains("no block gas left to run tx")
    );
}

/// cosmos-sdk v0.46 `SetAccount` writes the account and its number's entry
/// without reading the old value (so a stale number entry stays), and
/// `setBalance` writes the balance before its index entry; zero removes
/// both, the balance first. `subUnlockedCoins` names the spendable coin.
#[test]
fn v046_auth_and_bank_store_access() {
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    params_ctx(|ctx| {
        ctx.consensus_params.version = Some(VersionParams { app: 1 });
        let addr = [9u8; 20];
        let mut acc = x_auth::new_account_with_address(ctx, &addr).unwrap();
        acc.base_mut().account_number = 5;
        ctx.gas = kvstore::GasMeter::infinite();
        x_auth::set_account(ctx, &acc).unwrap();
        let one_set = ctx.gas.consumed();
        acc.base_mut().account_number = 77;
        x_auth::set_account(ctx, &acc).unwrap();
        // Two writes each time, no read.
        assert_eq!(ctx.gas.consumed(), 2 * one_set);
        let number = |n: u64| [b"accountNumber".as_slice(), &n.to_be_bytes()].concat();
        assert!(ctx.get(x_auth::STORE_KEY, &number(5)).unwrap().is_some());
        assert!(ctx.get(x_auth::STORE_KEY, &number(77)).unwrap().is_some());

        let descriptors = |ctx: &mut sdk::Context<'_>| -> Vec<&'static str> {
            ctx.gas.take_charges().into_iter().map(|(_, d)| d).collect()
        };
        ctx.gas.record_charges();
        x_bank::set_balance(ctx, &addr, &sdk::coins::Coin::new("utia", 5)).unwrap();
        assert_eq!(
            descriptors(ctx),
            [
                "WriteFlat",
                "WritePerByte",
                "WritePerByte",
                "Has",
                "WriteFlat",
                "WritePerByte",
                "WritePerByte"
            ]
        );
        let e = x_bank::sub_unlocked_coins(
            ctx,
            &addr,
            &sdk::coins::Coins(vec![sdk::coins::Coin::new("utia", 6)]),
        )
        .unwrap_err();
        assert_eq!(e.log, "5utia is smaller than 6utia: insufficient funds");
        ctx.gas.record_charges();
        x_bank::set_balance(ctx, &addr, &sdk::coins::Coin::new("utia", 0)).unwrap();
        assert_eq!(descriptors(ctx), ["Delete", "Delete"]);
        assert_eq!(
            x_bank::get_balance(ctx, &addr, "utia")
                .unwrap()
                .amount
                .to_string(),
            "0"
        );
    });
}

/// v0.46 `CreateValidator` below the minimum commission rate reads the
/// parameter twice (the check and the error message) and nothing else;
/// staking's single-parameter getters read one subspace key.
#[test]
#[allow(deprecated)]
fn v046_create_validator_below_min_commission() {
    use celestia_proto::cosmos::staking::v1beta1 as staking;
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let mut ms = v1_store();
    let mut ctx = sdk::Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
    ctx.consensus_params.version = Some(VersionParams { app: 1 });
    write_mainnet_genesis_params(&mut ctx).unwrap();
    let op = [4u8; 20];
    let msg = staking::MsgCreateValidator {
        description: Some(staking::Description {
            moniker: "m".into(),
            ..Default::default()
        }),
        commission: Some(staking::CommissionRates {
            rate: "10000000000000000".into(),
            max_rate: "200000000000000000".into(),
            max_change_rate: "10000000000000000".into(),
        }),
        min_self_delegation: "1".into(),
        delegator_address: x_auth::bech32(&op).unwrap(),
        validator_address: x_staking::addr::val_to_string(&op).unwrap(),
        pubkey: Some(celestia_proto::Any::pack(
            &celestia_proto::cosmos::crypto::ed25519::PubKey { key: vec![5; 32] },
        )),
        value: Some(celestia_proto::cosmos::base::v1beta1::Coin {
            denom: "utia".into(),
            amount: "1000000".into(),
        }),
    };
    let any = celestia_proto::Any::pack(&msg);
    msgs::validate_basic(&any).unwrap();
    ctx.gas = kvstore::GasMeter::new(1_000_000);
    ctx.gas.record_charges();
    let e = msgs::route(&mut ctx, &any).unwrap_err();
    assert_eq!((e.codespace, e.code), ("staking", 40));
    assert_eq!(
        e.log,
        "cannot set validator commission to less than minimum rate of 0.050000000000000000: commission cannot be less than min rate"
    );
    let flats = ctx
        .gas
        .take_charges()
        .iter()
        .filter(|(_, d)| *d == "ReadFlat")
        .count();
    assert_eq!(flats, 2);
    ctx.gas.record_charges();
    assert_eq!(x_staking::store::bond_denom(&mut ctx).unwrap(), "utia");
    assert_eq!(x_staking::store::max_entries(&mut ctx).unwrap(), 7);
    let reads = ctx.gas.take_charges();
    assert_eq!(reads.iter().filter(|(_, d)| *d == "ReadFlat").count(), 2);
}

/// The oracle's KV diff of block `height` in `store`
/// (`kv-diffs.jsonl.gz`): the keys set, with their values.
fn oracle_kv_set(height: i64, store: &str) -> BTreeMap<Vec<u8>, Vec<u8>> {
    let mut text = String::new();
    GzDecoder::new(
        &include_bytes!("../../../../../conformance/fixtures/mainnet/v1-oracle/kv-diffs.jsonl.gz")
            [..],
    )
    .read_to_string(&mut text)
    .unwrap();
    let d = text
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .find(|d| d["height"] == height && d["store"] == store)
        .unwrap();
    d["set"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                hex::decode(p[0].as_str().unwrap()).unwrap(),
                hex::decode(p[1].as_str().unwrap()).unwrap(),
            )
        })
        .collect()
}

/// ibc-go v6 `MsgCreateClient` of mainnet transaction 2:3 writes the
/// keys and values the Go oracle's block 2 wrote for `07-tendermint-0`
/// (client state, consensus state and its metadata), in v6's order of
/// store accesses, and emits v6's events.
#[test]
fn v6_create_client_matches_the_oracle() {
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let tx = crate::txpipe::decode_tx(&mainnet_txs(2)[3]).unwrap();
    let msg = tx.msgs()[0].clone();
    assert_eq!(msg.type_url, "/ibc.core.client.v1.MsgCreateClient");
    let mut ms = v1_store();
    let header = sdk::Header {
        chain_id: "celestia".into(),
        height: 2,
        // Block 2's time.
        time: pbjson_types::Timestamp {
            seconds: 1_698_760_802,
            nanos: 814_598_743,
        },
        ..sdk::Header::default()
    };
    let mut ctx = sdk::Context::new(&mut ms, header, sdk::ExecMode::Finalize);
    ctx.consensus_params.version = Some(VersionParams { app: 1 });
    params::set_ibc_client(
        &mut ctx,
        &serde_json::json!({"allowed_clients": ["06-solomachine", "07-tendermint"]}),
    )
    .unwrap();
    x_ibc::client::set_next_client_sequence(&mut ctx, 0).unwrap();
    msgs::validate_basic(&msg).unwrap();
    ctx.gas = kvstore::GasMeter::new(1_000_000);
    ctx.gas.record_charges();
    msgs::route(&mut ctx, &msg).unwrap();
    let descriptors: Vec<_> = ctx
        .gas
        .take_charges()
        .into_iter()
        .filter(|(_, d)| d.ends_with("Flat"))
        .map(|(_, d)| d)
        .collect();
    // AllowedClients, the client sequence, then five writes.
    assert_eq!(
        descriptors,
        [
            "ReadFlat",
            "ReadFlat",
            "WriteFlat",
            "WriteFlat",
            "WriteFlat",
            "WriteFlat",
            "WriteFlat",
            "WriteFlat"
        ]
    );
    let events: Vec<String> = ctx
        .events
        .events()
        .iter()
        .map(|e| {
            format!(
                "{}{:?}",
                e.r#type,
                e.attributes
                    .iter()
                    .map(|a| (&a.key, &a.value))
                    .collect::<Vec<_>>()
            )
        })
        .collect();
    assert_eq!(
        events,
        [
            r#"create_client[("client_id", "07-tendermint-0"), ("client_type", "07-tendermint"), ("consensus_height", "1-6122811")]"#,
            r#"message[("module", "ibc_client")]"#,
        ]
    );
    drop(ctx);
    let want = oracle_kv_set(2, x_ibc::STORE_KEY);
    for (k, v) in want
        .iter()
        .filter(|(k, _)| k.starts_with(b"clients/07-tendermint-0/"))
    {
        // The oracle's block also ran four more clients and its begin
        // blocker; client 0's processed time is block 2's time.
        assert_eq!(
            ms.get(x_ibc::STORE_KEY, k).unwrap().as_ref(),
            Some(v),
            "{}",
            String::from_utf8_lossy(k)
        );
    }
}

/// jsonpb unquotes a quoted integer and decodes the contents as JSON:
/// JSON whitespace is allowed, `null` is the zero value, and a leading
/// `+` or a leading zero is not a JSON number.
#[test]
fn quoted_integers_decode_as_json() {
    use serde_json::json;
    params_ctx(|ctx| {
        params::set_blob(
            ctx,
            &json!({"gas_per_blob_byte": " 8\n", "gov_max_square_size": "\t64 "}),
        )
        .unwrap();
        let get = |ctx: &mut sdk::Context<'_>, k: &[u8]| {
            String::from_utf8(x_params::get_raw(ctx, "blob", k).unwrap().unwrap()).unwrap()
        };
        assert_eq!(get(ctx, b"GasPerBlobByte"), "8");
        assert_eq!(get(ctx, b"GovMaxSquareSize"), "\"64\"");
    });
    let set = |gas: &str| {
        let mut out = None;
        params_ctx(|ctx| {
            let v = json!({"gas_per_blob_byte": gas, "gov_max_square_size": 64});
            out = Some(params::set_blob(ctx, &v).map_err(|e| e.to_string()));
        });
        out.unwrap()
    };
    let mut unquoted_null = None;
    params_ctx(|ctx| {
        let v = json!({"gas_per_blob_byte": null, "gov_max_square_size": 64});
        unquoted_null = Some(params::set_blob(ctx, &v).map_err(|e| e.to_string()));
    });
    assert_eq!(set("null"), unquoted_null.unwrap());
    for bad in [
        "+8", "08", "8 8", "", " ", "1e1", "8.0", "-8", "nul", "\u{a0}8",
    ] {
        assert!(set(bad).is_err(), "{bad:?}");
    }
    assert_eq!(set("0").is_err(), set("null").is_err());
}

/// Fails each transaction the way a not-ported path does: `flag` records
/// it (and may swallow the error), otherwise only the error says so.
struct Unported {
    flag: bool,
    swallow: bool,
}

impl Modules for Unported {
    fn init_genesis(&mut self, _: &mut Context<'_>, _: &[u8]) -> Result<Vec<ValidatorUpdate>> {
        Ok(Vec::new())
    }

    fn begin_block(&mut self, _: &mut Context<'_>, _: &TmHeader) -> Result<()> {
        Ok(())
    }

    fn deliver_tx(
        &mut self,
        _: &mut Context<'_>,
        _: &mut kvstore::GasMeter,
        tx: &[u8],
    ) -> TxOutcome {
        let result = if tx == b"ok" {
            Ok(TxSuccess::default())
        } else {
            let e = sdk::error::sdk::unknown_request("the v0.46 handler of X is not ported");
            let e = if self.flag { not_ported(e) } else { e };
            if self.swallow {
                Ok(TxSuccess::default())
            } else {
                Err(e)
            }
        };
        TxOutcome {
            gas_wanted: 1,
            gas_used: 1,
            events: Vec::new(),
            result,
        }
    }

    fn end_block(&mut self, _: &mut Context<'_>) -> Result<Vec<ValidatorUpdate>> {
        Ok(Vec::new())
    }
}

/// A signed-looking MsgSend transaction carrying `msgs` (unsigned: the
/// registry check runs before the ante handler).
fn raw_tx(msgs: Vec<celestia_proto::Any>) -> Vec<u8> {
    use celestia_proto::cosmos::tx::v1beta1::{AuthInfo, TxBody, TxRaw};
    use prost::Message as _;
    TxRaw {
        body_bytes: TxBody {
            messages: msgs,
            ..TxBody::default()
        }
        .encode_to_vec(),
        auth_info_bytes: AuthInfo::default().encode_to_vec(),
        signatures: vec![vec![0; 64]],
    }
    .encode_to_vec()
}

/// A transaction that reaches a not-ported behavior aborts the block
/// (FinalizeBlock fails, naming the height, the tx index and its message
/// types) instead of committing a failed result after the ante handler
/// took fee and sequence; also when the path swallowed the error.
#[test]
fn not_ported_transactions_abort_the_block() {
    let send = celestia_proto::Any {
        type_url: "/cosmos.staking.v1beta1.MsgUndelegate".to_owned(),
        value: Vec::new(),
    };
    for (flag, swallow) in [(true, false), (true, true)] {
        let mut ms = v1_store();
        let mut driver = Driver::new(Unported { flag, swallow });
        driver.init_chain(&mut ms, &init_request()).unwrap();
        let mut req = finalize_request(1);
        req.txs = vec![b"ok".to_vec(), raw_tx(vec![send.clone()])];
        let e = driver.finalize_block(&mut ms, &req).unwrap_err();
        assert!(e.log.contains("block 1 not executed: tx 1"), "{}", e.log);
        assert!(
            e.log.contains("[/cosmos.staking.v1beta1.MsgUndelegate]"),
            "{}",
            e.log
        );
        assert!(e.log.contains("handler of X is not ported"), "{}", e.log);
        assert!(driver.commit(&mut ms).is_err(), "nothing to commit");
    }
}

/// An error that only says "not ported" (no record) is an ordinary
/// failed transaction: error text can echo user input.
#[test]
fn unrecorded_not_ported_text_fails_only_the_tx() {
    let mut ms = v1_store();
    let mut driver = Driver::new(Unported {
        flag: false,
        swallow: false,
    });
    driver.init_chain(&mut ms, &init_request()).unwrap();
    let mut req = finalize_request(1);
    req.txs = vec![b"ok".to_vec(), b"x".to_vec()];
    let res = driver.finalize_block(&mut ms, &req).unwrap();
    assert_eq!(res.tx_results[0].code, 0);
    assert_ne!(res.tx_results[1].code, 0);
    assert!(res.tx_results[1].log.contains("not ported"));
    driver.commit(&mut ms).unwrap();
}

/// Only the ported v1 transaction path: InitGenesis, BeginBlock and
/// EndBlock do nothing.
struct TxOnly;

impl Modules for TxOnly {
    fn init_genesis(&mut self, _: &mut Context<'_>, _: &[u8]) -> Result<Vec<ValidatorUpdate>> {
        Ok(Vec::new())
    }

    fn begin_block(&mut self, _: &mut Context<'_>, _: &TmHeader) -> Result<()> {
        Ok(())
    }

    fn deliver_tx(
        &mut self,
        ctx: &mut Context<'_>,
        block_gas: &mut kvstore::GasMeter,
        tx: &[u8],
    ) -> TxOutcome {
        tx::deliver_tx(ctx, block_gas, tx)
    }

    fn end_block(&mut self, _: &mut Context<'_>) -> Result<Vec<ValidatorUpdate>> {
        Ok(Vec::new())
    }
}

/// A v1 `MsgSend` whose denom is the text "not ported" fails its
/// `ValidateBasic` like any invalid coin (Go just fails the tx); the
/// block is not aborted.
#[test]
fn user_input_saying_not_ported_does_not_abort_the_block() {
    use celestia_proto::cosmos::bank::v1beta1 as bank;
    use celestia_proto::cosmos::base::v1beta1::Coin;
    let addr = x_auth::bech32(&[1; 20]).unwrap();
    let send = celestia_proto::Any::pack(&bank::MsgSend {
        from_address: addr.clone(),
        to_address: addr,
        amount: vec![Coin {
            denom: "not ported".into(),
            amount: "1".into(),
        }],
    });
    let mut ms = v1_store();
    let mut driver = Driver::new(TxOnly);
    driver.init_chain(&mut ms, &init_request()).unwrap();
    let mut req = finalize_request(1);
    req.txs = vec![raw_tx(vec![send])];
    let res = driver
        .finalize_block(&mut ms, &req)
        .unwrap_or_else(|e| panic!("aborted: {}", e.log));
    let r = &res.tx_results[0];
    assert_eq!((r.codespace.as_str(), r.code), ("sdk", 10), "{}", r.log);
    assert!(r.log.contains("not ported"), "{}", r.log);
    driver.commit(&mut ms).unwrap();
}

/// The ante keepers' not-ported paths (shared crate `x-ante`) record
/// the not-ported behavior when the legacy driver supplies them.
#[test]
fn legacy_ante_deps_record_not_ported_paths() {
    use x_ante::AnteDeps as _;
    let update =
        celestia_proto::Any::pack(&celestia_proto::ibc::core::client::v1::MsgUpdateClient {
            client_id: "07-tendermint-0".into(),
            client_message: Some(celestia_proto::Any {
                type_url: "/x".into(),
                value: Vec::new(),
            }),
            signer: String::new(),
        });
    let mut ms = v1_store();
    let mut ctx = Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Check);
    let deps = tx::ante_deps();
    let _ = take_not_ported();
    assert!(
        deps.ibc_update_client_check(&mut ctx, &update, false)
            .is_err()
    );
    assert!(take_not_ported().is_some_and(|w| w.contains("IBC update client check")));
    assert!(
        deps.ibc_packet_is_noop(&mut ctx, x_ante::IbcPacketMsg::RecvPacket, &update, false)
            .is_err()
    );
    assert!(take_not_ported().is_some());
    // The main app's deps do not record.
    let main = crate::txpipe::ante_deps(None);
    assert!(
        main.ibc_update_client_check(&mut ctx, &update, false)
            .is_err()
    );
    assert!(take_not_ported().is_none());
}

/// Records a not-ported behavior and swallows it in one phase.
struct SwallowIn(&'static str);

impl SwallowIn {
    fn phase(&self, name: &str) {
        if self.0 == name {
            let _ = not_ported(sdk::error::sdk::not_supported(format!("{name} behavior")));
        }
    }
}

impl Modules for SwallowIn {
    fn init_genesis(&mut self, _: &mut Context<'_>, _: &[u8]) -> Result<Vec<ValidatorUpdate>> {
        self.phase("init_genesis");
        Ok(Vec::new())
    }

    fn begin_block(&mut self, _: &mut Context<'_>, _: &TmHeader) -> Result<()> {
        self.phase("begin_block");
        Ok(())
    }

    fn deliver_tx(
        &mut self,
        _: &mut Context<'_>,
        _: &mut kvstore::GasMeter,
        _: &[u8],
    ) -> TxOutcome {
        TxOutcome {
            gas_wanted: 1,
            gas_used: 1,
            events: Vec::new(),
            result: Ok(TxSuccess::default()),
        }
    }

    fn end_block(&mut self, _: &mut Context<'_>) -> Result<Vec<ValidatorUpdate>> {
        self.phase("end_block");
        Ok(Vec::new())
    }
}

/// A not-ported behavior that InitGenesis, BeginBlock or EndBlock
/// swallowed still fails the call; a stale record from before the block
/// does not.
#[test]
fn not_ported_in_genesis_and_blockers_is_not_lost() {
    let mut ms = v1_store();
    let e = Driver::new(SwallowIn("init_genesis"))
        .init_chain(&mut ms, &init_request())
        .unwrap_err();
    assert!(e.log.contains("init_genesis behavior"), "{}", e.log);
    for phase in ["begin_block", "end_block"] {
        let mut ms = v1_store();
        let mut driver = Driver::new(SwallowIn(phase));
        driver.init_chain(&mut ms, &init_request()).unwrap();
        let e = driver
            .finalize_block(&mut ms, &finalize_request(1))
            .unwrap_err();
        assert!(e.log.contains(&format!("{phase} behavior")), "{}", e.log);
        assert!(e.log.contains("block 1 not executed"), "{}", e.log);
    }
    let mut ms = v1_store();
    let mut driver = Driver::new(SwallowIn(""));
    driver.init_chain(&mut ms, &init_request()).unwrap();
    let _ = not_ported(sdk::error::sdk::not_supported("stale"));
    driver
        .finalize_block(&mut ms, &finalize_request(1))
        .unwrap();
    driver.commit(&mut ms).unwrap();
}

/// The v1 runTx decodes with the v1 registry: a message outside it (a
/// later release's bank `MsgUpdateParams`) fails the transaction as Go
/// does (a decode error, no gas wanted, no record); blobstream's
/// `MsgRegisterEVMAddress` decodes and fails its `ValidateBasic`, and so
/// does crisis `MsgVerifyInvariant` without a sender.
#[test]
fn v1_run_tx_decodes_with_the_v1_registry() {
    let run = |msg: celestia_proto::Any| {
        let mut ms = v1_store();
        let _v046 = sdk::version::SdkV046Scope::enter(1);
        let mut ctx = Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
        ctx.consensus_params = mainnet_consensus_params();
        let mut block_gas = kvstore::GasMeter::infinite();
        let _ = take_not_ported();
        let out = tx::deliver_tx(&mut ctx, &mut block_gas, &raw_tx(vec![msg]));
        (out, take_not_ported())
    };
    let later = celestia_proto::Any::pack(
        &celestia_proto::cosmos::bank::v1beta1::MsgUpdateParams::default(),
    );
    let (out, record) = run(later);
    let e = out.result.unwrap_err();
    assert_eq!(
        (e.codespace, e.code, out.gas_wanted),
        ("sdk", 2, 0),
        "{}",
        e.log
    );
    assert!(record.is_none());

    let evm = celestia_proto::Any {
        type_url: "/celestia.qgb.v1.MsgRegisterEVMAddress".to_owned(),
        value: Vec::new(),
    };
    let (out, record) = run(evm);
    assert!(out.result.is_err());
    assert!(record.is_none());

    let crisis = celestia_proto::Any::pack(
        &celestia_proto::cosmos::crisis::v1beta1::MsgVerifyInvariant::default(),
    );
    let (out, record) = run(crisis);
    let e = out.result.expect_err("an empty sender");
    assert_eq!(e.codespace, "sdk", "{}", e.log);
    assert_eq!(e.code, 7, "{}", e.log);
    assert!(e.log.starts_with("invalid sender address: "), "{}", e.log);
    assert!(record.is_none());
}

/// Runs a v1 transaction through `runTx`: its outcome and the not-ported
/// record (which aborts the block in the driver).
fn run_v1_tx(raw: &[u8]) -> (super::TxOutcome, Option<String>) {
    let mut ms = v1_store();
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let mut ctx = Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
    ctx.consensus_params = mainnet_consensus_params();
    let mut block_gas = kvstore::GasMeter::infinite();
    let _ = take_not_ported();
    let out = tx::deliver_tx(&mut ctx, &mut block_gas, raw);
    (out, take_not_ported())
}

/// `levels` `MsgExec`s nested through their `msgs` (the outermost with a
/// valid grantee, so its `ValidateBasic` passes), around a `MsgSend`.
fn nested_exec(levels: usize) -> celestia_proto::Any {
    use celestia_proto::cosmos::authz::v1beta1::MsgExec;
    let grantee = x_auth::bech32(&[1; 20]).unwrap();
    let mut msg = celestia_proto::Any::pack(&celestia_proto::cosmos::bank::v1beta1::MsgSend {
        from_address: grantee.clone(),
        to_address: grantee.clone(),
        amount: vec![],
    });
    for _ in 0..levels {
        msg = celestia_proto::Any::pack(&MsgExec {
            grantee: grantee.clone(),
            msgs: vec![msg],
        });
    }
    msg
}

/// `MsgVersioningGateKeeper.hasInvalidMsg` recurses into `MsgExec`
/// without a limit: past the legacy layer's nesting cap Go would go on
/// to the ante handler, so the block aborts instead of failing the tx.
#[test]
fn v1_gatekeeper_nesting_cap_aborts_the_block() {
    let (out, record) = run_v1_tx(&raw_tx(vec![nested_exec(70)]));
    assert!(out.result.is_err());
    let what = record.expect("not recorded");
    assert!(what.contains("max depth exceeded"), "{what}");
    // Within the cap the gatekeeper accepts and the ante handler runs.
    let (out, record) = run_v1_tx(&raw_tx(vec![nested_exec(60)]));
    assert!(out.result.is_err());
    assert!(record.is_none(), "{record:?}");
}

/// The decoder's recursion limit, which Go v0.46 does not have, aborts
/// the block under the v1 decoder instead of failing the tx.
#[test]
fn v1_decoder_recursion_limit_aborts_the_block() {
    let raw = raw_tx(vec![nested_exec(10_050)]);
    let (out, record) = run_v1_tx(&raw);
    assert!(out.result.is_err());
    let what = record.expect("not recorded");
    assert!(what.contains("recursion limit reached"), "{what}");
}

/// v0.46 `Commission.String()` as mainnet's first `MsgEditValidator`
/// (height 13) emitted it, and with a fractional update time.
#[test]
fn v046_commission_string_is_yaml() {
    let dec = |s: &str| math::LegacyDec::from_str_go(s).unwrap();
    let mut c = x_staking::types::Commission {
        rate: dec("0.05"),
        max_rate: dec("0.12"),
        max_change_rate: dec("0.02"),
        update_time: pbjson_types::Timestamp {
            seconds: 1_698_760_800,
            nanos: 0,
        },
    };
    assert_eq!(
        msgs::commission_yaml(&c),
        "commission_rates:\n  max_change_rate: \"0.020000000000000000\"\n  max_rate: \"0.120000000000000000\"\n  rate: \"0.050000000000000000\"\nupdate_time: \"2023-10-31T14:00:00Z\"\n"
    );
    c.update_time.nanos = 500_000_000;
    assert!(msgs::commission_yaml(&c).ends_with("update_time: \"2023-10-31T14:00:00.5Z\"\n"));
}

/// v0.46 `MsgEditValidator.ValidateBasic`: an empty description, a
/// non-positive minimum self delegation and a rate outside [0, 1] fail;
/// a present but empty decimal field panics (recovered as `ErrPanic`).
#[test]
fn v046_edit_validator_validate_basic() {
    use celestia_proto::cosmos::staking::v1beta1 as staking;
    let val = x_staking::addr::val_to_string(&[1; 20]).unwrap();
    let edit = |f: &dyn Fn(&mut staking::MsgEditValidator)| {
        let mut m = staking::MsgEditValidator {
            description: Some(staking::Description {
                moniker: "m".into(),
                ..staking::Description::default()
            }),
            validator_address: val.clone(),
            ..staking::MsgEditValidator::default()
        };
        f(&mut m);
        sdk::error::recover(|| msgs::validate_basic(&celestia_proto::Any::pack(&m)))
    };
    edit(&|_| {}).unwrap();
    let code = |f: &dyn Fn(&mut staking::MsgEditValidator)| {
        let e = edit(f).unwrap_err();
        (e.codespace, e.code)
    };
    assert_eq!(code(&|m| m.description = None), ("sdk", 18));
    assert_eq!(code(&|m| m.validator_address = "x".into()), ("sdk", 7));
    assert_eq!(code(&|m| m.min_self_delegation = "0".into()), ("sdk", 18));
    assert_eq!(
        code(&|m| m.commission_rate = "1000000000000000001".into()),
        ("sdk", 18)
    );
    // Empty strings are not encoded: build a present-but-empty field.
    let mut raw =
        <staking::MsgEditValidator as prost::Message>::encode_to_vec(&staking::MsgEditValidator {
            description: Some(staking::Description {
                moniker: "m".into(),
                ..staking::Description::default()
            }),
            validator_address: val.clone(),
            ..staking::MsgEditValidator::default()
        });
    raw.extend([0x1a, 0x00]);
    let any = celestia_proto::Any {
        type_url: "/cosmos.staking.v1beta1.MsgEditValidator".into(),
        value: raw,
    };
    let e = sdk::error::recover(|| msgs::validate_basic(&any)).unwrap_err();
    assert_eq!((e.codespace, e.code), ("undefined", 111_222));
}

/// ibc-go v6 `ValidateBasic` of the connection handshake: a zero proof
/// height fails `MsgConnectionOpenConfirm` (v8 no longer checks it),
/// with the SDK's `ErrInvalidHeight`; a bad signer is the SDK's
/// `ErrInvalidAddress`; a 09-localhost `MsgConnectionOpenInit` passes
/// (v8 rejects it here).
#[test]
fn v6_connection_validate_basic() {
    use celestia_proto::ibc::core::client::v1::Height;
    use celestia_proto::ibc::core::commitment::v1::MerklePrefix;
    use celestia_proto::ibc::core::connection::v1 as copb;
    let signer = x_auth::bech32(&[1; 20]).unwrap();
    let confirm = |h: u64, signer: &str| {
        ibc::validate_basic(&celestia_proto::Any::pack(
            &copb::MsgConnectionOpenConfirm {
                connection_id: "connection-0".into(),
                proof_ack: vec![1],
                proof_height: Some(Height {
                    revision_number: 0,
                    revision_height: h,
                }),
                signer: signer.into(),
            },
        ))
        .unwrap()
    };
    confirm(1, &signer).unwrap();
    let e = confirm(0, &signer).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 26), "{}", e.log);
    let e = confirm(1, "x").unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 7), "{}", e.log);
    let init = copb::MsgConnectionOpenInit {
        client_id: "09-localhost".into(),
        counterparty: Some(copb::Counterparty {
            client_id: "07-tendermint-0".into(),
            connection_id: String::new(),
            prefix: Some(MerklePrefix {
                key_prefix: b"ibc".to_vec(),
            }),
        }),
        version: None,
        delay_period: 0,
        signer,
    };
    ibc::validate_basic(&celestia_proto::Any::pack(&init))
        .unwrap()
        .unwrap();
}

/// ibc-go v6 `ValidateBasic` of the channel handshake: a zero proof
/// height fails `MsgChannelOpenAck` with the SDK's `ErrInvalidHeight`
/// (ibc-go v8 no longer checks it).
#[test]
fn v6_channel_validate_basic() {
    use celestia_proto::ibc::core::channel::v1 as chpb;
    use celestia_proto::ibc::core::client::v1::Height;
    let ack = |h: u64| {
        msgs::validate_basic(&celestia_proto::Any::pack(&chpb::MsgChannelOpenAck {
            port_id: "transfer".into(),
            channel_id: "channel-0".into(),
            counterparty_channel_id: "channel-1".into(),
            counterparty_version: "ics20-1".into(),
            proof_try: vec![1],
            proof_height: Some(Height {
                revision_number: 0,
                revision_height: h,
            }),
            signer: x_auth::bech32(&[1; 20]).unwrap(),
        }))
    };
    ack(1).unwrap();
    let e = ack(0).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 26), "{}", e.log);
}

/// The `update_client` event's header is the decoded header marshaled
/// again as gogoproto does: an absent vote's missing timestamp becomes
/// Go's zero time and missing non-nullable messages are written empty.
#[test]
fn v6_update_client_event_header_is_gogo_encoded() {
    use celestia_proto::celestia_core_proto::tendermint::types::{
        BlockIdFlag, Commit, CommitSig, SignedHeader,
    };
    use celestia_proto::ibc::lightclients::tendermint::v1 as tmpb;
    let h = tmpb::Header {
        signed_header: Some(SignedHeader {
            header: None,
            commit: Some(Commit {
                height: 5,
                signatures: vec![CommitSig {
                    block_id_flag: BlockIdFlag::Absent as i32,
                    ..CommitSig::default()
                }],
                ..Commit::default()
            }),
        }),
        ..tmpb::Header::default()
    };
    let g = ibc::gogo_header(h);
    let c = g.signed_header.unwrap().commit.unwrap();
    assert_eq!(c.signatures[0].timestamp, Some(sdk::time::go_zero()));
    assert!(c.block_id.unwrap().part_set_header.is_some());
    assert!(g.trusted_height.is_some());
}

/// ibc-go v6 `MsgTransfer.ValidateBasic` fails with SDK error codes
/// (ibc-go v8 uses its own codespace) and has no receiver length limit.
#[test]
fn v6_transfer_validate_basic() {
    use celestia_proto::cosmos::base::v1beta1::Coin;
    use celestia_proto::ibc::applications::transfer::v1 as trpb;
    use celestia_proto::ibc::core::client::v1::Height;
    let transfer = |amount: &str, receiver: String| {
        msgs::validate_basic(&celestia_proto::Any::pack(&trpb::MsgTransfer {
            source_port: "transfer".into(),
            source_channel: "channel-0".into(),
            token: Some(Coin {
                denom: "utia".into(),
                amount: amount.into(),
            }),
            sender: x_auth::bech32(&[1; 20]).unwrap(),
            receiver,
            timeout_height: Some(Height {
                revision_number: 1,
                revision_height: 10,
            }),
            timeout_timestamp: 0,
            memo: String::new(),
        }))
    };
    transfer("1", "x".repeat(4096)).unwrap();
    let e = transfer("0", "r".into()).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 5), "{}", e.log);
    let e = transfer("1", " ".into()).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 7), "{}", e.log);
}

/// ibc-go v6 `MsgAcknowledgement.ValidateBasic`: a zero proof height is
/// the SDK's `ErrInvalidHeight`, empty ack bytes are
/// `ErrInvalidAcknowledgement`.
#[test]
fn v6_acknowledgement_validate_basic() {
    use celestia_proto::ibc::core::channel::v1 as chpb;
    use celestia_proto::ibc::core::client::v1::Height;
    let ack = |h: u64, ack: &[u8]| {
        msgs::validate_basic(&celestia_proto::Any::pack(&chpb::MsgAcknowledgement {
            packet: Some(chpb::Packet {
                sequence: 1,
                source_port: "transfer".into(),
                source_channel: "channel-0".into(),
                destination_port: "transfer".into(),
                destination_channel: "channel-1".into(),
                data: vec![1],
                timeout_height: None,
                timeout_timestamp: 1,
            }),
            acknowledgement: ack.to_vec(),
            proof_acked: vec![1],
            proof_height: Some(Height {
                revision_number: 0,
                revision_height: h,
            }),
            signer: x_auth::bech32(&[1; 20]).unwrap(),
        }))
    };
    ack(1, b"{}").unwrap();
    let e = ack(0, b"{}").unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 26), "{}", e.log);
    let e = ack(1, b"").unwrap_err();
    assert_eq!((e.codespace, e.code), ("channel", 16), "{}", e.log);
}

/// ibc-go v6 `MsgRecvPacket.ValidateBasic`: a zero proof height is the
/// SDK's `ErrInvalidHeight`; an empty proof is `ErrInvalidProof`.
#[test]
fn v6_recv_packet_validate_basic() {
    use celestia_proto::ibc::core::channel::v1 as chpb;
    use celestia_proto::ibc::core::client::v1::Height;
    let recv = |h: u64, proof: Vec<u8>| {
        msgs::validate_basic(&celestia_proto::Any::pack(&chpb::MsgRecvPacket {
            packet: Some(chpb::Packet {
                sequence: 1,
                source_port: "transfer".into(),
                source_channel: "channel-1".into(),
                destination_port: "transfer".into(),
                destination_channel: "channel-0".into(),
                data: vec![1],
                timeout_height: None,
                timeout_timestamp: 1,
            }),
            proof_commitment: proof,
            proof_height: Some(Height {
                revision_number: 0,
                revision_height: h,
            }),
            signer: x_auth::bech32(&[1; 20]).unwrap(),
        }))
    };
    recv(1, vec![1]).unwrap();
    let e = recv(0, vec![1]).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 26), "{}", e.log);
    let e = recv(1, Vec::new()).unwrap_err();
    assert_eq!((e.codespace, e.code), ("commitment", 2), "{}", e.log);
}

/// v0.46 authz `ValidateBasic`: addresses, granter == grantee, and the
/// grant's authorization (a nil one panics, an empty type URL is an
/// invalid type, a stake authorization needs a type); a not-ported
/// authorization type aborts the block.
#[test]
fn v046_authz_validate_basic() {
    use celestia_proto::Any;
    use celestia_proto::cosmos::authz::v1beta1 as authz;
    use celestia_proto::cosmos::staking::v1beta1 as staking;
    let a = x_auth::bech32(&[1; 20]).unwrap();
    let b = x_auth::bech32(&[2; 20]).unwrap();
    let grant = |granter: &str, grantee: &str, auth: Option<Any>| {
        msgs::validate_basic(&Any::pack(&authz::MsgGrant {
            granter: granter.into(),
            grantee: grantee.into(),
            grant: Some(authz::Grant {
                authorization: auth,
                expiration: None,
            }),
        }))
    };
    let generic = Any::pack(&authz::GenericAuthorization { msg: "x".into() });
    grant(&a, &b, Some(generic.clone())).unwrap();
    let e = grant(&a, &a, Some(generic.clone())).unwrap_err();
    assert_eq!((e.codespace, e.code), ("authz", 7));
    let e = grant("x", &b, Some(generic)).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 7));
    assert!(e.log.starts_with("invalid granter address"), "{}", e.log);
    let e = sdk::error::recover(|| grant(&a, &b, None)).unwrap_err();
    assert_eq!((e.codespace, e.code), ("undefined", 111_222));
    let empty = Any {
        type_url: String::new(),
        value: Vec::new(),
    };
    let e = grant(&a, &b, Some(empty)).unwrap_err();
    assert_eq!(e.log, "expected <nil>, got <nil>: invalid type");
    let stake = |t: i32| {
        Some(Any::pack(&staking::StakeAuthorization {
            max_tokens: None,
            authorization_type: t,
            policy: None,
        }))
    };
    grant(&a, &b, stake(1)).unwrap();
    let e = grant(&a, &b, stake(0)).unwrap_err();
    assert_eq!((e.codespace, e.code), ("authz", 4));
    let _ = take_not_ported();
    // A v0.46 `SendAuthorization` without a spend limit is invalid.
    let send = Any::pack(&celestia_proto::cosmos::bank::v1beta1::SendAuthorization::default());
    let e = grant(&a, &b, Some(send)).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 10), "{}", e.log);
    assert!(take_not_ported().is_none());

    let exec = |grantee: &str, n: usize| {
        msgs::validate_basic(&Any::pack(&authz::MsgExec {
            grantee: grantee.into(),
            msgs: vec![Any::pack(&authz::GenericAuthorization::default()); n],
        }))
    };
    exec(&a, 1).unwrap();
    assert_eq!(exec(&a, 0).unwrap_err().code, 18);
    assert_eq!(exec("x", 1).unwrap_err().code, 7);
    let revoke = |t: &str| {
        msgs::validate_basic(&Any::pack(&authz::MsgRevoke {
            granter: a.clone(),
            grantee: b.clone(),
            msg_type_url: t.into(),
        }))
    };
    revoke("x").unwrap();
    assert_eq!(
        revoke("").unwrap_err().log,
        "missing method name: invalid request"
    );
}

/// ibc-go v6 `MsgTimeout.ValidateBasic`: proof, proof height, next
/// sequence receive, signer, then the packet.
#[test]
fn v6_timeout_validate_basic() {
    use celestia_proto::ibc::core::channel::v1 as chpb;
    use celestia_proto::ibc::core::client::v1::Height;
    let timeout = |h: u64, proof: Vec<u8>, next: u64, seq: u64| {
        msgs::validate_basic(&celestia_proto::Any::pack(&chpb::MsgTimeout {
            packet: Some(chpb::Packet {
                sequence: seq,
                source_port: "transfer".into(),
                source_channel: "channel-1".into(),
                destination_port: "transfer".into(),
                destination_channel: "channel-0".into(),
                data: vec![1],
                timeout_height: None,
                timeout_timestamp: 1,
            }),
            proof_unreceived: proof,
            proof_height: Some(Height {
                revision_number: 0,
                revision_height: h,
            }),
            next_sequence_recv: next,
            signer: x_auth::bech32(&[1; 20]).unwrap(),
        }))
    };
    timeout(1, vec![1], 1, 1).unwrap();
    let e = timeout(1, Vec::new(), 1, 1).unwrap_err();
    assert_eq!((e.codespace, e.code), ("commitment", 2), "{}", e.log);
    let e = timeout(0, vec![1], 1, 1).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 26), "{}", e.log);
    let e = timeout(1, vec![1], 0, 1).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 3), "{}", e.log);
    let e = timeout(1, vec![1], 1, 0).unwrap_err();
    assert_eq!((e.codespace, e.code), ("channel", 13), "{}", e.log);
}

/// cosmos-sdk v0.46 gov votes (v1beta1 and v1): `ValidateBasic` checks
/// the voter and the option; a vote on a proposal that does not exist
/// fails with `ErrUnknownProposal`; a vote on an existing proposal is
/// not ported.
#[test]
fn v046_gov_vote_on_unknown_proposal() {
    use celestia_proto::cosmos::gov::{v1, v1beta1};
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let mut ms = v1_store();
    let mut ctx = Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
    let voter = x_auth::bech32(&[3; 20]).unwrap();
    let old = |option: i32| {
        celestia_proto::Any::pack(&v1beta1::MsgVote {
            proposal_id: 1,
            voter: voter.clone(),
            option,
        })
    };
    let new = |option: i32| {
        celestia_proto::Any::pack(&v1::MsgVote {
            proposal_id: 1,
            voter: voter.clone(),
            option,
            metadata: String::new(),
        })
    };
    let builders: [&dyn Fn(i32) -> celestia_proto::Any; 2] = [&old, &new];
    for vote in builders {
        for bad in [0, 5] {
            let e = msgs::validate_basic(&vote(bad)).unwrap_err();
            assert_eq!((e.codespace, e.code), ("gov", 7), "{}", e.log);
        }
        msgs::validate_basic(&vote(1)).unwrap();
        let _ = take_not_ported();
        let e = msgs::route(&mut ctx, &vote(1)).unwrap_err();
        assert_eq!((e.codespace, e.code), ("gov", 2), "{}", e.log);
        assert!(take_not_ported().is_none());
    }
}

/// A v0.46 community pool spend proposal end to end: submitted with half
/// the minimum deposit (deposit period, inactive queue), deposited to the
/// minimum (voting period, active queue), voted yes by one validator of
/// two (quorum and threshold met), then tallied by the EndBlocker after
/// the voting period: it passes, the spend runs, the deposits are
/// refunded and the votes deleted. A second proposal that never reaches
/// the minimum is dropped (deleted, deposit refunded) after its deposit
/// period.
#[test]
#[allow(clippy::too_many_lines)]
fn v046_gov_community_pool_spend_proposal_lifecycle() {
    use celestia_proto::cosmos::gov::{v1, v1beta1};
    use prost::Message as _;
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let mut ms = v1_store();
    let (mut ctx, _) = v046_two_validators(&mut ms, 2);
    ctx.header.time = pbjson_types::Timestamp {
        seconds: 1_700_000_000,
        nanos: 0,
    };
    x_gov::set_proposal_id(&mut ctx, 1).unwrap();
    let utia = |n: u64| sdk::coins::Coins(vec![sdk::coins::Coin::new("utia", n)]);
    let pb = |n: u64| utia(n).to_proto();
    let addr_of = |name: &str, ctx: &mut sdk::Context<'_>| {
        let m = x_auth::get_module_account(ctx, name).unwrap().unwrap();
        x_auth::Account::Module(m).address().unwrap()
    };
    let bonded = addr_of("bonded_tokens_pool", &mut ctx);
    x_bank::set_balance(
        &mut ctx,
        &bonded,
        &sdk::coins::Coin::new("utia", 200_000_000),
    )
    .unwrap();
    let distr = addr_of("distribution", &mut ctx);
    x_bank::set_balance(&mut ctx, &distr, &sdk::coins::Coin::new("utia", 1_000)).unwrap();
    x_distribution::store::set_fee_pool(
        &mut ctx,
        &sdk::dec_coins::DecCoins::from_coins(&utia(1_000)).unwrap(),
    )
    .unwrap();
    let proposer = [9u8; 20];
    x_bank::set_balance(
        &mut ctx,
        &proposer,
        &sdk::coins::Coin::new("utia", 30_000_000_000),
    )
    .unwrap();
    let recipient = [8u8; 20];
    let spend = celestia_proto::Any::pack(
        &celestia_proto::cosmos::distribution::v1beta1::CommunityPoolSpendProposal {
            title: "t".into(),
            description: "d".into(),
            recipient: x_auth::bech32(&recipient).unwrap(),
            amount: pb(400),
        },
    );
    let submit = |deposit: u64| {
        celestia_proto::Any::pack(&v1beta1::MsgSubmitProposal {
            content: Some(spend.clone()),
            initial_deposit: pb(deposit),
            proposer: x_auth::bech32(&proposer).unwrap(),
        })
    };
    // Submit with half the minimum deposit (10,000,000,000 utia).
    msgs::validate_basic(&submit(5_000_000_000)).unwrap();
    let gas0 = ctx.gas.consumed();
    let r = msgs::route(&mut ctx, &submit(5_000_000_000)).unwrap();
    assert!(ctx.gas.consumed() > gas0);
    assert_eq!(
        v1beta1::MsgSubmitProposalResponse::decode(r.value.as_slice())
            .unwrap()
            .proposal_id,
        1
    );
    assert!(take_not_ported().is_none());
    let get = |ctx: &mut sdk::Context<'_>, id: u64| -> Option<v1::Proposal> {
        let mut k = vec![0x00];
        k.extend_from_slice(&id.to_be_bytes());
        ctx.get(x_gov::STORE_KEY, &k)
            .unwrap()
            .map(|b| v1::Proposal::decode(b.as_slice()).unwrap())
    };
    let p = get(&mut ctx, 1).unwrap();
    assert_eq!(p.status, v1::ProposalStatus::DepositPeriod as i32);
    assert_eq!(p.total_deposit, pb(5_000_000_000));
    assert_eq!(
        p.messages[0].type_url,
        "/cosmos.gov.v1.MsgExecLegacyContent"
    );
    let dep_end = p.deposit_end_time.unwrap();
    assert_eq!(dep_end.seconds, 1_700_000_000 + 604_800);
    // The submit dry run left the community pool alone.
    assert_eq!(
        x_bank::get_balance(&mut ctx, &recipient, "utia")
            .unwrap()
            .amount,
        math::Int::zero()
    );
    // Deposit the rest: the voting period starts.
    let dep = celestia_proto::Any::pack(&v1::MsgDeposit {
        proposal_id: 1,
        depositor: x_auth::bech32(&proposer).unwrap(),
        amount: pb(5_000_000_000),
    });
    msgs::validate_basic(&dep).unwrap();
    msgs::route(&mut ctx, &dep).unwrap();
    let p = get(&mut ctx, 1).unwrap();
    assert_eq!(p.status, v1::ProposalStatus::VotingPeriod as i32);
    assert_eq!(p.total_deposit, pb(10_000_000_000));
    assert!(
        x_gov::expired_queue_entries(&mut ctx, x_gov::INACTIVE_QUEUE_PREFIX, &dep_end)
            .unwrap()
            .is_empty()
    );
    // A second proposal that stays below the minimum.
    msgs::route(&mut ctx, &submit(1)).unwrap();
    // Validator 1 (half of the bonded tokens) votes yes.
    let yes = celestia_proto::Any::pack(&v1beta1::MsgVote {
        proposal_id: 1,
        voter: x_auth::bech32(&[1u8; 20]).unwrap(),
        option: 1,
    });
    msgs::route(&mut ctx, &yes).unwrap();
    // A vote on the proposal in its deposit period is inactive.
    let early = celestia_proto::Any::pack(&v1beta1::MsgVote {
        proposal_id: 2,
        voter: x_auth::bech32(&[1u8; 20]).unwrap(),
        option: 1,
    });
    let e = msgs::route(&mut ctx, &early).unwrap_err();
    assert_eq!((e.codespace, e.code), ("gov", 3), "{}", e.log);
    // After both periods: proposal 2 is dropped, proposal 1 passes.
    ctx.header.time = pbjson_types::Timestamp {
        seconds: 1_700_000_000 + 604_800 + 1,
        nanos: 0,
    };
    ctx.events.take();
    gov::end_block(&mut ctx).unwrap();
    assert!(take_not_ported().is_none());
    assert!(get(&mut ctx, 2).is_none());
    let p = get(&mut ctx, 1).unwrap();
    assert_eq!(p.status, v1::ProposalStatus::Passed as i32);
    let t = p.final_tally_result.unwrap();
    assert_eq!(t.yes_count, "100000000");
    assert_eq!(t.no_count, "0");
    assert_eq!(
        x_bank::get_balance(&mut ctx, &recipient, "utia")
            .unwrap()
            .amount,
        math::Int::from_u64(400)
    );
    // Every deposit refunded.
    assert_eq!(
        x_bank::get_balance(&mut ctx, &proposer, "utia")
            .unwrap()
            .amount,
        math::Int::from_u64(30_000_000_000)
    );
    let types: Vec<&str> = ctx
        .events
        .events()
        .iter()
        .map(|e| e.r#type.as_str())
        .collect();
    assert!(types.contains(&"inactive_proposal"), "{types:?}");
    assert_eq!(types.last(), Some(&"active_proposal"), "{types:?}");
    let mut prefix = vec![0x20];
    prefix.extend_from_slice(&1u64.to_be_bytes());
    let mut it = ctx.prefix_iter(x_gov::STORE_KEY, &prefix).unwrap();
    assert!(it.next(&mut ctx).unwrap().is_none());
    let now = ctx.header.time;
    assert!(
        x_gov::expired_queue_entries(&mut ctx, x_gov::ACTIVE_QUEUE_PREFIX, &now)
            .unwrap()
            .is_empty()
    );
}

/// A text proposal has no route in celestia-app v1: its submission fails
/// (gov code 9) and nothing is stored. A parameter change is validated and
/// submitted; its dry run changes nothing.
#[test]
fn v046_gov_text_proposal_has_no_handler() {
    use celestia_proto::cosmos::gov::v1beta1;
    use celestia_proto::cosmos::params::v1beta1::{ParamChange, ParameterChangeProposal};
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let mut ms = v1_store();
    let (mut ctx, _) = v046_two_validators(&mut ms, 2);
    x_gov::set_proposal_id(&mut ctx, 1).unwrap();
    let proposer = x_auth::bech32(&[9u8; 20]).unwrap();
    let text = celestia_proto::Any::pack(&v1beta1::MsgSubmitProposal {
        content: Some(celestia_proto::Any::pack(&v1beta1::TextProposal {
            title: "t".into(),
            description: "d".into(),
        })),
        initial_deposit: vec![],
        proposer: proposer.clone(),
    });
    msgs::validate_basic(&text).unwrap();
    let e = msgs::route(&mut ctx, &text).unwrap_err();
    assert_eq!((e.codespace, e.code), ("gov", 9), "{}", e.log);
    assert!(take_not_ported().is_none());
    // A parameter change: a blank title fails `ValidateBasic`; a valid
    // one is submitted (its dry run sets nothing).
    let change = |title: &str| {
        celestia_proto::Any::pack(&v1beta1::MsgSubmitProposal {
            content: Some(celestia_proto::Any::pack(&ParameterChangeProposal {
                title: title.into(),
                description: "d".into(),
                changes: vec![ParamChange {
                    subspace: "slashing".into(),
                    key: "SignedBlocksWindow".into(),
                    value: "\"10000\"".into(),
                }],
            })),
            initial_deposit: vec![],
            proposer: proposer.clone(),
        })
    };
    let e = msgs::validate_basic(&change("")).unwrap_err();
    assert_eq!((e.codespace, e.code), ("gov", 5), "{}", e.log);
    msgs::validate_basic(&change("t")).unwrap();
    let before = x_params::get_raw(&mut ctx, "slashing", b"SignedBlocksWindow").unwrap();
    msgs::route(&mut ctx, &change("t")).unwrap();
    assert!(take_not_ported().is_none());
    assert_eq!(
        x_params::get_raw(&mut ctx, "slashing", b"SignedBlocksWindow").unwrap(),
        before
    );
}

/// `Grant.expiration` (nullable stdtime) sent twice: gogoproto's
/// `StdTimeUnmarshal` keeps the last timestamp, prost would merge the
/// two (keeping the first one's nanos). The stored grant is Go's.
#[test]
fn v1_messages_are_read_as_gogoproto_decodes_them() {
    use celestia_proto::cosmos::authz::v1beta1::{GenericAuthorization, Grant, MsgGrant};
    use prost::Message as _;
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let mut ms = v1_store();
    let mut ctx = Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
    let (giver, taker) = ([1u8; 20], [2u8; 20]);
    let at = |seconds: i64, nanos: i32| pbjson_types::Timestamp { seconds, nanos };
    let grant = |exp| MsgGrant {
        granter: x_auth::bech32(&giver).unwrap(),
        grantee: x_auth::bech32(&taker).unwrap(),
        grant: Some(Grant {
            authorization: Some(celestia_proto::Any::pack(&GenericAuthorization {
                msg: "/cosmos.bank.v1beta1.MsgSend".into(),
            })),
            expiration: Some(exp),
        }),
    };
    // MsgGrant field 3 (grant) twice: prost merges the two `Grant`s
    // (authorization from the first, expiration merged field by field);
    // gogoproto merges the `Grant`s too but replaces the expiration.
    let mut value = grant(at(2_000_000_000, 7)).encode_to_vec();
    let mut second = Vec::new();
    prost::encoding::message::encode(
        3,
        &Grant {
            authorization: None,
            expiration: Some(at(2_000_000_001, 0)),
        },
        &mut second,
    );
    value.extend_from_slice(&second);
    let msg = celestia_proto::Any {
        type_url: "/cosmos.authz.v1beta1.MsgGrant".into(),
        value,
    };
    msgs::validate_basic(&msg).unwrap();
    msgs::route(&mut ctx, &msg).unwrap();
    let key = x_authz::grant_key(&taker, &giver, "/cosmos.bank.v1beta1.MsgSend");
    let stored = ctx.get(x_authz::STORE_KEY, &key).unwrap().unwrap();
    let stored = Grant::decode(stored.as_slice()).unwrap();
    assert_eq!(stored.expiration, Some(at(2_000_000_001, 0)));

    // A non-critical unknown field whose key is past `u32::MAX`: gogoproto
    // skips it (as Go's decoder accepted it), prost fails. The MsgSend's
    // own `ValidateBasic` decides (no coins: invalid coins).
    let mut value = celestia_proto::cosmos::bank::v1beta1::MsgSend {
        from_address: x_auth::bech32(&giver).unwrap(),
        to_address: x_auth::bech32(&taker).unwrap(),
        amount: Vec::new(),
    }
    .encode_to_vec();
    value.extend_from_slice(&[0xf8, 0xff, 0xff, 0xff, 0x3f, 0x00]);
    let send = celestia_proto::Any {
        type_url: "/cosmos.bank.v1beta1.MsgSend".into(),
        value,
    };
    let e = msgs::validate_basic(&send).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 10), "{}", e.log);
}

/// The ibc-go v6 tendermint `ClientState.trusting_period` (non-nullable
/// stdduration) sent twice inside `MsgCreateClient`'s client state `Any`:
/// the handlers see gogoproto's last value, not prost's merge.
#[test]
fn v1_nested_std_durations_are_read_as_gogoproto_decodes_them() {
    use celestia_proto::ibc::core::client::v1::MsgCreateClient;
    use celestia_proto::ibc::lightclients::tendermint::v1::ClientState;
    use prost::Message as _;
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let d = |seconds: i64, nanos: i32| pbjson_types::Duration { seconds, nanos };
    let mut cs = ClientState {
        chain_id: "x".into(),
        trusting_period: Some(d(100, 5)),
        ..ClientState::default()
    }
    .encode_to_vec();
    prost::encoding::message::encode(3, &d(200, 0), &mut cs);
    let msg = celestia_proto::Any::pack(&MsgCreateClient {
        client_state: Some(celestia_proto::Any {
            type_url: "/ibc.lightclients.tendermint.v1.ClientState".into(),
            value: cs,
        }),
        consensus_state: None,
        signer: String::new(),
    });
    let view = msgs::gogo_view(&msg).unwrap();
    let m = MsgCreateClient::decode(view.value.as_slice()).unwrap();
    let cs = ClientState::decode(m.client_state.unwrap().value.as_slice()).unwrap();
    assert_eq!(cs.trusting_period, Some(d(200, 0)));
}

/// cosmos-sdk v0.46 `CreateVestingAccount`: a continuous account starts
/// at the block time when its start time is 0, takes the next account
/// number and receives the coins; an existing account or `end <= start`
/// fails.
#[test]
fn v046_create_vesting_account() {
    use celestia_proto::cosmos::base::v1beta1::Coin;
    use celestia_proto::cosmos::vesting::v1beta1::MsgCreateVestingAccount;
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let mut ms = v1_store();
    let mut ctx = Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
    ctx.consensus_params.version = Some(VersionParams { app: 1 });
    write_mainnet_genesis_params(&mut ctx).unwrap();
    ctx.header.time = pbjson_types::Timestamp {
        seconds: 1_700_000_000,
        nanos: 5,
    };
    let (from, to) = ([1u8; 20], [2u8; 20]);
    let acc = x_auth::new_account_with_address(&mut ctx, &from).unwrap();
    x_auth::set_account(&mut ctx, &acc).unwrap();
    x_bank::set_balance(&mut ctx, &from, &sdk::coins::Coin::new("utia", 1000)).unwrap();
    let msg = |start: i64, end: i64| {
        celestia_proto::Any::pack(&MsgCreateVestingAccount {
            from_address: x_auth::bech32(&from).unwrap(),
            to_address: x_auth::bech32(&to).unwrap(),
            amount: vec![Coin {
                denom: "utia".into(),
                amount: "400".into(),
            }],
            end_time: end,
            delayed: false,
            start_time: start,
        })
    };
    let e = msgs::validate_basic(&msg(0, 0)).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 18), "{}", e.log);
    let e = msgs::route(&mut ctx, &msg(10, 10)).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 18), "{}", e.log);
    msgs::validate_basic(&msg(0, 1_800_000_000)).unwrap();
    msgs::route(&mut ctx, &msg(0, 1_800_000_000)).unwrap();
    let Some(x_auth::Account::ContinuousVesting(v)) = x_auth::get_account(&mut ctx, &to).unwrap()
    else {
        panic!("not a continuous vesting account")
    };
    assert_eq!(v.start_time, 1_700_000_000);
    let bva = v.base_vesting_account.unwrap();
    assert_eq!(bva.end_time, 1_800_000_000);
    assert_eq!(bva.base_account.unwrap().account_number, 1);
    let bal = x_bank::get_balance(&mut ctx, &to, "utia").unwrap().amount;
    assert_eq!(bal, math::Int::from_u64(400));
    let e = msgs::route(&mut ctx, &msg(0, 1_800_000_000)).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 18), "{}", e.log);
}

/// cosmos-sdk v0.46 feegrant: granting creates the grantee account and
/// queues the expiration; a duplicate fails; using the allowance stores
/// it through `UpdateAllowance` (`update_feegrant`, no account read);
/// revoking deletes only the grant, not its queue entry.
#[test]
fn v046_feegrant_grant_use_revoke() {
    use celestia_proto::cosmos::base::v1beta1::Coin;
    use celestia_proto::cosmos::feegrant::v1beta1::{
        BasicAllowance, MsgGrantAllowance, MsgRevokeAllowance,
    };
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let mut ms = v1_store();
    let mut ctx = Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
    ctx.header.time = pbjson_types::Timestamp {
        seconds: 1_000,
        nanos: 0,
    };
    let (from, to) = ([1u8; 20], [2u8; 20]);
    let exp = pbjson_types::Timestamp {
        seconds: 5_000,
        nanos: 0,
    };
    let grant = celestia_proto::Any::pack(&MsgGrantAllowance {
        granter: x_auth::bech32(&from).unwrap(),
        grantee: x_auth::bech32(&to).unwrap(),
        allowance: Some(celestia_proto::Any::pack(&BasicAllowance {
            spend_limit: vec![Coin {
                denom: "utia".into(),
                amount: "100".into(),
            }],
            expiration: Some(exp),
        })),
    });
    msgs::validate_basic(&grant).unwrap();
    msgs::route(&mut ctx, &grant).unwrap();
    assert!(x_auth::get_account(&mut ctx, &to).unwrap().is_some());
    let key = x_feegrant::allowance_key(&from, &to);
    let queue = x_feegrant::queue_key(&exp, &key[1..]);
    assert!(ctx.get(x_feegrant::STORE_KEY, &queue).unwrap().is_some());
    let e = msgs::route(&mut ctx, &grant).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 18), "{}", e.log);

    ctx.events.take();
    x_feegrant::use_granted_fees(
        &mut ctx,
        &from,
        &to,
        &sdk::coins::Coins::from_proto(&[Coin {
            denom: "utia".into(),
            amount: "40".into(),
        }])
        .unwrap(),
        &[],
    )
    .unwrap();
    let types: Vec<String> = ctx.events.take().into_iter().map(|e| e.r#type).collect();
    assert_eq!(types, ["use_feegrant", "update_feegrant"]);
    let left = x_feegrant::get_allowance(&mut ctx, &from, &to).unwrap();
    let x_feegrant::Allowance::Basic(b) = left else {
        panic!("not basic")
    };
    assert_eq!(b.spend_limit[0].amount, "60");

    let revoke = celestia_proto::Any::pack(&MsgRevokeAllowance {
        granter: x_auth::bech32(&from).unwrap(),
        grantee: x_auth::bech32(&to).unwrap(),
    });
    msgs::route(&mut ctx, &revoke).unwrap();
    assert!(ctx.get(x_feegrant::STORE_KEY, &key).unwrap().is_none());
    assert!(ctx.get(x_feegrant::STORE_KEY, &queue).unwrap().is_some());
    let e = msgs::route(&mut ctx, &revoke).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 38), "{}", e.log);
}

/// cosmos-sdk v0.46 `FundCommunityPool`: the coins move to the
/// distribution module account and into the community pool;
/// `ValidateBasic` rejects invalid coins and an empty depositor.
#[test]
fn v046_fund_community_pool() {
    use celestia_proto::cosmos::base::v1beta1::Coin;
    use celestia_proto::cosmos::distribution::v1beta1::MsgFundCommunityPool;
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let mut ms = v1_store();
    let mut ctx = Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
    x_distribution::store::set_fee_pool(&mut ctx, &sdk::dec_coins::DecCoins::new()).unwrap();
    let from = [7u8; 20];
    let acc = x_auth::new_account_with_address(&mut ctx, &from).unwrap();
    x_auth::set_account(&mut ctx, &acc).unwrap();
    x_bank::set_balance(&mut ctx, &from, &sdk::coins::Coin::new("utia", 50)).unwrap();
    let fund = |depositor: String, amount: &str| {
        celestia_proto::Any::pack(&MsgFundCommunityPool {
            amount: vec![Coin {
                denom: "utia".into(),
                amount: amount.into(),
            }],
            depositor,
        })
    };
    let e = msgs::validate_basic(&fund(String::new(), "1")).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 7), "{}", e.log);
    let e = msgs::validate_basic(&fund(x_auth::bech32(&from).unwrap(), "-1")).unwrap_err();
    assert_eq!((e.codespace, e.code), ("sdk", 10), "{}", e.log);
    let ok = fund(x_auth::bech32(&from).unwrap(), "20");
    msgs::validate_basic(&ok).unwrap();
    msgs::route(&mut ctx, &ok).unwrap();
    assert_eq!(
        x_distribution::store::get_fee_pool(&mut ctx)
            .unwrap()
            .to_string(),
        "20.000000000000000000utia"
    );
    let distr = x_auth::module_address("distribution");
    assert_eq!(
        x_bank::get_balance(&mut ctx, &distr, "utia")
            .unwrap()
            .amount,
        math::Int::from_u64(20)
    );
}

/// cosmos-sdk v0.46 `UpdateValidatorCommission` below the minimum rate
/// reads `MinCommissionRate` again for its error message: two metered
/// reads of the parameter (1141 gas each on mainnet) instead of one.
#[test]
fn v046_commission_below_min_rate_reads_the_param_twice() {
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let mut ms = v1_store();
    let (mut ctx, _) = v046_two_validators(&mut ms, 10);
    ctx.header.time = pbjson_types::Timestamp {
        seconds: 1_000_000,
        nanos: 0,
    };
    let mut v = x_staking::store::validator(&mut ctx, &[1; 20]).unwrap();
    v.commission.max_rate = math::LegacyDec::one();
    v.commission.max_change_rate = math::LegacyDec::one();
    let gas = |ctx: &mut sdk::Context<'_>, rate: &str| {
        ctx.gas = kvstore::GasMeter::infinite();
        let r = x_staking::validator::update_validator_commission(
            ctx,
            &v,
            &math::LegacyDec::from_str_go(rate).unwrap(),
        );
        (r.is_ok(), ctx.gas.consumed())
    };
    assert_eq!(gas(&mut ctx, "0.06"), (true, 1141));
    assert_eq!(gas(&mut ctx, "0.01"), (false, 2 * 1141));
}

/// v0.46 `DispatchActions` of a bank `MsgSend` signed by the grantee
/// itself (implicitly accepted) runs the send; an ibc-go v6
/// `MsgTransfer` whose sender does not parse panics in `GetSigners`.
#[test]
fn v046_exec_dispatches_send_and_transfer_signers() {
    use celestia_proto::cosmos::authz::v1beta1::MsgExec;
    use celestia_proto::cosmos::bank::v1beta1::MsgSend;
    use celestia_proto::cosmos::base::v1beta1::Coin;
    use celestia_proto::ibc::applications::transfer::v1::MsgTransfer;
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let mut ms = v1_store();
    let mut ctx = Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
    ctx.consensus_params.version = Some(VersionParams { app: 1 });
    write_mainnet_genesis_params(&mut ctx).unwrap();
    let (a, b) = ([1u8; 20], [2u8; 20]);
    let acc = x_auth::new_account_with_address(&mut ctx, &a).unwrap();
    x_auth::set_account(&mut ctx, &acc).unwrap();
    x_bank::set_balance(&mut ctx, &a, &sdk::coins::Coin::new("utia", 10)).unwrap();
    let exec = |m: celestia_proto::Any| {
        celestia_proto::Any::pack(&MsgExec {
            grantee: x_auth::bech32(&a).unwrap(),
            msgs: vec![m],
        })
    };
    let send = celestia_proto::Any::pack(&MsgSend {
        from_address: x_auth::bech32(&a).unwrap(),
        to_address: x_auth::bech32(&b).unwrap(),
        amount: vec![Coin {
            denom: "utia".into(),
            amount: "3".into(),
        }],
    });
    let _ = take_not_ported();
    msgs::route(&mut ctx, &exec(send)).unwrap();
    assert!(take_not_ported().is_none());
    assert_eq!(
        x_bank::get_balance(&mut ctx, &b, "utia").unwrap().amount,
        math::Int::from_u64(3)
    );
    let transfer = celestia_proto::Any::pack(&MsgTransfer {
        sender: "not-an-address".into(),
        ..MsgTransfer::default()
    });
    let e = sdk::error::recover(|| msgs::route(&mut ctx, &exec(transfer))).unwrap_err();
    assert!(e.is_panic(), "{}", e.log);
}

/// v0.46 `DispatchActions` of a nested authz `MsgGrant` (signer: its
/// granter) under a `GenericAuthorization` for `MsgGrant`: the nested
/// grant is stored (mainnet block 45631).
#[test]
fn v046_exec_dispatches_a_nested_grant() {
    use celestia_proto::cosmos::authz::v1beta1::{GenericAuthorization, Grant, MsgExec, MsgGrant};
    use prost::Message as _;
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let mut ms = v1_store();
    let mut ctx = Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
    let (owner, agent, voter) = ([1u8; 20], [2u8; 20], [3u8; 20]);
    let grant = |granter: &[u8], grantee: &[u8], msg: &str| MsgGrant {
        granter: x_auth::bech32(granter).unwrap(),
        grantee: x_auth::bech32(grantee).unwrap(),
        grant: Some(Grant {
            authorization: Some(celestia_proto::Any::pack(&GenericAuthorization {
                msg: msg.into(),
            })),
            expiration: None,
        }),
    };
    let outer = celestia_proto::Any::pack(&grant(&owner, &agent, "/cosmos.authz.v1beta1.MsgGrant"));
    msgs::route(&mut ctx, &outer).unwrap();
    let nested = celestia_proto::Any::pack(&grant(&owner, &voter, "/cosmos.gov.v1beta1.MsgVote"));
    let exec = celestia_proto::Any::pack(&MsgExec {
        grantee: x_auth::bech32(&agent).unwrap(),
        msgs: vec![nested],
    });
    let _ = take_not_ported();
    msgs::route(&mut ctx, &exec).unwrap();
    assert!(take_not_ported().is_none());
    let key = x_authz::grant_key(&voter, &owner, "/cosmos.gov.v1beta1.MsgVote");
    let stored = ctx.get(x_authz::STORE_KEY, &key).unwrap().unwrap();
    assert!(Grant::decode(stored.as_slice()).is_ok());
}

/// `MsgExec` dispatches the gov messages (signer: voter, depositor,
/// proposer): run by their own signer, they reach the gov server (an
/// unknown proposal fails with gov code 2).
#[test]
fn v046_exec_dispatches_gov_messages() {
    use celestia_proto::cosmos::authz::v1beta1::MsgExec;
    use celestia_proto::cosmos::gov::{v1, v1beta1};
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let mut ms = v1_store();
    let mut ctx = Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
    let me = x_auth::bech32(&[4u8; 20]).unwrap();
    let nested = [
        celestia_proto::Any::pack(&v1beta1::MsgVoteWeighted {
            proposal_id: 7,
            voter: me.clone(),
            options: vec![v1beta1::WeightedVoteOption {
                option: 1,
                weight: "1000000000000000000".into(),
            }],
        }),
        celestia_proto::Any::pack(&v1::MsgVoteWeighted {
            proposal_id: 7,
            voter: me.clone(),
            options: vec![v1::WeightedVoteOption {
                option: 1,
                weight: "1.000000000000000000".into(),
            }],
            metadata: String::new(),
        }),
        celestia_proto::Any::pack(&v1beta1::MsgDeposit {
            proposal_id: 7,
            depositor: me.clone(),
            amount: vec![],
        }),
        celestia_proto::Any::pack(&v1::MsgDeposit {
            proposal_id: 7,
            depositor: me.clone(),
            amount: vec![],
        }),
    ];
    for m in nested {
        let exec = celestia_proto::Any::pack(&MsgExec {
            grantee: me.clone(),
            msgs: vec![m.clone()],
        });
        let _ = take_not_ported();
        let e = msgs::route(&mut ctx, &exec).unwrap_err();
        assert!(take_not_ported().is_none(), "{}", m.type_url);
        assert_eq!((e.codespace, e.code), ("gov", 2), "{}", e.log);
    }
}

/// celestia's router runs a `MsgExec`-dispatched `MsgDelegate` whose
/// `ValidateBasic` fails on a zero amount: the delegation of 0 tokens
/// succeeds (mainnet block 52698). A nil amount panics in `IsZero`.
#[test]
fn v046_exec_runs_a_zero_amount_delegate() {
    use celestia_proto::cosmos::authz::v1beta1::MsgExec;
    use celestia_proto::cosmos::base::v1beta1::Coin;
    use celestia_proto::cosmos::staking::v1beta1::MsgDelegate;
    crate::modules::install_staking_hooks();
    let _v046 = sdk::version::SdkV046Scope::enter(1);
    let mut ms = v1_store();
    let (mut ctx, _) = v046_two_validators(&mut ms, 10);
    x_distribution::store::set_fee_pool(&mut ctx, &sdk::dec_coins::DecCoins::new()).unwrap();
    let del = [9u8; 20];
    let acc = x_auth::new_account_with_address(&mut ctx, &del).unwrap();
    x_auth::set_account(&mut ctx, &acc).unwrap();
    let delegate = |amount: Option<Coin>| {
        celestia_proto::Any::pack(&MsgExec {
            grantee: x_auth::bech32(&del).unwrap(),
            msgs: vec![celestia_proto::Any::pack(&MsgDelegate {
                delegator_address: x_auth::bech32(&del).unwrap(),
                validator_address: x_staking::addr::val_to_string(&[1; 20]).unwrap(),
                amount,
            })],
        })
    };
    let _ = take_not_ported();
    msgs::route(
        &mut ctx,
        &delegate(Some(Coin {
            denom: "utia".into(),
            amount: "0".into(),
        })),
    )
    .unwrap();
    assert!(take_not_ported().is_none());
    let d = x_staking::store::get_delegation(&mut ctx, &del, &[1; 20])
        .unwrap()
        .unwrap();
    assert!(d.shares.is_zero());
    let e = sdk::error::recover(|| msgs::route(&mut ctx, &delegate(None))).unwrap_err();
    assert!(e.is_panic(), "{}", e.log);
}

/// A v1 to v2 upgrade block whose module migrations fail (here the
/// `icahost` port capability is already taken) leaves no trace: the
/// app version 1 store set, no new stores, the app version, and the
/// same failure when the block is retried.
#[test]
fn failed_upgrade_migrations_leave_the_store_set_unchanged() {
    let mut ms = v1_store();
    let mut driver = Driver::new(NoModules);
    driver.set_upgrade_height_v2(2);
    driver.init_chain(&mut ms, &init_request()).unwrap();
    {
        let _v046 = sdk::version::SdkV046Scope::enter(1);
        let mut ctx = Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
        x_ibc::port::bind_port(&mut ctx, "icahost").unwrap();
    }
    for _ in 0..2 {
        assert!(
            driver
                .finalize_block(&mut ms, &finalize_request(1))
                .is_err()
        );
        assert!(ms.is_mounted("qgb"));
        for s in ["icahost", "packetfowardmiddleware", "signal"] {
            assert!(!ms.is_mounted(s), "{s}");
        }
        assert_eq!(driver.app_version(), 1);
        assert_eq!(app_version(&ms), 1);
    }
}
