//! `start --network local`: a single-validator devnet in one command —
//! `init`, a `validator` key in the test keyring, its genesis account and
//! gentx, `collect-gentxs`, then the node starts. Tx indexing and the app
//! gRPC server are enabled for local tooling.

use std::path::Path;

use anyhow::anyhow;

use crate::config::{File, set};
use crate::{genesis, init, keys, networks, util};

/// Genesis balance of the local validator (1 billion TIA).
pub const VALIDATOR_BALANCE: &str = "1000000000000000utia";
/// Self delegation of the local validator (1 million TIA).
pub const VALIDATOR_STAKE: &str = "1000000000000utia";
/// Name of the validator key.
pub const VALIDATOR_KEY: &str = "validator";

/// Creates the local devnet home (no-op when a genesis exists).
pub fn setup(home: &Path) -> anyhow::Result<()> {
    if home.join("config").join("genesis.json").exists() {
        return Ok(());
    }
    util::err(format!(
        "creating a single-validator devnet ({}) in {}",
        networks::LOCAL.chain_id,
        home.display()
    ));
    init::init(&init::InitOptions {
        home: home.to_path_buf(),
        moniker: "local".into(),
        chain_id: String::new(),
        network: Some(networks::LOCAL),
        overwrite: false,
        recover_mnemonic: None,
        download_genesis: false,
    })?;
    let mut kr = keyring::Keyring::new(home, keyring::Backend::Test);
    let mnemonic = keyring::hd::new_mnemonic();
    let key = kr
        .add_mnemonic(VALIDATOR_KEY, &mnemonic, "", keyring::hd::DEFAULT_HD_PATH)
        .map_err(|e| anyhow!("{e}"))?;
    let gfile = home.join("config").join("genesis.json");
    genesis::add_genesis_account(
        &gfile,
        &key.bech32(keys::ACC_PREFIX),
        VALIDATOR_BALANCE,
        false,
    )?;
    genesis::gentx(
        &genesis::GentxOptions {
            home: home.to_path_buf(),
            amount: VALIDATOR_STAKE.into(),
            ip: "127.0.0.1".into(),
            ..genesis::GentxOptions::default()
        },
        &key,
    )?;
    genesis::collect_gentxs(home, None)?;
    set(home, File::App, "grpc.enable", "true")?;
    set(home, File::Config, "p2p.pex", "false")?;
    util::err(format!(
        "validator key `{VALIDATOR_KEY}` ({}) in the test keyring; mnemonic:\n\n{mnemonic}\n",
        key.bech32(keys::ACC_PREFIX)
    ));
    Ok(())
}
