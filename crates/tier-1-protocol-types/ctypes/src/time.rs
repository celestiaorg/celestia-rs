//! Go `time.Time` as carried by gogoproto `stdtime` fields, plus
//! `types/time` (`Canonical`, `WeightedMedian`).

use crate::error::{Error, Result};

/// Unix seconds of Go's zero `time.Time` (0001-01-01T00:00:00Z).
pub const GO_ZERO_UNIX_SECONDS: i64 = -62_135_596_800;
/// First invalid Unix second (10000-01-01T00:00:00Z).
pub const MAX_VALID_UNIX_SECONDS: i64 = 253_402_300_800;

/// A UTC instant with nanosecond precision (Go `time.Time` after
/// `cmttime.Canonical`). `Default` is Go's zero time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Time {
    /// Seconds since the Unix epoch.
    pub seconds: i64,
    /// Nanoseconds in `[0, 1e9)`.
    pub nanos: i32,
}

impl Default for Time {
    fn default() -> Self {
        Self::ZERO
    }
}

impl Time {
    /// Go's zero time.
    pub const ZERO: Self = Self {
        seconds: GO_ZERO_UNIX_SECONDS,
        nanos: 0,
    };

    /// A time from Unix seconds and nanoseconds.
    #[must_use]
    pub const fn from_unix(seconds: i64, nanos: i32) -> Self {
        Self { seconds, nanos }
    }

    /// Go `t.IsZero()`.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        *self == Self::ZERO
    }

    /// The gogoproto `stdtime` encoding (`types.TimestampProto`).
    #[must_use]
    pub fn to_proto(&self) -> pbjson_types::Timestamp {
        pbjson_types::Timestamp {
            seconds: self.seconds,
            nanos: self.nanos,
        }
    }

    /// Decodes a non-nullable `stdtime` field: absent means Go's zero time,
    /// out-of-range values are rejected like `types.TimestampFromProto`.
    pub fn from_proto(ts: Option<&pbjson_types::Timestamp>) -> Result<Self> {
        let Some(ts) = ts else {
            return Ok(Self::ZERO);
        };
        if ts.seconds < GO_ZERO_UNIX_SECONDS {
            return Err(Error::msg(format!(
                "timestamp: &types.Timestamp{{Seconds:{}, Nanos:{}}} before 0001-01-01",
                ts.seconds, ts.nanos
            )));
        }
        if ts.seconds >= MAX_VALID_UNIX_SECONDS {
            return Err(Error::msg(format!(
                "timestamp: &types.Timestamp{{Seconds:{}, Nanos:{}}} after 10000-01-01",
                ts.seconds, ts.nanos
            )));
        }
        if ts.nanos < 0 || ts.nanos >= 1_000_000_000 {
            return Err(Error::msg(format!(
                "timestamp: &types.Timestamp{{Seconds:{}, Nanos:{}}}: nanos not in range [0, 1e9)",
                ts.seconds, ts.nanos
            )));
        }
        Ok(Self {
            seconds: ts.seconds,
            nanos: ts.nanos,
        })
    }

    /// `CanonicalTime`: RFC3339Nano in UTC (trailing fractional zeros trimmed).
    #[must_use]
    pub fn canonical_string(&self) -> String {
        let days = self.seconds.div_euclid(86_400);
        let secs = self.seconds.rem_euclid(86_400);
        let (y, m, d) = civil_from_days(days);
        let mut s = format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            y,
            m,
            d,
            secs / 3600,
            (secs / 60) % 60,
            secs % 60
        );
        if self.nanos != 0 {
            let frac = format!("{:09}", self.nanos);
            s.push('.');
            s.push_str(frac.trim_end_matches('0'));
        }
        s.push('Z');
        s
    }

    /// `time.Time.String()` of a UTC time:
    /// `2006-01-02 15:04:05.999999999 +0000 UTC`.
    #[must_use]
    pub fn go_string(&self) -> String {
        let c = self.canonical_string();
        // "YYYY-MM-DDTHH:MM:SS[.frac]Z"
        let body = c.trim_end_matches('Z').replacen('T', " ", 1);
        format!("{body} +0000 UTC")
    }

    /// Nanoseconds since the Unix epoch as i128 (for comparisons/arithmetic).
    #[must_use]
    pub fn unix_nanos(&self) -> i128 {
        i128::from(self.seconds) * 1_000_000_000 + i128::from(self.nanos)
    }
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (
        y,
        u32::try_from(m).unwrap_or(0),
        u32::try_from(d).unwrap_or(0),
    )
}

