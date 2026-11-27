//! Networks the census crawls, with the seeds and peers of
//! `celestia-rs/src/networks.rs` (kept in sync by hand; any reachable node
//! of the network works as a starting point).

/// A crawlable network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Network {
    /// Name used in file names and the dashboard.
    pub name: &'static str,
    /// Chain ID.
    pub chain_id: &'static str,
    /// Starting addresses (`id@host:port`).
    pub seeds: &'static [&'static str],
    /// Public CometBFT RPC servers whose `/net_info` lists their peers.
    pub rpcs: &'static [&'static str],
}

/// Mainnet beta.
pub const MAINNET: Network = Network {
    name: "mainnet",
    chain_id: "celestia",
    seeds: &[
        "acca7837e4eb5f9dc7f5a94ed1d82edda6931ff8@seed.celestia.pops.one:26656",
        "12ad7c73c7e1f2460941326937a039139aa78884@celestia-mainnet-seed.itrocket.net:40656",
        "975237ec35350f0643bce6618654cd1c76bf1dc0@consensus-full-seed-1.celestia-bootstrap.net:26656",
        "d9bfa29e0cf9c4ce0cc9c26d98e5d97228f93b0b@65.109.88.38:12056",
        "fc25a389790d9f1e8dc39496313e1e0fe2b138f7@135.181.138.95:2890",
        "d513f98bb4f8859f94bde2efc6bf18d29f732c48@188.40.102.137:15656",
        "ce99d7da2530f75d05880d13d9eda384d5a1afe4@peer.celestia.node75.org:23356",
    ],
    rpcs: &[
        "https://celestia-rpc.polkachu.com",
        "https://celestia-rpc.publicnode.com",
        "https://rpc.celestia.pops.one",
        "https://celestia-mainnet-rpc.itrocket.net",
        "https://celestia.rpc.kjnodes.com",
    ],
};

/// Mocha testnet.
pub const MOCHA: Network = Network {
    name: "mocha",
    chain_id: "mocha-5",
    seeds: &[
        "ee9f90974f85c59d3861fc7f7edb10894f6ac3c8@84.32.215.148:26656",
        "b402fe40f3474e9e208840702e1b7aa37f2edc4b@celestia-testnet-seed.itrocket.net:14656",
        "daf2cecee2bd7f1b3bf94839f993f807c6b15fbf@celestia-testnet-peer.itrocket.net:11656",
    ],
    rpcs: &[
        "https://rpc-mocha.pops.one",
        "https://celestia-testnet-rpc.itrocket.net",
        "https://celestia-testnet-rpc.polkachu.com",
    ],
};

/// Every crawlable network.
pub const ALL: [Network; 2] = [MAINNET, MOCHA];

/// Looks up a network by name or chain ID.
#[must_use]
pub fn by_name(name: &str) -> Option<Network> {
    ALL.into_iter()
        .find(|n| n.name == name || n.chain_id == name)
}
