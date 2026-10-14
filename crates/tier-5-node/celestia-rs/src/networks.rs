//! Built-in networks (`--network mainnet|mocha|arabica|local`).
//!
//! Chain IDs and genesis SHA-256 hashes come from celestia-app v10.4.0
//! (`pkg/appconsts/chain_ids.go`, `cmd/celestia-appd/cmd/download_genesis.go`),
//! seeds / peers / state-sync RPC servers from its `scripts/mainnet.sh` and
//! `scripts/mocha.sh`. Genesis files are fetched at runtime from the URL list
//! and verified against the embedded hash; nothing here is contacted at
//! build or test time.
//!
//! Arabica (`arabica-11`) has no genesis hash in celestia-app and no
//! seeds in its scripts: its genesis is accepted when the chain ID matches,
//! and peers must be given with `--p2p.seeds` / `--p2p.persistent_peers`.

use std::path::Path;

use anyhow::{Context as _, anyhow, bail};

/// A public (or local) network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Network {
    /// `--network` name.
    pub name: &'static str,
    /// Chain ID.
    pub chain_id: &'static str,
    /// SHA-256 of the canonical genesis file (`download-genesis`).
    pub genesis_sha256: Option<&'static str>,
    /// Where the genesis file can be downloaded (tried in order).
    pub genesis_urls: &'static [&'static str],
    /// `[p2p] seeds`.
    pub seeds: &'static [&'static str],
    /// `[p2p] persistent_peers`.
    pub persistent_peers: &'static [&'static str],
    /// Public RPC servers for state sync (`[statesync] rpc_servers`).
    pub rpc_servers: &'static [&'static str],
}

/// Mainnet beta (`celestia`).
pub const MAINNET: Network = Network {
    name: "mainnet",
    chain_id: "celestia",
    genesis_sha256: Some("9727aac9bbfb021ce7fc695a92f901986421283a891b89e0af97bc9fad187793"),
    genesis_urls: &[
        "https://raw.githubusercontent.com/celestiaorg/networks/master/celestia/genesis.json",
    ],
    seeds: &[
        "12ad7c73c7e1f2460941326937a039139aa78884@celestia-mainnet-seed.itrocket.net:40656",
        "400f3d9e30b69e78a7fb891f60d76fa3c73f0ecc@celestia.rpc.kjnodes.com:12059",
        "59df4b3832446cd0f9c369da01f2aa5fe9647248@135.181.220.61:11756",
        "86bd5cb6e762f673f1706e5889e039d5406b4b90@seed.celestia.node75.org:20356",
        "ebc272824924ea1a27ea3183dd0b9ba713494f83@celestia-mainnet-seed.autostake.com:27206",
        "9b1d22c3a78487d1a664a4b6a331fce527d14fb4@seed.celestia.mainnet.dteam.tech:27656",
        "9aa8a73ea9364aa3cf7806d4dd25b6aed88d8152@celestia.seed.mzonder.com:13156",
        "6f6a3a908634b79b6fe7c4988efec2553f188234@celestia.seed.nodeshub.online:11656",
        "e8657b97bcfcf7e522f2481f17358c4273ee0d55@185.144.99.12:26656",
        "2030b8d022cd3c65b6f267943df82d69c3e6ba64@celestia-rpc.tienthuattoan.com:26656",
        "20e1000e88125698264454a884812746c2eb4807@seeds.lavenderfive.com:11656",
        "0c8ec01f1c37734274e7ac2f91021a55194bb0bb@65.109.26.242:11656",
        "edc6bc6ee3c37a698225e17bd4b8c687ee05f977@celestia-seed.easy2stake.com:26756",
    ],
    persistent_peers: &[],
    rpc_servers: &["https://celestia-rpc.polkachu.com:443"],
};

/// Mocha testnet (`mocha-5`).
pub const MOCHA: Network = Network {
    name: "mocha",
    chain_id: "mocha-5",
    genesis_sha256: Some("82db3ac5ab8485e4784054bd10457533a64563f055c0d234a3134603a9c85d33"),
    genesis_urls: &[
        "https://raw.githubusercontent.com/celestiaorg/networks/master/mocha-5/genesis.json",
    ],
    seeds: &[
        "ee9f90974f85c59d3861fc7f7edb10894f6ac3c8@84.32.215.148:26656",
        "b402fe40f3474e9e208840702e1b7aa37f2edc4b@celestia-testnet-seed.itrocket.net:14656",
    ],
    persistent_peers: &[
        "daf2cecee2bd7f1b3bf94839f993f807c6b15fbf@celestia-testnet-peer.itrocket.net:11656",
    ],
    rpc_servers: &["https://rpc-mocha.pops.one:443"],
};