/// `cmttime.WeightedTime`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeightedTime {
    /// Time.
    pub time: Time,
    /// Weight (voting power).
    pub weight: i64,
}

/// `cmttime.WeightedMedian`. `None` entries are skipped (sorted last).
#[must_use]
pub fn weighted_median(
    weighted_times: &mut [Option<WeightedTime>],
    total_voting_power: i64,
) -> Time {
    let mut median = total_voting_power / 2;
    weighted_times.sort_by(|a, b| match (a, b) {
        (Some(a), Some(b)) => a.time.cmp(&b.time),
        (Some(_), None) => core::cmp::Ordering::Less,
        (None, Some(_)) => core::cmp::Ordering::Greater,
        (None, None) => core::cmp::Ordering::Equal,
    });
    for wt in weighted_times.iter().flatten() {
        if median <= wt.weight {
            return wt.time;
        }
        median -= wt.weight;
    }
    Time::ZERO
}

/// Decodes a gogoproto `stdduration` field into nanoseconds
/// (`types.DurationFromProto`). Absent means zero.
pub fn duration_from_proto(d: Option<&pbjson_types::Duration>) -> Result<i64> {
    const MAX_SECONDS: i64 = 315_576_000_000;
    let Some(d) = d else { return Ok(0) };
    let repr = format!(
        "&types.Duration{{Seconds:{}, Nanos:{}}}",
        d.seconds, d.nanos
    );
    if d.seconds < -MAX_SECONDS || d.seconds > MAX_SECONDS {
        return Err(Error::msg(format!(
            "duration: {repr}: seconds out of range"
        )));
    }
    if d.nanos <= -1_000_000_000 || d.nanos >= 1_000_000_000 {
        return Err(Error::msg(format!("duration: {repr}: nanos out of range")));
    }
    if (d.seconds < 0 && d.nanos > 0) || (d.seconds > 0 && d.nanos < 0) {
        return Err(Error::msg(format!(
            "duration: {repr}: seconds and nanos have different signs"
        )));
    }
    let mut ns = d.seconds.wrapping_mul(1_000_000_000);
    if ns / 1_000_000_000 != d.seconds {
        return Err(Error::msg(format!(
            "duration: {repr} is out of range for time.Duration"
        )));
    }
    if d.nanos != 0 {
        ns = ns.wrapping_add(i64::from(d.nanos));
        if (ns < 0) != (d.nanos < 0) {
            return Err(Error::msg(format!(
                "duration: {repr} is out of range for time.Duration"
            )));
        }
    }
    Ok(ns)
}

/// Encodes nanoseconds as a gogoproto `stdduration` (`types.DurationProto`).
#[must_use]
pub fn duration_to_proto(ns: i64) -> pbjson_types::Duration {
    let secs = ns / 1_000_000_000;
    let nanos = ns - secs * 1_000_000_000;
    pbjson_types::Duration {
        seconds: secs,
        nanos: i32::try_from(nanos).unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical() {
        assert_eq!(Time::ZERO.canonical_string(), "0001-01-01T00:00:00Z");
        assert_eq!(
            Time::from_unix(1_700_000_000, 120_000_000).canonical_string(),
            "2023-11-14T22:13:20.12Z"
        );
        assert_eq!(
            Time::from_unix(0, 0).canonical_string(),
            "1970-01-01T00:00:00Z"
        );
    }
}
