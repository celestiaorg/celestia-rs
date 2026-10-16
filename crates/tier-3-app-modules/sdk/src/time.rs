//! Go `time.Time` / `time.Duration` helpers over protobuf `Timestamp` /
//! `Duration`, and the SDK's sortable time encoding (`sdk.FormatTimeBytes`,
//! also the `collections`/`sdk.TimeKey` key codec).
//!
//! Instants are handled as `i128` nanoseconds since the Unix epoch, which
//! covers Go's whole `time.Time` range without overflow.

use std::fmt::Write as _;

use pbjson_types::{Duration, Timestamp};

/// Go `sdk.SortableTimeFormat`.
pub const SORTABLE_TIME_FORMAT: &str = "2006-01-02T15:04:05.000000000";
/// Length of [`format_time_bytes`] output for years 0..=9999.
pub const SORTABLE_TIME_LEN: usize = 29;

const NANOS_PER_SEC: i128 = 1_000_000_000;

/// Nanoseconds since the Unix epoch.
#[must_use]
pub fn to_nanos(ts: &Timestamp) -> i128 {
    i128::from(ts.seconds) * NANOS_PER_SEC + i128::from(ts.nanos)
}

/// Inverse of [`to_nanos`] (normalized: `0 <= nanos < 1e9`).
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn from_nanos(n: i128) -> Timestamp {
    Timestamp {
        seconds: n.div_euclid(NANOS_PER_SEC) as i64,
        nanos: n.rem_euclid(NANOS_PER_SEC) as i32,
    }
}

/// Duration in nanoseconds (Go `time.Duration` is `int64` nanoseconds; a
/// gogoproto `stdduration` is decoded into it with overflow checking).
#[must_use]
pub fn duration_nanos(d: &Duration) -> i128 {
    i128::from(d.seconds) * NANOS_PER_SEC + i128::from(d.nanos)
}

/// Go `Duration` from nanoseconds.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn duration_from_nanos(n: i64) -> Duration {
    let n = i128::from(n);
    // Go's Duration proto encoding keeps seconds and nanos with the same sign.
    Duration {
        seconds: (n / NANOS_PER_SEC) as i64,
        nanos: (n % NANOS_PER_SEC) as i32,
    }
}

/// Go `t.Add(d)`.
#[must_use]
pub fn add(ts: &Timestamp, d_nanos: i128) -> Timestamp {
    from_nanos(to_nanos(ts) + d_nanos)
}

/// Go `a.Sub(b)` in nanoseconds: saturates to the `time.Duration` range.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn sub(a: &Timestamp, b: &Timestamp) -> i64 {
    (to_nanos(a) - to_nanos(b)).clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

/// Go `t.IsZero()` (`0001-01-01T00:00:00Z`).
#[must_use]
pub fn is_zero(ts: &Timestamp) -> bool {
    ts.seconds == GO_ZERO_UNIX_SECONDS && ts.nanos == 0
}

/// Unix seconds of Go's zero `time.Time`.
pub const GO_ZERO_UNIX_SECONDS: i64 = -62_135_596_800;

/// Go's zero `time.Time` as a protobuf timestamp (what gogoproto `stdtime`
/// decodes an absent non-nullable timestamp field into).
#[must_use]
pub fn go_zero() -> Timestamp {
    Timestamp {
        seconds: GO_ZERO_UNIX_SECONDS,
        nanos: 0,
    }
}

// Howard Hinnant's civil-date algorithms.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Broken-down UTC time: (year, month, day, hour, minute, second, nanos).
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn to_civil(ts: &Timestamp) -> (i64, u32, u32, u32, u32, u32, u32) {
    let n = to_nanos(ts);
    let secs = n.div_euclid(NANOS_PER_SEC) as i64;
    let nanos = n.rem_euclid(NANOS_PER_SEC) as u32;
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400) as u32;
    let (y, m, d) = civil_from_days(days);
    (y, m, d, sod / 3600, (sod % 3600) / 60, sod % 60, nanos)
}

fn push_year(out: &mut String, y: i64) {
    // Go appendInt(b, year, 4).
    if y < 0 {
        out.push('-');
    }
    let _ = write!(out, "{:04}", y.unsigned_abs());
}

