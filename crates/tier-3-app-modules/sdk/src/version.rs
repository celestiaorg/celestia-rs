//! The app version of the state being executed, for code that Go runs
//! without a context (e.g. `ValidateBasic`) but whose behavior depends on
//! the celestia-app release that executes the block (ADR 0004: app version
//! 9 is celestia-app v9.0.8).
//!
//! The app sets it on the executing thread at every ABCI entry point
//! ([`set_app_version`]). Usually it equals the committed consensus app
//! version. A governance/cosmovisor upgrade can run the v10 executable while
//! that committed value remains 9, so context-aware compiled behavior must
//! use [`executing`] rather than assuming the two identities are equal.

use std::cell::Cell;

std::thread_local! {
    static APP_VERSION: Cell<u64> = const { Cell::new(0) };
}

/// Sets the app version of the calling thread's current ABCI call.
pub fn set_app_version(v: u64) {
    APP_VERSION.with(|c| c.set(v));
}

/// The app version set by [`set_app_version`] (0 when unset).
#[must_use]
pub fn app_version() -> u64 {
    APP_VERSION.with(Cell::get)
}

/// The executing celestia-app release, falling back to a context's committed
/// app version in isolated module tests where the app did not set one.
#[must_use]
pub fn executing(context_version: u64) -> u64 {
    let current = app_version();
    if current == 0 {
        context_version
    } else {
        current
    }
}

/// The cosmos-sdk line a celestia-app release is built on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdkLine {
    /// cosmos-sdk v0.46 (celestia-app v1 to v3, ABCI 1.0).
    V046,
    /// cosmos-sdk v0.50 and its forks (celestia-app v4 and later, ABCI 2.0).
    V050,
}

/// What one app version means on mainnet: the celestia-app release that
/// defines it and the facts that select per-version behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Release {
    /// The app version (`appconsts.Version`).
    pub app_version: u64,
    /// The cosmos-sdk line of the release.
    pub sdk_line: SdkLine,
    /// IAVL stores in the commit info of a block executed at this version.
    pub stores: &'static [&'static str],
    /// Non-IAVL stores that the commit info also lists, with an empty hash
    /// (cosmos-sdk v0.46 commits memory stores; v0.50 does not).
    pub commit_memory_stores: &'static [&'static str],
    /// Upgrade plan names the release's binary registers a handler for.
    pub upgrade_handlers: &'static [&'static str],
    /// The app version whose state machine celestia-rs runs for this one.
    /// Equal to `app_version` once that version is ported; app versions
    /// 1 to 8 run as 9 until historical execution exists.
    pub runs_as: u64,
}

/// celestia-app v1 (v3.13.0 `versionedStoreKeys(1)` plus the base
/// `params` key).
const V1_STORES: &[&str] = &[
    "acc",
    "authz",
    "bank",
    "qgb",
    "blob",
    "capability",
    "distribution",
    "evidence",
    "feegrant",
    "gov",
    "ibc",
    "transfer",
    "mint",
    "params",
    "slashing",
    "staking",
    "upgrade",
];

/// celestia-app v2 and v3: v1 without `qgb`, with `icahost`, `signal` and
/// packet forward middleware.
const V2_STORES: &[&str] = &[
    "acc",
    "authz",
    "bank",
    "blob",
    "capability",
    "distribution",
    "evidence",
    "feegrant",
    "gov",
    "ibc",
    "transfer",
    "mint",
    "params",
    "slashing",
    "staking",
    "upgrade",
    "icahost",
    "packetfowardmiddleware",
    "signal",
];

/// celestia-app v4 to v6: v3 plus the stores the v4 store loader adds.
const V4_STORES: &[&str] = &[
    "acc",
    "authz",
    "bank",
    "blob",
    "capability",
    "distribution",
    "evidence",
    "feegrant",
    "gov",
    "ibc",
    "transfer",
    "mint",
    "params",
    "slashing",
    "staking",
    "upgrade",
    "icahost",
    "packetfowardmiddleware",
    "signal",
    "circuit",
    "consensus",
    "hyperlane",
    "warp",
    "minfee",
];

/// celestia-app v7 to v9: v6 plus `zkism`.
const V8_STORES: &[&str] = &[
    "acc",
    "authz",
    "bank",
    "blob",
    "capability",
    "distribution",
    "evidence",
    "feegrant",
    "gov",
    "ibc",
    "transfer",
    "mint",
    "params",
    "slashing",
    "staking",
    "upgrade",
    "icahost",
    "packetfowardmiddleware",
    "signal",
    "circuit",
    "consensus",
    "hyperlane",
    "warp",
    "minfee",
    "zkism",
];

