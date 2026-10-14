//! Operator CLI parity with Go `celestia-appd` (v10.4.0) on Go-generated
//! fixtures (`tests/fixtures/gentx`: a Go home after `init`,
//! `keys add val --recover`, `genesis add-genesis-account`,
//! `genesis gentx … --fees 500000utia --ip 127.0.0.1` and
//! `collect-gentxs`; `templates/`: Go `init` output):
//!
//! - `init` writes Go's config.toml / app.toml / client.toml / genesis;
//! - `genesis gentx` signs the byte-identical gentx (deterministic
//!   secp256k1 signatures) and `collect-gentxs` produces Go's genesis;
//! - `genesis validate` accepts it and runs its InitChain;
//! - `config set` keeps the file layout.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use celestia_rs::{config, genesis, init};
use serde_json::Value;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/gentx");
const MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

fn json(p: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
}

fn init_home(home: &Path, chain: &str) -> init::InitInfo {
    init::init(&init::InitOptions {
        home: home.to_path_buf(),
        moniker: "n0".into(),
        chain_id: chain.into(),
        network: None,
        overwrite: true,
        recover_mnemonic: None,
        download_genesis: false,
    })
    .unwrap()
}

#[test]
fn init_writes_go_templates() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let info = init_home(home, "rs-test-1");
    assert_eq!(info.chain_id, "rs-test-1");
    assert_eq!(info.node_id.len(), 40);
    let cfg = std::fs::read_to_string(home.join("config/config.toml")).unwrap();
    assert_eq!(cfg, config::CONFIG_TEMPLATE.replace("{{MONIKER}}", "n0"));
    assert_eq!(
        std::fs::read_to_string(home.join("config/app.toml")).unwrap(),
        config::APP_TEMPLATE
    );
    let g = json(&home.join("config/genesis.json"));
    assert_eq!(g["chain_id"], "rs-test-1");
    assert_eq!(g["consensus"]["params"]["version"]["app"], "10");
    assert_eq!(g["app_state"]["staking"]["params"]["bond_denom"], "utia");
    // Go key file formats.
    let pv = json(&home.join("data/priv_validator_state.json"));
    assert_eq!(pv["height"], "0");
    let nk = json(&home.join("config/node_key.json"));
    assert_eq!(nk["priv_key"]["type"], "tendermint/PrivKeyEd25519");
    let pk = json(&home.join("config/priv_validator_key.json"));
    assert_eq!(pk["pub_key"]["type"], "tendermint/PubKeyEd25519");
    // A second init refuses to overwrite the genesis.
    assert!(
        init::init(&init::InitOptions {
            home: home.to_path_buf(),
            moniker: "n0".into(),
            chain_id: "x".into(),
            network: None,
            overwrite: false,
            recover_mnemonic: None,
            download_genesis: false,
        })
        .is_err()
    );
    // --recover derives the validator key from the mnemonic like Go
    // (ed25519 seed = sha256(mnemonic)).
    let dir2 = tempfile::tempdir().unwrap();
    init::init(&init::InitOptions {
        home: dir2.path().to_path_buf(),
        moniker: "n0".into(),
        chain_id: "x".into(),
        network: None,
        overwrite: false,
        recover_mnemonic: Some(MNEMONIC.into()),
        download_genesis: false,
    })
    .unwrap();
    let k = privval::FilePvKey::from_json(
        &std::fs::read(dir2.path().join("config/priv_validator_key.json")).unwrap(),
    )
    .unwrap();
    let want = privval::FilePvKey::from_priv_key(crypto::ed25519::private_key_from_seed(
        &hash::sha256(MNEMONIC.as_bytes()),
    ))
    .unwrap();
    assert_eq!(k.pub_key, want.pub_key);
}

#[test]
fn network_init_sets_peers_and_chain() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let info = init::init(&init::InitOptions {
        home: home.to_path_buf(),
        moniker: "m".into(),
        chain_id: String::new(),
        network: Some(celestia_rs::networks::MOCHA),
        overwrite: false,
        recover_mnemonic: None,
        // Offline: the placeholder genesis stays and the note says so.
        download_genesis: false,
    })
    .unwrap();
    assert_eq!(info.chain_id, "mocha-5");
    assert!(info.genesis_note.unwrap().contains("download-genesis"));
    let seeds = config::get(home, config::File::Config, "p2p.seeds").unwrap();
    assert_eq!(seeds, celestia_rs::networks::MOCHA.seeds.join(","));
    let peers = config::get(home, config::File::Config, "p2p.persistent_peers").unwrap();
    assert_eq!(
        peers,
        celestia_rs::networks::MOCHA.persistent_peers.join(",")
    );
    // The placeholder genesis is not mocha's: start --network would
    // download (or refuse).
    let bz = std::fs::read(home.join("config/genesis.json")).unwrap();
    assert!(celestia_rs::networks::verify_genesis(&celestia_rs::networks::MOCHA, &bz).is_err());
}

