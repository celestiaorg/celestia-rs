//! `Keeper.Tally` (staking-weighted vote tally).

use std::collections::BTreeMap;

use celestia_proto::cosmos::gov::v1::{Proposal, TallyResult, WeightedVoteOption};
use math::{Int, LegacyDec};
use sdk::{Context, Result};

use crate::keeper::{decode, parse_dec};
use crate::{STORE_KEY, VOTES_PREFIX, get_params};

fn math_err(e: math::MathError) -> sdk::SdkError {
    sdk::error::go_panic(e.to_string())
}

/// Outcome of a tally.
#[derive(Debug, Clone, PartialEq)]
pub struct Tally {
    /// The proposal passes.
    pub passes: bool,
    /// Deposits are burned.
    pub burn_deposits: bool,
    /// Final tally.
    pub result: TallyResult,
}

struct ValGovInfo {
    bonded_tokens: Int,
    delegator_shares: LegacyDec,
    deductions: LegacyDec,
    vote: Vec<WeightedVoteOption>,
}

fn add_weighted(
    results: &mut [LegacyDec; 5],
    power: &LegacyDec,
    options: &[WeightedVoteOption],
) -> Result<()> {
    for o in options {
        // Go ignores the parse error (weights were validated on vote).
        let w = LegacyDec::from_str_go(&o.weight).unwrap_or_default();
        let sub = power.checked_mul(&w).map_err(math_err)?;
        let i = usize::try_from(o.option).unwrap_or(0).min(4);
        results[i] = results[i].checked_add(&sub).map_err(math_err)?;
    }
    Ok(())
}

/// `Tally`: deletes the proposal's votes as they are counted.
pub fn tally(ctx: &mut Context<'_>, proposal: &Proposal) -> Result<Tally> {
    let mut results: [LegacyDec; 5] = Default::default();
    let mut total = LegacyDec::zero();
    let mut vals: BTreeMap<Vec<u8>, ValGovInfo> = BTreeMap::new();
    for v in x_staking::store::get_bonded_validators_by_power(ctx)? {
        let op = v.operator()?;
        vals.insert(
            op,
            ValGovInfo {
                bonded_tokens: v.tokens.clone(),
                delegator_shares: v.delegator_shares.clone(),
                deductions: LegacyDec::zero(),
                vote: Vec::new(),
            },
        );
    }

    let mut prefix = vec![VOTES_PREFIX];
    prefix.extend_from_slice(&proposal.id.to_be_bytes());
    let mut it = ctx.prefix_iter(STORE_KEY, &prefix)?;
    while let Some((key, value)) = it.next(ctx)? {
        let vote: celestia_proto::cosmos::gov::v1::Vote = decode(&value)?;
        let voter = sdk::address::acc_from_bech32(&vote.voter)?;
        if let Some(val) = vals.get_mut(&voter) {
            val.vote.clone_from(&vote.options);
        }
        let mut dit =
            ctx.prefix_iter(x_staking::STORE_KEY, &x_staking::keys::delegations(&voter))?;
        while let Some((_, dv)) = dit.next(ctx)? {
            let d = x_staking::types::Delegation::decode(&dv)?;
            let val_addr = x_staking::addr::val_from_string(&d.validator_address)?;
            if let Some(val) = vals.get_mut(&val_addr) {
                val.deductions = val.deductions.checked_add(&d.shares).map_err(math_err)?;
                let power = d
                    .shares
                    .checked_mul_int(&val.bonded_tokens)
                    .and_then(|x| x.checked_quo(&val.delegator_shares))
                    .map_err(math_err)?;
                add_weighted(&mut results, &power, &vote.options)?;
                total = total.checked_add(&power).map_err(math_err)?;
            }
        }
        ctx.delete(STORE_KEY, &key)?;
    }

    for val in vals.values() {
        if val.vote.is_empty() {
            continue;
        }
        let after = val
            .delegator_shares
            .checked_sub(&val.deductions)
            .map_err(math_err)?;
        let power = after
            .checked_mul_int(&val.bonded_tokens)
            .and_then(|x| x.checked_quo(&val.delegator_shares))
            .map_err(math_err)?;
        add_weighted(&mut results, &power, &val.vote)?;
        total = total.checked_add(&power).map_err(math_err)?;
    }

    let params = get_params(ctx)?;
    // Option order: 1 yes, 2 abstain, 3 no, 4 no with veto.
    let result = TallyResult {
        yes_count: results[1].truncate_int().to_string(),
        abstain_count: results[2].truncate_int().to_string(),
        no_count: results[3].truncate_int().to_string(),
        no_with_veto_count: results[4].truncate_int().to_string(),
    };
    let done = |passes, burn_deposits, result: &TallyResult| Tally {
        passes,
        burn_deposits,
        result: result.clone(),
    };

    let total_bonded = x_staking::validator::total_bonded_tokens(ctx)?;
    if total_bonded.is_zero() {
        return Ok(done(false, false, &result));
    }
    let percent = total
        .checked_quo(&total_bonded.to_legacy_dec())
        .map_err(math_err)?;
    let quorum = parse_dec(&params.quorum).unwrap_or_default();
    if percent < quorum {
        return Ok(done(false, params.burn_vote_quorum, &result));
    }
    let non_abstain = total.checked_sub(&results[2]).map_err(math_err)?;
    if non_abstain.is_zero() {
        return Ok(done(false, false, &result));
    }
    let veto = parse_dec(&params.veto_threshold).unwrap_or_default();
    if results[4].checked_quo(&total).map_err(math_err)? > veto {
        return Ok(done(false, params.burn_vote_veto, &result));
    }
    let threshold = parse_dec(if proposal.expedited {
        &params.expedited_threshold
    } else {
        &params.threshold
    })
    .unwrap_or_default();
    if results[1].checked_quo(&non_abstain).map_err(math_err)? > threshold {
        return Ok(done(true, false, &result));
    }
    Ok(done(false, false, &result))
}
