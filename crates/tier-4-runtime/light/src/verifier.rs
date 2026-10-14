//! Header verification (`light/verifier.go`): adjacent, non-adjacent
//! (skipping, trust level), backwards, trusting period and clock drift.
//!
//! Durations are Go `time.Duration`s (nanoseconds, `i64`).

use ctypes::gofmt::{duration_string, hex_upper};
use ctypes::{Fraction, Header, SignedHeader, Time, ValidatorSet};

use crate::errors::{Error, Result};

/// `DefaultTrustLevel`: 1/3.
pub const DEFAULT_TRUST_LEVEL: Fraction = Fraction::ONE_THIRD;

/// `t.Add(d)`.
#[must_use]
pub fn time_add(t: Time, d: i64) -> Time {
    let n = t.unix_nanos() + i128::from(d);
    let seconds = n.div_euclid(1_000_000_000);
    let nanos = n.rem_euclid(1_000_000_000);
    Time {
        seconds: i64::try_from(seconds).unwrap_or(if seconds < 0 { i64::MIN } else { i64::MAX }),
        nanos: i32::try_from(nanos).unwrap_or(0),
    }
}

/// The current wall-clock time (UTC).
#[must_use]
pub fn now() -> Time {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    Time {
        seconds: i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
        nanos: i32::try_from(d.subsec_nanos()).unwrap_or(0),
    }
}

pub(crate) fn header(sh: &SignedHeader) -> Result<&Header> {
    sh.header.as_ref().ok_or_else(|| {
        Error::msg("runtime error: invalid memory address or nil pointer dereference")
    })
}

pub(crate) fn commit(sh: &SignedHeader) -> Result<&ctypes::Commit> {
    sh.commit.as_ref().ok_or_else(|| {
        Error::msg("runtime error: invalid memory address or nil pointer dereference")
    })
}

/// `VerifyNonAdjacent`.
#[allow(clippy::too_many_arguments)]
pub fn verify_non_adjacent(
    trusted_header: &SignedHeader,
    trusted_vals: &ValidatorSet,
    untrusted_header: &SignedHeader,
    untrusted_vals: &ValidatorSet,
    trusting_period: i64,
    now: Time,
    max_clock_drift: i64,
    trust_level: Fraction,
) -> Result<()> {
    let th = header(trusted_header)?;
    let uh = header(untrusted_header)?;
    if uh.height == th.height + 1 {
        return Err(Error::msg("headers must be non adjacent in height"));
    }
    if header_expired(th, trusting_period, now) {
        return Err(Error::OldHeaderExpired {
            at: time_add(th.time, trusting_period),
            now,
        });
    }
    verify_new_header_and_vals(untrusted_header, untrusted_vals, th, now, max_clock_drift)
        .map_err(|e| Error::InvalidHeader(Box::new(e)))?;

    let ucommit = commit(untrusted_header)?;
    if let Err(e) =
        ctypes::verify_commit_light_trusting(&th.chain_id, trusted_vals, ucommit, trust_level)
    {
        if e.is_not_enough_voting_power_signed() {
            return Err(Error::NewValSetCantBeTrusted(e));
        }
        return Err(Error::Types(e));
    }

    ctypes::verify_commit_light(
        &th.chain_id,
        untrusted_vals,
        &ucommit.block_id,
        uh.height,
        ucommit,
    )
    .map_err(|e| Error::InvalidHeader(Box::new(Error::Types(e))))
}

/// `VerifyAdjacent`.
pub fn verify_adjacent(
    trusted_header: &SignedHeader,
    untrusted_header: &SignedHeader,
    untrusted_vals: &ValidatorSet,
    trusting_period: i64,
    now: Time,
    max_clock_drift: i64,
) -> Result<()> {
    let th = header(trusted_header)?;
    let uh = header(untrusted_header)?;
    if uh.height != th.height + 1 {
        return Err(Error::msg("headers must be adjacent in height"));
    }
    if header_expired(th, trusting_period, now) {
        return Err(Error::OldHeaderExpired {
            at: time_add(th.time, trusting_period),
            now,
        });
    }
    verify_new_header_and_vals(untrusted_header, untrusted_vals, th, now, max_clock_drift)
        .map_err(|e| Error::InvalidHeader(Box::new(e)))?;

    if uh.validators_hash != th.next_validators_hash {
        return Err(Error::msg(format!(
            "expected old header next validators ({}) to match those from new header ({})",
            hex_upper(&th.next_validators_hash),
            hex_upper(&uh.validators_hash)
        )));
    }

    let ucommit = commit(untrusted_header)?;
    ctypes::verify_commit_light(
        &th.chain_id,
        untrusted_vals,
        &ucommit.block_id,
        uh.height,
        ucommit,
    )
    .map_err(|e| Error::InvalidHeader(Box::new(Error::Types(e))))
}

