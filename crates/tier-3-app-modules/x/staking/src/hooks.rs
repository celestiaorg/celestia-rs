//! Staking hooks (`types.StakingHooks`).
//!
//! The implementations live in the modules that react to staking changes
//! (distribution, slashing), which depend on this crate; the app installs
//! the combined hooks once at start-up with [`set_hooks`], in Go's order
//! (`NewMultiStakingHooks(distr, slashing)`).

use std::sync::{Arc, RwLock};

use math::LegacyDec;
use sdk::{Context, Result};

/// `StakingHooks`. Every method defaults to a no-op.
#[allow(unused_variables)]
pub trait StakingHooks: Send + Sync {
    /// `AfterValidatorCreated`.
    fn after_validator_created(&self, ctx: &mut Context<'_>, val: &[u8]) -> Result<()> {
        Ok(())
    }
    /// `BeforeValidatorModified`.
    fn before_validator_modified(&self, ctx: &mut Context<'_>, val: &[u8]) -> Result<()> {
        Ok(())
    }
    /// `AfterValidatorRemoved`.
    fn after_validator_removed(
        &self,
        ctx: &mut Context<'_>,
        cons: &[u8],
        val: &[u8],
    ) -> Result<()> {
        Ok(())
    }
    /// `AfterValidatorBonded`.
    fn after_validator_bonded(&self, ctx: &mut Context<'_>, cons: &[u8], val: &[u8]) -> Result<()> {
        Ok(())
    }
    /// `AfterValidatorBeginUnbonding`.
    fn after_validator_begin_unbonding(
        &self,
        ctx: &mut Context<'_>,
        cons: &[u8],
        val: &[u8],
    ) -> Result<()> {
        Ok(())
    }
    /// `BeforeDelegationCreated`.
    fn before_delegation_created(
        &self,
        ctx: &mut Context<'_>,
        del: &[u8],
        val: &[u8],
    ) -> Result<()> {
        Ok(())
    }
    /// `BeforeDelegationSharesModified`.
    fn before_delegation_shares_modified(
        &self,
        ctx: &mut Context<'_>,
        del: &[u8],
        val: &[u8],
    ) -> Result<()> {
        Ok(())
    }
    /// `BeforeDelegationRemoved`.
    fn before_delegation_removed(
        &self,
        ctx: &mut Context<'_>,
        del: &[u8],
        val: &[u8],
    ) -> Result<()> {
        Ok(())
    }
    /// `AfterDelegationModified`.
    fn after_delegation_modified(
        &self,
        ctx: &mut Context<'_>,
        del: &[u8],
        val: &[u8],
    ) -> Result<()> {
        Ok(())
    }
    /// `BeforeValidatorSlashed`.
    fn before_validator_slashed(
        &self,
        ctx: &mut Context<'_>,
        val: &[u8],
        fraction: &LegacyDec,
    ) -> Result<()> {
        Ok(())
    }
    /// `AfterUnbondingInitiated`.
    fn after_unbonding_initiated(&self, ctx: &mut Context<'_>, id: u64) -> Result<()> {
        Ok(())
    }
}

/// `MultiStakingHooks`: calls each hook in order, stopping at the first error.
pub struct MultiStakingHooks(pub Vec<Box<dyn StakingHooks>>);

impl std::fmt::Debug for MultiStakingHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MultiStakingHooks({})", self.0.len())
    }
}

macro_rules! multi {
    ($self:ident, $m:ident ( $($a:expr),* )) => {{
        for h in &$self.0 {
            h.$m($($a),*)?;
        }
        Ok(())
    }};
}

impl StakingHooks for MultiStakingHooks {
    fn after_validator_created(&self, ctx: &mut Context<'_>, val: &[u8]) -> Result<()> {
        multi!(self, after_validator_created(ctx, val))
    }
    fn before_validator_modified(&self, ctx: &mut Context<'_>, val: &[u8]) -> Result<()> {
        multi!(self, before_validator_modified(ctx, val))
    }
    fn after_validator_removed(
        &self,
        ctx: &mut Context<'_>,
        cons: &[u8],
        val: &[u8],
    ) -> Result<()> {
        multi!(self, after_validator_removed(ctx, cons, val))
    }
    fn after_validator_bonded(&self, ctx: &mut Context<'_>, cons: &[u8], val: &[u8]) -> Result<()> {
        multi!(self, after_validator_bonded(ctx, cons, val))
    }
    fn after_validator_begin_unbonding(
        &self,
        ctx: &mut Context<'_>,
        cons: &[u8],
        val: &[u8],
    ) -> Result<()> {
        multi!(self, after_validator_begin_unbonding(ctx, cons, val))
    }
    fn before_delegation_created(
        &self,
        ctx: &mut Context<'_>,
        del: &[u8],
        val: &[u8],
    ) -> Result<()> {
        multi!(self, before_delegation_created(ctx, del, val))
    }
    fn before_delegation_shares_modified(
        &self,
        ctx: &mut Context<'_>,
        del: &[u8],
        val: &[u8],
    ) -> Result<()> {
        multi!(self, before_delegation_shares_modified(ctx, del, val))
    }
    fn before_delegation_removed(
        &self,
        ctx: &mut Context<'_>,
        del: &[u8],
        val: &[u8],
    ) -> Result<()> {
        multi!(self, before_delegation_removed(ctx, del, val))
    }
    fn after_delegation_modified(
        &self,
        ctx: &mut Context<'_>,
        del: &[u8],
        val: &[u8],
    ) -> Result<()> {
        multi!(self, after_delegation_modified(ctx, del, val))
    }
    fn before_validator_slashed(
        &self,
        ctx: &mut Context<'_>,
        val: &[u8],
        fraction: &LegacyDec,
    ) -> Result<()> {
        multi!(self, before_validator_slashed(ctx, val, fraction))
    }
    fn after_unbonding_initiated(&self, ctx: &mut Context<'_>, id: u64) -> Result<()> {
        multi!(self, after_unbonding_initiated(ctx, id))
    }
}

struct NoHooks;
impl StakingHooks for NoHooks {}

static HOOKS: RwLock<Option<Arc<dyn StakingHooks>>> = RwLock::new(None);

/// Installs the process-wide staking hooks (`Keeper.SetHooks`), replacing
/// any previous ones.
pub fn set_hooks(h: Arc<dyn StakingHooks>) {
    if let Ok(mut g) = HOOKS.write() {
        *g = Some(h);
    }
}

/// The installed hooks (no-op when none were set).
#[must_use]
pub fn hooks() -> Arc<dyn StakingHooks> {
    HOOKS
        .read()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_else(|| Arc::new(NoHooks))
}
