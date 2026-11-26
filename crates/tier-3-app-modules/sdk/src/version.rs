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
    static SDK_V046: Cell<bool> = const { Cell::new(false) };
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

/// Whether cosmos-sdk v0.46 semantics are active on the calling thread:
/// an [`SdkV046Scope`] is alive.
#[must_use]
pub fn sdk_v046_active() -> bool {
    SDK_V046.with(Cell::get)
}

/// Whether shared modules take their cosmos-sdk v0.46 (celestia-app v1 to
/// v3) paths: parameters in `x/params` subspaces and v0.46 state layouts.
///
/// True only inside an [`SdkV046Scope`] (which the legacy block driver
/// holds while it executes a v0.46 block) whose executing app version
/// ([`executing`]) is 1 to 3. The app version alone never selects these
/// paths: the main app runs app versions 1 to 8 as v9.
#[must_use]
pub fn is_sdk_v046(context_version: u64) -> bool {
    sdk_v046_active() && matches!(executing(context_version), 1..=3)
}

/// Turns cosmos-sdk v0.46 semantics on for the calling thread at an app
/// version until dropped, then restores the thread's previous app
/// version and v0.46 state (also when unwinding). Only the legacy block
/// driver (and tests of its modules) enters one: no block or genesis
/// content can.
#[derive(Debug)]
#[must_use = "v0.46 semantics end when the scope is dropped"]
pub struct SdkV046Scope {
    prev_app_version: u64,
    prev_active: bool,
    // The thread-local state is restored on the thread that set it.
    _not_send: std::marker::PhantomData<*const ()>,
}

impl SdkV046Scope {
    /// Sets the thread's app version to `v` and turns v0.46 semantics on.
    pub fn enter(v: u64) -> Self {
        let scope = Self {
            prev_app_version: app_version(),
            prev_active: sdk_v046_active(),
            _not_send: std::marker::PhantomData,
        };
        set_app_version(v);
        SDK_V046.with(|c| c.set(true));
        scope
    }
}

impl Drop for SdkV046Scope {
    fn drop(&mut self) {
        set_app_version(self.prev_app_version);
        SDK_V046.with(|c| c.set(self.prev_active));
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
    /// The cosmos-sdk the release's `go.mod` replaces cosmos-sdk with.
    pub cosmos_sdk: CosmosSdk,
    /// The hyperlane-cosmos the release builds on (`None` before v4).
    pub hyperlane: Option<HyperlaneCosmos>,
    /// The go-square whose square layout the release constructs.
    pub go_square: GoSquare,
    /// The app version whose state machine celestia-rs runs for this one.
    /// Equal to `app_version` once that version is ported: app version 1
    /// runs as itself on the cosmos-sdk v0.46 driver; app versions 2 to 8
    /// run as 9 until their historical execution exists.
    pub runs_as: u64,
}

/// The cosmos-sdk a celestia-app release builds on (its `go.mod`
/// `replace`), oldest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CosmosSdk {
    /// celestiaorg/cosmos-sdk v1.31.2-sdk-v0.46.16 (celestia-app v3.13.0).
    V0_46,
    /// celestiaorg/cosmos-sdk v1.29.4-sdk-v0.50.14 (celestia-app v4.1.0).
    V0_50Celestia1_29,
    /// celestiaorg/cosmos-sdk v1.31.0-sdk-v0.50.14 (celestia-app v5.0.12).
    V0_50Celestia1_31,
    /// celestiaorg/cosmos-sdk v0.51.8 (celestia-app v6.4.10).
    V0_51_8,
    /// celestiaorg/cosmos-sdk v0.52.1 (celestia-app v7.0.3).
    V0_52_1,
    /// celestiaorg/cosmos-sdk v0.52.6 (celestia-app v8.0.8).
    V0_52_6,
    /// celestiaorg/cosmos-sdk v0.52.8 (celestia-app v9.0.8).
    V0_52_8,
    /// celestiaorg/cosmos-sdk v0.52.12 (celestia-app v10 and later).
    V0_52_12,
}