/// `Verify`: adjacent or non-adjacent depending on the heights.
#[allow(clippy::too_many_arguments)]
pub fn verify(
    trusted_header: &SignedHeader,
    trusted_vals: &ValidatorSet,
    untrusted_header: &SignedHeader,
    untrusted_vals: &ValidatorSet,
    trusting_period: i64,
    now: Time,
    max_clock_drift: i64,
    trust_level: Fraction,
) -> Result<()> {
    if header(untrusted_header)?.height != header(trusted_header)?.height + 1 {
        return verify_non_adjacent(
            trusted_header,
            trusted_vals,
            untrusted_header,
            untrusted_vals,
            trusting_period,
            now,
            max_clock_drift,
            trust_level,
        );
    }
    verify_adjacent(
        trusted_header,
        untrusted_header,
        untrusted_vals,
        trusting_period,
        now,
        max_clock_drift,
    )
}

fn verify_new_header_and_vals(
    untrusted_header: &SignedHeader,
    untrusted_vals: &ValidatorSet,
    trusted_header: &Header,
    now: Time,
    max_clock_drift: i64,
) -> Result<()> {
    untrusted_header
        .validate_basic(&trusted_header.chain_id)
        .map_err(|e| Error::msg(format!("untrustedHeader.ValidateBasic failed: {e}")))?;
    let uh = header(untrusted_header)?;
    if uh.height <= trusted_header.height {
        return Err(Error::msg(format!(
            "expected new header height {} to be greater than one of old header {}",
            uh.height, trusted_header.height
        )));
    }
    if uh.time <= trusted_header.time {
        return Err(Error::msg(format!(
            "expected new header time {} to be after old header time {}",
            uh.time.go_string(),
            trusted_header.time.go_string()
        )));
    }
    if uh.time >= time_add(now, max_clock_drift) {
        return Err(Error::msg(format!(
            "new header has a time from the future {} (now: {}; max clock drift: {})",
            uh.time.go_string(),
            now.go_string(),
            duration_string(max_clock_drift)
        )));
    }
    let vh = untrusted_vals.hash();
    if uh.validators_hash != vh {
        return Err(Error::msg(format!(
            "expected new header validators ({}) to match those that were supplied ({}) at height {}",
            hex_upper(&uh.validators_hash),
            hex_upper(&vh),
            uh.height
        )));
    }
    Ok(())
}

/// `ValidateTrustLevel`: the trust level must be within [1/3, 1].
pub fn validate_trust_level(lvl: Fraction) -> Result<()> {
    if lvl.numerator.wrapping_mul(3) < lvl.denominator
        || lvl.numerator > lvl.denominator
        || lvl.denominator == 0
    {
        return Err(Error::msg(format!(
            "trustLevel must be within [1/3, 1], given {}/{}",
            lvl.numerator, lvl.denominator
        )));
    }
    Ok(())
}

/// `HeaderExpired`.
#[must_use]
pub fn header_expired(h: &Header, trusting_period: i64, now: Time) -> bool {
    time_add(h.time, trusting_period) <= now
}

/// `VerifyBackwards`: `untrusted` is the block right below `trusted`.
pub fn verify_backwards(untrusted: &Header, trusted: &Header) -> Result<()> {
    let invalid = |e: Error| Error::InvalidHeader(Box::new(e));
    untrusted
        .validate_basic()
        .map_err(|e| invalid(Error::Types(e)))?;
    if untrusted.chain_id != trusted.chain_id {
        return Err(invalid(Error::msg("header belongs to another chain")));
    }
    if untrusted.time >= trusted.time {
        return Err(invalid(Error::msg(format!(
            "expected older header time {} to be before new header time {}",
            untrusted.time.go_string(),
            trusted.time.go_string()
        ))));
    }
    let uhash = untrusted.hash().unwrap_or_default();
    if uhash != trusted.last_block_id.hash {
        return Err(invalid(Error::msg(format!(
            "older header hash {} does not match trusted header's last block {}",
            hex_upper(&uhash),
            hex_upper(&trusted.last_block_id.hash)
        ))));
    }
    Ok(())
}
