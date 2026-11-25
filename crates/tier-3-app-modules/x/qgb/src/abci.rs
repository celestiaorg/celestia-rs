//! The blobstream EndBlocker (`abci.go`): a new valset attestation when
//! there is none yet, a validator began unbonding this block, or the
//! normalized power moved by more than 5 %; data commitments every
//! `DataCommitmentWindow` blocks; pruning of attestations older than
//! three weeks.
//!
//! Go panics on every error here, which halts the node; these functions
//! return the error instead.

use math::LegacyDec;
use pbjson_types::Timestamp;
use sdk::{Context, Result};

use crate::types::{self, Attestation, DataCommitment, Valset};
use crate::valset::{self, InternalBridgeValidator};
use crate::{errors, get_attestation};

/// `AttestationExpiryTime`: three weeks, in seconds.
pub const ATTESTATION_EXPIRY_SECONDS: i64 = 3 * 7 * 24 * 60 * 60;

/// `SignificantPowerDifferenceThreshold` (0.05).
fn significant_power_difference_threshold() -> Result<LegacyDec> {
    LegacyDec::new_with_prec(5, 2).map_err(|e| sdk::error::sdk::logic(e.to_string()))
}

/// Go's `uint64(ctx.BlockHeight())`.
#[allow(clippy::cast_sign_loss)]
const fn block_height_u64(h: i64) -> u64 {
    h as u64
}

/// `normalizeValidatorPower`: `raw * 2^32 / total`, truncated (`total` is
/// the positive sum of every raw power, so the quotient fits).
fn normalize_validator_power(raw: u64, total: u128) -> Result<u64> {
    let scaled = u128::from(raw) << 32;
    let q = scaled
        .checked_div(total)
        .ok_or_else(|| sdk::error::sdk::logic("division by zero"))?;
    // Go's `big.Int.Uint64` keeps the low 64 bits.
    #[allow(clippy::cast_possible_truncation)]
    Ok(q as u64)
}

/// Both nonces, or the error Go's getters return before initialization.
fn nonce_range(ctx: &mut Context<'_>) -> Result<(u64, u64)> {
    let latest =
        crate::latest_attestation_nonce(ctx)?.ok_or_else(errors::latest_nonce_not_initialized)?;
    let earliest = crate::earliest_available_attestation_nonce(ctx)?
        .ok_or_else(errors::earliest_nonce_not_initialized)?;
    Ok((earliest, latest))
}

/// The attestations from `from` down to `to` (inclusive) until `pick`
/// returns one; an absent nonce is `on_missing`'s error. Bounded by the
/// stored nonce range.
fn find_back<T>(
    ctx: &mut Context<'_>,
    from: u64,
    to: u64,
    on_missing: impl Fn(u64) -> sdk::SdkError,
    mut pick: impl FnMut(Attestation) -> Option<T>,
) -> Result<Option<T>> {
    let mut i = from;
    while i >= to {
        let at = crate::get_attestation(ctx, i)?.ok_or_else(|| on_missing(i))?;
        if let Some(t) = pick(at) {
            return Ok(Some(t));
        }
        let Some(next) = i.checked_sub(1) else { break };
        i = next;
    }
    Ok(None)
}

/// `GetLatestValset`: the newest stored valset. Go panics when a nonce in
/// range is missing or no valset is left with nonce 1 still available;
/// once older ones were pruned it falls back to the current valset.
pub fn get_latest_valset(ctx: &mut Context<'_>) -> Result<Valset> {
    let (earliest, latest) = nonce_range(ctx)?;
    let missing =
        |i| errors::nil_attestation(&format!("stumbled upon nil attestation for nonce {i}"));
    let found = find_back(ctx, latest, earliest, missing, |at| match at {
        Attestation::Valset(v) => Some(v),
        Attestation::DataCommitment(_) => None,
    })?;
    match found {
        Some(v) => Ok(v),
        None if earliest == 1 => Err(sdk::error::sdk::logic(
            "couldn't find latest valset: not found",
        )),
        None => get_current_valset(ctx),
    }
}

