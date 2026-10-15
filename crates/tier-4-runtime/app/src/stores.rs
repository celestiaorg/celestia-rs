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

/// Stores (and modules) celestia-app v10 added: absent from app version 9
/// (celestia-app v9.0.8 without the `fibre` build tag), mounted by the v10
/// upgrade's store loader (`StoreUpgrades.Added`).
pub const ADDED_IN_V10: &[&str] = &["fibre", "valaddr"];

/// Whether `app_version` is executed with celestia-app v9.0.8 semantics
/// (ADR 0004). App version 0 (unset) is treated as current.
#[must_use]
pub const fn is_v9(app_version: u64) -> bool {
    app_version != 0 && app_version < 10
}

/// Whether module or store `name` exists at `app_version`.
#[must_use]
pub fn exists_at(name: &str, app_version: u64) -> bool {
    !(is_v9(app_version) && ADDED_IN_V10.contains(&name))
}

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
