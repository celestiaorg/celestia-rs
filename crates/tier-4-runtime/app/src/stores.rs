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

/// Whether module or store `name` exists in the state machine celestia-rs
/// runs for `app_version` ([`sdk::version::Release::executing`]). Names
/// that are not stores (most module names) always exist. App versions 2,
/// 3 and 5 to 8 run as 9, so this is the v9 store set for them, not their
/// mainnet one ([`sdk::version::Release::stores`]); app version 1 runs as
/// itself (its own 17 stores, on the v0.46 driver) and app version 4 too
/// (no `zkism`).
#[must_use]
pub fn exists_at(name: &str, app_version: u64) -> bool {
    !IAVL_STORES.contains(&name) || sdk::version::Release::executing(app_version).has_store(name)
}

/// Mounts the stores of `release` (its own commit info, unlike
/// [`all`]): its IAVL stores, its cosmos-sdk v0.46 memory store entries,
/// and the transient and memory stores modules use. For historical
/// execution of a release as itself; the current app mounts [`all`].
pub fn mount_release(
    ms: &mut sdk::MultiStore,
    release: &sdk::version::Release,
) -> Result<(), sdk::StoreError> {
    for name in release.stores {
        ms.mount(name, Kind::Iavl)?;
    }
    for name in release.commit_memory_stores {
        ms.mount_commit_memory(name)?;
    }
    ms.mount(TRANSIENT_PARAMS, Kind::Transient)?;
    ms.mount(MEMORY_CAPABILITY, Kind::Memory)
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

    /// A release's stores mount as its commit info: app version 1 lists
    /// the 17 v1 IAVL stores (with `params` and `qgb`) and
    /// `mem_capability` with the empty commit ID; v0.50 releases list no
    /// memory store.
    #[test]
    fn mount_release_lists_the_release_commit_info() {
        for v in 1..=12 {
            let release = Release::for_app_version(v).expect("known app version");
            let mut ms = sdk::MultiStore::new();
            mount_release(&mut ms, release).expect("mount");
            ms.commit().expect("commit");
            let info = ms.root().last_commit_info().expect("commit info");
            let mut expected: Vec<&str> = release
                .stores
                .iter()
                .chain(release.commit_memory_stores)
                .copied()
                .collect();
            expected.sort_unstable();
            let names: Vec<&str> = info.store_infos.iter().map(|s| s.name.as_str()).collect();
            assert_eq!(names, expected, "v{v}");
            for s in &info.store_infos {
                let memory = release.commit_memory_stores.contains(&s.name.as_str());
                assert_eq!(s.commit_id.is_none(), memory, "{} at v{v}", s.name);
            }
            assert!(ms.is_mounted(TRANSIENT_PARAMS) && ms.is_mounted(MEMORY_CAPABILITY));
        }
        let v1 = Release::for_app_version(1).expect("v1");
        assert_eq!(v1.stores.len() + v1.commit_memory_stores.len(), 18);
        assert!(v1.has_store("qgb") && v1.has_store("params"));
    }

    /// Before the release table, a name was missing only when a v9
    /// executable ran it and v10 added it. Every app version but 1 and 4
    /// keeps that; app versions 1, 4 and 5 run as themselves, with their own
    /// store sets (v4 and v5 have no `zkism`).
    #[test]
    fn exists_at_is_unchanged_for_every_app_version() {
        let names = IAVL_STORES
            .iter()
            .copied()
            .chain(["auth", "genutil", "qgb"]);
        for name in names {
            for own in [1, 4, 5] {
                let r = Release::for_app_version(own).expect("known app version");
                assert_eq!(
                    exists_at(name, own),
                    !IAVL_STORES.contains(&name) || r.has_store(name),
                    "{name} at v{own}"
                );
            }
            for v in (0..=13).filter(|v| !matches!(v, 1 | 4 | 5)) {
                let before = !(v != 0 && v < 10 && ADDED_IN_V10.contains(&name));
                assert_eq!(exists_at(name, v), before, "{name} at v{v}");
            }
        }
    }
}
