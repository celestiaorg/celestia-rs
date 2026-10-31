//! Rust build identity and separately named Go compatibility targets.

/// This node's software version, shared by the CLI and P2P/RPC metadata.
pub const SOFTWARE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// A transparent Comet API compatibility prefix and Rust software identity.
///
/// Clients that dispatch by the `0.38.*` prefix keep selecting the Comet 0.38
/// adapter. The suffix identifies the implementation; numeric wire protocols
/// and the actual node build version remain separate.
#[must_use]
pub fn node_version() -> String {
    format!(
        "{}-celestia-rs.{SOFTWARE_VERSION}",
        p2p::node_info::TM_CORE_SEMVER
    )
}

/// The celestia-core release targeted by the node implementation.
pub const CORE_COMPATIBILITY_VERSION: &str = "0.42.3";

/// Records this binary's actual build identity for Cosmos `GetNodeInfo`.
///
/// Also called by library node startup, so embedded node users do not silently
/// report the `app` library crate's package version in place of the node's.
pub fn register_build_info() {
    app::node_services::set_build_info(
        SOFTWARE_VERSION,
        env!("CELESTIA_RS_GIT_COMMIT"),
        env!("CELESTIA_RS_BUILD_PROFILE"),
        env!("CELESTIA_RS_TOOLCHAIN"),
    );
}
