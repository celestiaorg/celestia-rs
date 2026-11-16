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

/// Whether `v` is executed with celestia-app v9.0.8 semantics (0 = unset
/// is current).
#[must_use]
pub const fn is_v9(v: u64) -> bool {
    v != 0 && v < 10
}

/// Whether `v` is executed with celestia-app v11 semantics (0 = unset is
/// the current release, v10).
#[must_use]
pub const fn is_v11(v: u64) -> bool {
    v >= 11
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
