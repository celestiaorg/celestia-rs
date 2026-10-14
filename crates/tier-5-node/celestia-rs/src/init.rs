//! `celestia-rs init <moniker>`: a node home identical to Go
//! `celestia-appd init`'s (celestia-app v10.4.0 templates for config.toml,
//! app.toml, client.toml and the default genesis; Go-format node key and
//! validator key / sign state), so either binary can run it.
//!
//! With `--network`, the chain ID, seeds and persistent peers of a public
//! network are written into config.toml and its genesis is downloaded and
//! verified (network access needed; on failure Go's placeholder genesis
//! stays and `download-genesis` can be retried).

use std::path::{Path, PathBuf};

use anyhow::{Context as _, anyhow, bail};

use crate::config::{APP_TEMPLATE, CLIENT_TEMPLATE, CONFIG_TEMPLATE, GENESIS_TEMPLATE};
use crate::networks::Network;
use crate::util;

/// Options of `init`.
#[derive(Debug, Clone)]
pub struct InitOptions {
    /// Node home.
    pub home: PathBuf,
    /// Moniker.
    pub moniker: String,
    /// Chain ID (random `test-chain-XXXXXX` when empty, like Go).
    pub chain_id: String,
    /// Built-in network (sets the chain ID, seeds and genesis).
    pub network: Option<Network>,
    /// Overwrite an existing genesis.json.
    pub overwrite: bool,
    /// Derive the validator key from this mnemonic (`--recover`).
    pub recover_mnemonic: Option<String>,
    /// Try to download the network genesis.
    pub download_genesis: bool,
}

/// What `init` created (Go's printed `printInfo`).
#[derive(Debug, Clone)]
pub struct InitInfo {
    /// Moniker.
    pub moniker: String,
    /// Chain ID.
    pub chain_id: String,
    /// P2P node ID.
    pub node_id: String,
    /// Genesis download outcome (network homes).
    pub genesis_note: Option<String>,
}

fn random_suffix() -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    use rand::Rng as _;
    let mut rng = rand::thread_rng();
    (0..6)
        .map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char)
        .collect()
}

/// The config.toml template for `moniker`.
#[must_use]
pub fn config_toml(moniker: &str) -> String {
    CONFIG_TEMPLATE.replace("{{MONIKER}}", moniker)
}

/// The client.toml template for `chain_id`.
#[must_use]
pub fn client_toml(chain_id: &str) -> String {
    CLIENT_TEMPLATE.replace("{{CHAIN_ID}}", chain_id)
}

/// The default genesis (Go `celestia-appd init`) for `chain_id` at
/// `genesis_time` (RFC 3339).
#[must_use]
pub fn default_genesis(chain_id: &str, genesis_time: &str) -> String {
    GENESIS_TEMPLATE
        .replace("{{CHAIN_ID}}", chain_id)
        .replace("{{GENESIS_TIME}}", genesis_time)
}

/// Sets `[p2p] seeds` / `persistent_peers` of a written config.toml.
pub fn apply_network_peers(home: &Path, net: &Network) -> anyhow::Result<()> {
    use crate::config::{File, set};
    if !net.seeds.is_empty() {
        set(home, File::Config, "p2p.seeds", &net.seeds.join(","))?;
    }
    if !net.persistent_peers.is_empty() {
        set(
            home,
            File::Config,
            "p2p.persistent_peers",
            &net.persistent_peers.join(","),
        )?;
    }
    Ok(())
}

/// Writes the validator key files unless they exist (Go
/// `InitializeNodeValidatorFilesFromMnemonic`).
fn validator_files(home: &Path, mnemonic: Option<&str>) -> anyhow::Result<()> {
    let key = home.join("config").join("priv_validator_key.json");
    let state = home.join("data").join("priv_validator_state.json");
    if key.exists() {
        if !state.exists() {
            let pv = privval::FilePv::load_empty_state(&key, &state).map_err(|e| anyhow!("{e}"))?;
            pv.last_sign_state.save().map_err(|e| anyhow!("{e}"))?;
        }
        return Ok(());
    }
    let pv = match mnemonic {
        Some(m) => {
            let seed = hash::sha256(m.as_bytes());
            let k =
                privval::FilePvKey::from_priv_key(crypto::ed25519::private_key_from_seed(&seed))
                    .map_err(|e| anyhow!("{e}"))?;
            privval::FilePv::new(k, Some(key.clone()), Some(state.clone()))
        }
        None => privval::FilePv::generate(&key, &state).map_err(|e| anyhow!("{e}"))?,
    };
    pv.save().map_err(|e| anyhow!("{e}"))?;
    restrict(&key)?;
    Ok(())
}