/// `GetLatestDataCommitment`: the newest stored data commitment, or
/// `ErrDataCommitmentNotFound`.
pub fn get_latest_data_commitment(ctx: &mut Context<'_>) -> Result<DataCommitment> {
    let (earliest, latest) = nonce_range(ctx)?;
    let missing = |i| errors::attestation_not_found(&format!("nonce {i}"));
    find_back(ctx, latest, earliest, missing, |at| match at {
        Attestation::DataCommitment(d) => Some(d),
        Attestation::Valset(_) => None,
    })?
    .ok_or_else(|| sdk::SdkError::new(errors::CODESPACE, 30, "data commitment not found"))
}

/// `HasDataCommitmentInStore`: whether any available attestation is a
/// data commitment (false before the nonces are initialized).
pub fn has_data_commitment(ctx: &mut Context<'_>) -> Result<bool> {
    if crate::latest_attestation_nonce(ctx)?.is_none() {
        return Ok(false);
    }
    let (earliest, latest) = nonce_range(ctx)?;
    let mut i = earliest;
    while i <= latest {
        let at = crate::get_attestation(ctx, i)?
            .ok_or_else(|| errors::attestation_not_found(&format!("nonce {i}")))?;
        if matches!(at, Attestation::DataCommitment(_)) {
            return Ok(true);
        }
        let Some(next) = i.checked_add(1) else { break };
        i = next;
    }
    Ok(false)
}

/// `GetCurrentValset`: the bonded validators by power with their last
/// power, normalized to a total of `2^32`, and their EVM addresses (a
/// validator without one gets its default address stored here).
pub fn get_current_valset(ctx: &mut Context<'_>) -> Result<Valset> {
    let validators = x_staking::store::get_bonded_validators_by_power(ctx)?;
    if validators.is_empty() {
        return Err(errors::no_validators());
    }
    let mut members = Vec::with_capacity(validators.len());
    let mut total: u128 = 0;
    for v in &validators {
        let val = v.operator()?;
        if val.is_empty() || val.len() > 255 {
            return Err(sdk::error::sdk::invalid_address(
                "invalid validator address",
            ));
        }
        let power = x_staking::store::get_last_validator_power(ctx, &val)?;
        // `sdk.NewInt(p).Uint64()` panics on a negative power.
        let power =
            u64::try_from(power).map_err(|_| sdk::error::sdk::logic("Uint64() out of bounds"))?;
        let evm = if let Some(a) = crate::get_evm_address(ctx, &val)? {
            a
        } else {
            // Logged as an error in Go.
            let a = types::default_evm_address(&val);
            crate::set_evm_address(ctx, &val, &a)?;
            a
        };
        let bv = types::BridgeValidator {
            power,
            evm_address: types::checksum_hex(&evm),
        };
        members.push(InternalBridgeValidator::new(&bv)?);
        total = total
            .checked_add(u128::from(power))
            .ok_or_else(|| sdk::error::sdk::logic("power overflow"))?;
    }
    for m in &mut members {
        m.power = normalize_validator_power(m.power, total)?;
    }
    let latest =
        crate::latest_attestation_nonce(ctx)?.ok_or_else(errors::latest_nonce_not_initialized)?;
    valset::new_valset(
        latest.wrapping_add(1),
        block_height_u64(ctx.header.height),
        members,
        ctx.header.time,
    )
    .map_err(|e| e.wrap("generated invalid valset"))
}

/// `handleValsetRequest`.
fn handle_valset_request(ctx: &mut Context<'_>) -> Result<()> {
    let latest_valset = match crate::latest_attestation_nonce(ctx)? {
        Some(n) if n != 0 => Some(get_latest_valset(ctx)?),
        _ => None,
    };
    let latest_unbonding = crate::latest_unbonding_block_height(ctx)?;
    let mut significant = false;
    if let Some(latest) = &latest_valset {
        let current = match get_current_valset(ctx) {
            Ok(v) => v,
            // Logged and skipped in Go.
            Err(e) if (e.codespace, e.code) == (errors::CODESPACE, 12) => return Ok(()),
            Err(e) => return Err(e),
        };
        let cur = valset::to_internal(&current.members)
            .map_err(|e| e.wrap("invalid current valset members"))?;
        let old = valset::to_internal(&latest.members)
            .map_err(|e| e.wrap("invalid latest valset members"))?;
        significant = valset::power_diff(&cur, &old)? > significant_power_difference_threshold()?;
    }
    if latest_valset.is_none()
        || latest_unbonding == block_height_u64(ctx.header.height)
        || significant
    {
        let valset = get_current_valset(ctx)?;
        crate::set_attestation_request(ctx, &Attestation::Valset(valset))?;
    }
    Ok(())
}