/// celestia-app v10 and later: v9 plus `fibre` and `valaddr`.
const V10_STORES: &[&str] = &[
    "acc",
    "authz",
    "bank",
    "blob",
    "capability",
    "distribution",
    "evidence",
    "feegrant",
    "gov",
    "ibc",
    "transfer",
    "mint",
    "params",
    "slashing",
    "staking",
    "upgrade",
    "icahost",
    "packetfowardmiddleware",
    "signal",
    "circuit",
    "consensus",
    "hyperlane",
    "warp",
    "minfee",
    "zkism",
    "fibre",
    "valaddr",
];

/// The memory store cosmos-sdk v0.46 lists in the commit info.
const V046_MEMORY_STORES: &[&str] = &["mem_capability"];

const fn release(
    app_version: u64,
    sdk_line: SdkLine,
    stores: &'static [&'static str],
    upgrade_handlers: &'static [&'static str],
) -> Release {
    let commit_memory_stores: &[&str] = match sdk_line {
        SdkLine::V046 => V046_MEMORY_STORES,
        SdkLine::V050 => &[],
    };
    Release {
        app_version,
        sdk_line,
        stores,
        commit_memory_stores,
        upgrade_handlers,
        // Historical execution (app versions 1 to 8) is not ported yet.
        runs_as: if app_version < 9 { 9 } else { app_version },
    }
}

/// Mainnet app versions 1 to 12. Executing binaries: celestia-app v3.13.0
/// (1 to 3), v4.1.0, v5.0.12, v6.4.10, v7 (never ran on mainnet), v8.0.8,
/// v9.0.8, then v10, v11 and v12.
const RELEASES: [Release; 12] = [
    release(1, SdkLine::V046, V1_STORES, &[]),
    release(2, SdkLine::V046, V2_STORES, &[]),
    release(3, SdkLine::V046, V2_STORES, &[]),
    release(4, SdkLine::V050, V4_STORES, &["v4"]),
    release(5, SdkLine::V050, V4_STORES, &["v4", "v5"]),
    release(6, SdkLine::V050, V4_STORES, &["v6"]),
    release(7, SdkLine::V050, V8_STORES, &["v7"]),
    release(8, SdkLine::V050, V8_STORES, &["v8"]),
    release(9, SdkLine::V050, V8_STORES, &["v9"]),
    release(10, SdkLine::V050, V10_STORES, &["v10"]),
    release(11, SdkLine::V050, V10_STORES, &["v11"]),
    release(12, SdkLine::V050, V10_STORES, &["v12"]),
];

/// The release that runs when no app version is set (0): app version 10.
pub const CURRENT_APP_VERSION: u64 = 10;

const CURRENT_RELEASE: &Release = &RELEASES[9];

impl Release {
    /// The release that defines `v`, or `None` outside `1..=12`.
    #[must_use]
    pub const fn for_app_version(v: u64) -> Option<&'static Self> {
        let mut i = 0;
        while i < RELEASES.len() {
            if RELEASES[i].app_version == v {
                return Some(&RELEASES[i]);
            }
            i += 1;
        }
        None
    }

    /// The release whose state machine celestia-rs runs for app version
    /// `v`. Unset (0) and versions past the table run as the current
    /// release, app version 10.
    #[must_use]
    pub const fn executing(v: u64) -> &'static Self {
        let runs_as = match Self::for_app_version(v) {
            Some(r) => r.runs_as,
            None => CURRENT_APP_VERSION,
        };
        match Self::for_app_version(runs_as) {
            Some(r) => r,
            None => CURRENT_RELEASE,
        }
    }

    /// Whether the release mounts store `name`.
    #[must_use]
    pub fn has_store(&self, name: &str) -> bool {
        self.stores.contains(&name)
    }
}

/// Whether `v` is executed with celestia-app v9.0.8 semantics (0 = unset
/// is current).
#[must_use]
pub const fn is_v9(v: u64) -> bool {
    Release::executing(v).app_version == 9
}

/// First app version for the mainline consensus changes excluded from the
/// v11 patch release (v11.0.0-corto / v11.0.0-mocha, issue #385).
/// The frozen-account rules are specific to the v11 patch executable.
pub const MAINLINE_RULES_FROM: u64 = 12;

/// Whether `v` is executed with celestia-app v12 semantics (0 = unset is
/// the current release, v10).
#[must_use]
pub const fn is_v12(v: u64) -> bool {
    v >= MAINLINE_RULES_FROM
}

