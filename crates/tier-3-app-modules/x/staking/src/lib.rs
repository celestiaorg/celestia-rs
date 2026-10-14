//! `x/staking` (celestiaorg/cosmos-sdk v0.52.12, as wired by celestia-app
//! v10.4.0).
//!
//! Plain KV store layout (see [`keys`]); values are gogoproto messages with
//! `math` customtype fields (see [`types`]). Celestia specifics: the fork's
//! 60% maximum commission rate and `utia` as the default bond denom.
//!
//! Keepers are free functions over [`sdk::Context`]. Hooks into
//! distribution and slashing go through the process-wide [`hooks`]
//! registry, which the app installs at start-up.

pub mod abci;
pub mod addr;
pub mod delegation;
pub mod errors;
pub mod genesis;
pub mod hooks;
pub mod keys;
pub mod msgs;
pub mod slash;
pub mod store;
pub mod types;
pub mod validator;

pub use abci::{begin_block, end_block};
pub use genesis::init_genesis;
pub use hooks::{MultiStakingHooks, StakingHooks, set_hooks};
pub use msgs::{
    handle_begin_redelegate, handle_cancel_unbonding_delegation, handle_create_validator,
    handle_delegate, handle_edit_validator, handle_undelegate, handle_update_params,
};
pub use slash::slash;
pub use validator::{apply_and_return_validator_set_updates, jail, unjail};

/// Store key.
pub const STORE_KEY: &str = "staking";
/// Module name (error codespace).
pub const MODULE_NAME: &str = "staking";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 5;
/// Bonded tokens pool module account.
pub const BONDED_POOL: &str = "bonded_tokens_pool";
/// Not-bonded tokens pool module account.
pub const NOT_BONDED_POOL: &str = "not_bonded_tokens_pool";
/// `sdk.DefaultBondDenom` as set by celestia-app (`appconsts.BondDenom`).
pub const DEFAULT_BOND_DENOM: &str = "utia";

#[cfg(test)]
mod tests;