/// Go `sdk.FormatTimeBytes`: `t.UTC().Round(0).Format(SortableTimeFormat)`.
#[must_use]
pub fn format_time_bytes(ts: &Timestamp) -> Vec<u8> {
    let (y, mo, d, h, mi, s, ns) = to_civil(ts);
    let mut out = String::with_capacity(SORTABLE_TIME_LEN);
    push_year(&mut out, y);
    let _ = write!(out, "-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{ns:09}");
    out.into_bytes()
}

/// Go `time.Format(time.RFC3339Nano)` in UTC (trailing fractional zeros
/// trimmed, `Z` suffix): the `String`-free formatting used in events and
/// logs.
#[must_use]
pub fn format_rfc3339_nano(ts: &Timestamp) -> String {
    let (y, mo, d, h, mi, s, ns) = to_civil(ts);
    let mut out = String::new();
    push_year(&mut out, y);
    let _ = write!(out, "-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}");
    if ns != 0 {
        let frac = format!("{ns:09}");
        out.push('.');
        out.push_str(frac.trim_end_matches('0'));
    }
    out.push('Z');
    out
}

/// Go `time.Time.String()` of a UTC time (`%v` / `%s`):
/// `2006-01-02 15:04:05.999999999 +0000 UTC`.
#[must_use]
pub fn go_time_string(ts: &Timestamp) -> String {
    let s = format_rfc3339_nano(ts);
    let body = s.strip_suffix('Z').unwrap_or(&s).replacen('T', " ", 1);
    format!("{body} +0000 UTC")
}

/// Go `time.Duration(n).String()` (`72h3m0.5s`, `1.5ms`, `0s`); `n` is
/// clamped to the `int64` range.
#[must_use]
pub fn go_duration_string(n: i128) -> String {
    let dur = i64::try_from(n).unwrap_or(if n < 0 { i64::MIN } else { i64::MAX });
    let mut rem = dur.unsigned_abs();
    let mut out = String::new();
    if rem < 1_000_000_000 {
        if rem == 0 {
            return "0s".to_owned();
        }
        let (prec, unit) = if rem < 1_000 {
            (0, "ns")
        } else if rem < 1_000_000 {
            (3, "\u{b5}s")
        } else {
            (6, "ms")
        };
        let (frac, rest) = fmt_frac(rem, prec);
        let _ = write!(out, "{rest}{frac}{unit}");
    } else {
        let (frac, secs) = fmt_frac(rem, 9);
        rem = secs;
        let sec = format!("{}{frac}s", rem % 60);
        rem /= 60;
        if rem > 0 {
            let m = format!("{}m", rem % 60);
            rem /= 60;
            if rem > 0 {
                let _ = write!(out, "{rem}h");
            }
            out.push_str(&m);
        }
        out.push_str(&sec);
    }
    if dur < 0 {
        out.insert(0, '-');
    }
    out
}

/// Go `fmtFrac`: the fractional digits of `v / 10^prec` without trailing
/// zeros (with a leading '.' when non-empty) and the integer part.
fn fmt_frac(mut v: u64, prec: u32) -> (String, u64) {
    let mut digits = Vec::new();
    let mut print = false;
    for _ in 0..prec {
        let digit = v % 10;
        print = print || digit != 0;
        if print {
            digits.push(char::from(b'0' + u8::try_from(digit).unwrap_or(0)));
        }
        v /= 10;
    }
    let mut s = String::new();
    if print {
        s.push('.');
        s.extend(digits.iter().rev());
    }
    (s, v)
}