/// The `%v` frame `cosmossdk.io/errors` appends to an error wrapped at
/// `file_line` (`x/staking/types/authz.go:193`) of the executing
/// celestia-app's cosmos-sdk: ` [celestiaorg/cosmos-sdk@v0.52.12/...]`.
#[must_use]
pub fn cosmos_sdk_frame(file_line: &str) -> String {
    let v = if is_v9(app_version()) {
        "v0.52.8"
    } else {
        "v0.52.12"
    };
    format!(" [celestiaorg/cosmos-sdk@{v}/{file_line}]")
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: &[&str] = &[
        "acc",
        "authz",
        "bank",
        "qgb",
        "blob",
        "capability",
        "distribution",
        "evidence",
        "feegrant",
        "gov",
        "ibc",
        "transfer",
        "mint",
        "params",
        "slashing",
        "staking",
        "upgrade",
    ];

    fn sorted<'a>(names: &[&'a str]) -> Vec<&'a str> {
        let mut v = names.to_vec();
        v.sort_unstable();
        v
    }

    fn with(base: &[&'static str], add: &[&'static str], remove: &[&str]) -> Vec<&'static str> {
        let mut v: Vec<&str> = base
            .iter()
            .copied()
            .filter(|s| !remove.contains(s))
            .collect();
        v.extend_from_slice(add);
        v
    }

    /// Mainnet commit info per app version (celestia-app v3.13.0
    /// `versionedStoreKeys`, v4.1.0 and v8.0.8 store loaders, v10 adds
    /// fibre and valaddr).
    #[test]
    fn releases_record_the_mainnet_sdk_line_and_store_set() {
        let v2 = with(
            V1,
            &["icahost", "packetfowardmiddleware", "signal"],
            &["qgb"],
        );
        let v4 = with(
            &v2,
            &["circuit", "consensus", "hyperlane", "warp", "minfee"],
            &[],
        );
        let v8 = with(&v4, &["zkism"], &[]);
        let v10 = with(&v8, &["fibre", "valaddr"], &[]);
        for v in 1..=12 {
            let r = Release::for_app_version(v).expect("known app version");
            assert_eq!(r.app_version, v);
            let (line, stores, memory): (_, &[&str], &[&str]) = match v {
                1 => (SdkLine::V046, V1, &["mem_capability"]),
                2 | 3 => (SdkLine::V046, &v2, &["mem_capability"]),
                4..=6 => (SdkLine::V050, &v4, &[]),
                7..=9 => (SdkLine::V050, &v8, &[]),
                _ => (SdkLine::V050, &v10, &[]),
            };
            assert_eq!(r.sdk_line, line, "sdk line of v{v}");
            assert_eq!(sorted(r.stores), sorted(stores), "stores of v{v}");
            assert_eq!(r.commit_memory_stores, memory, "memory stores of v{v}");
        }
        assert_eq!(Release::for_app_version(0), None);
        assert_eq!(Release::for_app_version(13), None);
    }

    /// Plan names each Go binary registers (`app/upgrades.go`): v3.13.0
    /// none (v1 -> v2 and v2 -> v3 are not x/upgrade plans), v5.0.12 both
    /// `v4` and `v5`, the others `v{appconsts.Version}`.
    #[test]
    fn releases_record_their_upgrade_handlers() {
        let names = |v| Release::for_app_version(v).map(|r| r.upgrade_handlers);
        for v in 1..=3 {
            assert_eq!(names(v), Some(&[][..]));
        }
        assert_eq!(names(4), Some(&["v4"][..]));
        assert_eq!(names(5), Some(&["v4", "v5"][..]));
        for (v, name) in [
            (6, "v6"),
            (7, "v7"),
            (8, "v8"),
            (9, "v9"),
            (10, "v10"),
            (11, "v11"),
            (12, "v12"),
        ] {
            assert_eq!(names(v), Some(&[name][..]));
        }
    }

    /// Nothing below app version 9 is executed with its own rules yet:
    /// those versions keep running as v9 (no behavior change).
    #[test]
    fn historical_versions_run_as_v9() {
        for v in 1..=12 {
            let r = Release::for_app_version(v).expect("known app version");
            assert_eq!(r.runs_as, v.max(9), "v{v}");
            assert_eq!(is_v9(v), r.runs_as == 9, "v{v}");
        }
        assert!(!is_v9(0));
        assert!(!is_v9(13));
        assert_eq!(CURRENT_RELEASE.app_version, CURRENT_APP_VERSION);
        for v in [0, 13, u64::MAX] {
            assert_eq!(Release::executing(v), CURRENT_RELEASE, "v{v}");
        }
    }
}