fn restrict(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    let _ = path;
    Ok(())
}

/// `init`.
pub fn init(opts: &InitOptions) -> anyhow::Result<InitInfo> {
    let home = &opts.home;
    let cfg_dir = home.join("config");
    std::fs::create_dir_all(&cfg_dir).with_context(|| format!("create {}", cfg_dir.display()))?;
    std::fs::create_dir_all(home.join("data"))?;
    let chain_id = match (&opts.network, opts.chain_id.as_str()) {
        (Some(n), "") => n.chain_id.to_owned(),
        (Some(n), c) if c != n.chain_id => {
            bail!("--chain-id {c} conflicts with --network {}", n.name)
        }
        (_, "") => format!("test-chain-{}", random_suffix()),
        (_, c) => c.to_owned(),
    };
    if let Some(m) = &opts.recover_mnemonic {
        keyring::hd::validate_mnemonic(m).map_err(|_| anyhow!("invalid mnemonic"))?;
    }
    let node_key = crate::net::node_key(home)?;
    restrict(&cfg_dir.join("node_key.json"))?;
    validator_files(home, opts.recover_mnemonic.as_deref())?;
    let genesis = cfg_dir.join("genesis.json");
    if genesis.exists() && !opts.overwrite {
        bail!("genesis.json file already exists: {}", genesis.display());
    }
    // app.toml and client.toml are only created when missing (the SDK's
    // interceptConfigs); config.toml is always rewritten (Go's init).
    let app = cfg_dir.join("app.toml");
    if !app.exists() {
        std::fs::write(&app, APP_TEMPLATE)?;
    }
    let client = cfg_dir.join("client.toml");
    if !client.exists() {
        util::write_private(&client, client_toml(&chain_id).as_bytes())?;
    }
    util::write_private(
        &genesis,
        default_genesis(&chain_id, &util::now_rfc3339()).as_bytes(),
    )?;
    std::fs::write(cfg_dir.join("config.toml"), config_toml(&opts.moniker))?;
    let mut genesis_note = None;
    if let Some(net) = &opts.network {
        apply_network_peers(home, net)?;
        if net.name == "local" {
            crate::config::set(home, crate::config::File::Config, "tx_index.indexer", "kv")?;
        } else if opts.download_genesis {
            genesis_note = Some(match crate::networks::download_genesis(net, &genesis) {
                Ok(()) => format!("downloaded and verified the {} genesis", net.chain_id),
                Err(e) => format!(
                    "{e:#}\nThe default genesis was written instead; retry with `celestia-rs download-genesis --network {}`.",
                    net.name
                ),
            });
        } else {
            genesis_note = Some(format!(
                "The chain ID {} is a public network.\nPlease download the genesis file via:\n\tcelestia-rs download-genesis --network {}",
                net.chain_id, net.name
            ));
        }
    } else if let Some(net) = crate::networks::by_chain_id(&chain_id) {
        if net.genesis_sha256.is_some() {
            genesis_note = Some(format!(
                "The chain ID {chain_id} is a public network.\nPlease download the genesis file via:\n\tcelestia-rs download-genesis --network {}",
                net.name
            ));
        }
    }
    Ok(InitInfo {
        moniker: opts.moniker.clone(),
        chain_id,
        node_id: node_key.id().to_string(),
        genesis_note,
    })
}

/// Go's `displayInfo` JSON (stderr).
pub fn print_info(home: &Path, info: &InitInfo) -> anyhow::Result<()> {
    let genesis: serde_json::Value =
        serde_json::from_slice(&std::fs::read(home.join("config").join("genesis.json"))?)?;
    let v = serde_json::json!({
        "moniker": info.moniker,
        "chain_id": info.chain_id,
        "node_id": info.node_id,
        "gentxs_dir": "",
        "app_message": genesis["app_state"],
    });
    if let Some(note) = &info.genesis_note {
        util::out(note);
    }
    util::err(util::json_indent(&v, " "));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_have_placeholders() {
        assert!(config_toml("x").contains("moniker = \"x\""));
        let g = default_genesis("c", "2026-01-01T00:00:00Z");
        let v: serde_json::Value = serde_json::from_str(&g).unwrap();
        assert_eq!(v["chain_id"], "c");
        assert_eq!(v["consensus"]["params"]["version"]["app"], "10");
    }
}