/// A Go home (chain `gx-1`) with the funded `val` key: Go's node and
/// validator keys, Go's genesis without the collected gentx.
fn go_home(home: &Path) -> keyring::KeyRecord {
    let cfg = home.join("config");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::create_dir_all(home.join("data")).unwrap();
    for f in ["node_key.json", "priv_validator_key.json"] {
        std::fs::copy(Path::new(FIXTURE).join("config").join(f), cfg.join(f)).unwrap();
    }
    std::fs::write(cfg.join("config.toml"), init::config_toml("n0")).unwrap();
    let mut g = json(&Path::new(FIXTURE).join("expected-genesis.json"));
    g["app_state"]["genutil"]["gen_txs"] = Value::Array(Vec::new());
    genesis::write(&cfg.join("genesis.json"), &g).unwrap();
    let mut kr = keyring::Keyring::new(home, keyring::Backend::Test);
    kr.add_mnemonic("val", MNEMONIC, "", keyring::hd::DEFAULT_HD_PATH)
        .unwrap()
}

#[test]
fn gentx_and_collect_match_go() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let key = go_home(home);
    let out = genesis::gentx(
        &genesis::GentxOptions {
            home: home.to_path_buf(),
            amount: "5000000000utia".into(),
            ip: "127.0.0.1".into(),
            fee: celestia_rs::txs::FeeOptions {
                fees: "500000utia".into(),
                ..Default::default()
            },
            ..genesis::GentxOptions::default()
        },
        &key,
    )
    .unwrap();
    let ours = json(&out);
    let go = json(&Path::new(FIXTURE).join("expected-gentx.json"));
    assert_eq!(ours, go, "gentx differs from Go's");
    // Insufficient balance is refused like Go's ValidateAccountInGenesis.
    assert!(
        genesis::gentx(
            &genesis::GentxOptions {
                home: home.to_path_buf(),
                amount: "999999999999999utia".into(),
                ip: "127.0.0.1".into(),
                output_document: Some(home.join("x.json")),
                ..genesis::GentxOptions::default()
            },
            &key,
        )
        .is_err()
    );
    let peers = genesis::collect_gentxs(home, None).unwrap();
    assert_eq!(peers, "", "our own moniker is excluded");
    let g = json(&home.join("config/genesis.json"));
    let want = json(&Path::new(FIXTURE).join("expected-genesis.json"));
    assert_eq!(g["app_state"], want["app_state"]);
    // InitChain accepts it and yields the validator.
    assert_eq!(
        genesis::validate(&home.join("config/genesis.json")).unwrap(),
        1
    );
}

#[test]
fn add_genesis_account_like_go() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    init_home(home, "acc-1");
    let g = home.join("config/genesis.json");
    let a = "celestia1r5v5srda7xfth3hn2s26txvrcrntldju2pktdj";
    let b = "celestia1ler28e3jnqgq3qqaeleukcf9tw9uh2wusscn9m";
    genesis::add_genesis_account(&g, b, "5utia", false).unwrap();
    genesis::add_genesis_account(&g, a, "7utia,3foo", false).unwrap();
    assert!(genesis::add_genesis_account(&g, a, "1utia", false).is_err());
    genesis::add_genesis_account(&g, a, "1utia", true).unwrap();
    let v = json(&g);
    let accs = v["app_state"]["auth"]["accounts"].as_array().unwrap();
    // SanitizeGenesisAccounts renumbers duplicates (both start at 0).
    let nums: Vec<&str> = accs
        .iter()
        .map(|a| a["account_number"].as_str().unwrap())
        .collect();
    assert_eq!(nums, ["0", "1"]);
    // Balances sorted by address bytes, supply summed.
    let bals = v["app_state"]["bank"]["balances"].as_array().unwrap();
    assert_eq!(bals[0]["address"], a);
    assert_eq!(bals[0]["coins"][1]["amount"], "8");
    assert_eq!(
        v["app_state"]["bank"]["supply"],
        serde_json::json!([{"denom": "foo", "amount": "3"}, {"denom": "utia", "amount": "13"}])
    );
}

#[test]
fn config_set_get() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    init_home(home, "cfg-1");
    config::set(home, config::File::App, "minimum-gas-prices", "0.002utia").unwrap();
    config::set(home, config::File::App, "pruning", "custom").unwrap();
    config::set(home, config::File::App, "pruning-keep-recent", "100").unwrap();
    config::set(home, config::File::App, "pruning-interval", "10").unwrap();
    config::set(home, config::File::App, "min-retain-blocks", "0").unwrap();
    let files = config::ConfigFiles::load(home).unwrap();
    let p = config::pruning_options(&files).unwrap();
    assert_eq!((p.keep_recent, p.interval), (100, 10));
    assert_eq!(files.int(config::File::App, "min-retain-blocks"), 0);
    assert_eq!(
        config::get(home, config::File::App, "minimum-gas-prices").unwrap(),
        "0.002utia"
    );
    let app = std::fs::read_to_string(home.join("config/app.toml")).unwrap();
    assert_eq!(
        app.lines().count(),
        config::APP_TEMPLATE.lines().count(),
        "layout kept"
    );
}
