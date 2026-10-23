//! `hyperlane` module: port of hyperlane-cosmos v1.3.1 `x/core` with its
//! `01_interchain_security` and `02_post_dispatch` submodules, which share
//! the `hyperlane` store through `collections` prefixes:
//!
//! | Prefix | Collection |
//! |---|---|
//! | `00 01` | mailboxes (`u64` → `Mailbox`) |
//! | `00 02` | mailbox sequence |
//! | `00 03` | delivered/dispatched messages (`(u64, id)` key set) |
//! | `00 04` / `00 05` / `00 06` | ISM / post-dispatch / app router sequences |
//! | `01 00` | ISMs (`u64` → `Any`) |
//! | `01 02` | validator storage locations (`(u64, bytes, u64)` → string) |
//! | `02 01` | IGPs |
//! | `02 03` | IGP destination gas configs (`(u64, u32)`) |
//! | `02 04` | merkle tree hooks |
//! | `02 05` | noop hooks |
//!
//! The module has no Begin/EndBlock logic. Keepers are free functions over
//! [`sdk::Context`]; the in-memory routers of Go live in [`Deps`].

pub mod coins;
pub mod core;
pub mod errors;
pub mod events;
pub mod genesis;
pub mod ism;
pub mod msg;
pub mod post_dispatch;
pub mod router;
pub mod testing;
pub mod util;

pub use coins::{Coin, Coins};
pub use genesis::{export_genesis, init_genesis_json};
pub use msg::handle_msg;
pub use router::{
    BankKeeper, Deps, HyperlaneApp, InterchainSecurityModule, PostDispatchModule, Router,
    StandardHookMetadata,
};
pub use util::{HexAddress, HyperlaneMessage};

/// Store key.
pub const STORE_KEY: &str = "hyperlane";
/// Module name (also the module account collecting IGP payments).
pub const MODULE_NAME: &str = "hyperlane";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 1;

/// `MailboxesKey`.
pub const MAILBOXES_KEY: [u8; 2] = [0, 1];
/// `MailboxesSequenceKey`.
pub const MAILBOXES_SEQUENCE_KEY: [u8; 2] = [0, 2];
/// `MessagesKey`.
pub const MESSAGES_KEY: [u8; 2] = [0, 3];
/// `IsmRouterKey`.
pub const ISM_ROUTER_KEY: [u8; 2] = [0, 4];
/// `PostDispatchRouterKey`.
pub const POST_DISPATCH_ROUTER_KEY: [u8; 2] = [0, 5];
/// `AppRouterKey`.
pub const APP_ROUTER_KEY: [u8; 2] = [0, 6];
/// ISM `IsmsKey`.
pub const ISMS_KEY: [u8; 2] = [1, 0];
/// ISM `StorageLocationsKey`.
pub const STORAGE_LOCATIONS_KEY: [u8; 2] = [1, 2];
/// Post-dispatch `InterchainGasPaymasterKey`.
pub const IGPS_KEY: [u8; 2] = [2, 1];
/// Post-dispatch `InterchainGasPaymasterConfigsKey`.
pub const IGP_CONFIGS_KEY: [u8; 2] = [2, 3];
/// Post-dispatch `MerkleTreeHooksKey`.
pub const MERKLE_TREE_HOOKS_KEY: [u8; 2] = [2, 4];
/// Post-dispatch `NoopHooksKey`.
pub const NOOP_HOOKS_KEY: [u8; 2] = [2, 5];

/// Whether the context executes app version 9: upstream hyperlane-cosmos
/// v1.1.0 (celestia-app v9.0.8, ADR 0004) instead of celestia's v1.3.0.
pub(crate) fn is_v9(ctx: &sdk::Context<'_>) -> bool {
    let v = ctx.consensus_params.version.as_ref().map_or(0, |v| v.app);
    v != 0 && v < 10
}

/// `authtypes.NewModuleAddress(name)`: `sha256(name)[:20]`.
#[must_use]
pub fn module_address(name: &str) -> Vec<u8> {
    hash::sha256(name.as_bytes())[..20].to_vec()
}
