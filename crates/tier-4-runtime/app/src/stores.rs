//! Store keys mounted by celestia-app v10 (`allStoreKeys`, `tkeys`, `memKeys`).

use sdk::Kind;

/// IAVL store names, as they appear in the commit info.
pub const IAVL_STORES: &[&str] = &[
    "acc",
    "authz",
    "bank",
    "staking",
    "mint",
    "distribution",
    "slashing",
    "gov",
    "params",
    "upgrade",
    "feegrant",
    "evidence",
    "capability",
    "transfer",
    "ibc",
    "packetfowardmiddleware",
    "icahost",
    "signal",
    "blob",
    "minfee",
    "consensus",
    "circuit",
    "hyperlane",
    "warp",
    "zkism",
    "valaddr",
    "fibre",
];

/// Transient store of `x/params`.
pub const TRANSIENT_PARAMS: &str = "transient_params";
/// Memory store of `x/capability`.
pub const MEMORY_CAPABILITY: &str = "memory:capability";

/// All stores with their kinds.
pub fn all() -> impl Iterator<Item = (&'static str, Kind)> {
    IAVL_STORES.iter().map(|s| (*s, Kind::Iavl)).chain([
        (TRANSIENT_PARAMS, Kind::Transient),
        (MEMORY_CAPABILITY, Kind::Memory),
    ])
}