/// Arabica devnet (`arabica-11`).
pub const ARABICA: Network = Network {
    name: "arabica",
    chain_id: "arabica-11",
    genesis_sha256: None,
    genesis_urls: &[
        "https://raw.githubusercontent.com/celestiaorg/networks/master/arabica-11/genesis.json",
    ],
    seeds: &[],
    persistent_peers: &[],
    rpc_servers: &["https://rpc.celestia-arabica-11.com:443"],
};

/// A single-validator devnet created on this machine (`start --network
/// local`).
pub const LOCAL: Network = Network {
    name: "local",
    chain_id: "celestia-rs-local",
    genesis_sha256: None,
    genesis_urls: &[],
    seeds: &[],
    persistent_peers: &[],
    rpc_servers: &[],
};

/// Every built-in network.
pub const ALL: [Network; 4] = [MAINNET, MOCHA, ARABICA, LOCAL];

/// Looks up `--network` (name or chain ID).
pub fn by_name(name: &str) -> anyhow::Result<Network> {
    ALL.iter()
        .find(|n| n.name == name || n.chain_id == name)
        .copied()
        .ok_or_else(|| {
            anyhow!(
                "unknown network {name:?} (known: {})",
                ALL.map(|n| n.name).join(", ")
            )
        })
}

/// The built-in network of a chain ID, if any.
#[must_use]
pub fn by_chain_id(chain_id: &str) -> Option<Network> {
    ALL.iter().find(|n| n.chain_id == chain_id).copied()
}

/// Lower-case hex SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(hash::sha256(bytes))
}

/// Checks a genesis file against the network: the chain ID always, the
/// SHA-256 when the network pins one.
pub fn verify_genesis(net: &Network, bytes: &[u8]) -> anyhow::Result<()> {
    if let Some(want) = net.genesis_sha256 {
        let got = sha256_hex(bytes);
        if got != want {
            bail!(
                "genesis sha256 mismatch for {}: got {got}, expected {want}",
                net.chain_id
            );
        }
    }
    let v: serde_json::Value = serde_json::from_slice(bytes).context("genesis is not JSON")?;
    let chain_id = v["chain_id"].as_str().unwrap_or_default();
    if chain_id != net.chain_id {
        bail!(
            "genesis chain_id {chain_id:?} does not match network {} ({})",
            net.name,
            net.chain_id
        );
    }
    Ok(())
}

/// Downloads the network's genesis to `dest` (first URL that yields a
/// verified file wins). Uses the system `curl` (present on macOS and
/// common Linux distributions), so the binary carries no TLS stack.
pub fn download_genesis(net: &Network, dest: &Path) -> anyhow::Result<()> {
    if net.genesis_urls.is_empty() {
        bail!("network {} has no genesis download URLs", net.name);
    }
    let mut errors = Vec::new();
    for url in net.genesis_urls {
        let tmp = dest.with_extension("download");
        let out = std::process::Command::new("curl")
            .args(["-fsSL", "--max-time", "120", "-o"])
            .arg(&tmp)
            .arg(url)
            .output();
        let res = match out {
            Ok(o) if o.status.success() => std::fs::read(&tmp)
                .map_err(anyhow::Error::from)
                .and_then(|bz| verify_genesis(net, &bz).map(|()| bz)),
            Ok(o) => Err(anyhow!(
                "curl: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            )),
            Err(e) => Err(anyhow!("run curl: {e}")),
        };
        let _ = std::fs::remove_file(&tmp);
        match res {
            Ok(bz) => {
                std::fs::write(dest, bz)?;
                return Ok(());
            }
            Err(e) => errors.push(format!("{url}: {e:#}")),
        }
    }
    bail!(
        "could not download the {} genesis:\n  {}",
        net.name,
        errors.join("\n  ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup() {
        assert_eq!(by_name("mocha").unwrap().chain_id, "mocha-5");
        assert_eq!(by_name("celestia").unwrap().name, "mainnet");
        assert!(by_name("nope").is_err());
        // Peers are well-formed `id@host:port`.
        for n in ALL {
            for p in n.seeds.iter().chain(n.persistent_peers) {
                let (id, addr) = p.split_once('@').unwrap();
                assert_eq!(id.len(), 40, "{p}");
                assert!(hex::decode(id).is_ok(), "{p}");
                assert!(addr.rsplit_once(':').unwrap().1.parse::<u16>().is_ok());
            }
        }
    }

    #[test]
    fn genesis_verification() {
        let mut net = LOCAL;
        let g = br#"{"chain_id":"celestia-rs-local"}"#;
        verify_genesis(&net, g).unwrap();
        net.genesis_sha256 = Some("00");
        assert!(verify_genesis(&net, g).is_err());
        assert!(verify_genesis(&MOCHA, br#"{"chain_id":"mocha-5"}"#).is_err());
    }
}