impl CosmosSdk {
    /// The celestiaorg/cosmos-sdk module version (the `@version` in Go's
    /// module cache paths).
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::V0_46 => "v1.31.2-sdk-v0.46.16",
            Self::V0_50Celestia1_29 => "v1.29.4-sdk-v0.50.14",
            Self::V0_50Celestia1_31 => "v1.31.0-sdk-v0.50.14",
            Self::V0_51_8 => "v0.51.8",
            Self::V0_52_1 => "v0.52.1",
            Self::V0_52_6 => "v0.52.6",
            Self::V0_52_8 => "v0.52.8",
            Self::V0_52_12 => "v0.52.12",
        }
    }
}

/// The hyperlane-cosmos a celestia-app release builds on, oldest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HyperlaneCosmos {
    /// bcp-innovations v1.0.1 (celestia-app v4 to v6).
    V1_0_1,
    /// bcp-innovations v1.1.0 (celestia-app v7 to v9).
    V1_1_0,
    /// Celestia's v1.3 line (celestia-app v10 and later).
    CelestiaV1_3,
}

/// The go-square whose square layout a celestia-app release constructs,
/// oldest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GoSquare {
    /// go-square v1 (celestia-app v3.13.0 at app versions 1 and 2).
    V1,
    /// go-square v2 (app versions 3 to 5).
    V2,
    /// go-square v3 (app versions 6 to 8).
    V3,
    /// go-square v4.0.0-rc4 (app version 9).
    V4Rc4,
    /// go-square v4 (app version 10 and later).
    V4,
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

/// The dependencies of a release: cosmos-sdk, hyperlane-cosmos, go-square.
type Deps = (CosmosSdk, Option<HyperlaneCosmos>, GoSquare);

const fn entry(
    app_version: u64,
    sdk_line: SdkLine,
    stores: &'static [&'static str],
    upgrade_handlers: &'static [&'static str],
    (cosmos_sdk, hyperlane, go_square): Deps,
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
        cosmos_sdk,
        hyperlane,
        go_square,
        // App version 1 executes as itself (the cosmos-sdk v0.46 driver)
        // and app version 4 as celestia-app v4.1.0; historical execution
        // of app versions 2, 3 and 5 to 8 is not ported yet.
        runs_as: if matches!(app_version, 2 | 3 | 5..=8) {
            9
        } else {
            app_version
        },
    }
}

