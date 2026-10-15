//! `time.Time` as go-amino handles it (UTC, Unix seconds + nanoseconds).

/// Unix seconds of `time.Time{}` (0001-01-01T00:00:00Z).
pub(crate) const ZERO_SECS: i64 = -62_135_596_800;
/// go-amino `minSeconds` / `maxSeconds`.
pub(crate) const MIN_SECS: i64 = -62_135_596_800;
pub(crate) const MAX_SECS: i64 = 253_402_300_800;

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let m = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Go `t.UTC().Format(time.RFC3339Nano)`, or `None` when `MarshalJSON`
/// fails (year outside `[0, 9999]`).
pub(crate) fn format_rfc3339_nano(secs: i64, nanos: u32) -> Option<String> {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    if !(0..=9999).contains(&y) {
        return None;
    }
    let mut s = format!(
        "{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    );
    if nanos != 0 {
        let frac = format!("{nanos:09}");
        s.push('.');
        s.push_str(frac.trim_end_matches('0'));
    }
    s.push('Z');
    Some(s)
}

fn is_leap(y: i64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

fn days_in(y: i64, m: u32) -> u32 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn digits(s: &[u8]) -> Option<u32> {
    if s.is_empty() || !s.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(s).ok()?.parse().ok()
}

/// Go `time.Time.UnmarshalJSON` of a quoted RFC 3339 string ending in `Z`
/// (go-amino checks the `Z` first): Unix seconds and nanoseconds.
pub(crate) fn parse_rfc3339(quoted: &[u8]) -> Result<(i64, u32), String> {
    let inner = &quoted[1..quoted.len() - 1];
    let bad = || {
        format!(
            "parsing time {} as \"2006-01-02T15:04:05Z07:00\": cannot parse {} as \"2006\"",
            String::from_utf8_lossy(quoted),
            String::from_utf8_lossy(quoted)
        )
    };
    if inner.len() < 20
        || inner[4] != b'-'
        || inner[7] != b'-'
        || inner[10] != b'T'
        || inner[13] != b':'
        || inner[16] != b':'
    {
        return Err(bad());
    }
    let y = digits(&inner[0..4]).ok_or_else(bad)?;
    let mo = digits(&inner[5..7]).ok_or_else(bad)?;
    let d = digits(&inner[8..10]).ok_or_else(bad)?;
    let h = digits(&inner[11..13]).ok_or_else(bad)?;
    let mi = digits(&inner[14..16]).ok_or_else(bad)?;
    let se = digits(&inner[17..19]).ok_or_else(bad)?;
    let mut rest = &inner[19..];
    let mut nanos = 0u32;
    if rest.first() == Some(&b'.') {
        let n = rest[1..].iter().take_while(|c| c.is_ascii_digit()).count();
        if n == 0 || n > 9 {
            return Err(bad());
        }
        let frac = std::str::from_utf8(&rest[1..=n]).map_err(|_| bad())?;
        nanos = format!("{frac:0<9}").parse().map_err(|_| bad())?;
        rest = &rest[1 + n..];
    }
    if rest != b"Z" {
        return Err(bad());
    }
    let range = |what: &str| {
        format!(
            "parsing time {}: {what} out of range",
            String::from_utf8_lossy(quoted)
        )
    };
    if !(1..=12).contains(&mo) {
        return Err(range("month"));
    }
    let y = i64::from(y);
    if d < 1 || d > days_in(y, mo) {
        return Err(range("day"));
    }
    if h > 23 {
        return Err(range("hour"));
    }
    if mi > 59 {
        return Err(range("minute"));
    }
    if se > 59 {
        return Err(range("second"));
    }
    let secs = days_from_civil(y, mo, d) * 86_400 + i64::from(h * 3600 + mi * 60 + se);
    Ok((secs, nanos))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        assert_eq!(
            format_rfc3339_nano(0, 0).as_deref(),
            Some("1970-01-01T00:00:00Z")
        );
        assert_eq!(
            format_rfc3339_nano(ZERO_SECS, 0).as_deref(),
            Some("0001-01-01T00:00:00Z")
        );
        assert_eq!(
            format_rfc3339_nano(1_700_000_000, 120_000_000).as_deref(),
            Some("2023-11-14T22:13:20.12Z")
        );
        assert_eq!(
            parse_rfc3339(b"\"2023-11-14T22:13:20.12Z\""),
            Ok((1_700_000_000, 120_000_000))
        );
        assert_eq!(
            parse_rfc3339(b"\"0001-01-01T00:00:00Z\""),
            Ok((ZERO_SECS, 0))
        );
    }
}
