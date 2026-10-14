//! `x/distribution` (celestiaorg/cosmos-sdk v0.52.12).
//!
//! F1 fee distribution: validator periods, historical/current rewards,
//! accumulated commission, outstanding rewards, delegator starting info and
//! slash events. Celestia's fork adds `UserOutstandingRewards`: rewards
//! accrued when a delegation's shares change are parked per
//! `(delegator, validator)` instead of being paid out immediately, and are
//! paid on the next explicit withdrawal.

pub mod errors;
pub mod genesis;
pub mod keeper;
pub mod msgs;
pub mod store;

pub use genesis::init_genesis;
pub use keeper::{Hooks, begin_block};
pub use msgs::{
    handle_community_pool_spend, handle_deposit_validator_rewards_pool, handle_fund_community_pool,
    handle_set_withdraw_address, handle_update_params, handle_withdraw_delegator_reward,
    handle_withdraw_validator_commission,
};

/// Store key.
pub const STORE_KEY: &str = "distribution";
/// Module name (module account, error codespace).
pub const MODULE_NAME: &str = "distribution";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 4;

#[cfg(test)]
mod tests;
