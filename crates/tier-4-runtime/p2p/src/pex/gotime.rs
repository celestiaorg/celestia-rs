//! Go `time.Time` JSON encoding (RFC 3339 with nanoseconds) for
//! `addrbook.json`. The zero time (`0001-01-01T00:00:00Z`) is `None`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Days since 1970-01-01 for a proleptic Gregorian date (H. Hinnant).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Inverse of [`days_from_civil`].
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Format as Go's `time.Time.MarshalJSON` in UTC.
#[must_use]
pub fn format(t: Option<SystemTime>) -> String {
    let Some(t) = t else {
        return "0001-01-01T00:00:00Z".into();
    };
    let (secs, nanos) = match t.duration_since(UNIX_EPOCH) {
        Ok(d) => (
            i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
            d.subsec_nanos(),
        ),
        Err(e) => {
            let d = e.duration();
            let s = i64::try_from(d.as_secs()).unwrap_or(i64::MAX);
            if d.subsec_nanos() == 0 {
                (-s, 0)
            } else {
                (-s - 1, 1_000_000_000 - d.subsec_nanos())
            }
        }
    };
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
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
    s
}

fn num(s: &str) -> Result<i64, String> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("bad number {s:?}"));
    }
    s.parse().map_err(|e| format!("{e}"))
}

/// Parse an RFC 3339 timestamp (any offset).
#[allow(clippy::many_single_char_names)]
pub fn parse(s: &str) -> Result<Option<SystemTime>, String> {
    let b = s.as_bytes();
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return Err(format!("bad time {s:?}"));
    }
    let y = num(&s[0..4])?;
    let mo = u32::try_from(num(&s[5..7])?).map_err(|e| e.to_string())?;
    let d = u32::try_from(num(&s[8..10])?).map_err(|e| e.to_string())?;
    let h = num(&s[11..13])?;
    let mi = num(&s[14..16])?;
    let se = num(&s[17..19])?;
    let mut rest = &s[19..];
    let mut nanos: u32 = 0;
    if let Some(r) = rest.strip_prefix('.') {
        let end = r.find(|c: char| !c.is_ascii_digit()).unwrap_or(r.len());
        let digits = &r[..end];
        let mut padded = digits.chars().take(9).collect::<String>();
        while padded.len() < 9 {
            padded.push('0');
        }
        nanos = u32::try_from(num(&padded)?).map_err(|e| e.to_string())?;
        rest = &r[end..];
    }
    let offset = if rest == "Z" {
        0
    } else if rest.len() == 6
        && (rest.starts_with('+') || rest.starts_with('-'))
        && &rest[3..4] == ":"
    {
        let v = num(&rest[1..3])? * 3600 + num(&rest[4..6])? * 60;
        if rest.starts_with('-') { -v } else { v }
    } else {
        return Err(format!("bad zone in {s:?}"));
    };
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return Err(format!("bad date {s:?}"));
    }
    let secs = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se - offset;
    if y == 1 && mo == 1 && d == 1 && secs == days_from_civil(1, 1, 1) * 86_400 && nanos == 0 {
        return Ok(None);
    }
    let t = if secs >= 0 {
        UNIX_EPOCH + Duration::new(u64::try_from(secs).unwrap_or(0), nanos)
    } else {
        UNIX_EPOCH - Duration::from_secs(secs.unsigned_abs())
            + Duration::from_nanos(u64::from(nanos))
    };
    Ok(Some(t))
}

/// Serde adapter for `Option<SystemTime>` fields.
pub mod serde_go {
    use std::time::SystemTime;

    use serde::{Deserialize, Deserializer, Serializer};

    /// Serialize.
    #[allow(clippy::ref_option)]
    pub fn serialize<S: Serializer>(t: &Option<SystemTime>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::format(*t))
    }

    /// Deserialize.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<SystemTime>, D::Error> {
        let s = String::deserialize(d)?;
        super::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        assert_eq!(parse("0001-01-01T00:00:00Z").unwrap(), None);
        assert_eq!(format(None), "0001-01-01T00:00:00Z");
        let s = "2026-10-14T09:49:06.78143081Z";
        assert_eq!(format(parse(s).unwrap()), s);
        assert_eq!(
            format(parse("2026-10-14T11:49:06+02:00").unwrap()),
            "2026-10-14T09:49:06Z"
        );
        assert_eq!(format(Some(UNIX_EPOCH)), "1970-01-01T00:00:00Z");
    }
}