/// Mainnet app versions 1 to 12. Executing binaries: celestia-app v3.13.0
/// (1 to 3), v4.1.0, v5.0.12, v6.4.10, v7 (never ran on mainnet), v8.0.8,
/// v9.0.8, then v10, v11 and v12.
const RELEASES: [Release; 12] = {
    use CosmosSdk as Sdk;
    use GoSquare as Sq;
    use HyperlaneCosmos as Hl;
    use SdkLine::{V046, V050};
    [
        entry(1, V046, V1_STORES, &[], (Sdk::V0_46, None, Sq::V1)),
        entry(2, V046, V2_STORES, &[], (Sdk::V0_46, None, Sq::V1)),
        entry(3, V046, V2_STORES, &[], (Sdk::V0_46, None, Sq::V2)),
        entry(
            4,
            V050,
            V4_STORES,
            &["v4"],
            (Sdk::V0_50Celestia1_29, Some(Hl::V1_0_1), Sq::V2),
        ),
        entry(
            5,
            V050,
            V4_STORES,
            &["v4", "v5"],
            (Sdk::V0_50Celestia1_31, Some(Hl::V1_0_1), Sq::V2),
        ),
        entry(
            6,
            V050,
            V4_STORES,
            &["v6"],
            (Sdk::V0_51_8, Some(Hl::V1_0_1), Sq::V3),
        ),
        entry(
            7,
            V050,
            V8_STORES,
            &["v7"],
            (Sdk::V0_52_1, Some(Hl::V1_1_0), Sq::V3),
        ),
        entry(
            8,
            V050,
            V8_STORES,
            &["v8"],
            (Sdk::V0_52_6, Some(Hl::V1_1_0), Sq::V3),
        ),
        entry(
            9,
            V050,
            V8_STORES,
            &["v9"],
            (Sdk::V0_52_8, Some(Hl::V1_1_0), Sq::V4Rc4),
        ),
        entry(
            10,
            V050,
            V10_STORES,
            &["v10"],
            (Sdk::V0_52_12, Some(Hl::CelestiaV1_3), Sq::V4),
        ),
        entry(
            11,
            V050,
            V10_STORES,
            &["v11"],
            (Sdk::V0_52_12, Some(Hl::CelestiaV1_3), Sq::V4),
        ),
        entry(
            12,
            V050,
            V10_STORES,
            &["v12"],
            (Sdk::V0_52_12, Some(Hl::CelestiaV1_3), Sq::V4),
        ),
    ]
};

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

    // Feature predicates. Shared code asks the executing release
    // ([`release`], [`Release::executing`]) one of these instead of
    // comparing app version numbers. Each answer holds for every app
    // version the release runs: an app version that runs as v9 takes
    // v9's answers until it runs as itself.
    //
    // App version 1 runs on the cosmos-sdk v0.46 driver, which selects its
    // own paths with [`is_sdk_v046`]. Elsewhere it keeps the v9 answers
    // that the Go v1 oracle was matched against ([`Self::keeps_v9_rules`]).

    /// App version 1 takes v9's answers outside its cosmos-sdk v0.46 paths.
    const fn keeps_v9_rules(&self) -> bool {
        self.app_version == 1
    }

    /// Upstream hyperlane-cosmos (bcp-innovations v1.0.1 or v1.1.0), not
    /// Celestia's v1.3 line: x/core and x/warp keep the upstream
    /// validation, gas, payment and amino names. True before v4 (no
    /// hyperlane modules). v1.0.1 and v1.1.0 differ only in
    /// `proto_init.go` (proto registration order).
    #[must_use]
    pub const fn upstream_hyperlane(&self) -> bool {
        !matches!(self.hyperlane, Some(HyperlaneCosmos::CelestiaV1_3))
    }

    /// cosmos-sdk before celestiaorg v0.52.12, with `cosmossdk.io/x/evidence`
    /// v0.1.1: x/consensus allows a `block.MaxGas` change, `addDecCoins`
    /// does not recover an overflow, evidence for an unknown validator
    /// fails BeginBlock, and FinalizeBlock delivers each tx in turn.
    #[must_use]
    pub const fn sdk_before_v0_52_12(&self) -> bool {
        !matches!(self.cosmos_sdk, CosmosSdk::V0_52_12)
    }

    /// The release registers the fibre and valaddr modules (celestia-app
    /// v10): their stores, module accounts, messages, ante decorators and
    /// query services.
    #[must_use]
    pub fn has_fibre(&self) -> bool {
        self.has_store("fibre")
    }

    /// Transactions decode, check and enter proposals with celestia-app
    /// v9.0.8's interface registry, gogoproto schema and tx pipeline
    /// (`txdecode::decode_v9`, the v9 CheckTx and proposal paths). App
    /// versions 4 to 8 take this family: it is the nearest port of their
    /// own registries, which are not ported.
    #[must_use]
    pub const fn v9_registry(&self) -> bool {
        self.sdk_before_v0_52_12()
    }

    /// The release predates celestia-app v10: a v10 change that is not one
    /// of the dependency changes above (x/forwarding, x/zkism, the ante fee
    /// denom check, signal and upgrade switching). x/forwarding is
    /// identical in v8.0.8 and v9.0.8; x/zkism differs in vkey validation
    /// details only.
    #[must_use]
    pub const fn before_v10(&self) -> bool {
        self.app_version < 10
    }

    /// go-square v4.0.0-rc4: the square layout counts signer bytes and
    /// classifies fibre transactions (app version 9 only).
    #[must_use]
    pub const fn go_square_v4_rc4(&self) -> bool {
        matches!(self.go_square, GoSquare::V4Rc4) || self.keeps_v9_rules()
    }

    /// celestia-app v9.0.8 `hasDuplicateTxRawField`: CheckTx and
    /// PrepareProposal reject a `TxRaw` with a repeated top-level field
    /// (app version 9 only; v10 decodes strictly instead).
    #[must_use]
    pub const fn rejects_duplicate_tx_raw_fields(&self) -> bool {
        self.app_version == 9 || self.keeps_v9_rules()
    }
}

