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
    sdk::version::is_v9(app_version)
}

/// Whether module or store `name` exists in the state machine celestia-rs
/// runs for `app_version` ([`sdk::version::Release::executing`]). Names
/// that are not stores (most module names) always exist. App versions 1 to
/// 8 run as 9, so this is the v9 store set for them, not their mainnet one
/// ([`sdk::version::Release::stores`]).
#[must_use]
pub fn exists_at(name: &str, app_version: u64) -> bool {
    !IAVL_STORES.contains(&name) || sdk::version::Release::executing(app_version).has_store(name)
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

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::version::Release;
    use std::collections::BTreeSet;

    fn set(names: &[&'static str]) -> BTreeSet<&'static str> {
        names.iter().copied().collect()
    }

    /// The mounted stores, the v10 additions and the release table agree.
    #[test]
    fn store_lists_match_the_release_table() {
        let current = Release::executing(0);
        assert_eq!(set(current.stores), set(IAVL_STORES));
        let v9 = Release::executing(9);
        let added: BTreeSet<_> = set(current.stores)
            .difference(&set(v9.stores))
            .copied()
            .collect();
        assert_eq!(added, set(ADDED_IN_V10));
        for v in 10..=12 {
            assert_eq!(set(Release::executing(v).stores), set(IAVL_STORES), "v{v}");
        }
    }

    /// Before the release table, a name was missing only when a v9
    /// executable ran it and v10 added it. Every app version keeps that.
    #[test]
    fn exists_at_is_unchanged_for_every_app_version() {
        let names = IAVL_STORES
            .iter()
            .copied()
            .chain(["auth", "genutil", "qgb"]);
        for name in names {
            for v in 0..=13 {
                let before = !(v != 0 && v < 10 && ADDED_IN_V10.contains(&name));
                assert_eq!(exists_at(name, v), before, "{name} at v{v}");
            }
        }
    }
}