fn digits(b: &[u8]) -> Option<u32> {
    if b.is_empty() || !b.iter().all(u8::is_ascii_digit) {
        return None;
    }
    b.iter().try_fold(0u32, |acc, c| {
        acc.checked_mul(10)?.checked_add(u32::from(c - b'0'))
    })
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Go `sdk.ParseTimeBytes` (`time.Parse(SortableTimeFormat, …).UTC()`) for
/// four-digit years.
#[must_use]
#[allow(clippy::many_single_char_names)]
pub fn parse_time_bytes(b: &[u8]) -> Option<Timestamp> {
    if b.len() != SORTABLE_TIME_LEN
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'.'
    {
        return None;
    }
    let y = i64::from(digits(&b[0..4])?);
    let mo = digits(&b[5..7])?;
    let d = digits(&b[8..10])?;
    let h = digits(&b[11..13])?;
    let mi = digits(&b[14..16])?;
    let s = digits(&b[17..19])?;
    let ns = digits(&b[20..29])?;
    if !(1..=12).contains(&mo) || d < 1 || d > days_in_month(y, mo) || h > 23 || mi > 59 || s > 59 {
        return None;
    }
    let secs = days_from_civil(y, mo, d) * 86_400
        + i64::from(h) * 3600
        + i64::from(mi) * 60
        + i64::from(s);
    Some(from_nanos(
        i128::from(secs) * NANOS_PER_SEC + i128::from(ns),
    ))
}

/// Go `t.Add(d)` for a protobuf duration.
#[must_use]
pub fn add_duration(ts: &Timestamp, d: &Duration) -> Timestamp {
    add(ts, duration_nanos(d))
}

/// Orders two instants.
#[must_use]
pub fn cmp(a: &Timestamp, b: &Timestamp) -> std::cmp::Ordering {
    to_nanos(a).cmp(&to_nanos(b))
}

/// Go `a.Before(b)`.
#[must_use]
pub fn before(a: &Timestamp, b: &Timestamp) -> bool {
    to_nanos(a) < to_nanos(b)
}

/// Go `a.After(b)`.
#[must_use]
pub fn after(a: &Timestamp, b: &Timestamp) -> bool {
    to_nanos(a) > to_nanos(b)
}

/// Go `time.Unix(0, 0).UTC()`.
#[must_use]
pub fn unix_epoch() -> Timestamp {
    Timestamp::default()
}

/// Go `t.UTC().Format(time.RFC3339)` (no fractional seconds).
#[must_use]
pub fn format_rfc3339(ts: &Timestamp) -> String {
    let (y, mo, d, h, mi, s, _) = to_civil(ts);
    let mut out = String::new();
    push_year(&mut out, y);
    let _ = write!(out, "-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sortable_format_matches_go() {
        // 2026-10-13T05:29:15.701265606Z (devnet fibre floor).
        let ts = Timestamp {
            seconds: 1_791_869_355,
            nanos: 701_265_606,
        };
        assert_eq!(
            format_time_bytes(&ts),
            b"2026-10-13T05:29:15.701265606".to_vec()
        );
        assert_eq!(parse_time_bytes(&format_time_bytes(&ts)), Some(ts));
        assert_eq!(
            format_time_bytes(&go_zero()),
            b"0001-01-01T00:00:00.000000000".to_vec()
        );
        assert_eq!(
            format_time_bytes(&Timestamp::default()),
            b"1970-01-01T00:00:00.000000000".to_vec()
        );
        assert_eq!(parse_time_bytes(b"2026-02-30T00:00:00.000000000"), None);
    }

    #[test]
    fn arithmetic() {
        let a = Timestamp {
            seconds: 10,
            nanos: 900_000_000,
        };
        let b = add(&a, 200_000_000);
        assert_eq!(
            b,
            Timestamp {
                seconds: 11,
                nanos: 100_000_000
            }
        );
        assert_eq!(sub(&b, &a), 200_000_000);
        assert_eq!(add(&a, -11 * NANOS_PER_SEC).seconds, -1);
        assert_eq!(
            sub(
                &go_zero(),
                &Timestamp {
                    seconds: i64::MAX / 2,
                    nanos: 0
                }
            ),
            i64::MIN
        );
        assert_eq!(format_rfc3339_nano(&a), "1970-01-01T00:00:10.9Z");
        assert_eq!(
            format_rfc3339_nano(&Timestamp::default()),
            "1970-01-01T00:00:00Z"
        );
        assert_eq!(
            duration_nanos(&duration_from_nanos(-1_500_000_000)),
            -1_500_000_000
        );
    }
}