/// `NextDataCommitment`.
fn next_data_commitment(ctx: &mut Context<'_>, window: u64) -> Result<DataCommitment> {
    let latest =
        crate::latest_attestation_nonce(ctx)?.ok_or_else(errors::latest_nonce_not_initialized)?;
    let (begin_block, end_block) = if has_data_commitment(ctx)? {
        let last = get_latest_data_commitment(ctx)?.end_block;
        (last, last.wrapping_add(window))
    } else {
        (1, window.wrapping_add(1))
    };
    Ok(DataCommitment {
        nonce: latest.wrapping_add(1),
        begin_block,
        end_block,
        time: Some(ctx.header.time),
    })
}

/// `handleDataCommitmentRequest`: creates every data commitment whose
/// window has elapsed. Each one advances the latest end block by the
/// window (at least 100 by the param's validation), so the loop runs at
/// most `height / window + 1` times.
pub(crate) fn handle_data_commitment_request(ctx: &mut Context<'_>) -> Result<()> {
    let window = crate::data_commitment_window(ctx)?;
    if window == 0 {
        // Go would loop forever; the param's validation rules it out.
        return Err(sdk::error::sdk::logic("data commitment window is zero"));
    }
    #[allow(clippy::cast_possible_wrap)]
    let window_i64 = window as i64;
    let height = ctx.header.height;
    loop {
        let due = if has_data_commitment(ctx)? {
            let latest = get_latest_data_commitment(ctx)?;
            #[allow(clippy::cast_possible_wrap)]
            let end = latest.end_block as i64;
            height.wrapping_sub(end) >= window_i64
        } else {
            height >= window_i64
        };
        if !due {
            return Ok(());
        }
        let dc = next_data_commitment(ctx, window)
            .map_err(|e| e.wrap("couldn't get current data commitment"))?;
        crate::set_attestation_request(ctx, &Attestation::DataCommitment(dc))?;
    }
}

/// `pruneAttestations`: deletes the attestations, oldest first and never
/// the latest, whose block time is at least three weeks before this
/// block's, then records the new earliest nonce. Like Go, a missing or
/// undecodable attestation stops the pruning without updating the
/// earliest nonce (deletions already made stay).
pub(crate) fn prune_attestations(ctx: &mut Context<'_>) -> Result<()> {
    let Some(latest) = crate::latest_attestation_nonce(ctx)? else {
        return Ok(());
    };
    let Some(earliest) = crate::earliest_available_attestation_nonce(ctx)? else {
        return Ok(());
    };
    let now = ctx.header.time;
    let mut next = earliest;
    while next < latest {
        let Ok(Some(at)) = get_attestation(ctx, next) else {
            return Ok(());
        };
        if unexpired(&at.block_time(), &now) {
            break;
        }
        crate::delete_attestation(ctx, next)?;
        next += 1;
    }
    if next > earliest {
        crate::set_earliest_available_attestation_nonce(ctx, next)?;
    }
    Ok(())
}

/// `attestationTime.Add(AttestationExpiryTime).After(now)`: the
/// attestation (and every later one) is kept.
fn unexpired(at: &Timestamp, now: &Timestamp) -> bool {
    let expiry = Timestamp {
        seconds: at.seconds.saturating_add(ATTESTATION_EXPIRY_SECONDS),
        nanos: at.nanos,
    };
    sdk::time::after(&expiry, now)
}

/// `EndBlocker`: the valset first (a new validator set signs from now
/// on), then data commitments, then pruning.
pub fn end_block(ctx: &mut Context<'_>) -> Result<()> {
    handle_valset_request(ctx)?;
    handle_data_commitment_request(ctx)?;
    prune_attestations(ctx)
}