/// The release celestia-rs runs for the calling thread's executing app
/// version ([`executing`]), falling back to `context_version` where the app
/// set none.
#[must_use]
pub fn release(context_version: u64) -> &'static Release {
    Release::executing(executing(context_version))
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
    let r = release(0);
    let v = if r.keeps_v9_rules() {
        CosmosSdk::V0_52_8.tag()
    } else {
        r.cosmos_sdk.tag()
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

    /// The main app sets the raw app version: that
    /// alone never selects cosmos-sdk v0.46 semantics.
    #[test]
    fn raw_app_versions_1_to_3_are_not_sdk_v046() {
        let prev = app_version();
        for v in 0..=13 {
            set_app_version(v);
            assert!(!is_sdk_v046(v), "v{v}");
            assert!(!is_sdk_v046(0), "v{v}");
        }
        set_app_version(prev);
    }

    /// An [`SdkV046Scope`] selects v0.46 semantics at app versions 1 to 3
    /// only, nests, and restores the previous thread state when dropped.
    #[test]
    fn sdk_v046_scope_is_explicit_and_restores() {
        set_app_version(9);
        {
            let _outer = SdkV046Scope::enter(2);
            assert!(sdk_v046_active());
            assert_eq!(app_version(), 2);
            assert!(is_sdk_v046(0) && is_sdk_v046(9));
            {
                let _inner = SdkV046Scope::enter(4);
                assert!(sdk_v046_active() && !is_sdk_v046(1));
            }
            assert_eq!(app_version(), 2);
            assert!(is_sdk_v046(0));
        }
        assert_eq!(app_version(), 9);
        assert!(!sdk_v046_active() && !is_sdk_v046(1));
        let unwound = std::panic::catch_unwind(|| {
            let _scope = SdkV046Scope::enter(1);
            panic!("unwind");
        });
        assert!(unwound.is_err());
        assert_eq!(app_version(), 9);
        assert!(!sdk_v046_active());
        set_app_version(0);
    }

    /// The executing release answers every feature predicate as the
    /// former `is_v9(v)` split (`1..=9`) did: refactoring the call sites
    /// onto the predicates changes no behavior while app versions 2 to 8
    /// run as 9.
    #[test]
    fn executing_release_predicates_keep_the_v9_split() {
        for v in (0..=14).chain([u64::MAX]) {
            let old_v9 = matches!(v, 1..=9);
            let r = Release::executing(v);
            assert_eq!(r.upstream_hyperlane(), old_v9, "v{v}");
            assert_eq!(r.sdk_before_v0_52_12(), old_v9, "v{v}");
            assert_eq!(r.has_fibre(), !old_v9, "v{v}");
            assert_eq!(r.v9_registry(), old_v9, "v{v}");
            assert_eq!(r.before_v10(), old_v9, "v{v}");
            assert_eq!(r.go_square_v4_rc4(), old_v9, "v{v}");
            assert_eq!(r.rejects_duplicate_tx_raw_fields(), old_v9, "v{v}");
        }
    }

    /// The per-release answers for app versions 2 to 8 (they take effect
    /// once a version runs as itself): v4 to v8 share the pre-v10
    /// dependencies and registry family with v9, but not go-square
    /// v4.0.0-rc4 nor v9's duplicate `TxRaw` field rejection.
    #[test]
    fn historical_releases_record_their_own_features() {
        use HyperlaneCosmos as Hl;
        for v in 2..=8 {
            let r = Release::for_app_version(v).expect("known app version");
            assert!(r.upstream_hyperlane(), "v{v}");
            assert!(r.sdk_before_v0_52_12(), "v{v}");
            assert!(!r.has_fibre(), "v{v}");
            assert!(r.v9_registry(), "v{v}");
            assert!(r.before_v10(), "v{v}");
            assert!(!r.go_square_v4_rc4(), "v{v}");
            assert!(!r.rejects_duplicate_tx_raw_fields(), "v{v}");
        }
        let deps = |v| {
            let r = Release::for_app_version(v).expect("known app version");
            (r.cosmos_sdk.tag(), r.hyperlane, r.go_square)
        };
        assert_eq!(deps(3), ("v1.31.2-sdk-v0.46.16", None, GoSquare::V2));
        assert_eq!(
            deps(4),
            ("v1.29.4-sdk-v0.50.14", Some(Hl::V1_0_1), GoSquare::V2)
        );
        assert_eq!(
            deps(5),
            ("v1.31.0-sdk-v0.50.14", Some(Hl::V1_0_1), GoSquare::V2)
        );
        assert_eq!(deps(6), ("v0.51.8", Some(Hl::V1_0_1), GoSquare::V3));
        assert_eq!(deps(7), ("v0.52.1", Some(Hl::V1_1_0), GoSquare::V3));
        assert_eq!(deps(8), ("v0.52.6", Some(Hl::V1_1_0), GoSquare::V3));
        assert_eq!(deps(9), ("v0.52.8", Some(Hl::V1_1_0), GoSquare::V4Rc4));
        for v in 10..=12 {
            assert_eq!(
                deps(v),
                ("v0.52.12", Some(Hl::CelestiaV1_3), GoSquare::V4),
                "v{v}"
            );
        }
    }

    /// The cosmos-sdk frame names the executing release's cosmos-sdk; app
    /// version 1 keeps v9's (its v0.46 driver never formats one).
    #[test]
    fn cosmos_sdk_frame_names_the_executing_cosmos_sdk() {
        let prev = app_version();
        for (v, tag) in [
            (0, "v0.52.12"),
            (1, "v0.52.8"),
            (5, "v0.52.8"),
            (9, "v0.52.8"),
            (10, "v0.52.12"),
            (13, "v0.52.12"),
        ] {
            set_app_version(v);
            assert_eq!(
                cosmos_sdk_frame("x/staking/types/authz.go:193"),
                format!(" [celestiaorg/cosmos-sdk@{tag}/x/staking/types/authz.go:193]"),
                "v{v}"
            );
        }
        set_app_version(prev);
    }

    /// `release` resolves the thread's executing app version, falling back
    /// to the context's.
    #[test]
    fn release_follows_the_executing_app_version() {
        let prev = app_version();
        set_app_version(0);
        assert_eq!(release(5).app_version, 9);
        assert_eq!(release(0).app_version, CURRENT_APP_VERSION);
        set_app_version(1);
        assert_eq!(release(10).app_version, 1);
        set_app_version(11);
        assert_eq!(release(9).app_version, 11);
        set_app_version(prev);
    }

    /// App version 1 executes as itself (the v0.46 driver); app versions
    /// 2 to 8 keep running as v9 (no behavior change).
    #[test]
    fn historical_versions_run_as_v9_except_v1() {
        for v in 1..=12 {
            let r = Release::for_app_version(v).expect("known app version");
            let runs_as = if v == 1 { 1 } else { v.max(9) };
            assert_eq!(r.runs_as, runs_as, "v{v}");
        }
        assert_eq!(Release::executing(1).app_version, 1);
        assert_eq!(Release::executing(1).sdk_line, SdkLine::V046);
        for v in 2..=3 {
            assert_eq!(Release::executing(v).sdk_line, SdkLine::V050, "v{v}");
        }
        assert_eq!(CURRENT_RELEASE.app_version, CURRENT_APP_VERSION);
        for v in [0, 13, u64::MAX] {
            assert_eq!(Release::executing(v), CURRENT_RELEASE, "v{v}");
        }
    }
}
