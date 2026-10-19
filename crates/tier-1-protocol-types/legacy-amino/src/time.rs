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

/// Go's `time.quote`: quoted, every byte outside 0x20..0x80 as `\xNN`,
/// `"` and `\\` escaped.
fn go_quote(s: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("\"");
    for &b in s {
        if (0x20..0x80).contains(&b) {
            if b == b'"' || b == b'\\' {
                out.push('\\');
            }
            out.push(char::from(b));
        } else {
            let _ = write!(out, "\\x{b:02x}");
        }
    }
    out.push('"');
    out
}

/// `time.Parse(time.RFC3339, value)` (Go 1.26 `format.go` `parse` over
/// the layout's chunks `2006` `-` `01` `-` `02` `T` `15` `:` `04` `:` `05`
/// `Z07:00`): Unix seconds and nanoseconds, or Go's `ParseError` text.
fn go_parse_rfc3339(value: &[u8]) -> Result<(i64, u32), String> {
    const LAYOUT: &str = "2006-01-02T15:04:05Z07:00";
    let elem_err = |layout_elem: &str, value_elem: &[u8]| {
        format!(
            "parsing time {} as {}: cannot parse {} as {}",
            go_quote(value),
            go_quote(LAYOUT.as_bytes()),
            go_quote(value_elem),
            go_quote(layout_elem.as_bytes())
        )
    };
    let msg_err = |msg: &str| format!("parsing time {}{msg}", go_quote(value));
    let digit = |s: &[u8], i: usize| s.get(i).is_some_and(u8::is_ascii_digit);
    // `getnum`: one or two digits (two when `fixed`).
    let getnum = |s: &[u8], fixed: bool| -> Option<(i64, usize)> {
        if !digit(s, 0) {
            return None;
        }
        if !digit(s, 1) {
            return (!fixed).then(|| (i64::from(s[0] - b'0'), 1));
        }
        Some((i64::from(s[0] - b'0') * 10 + i64::from(s[1] - b'0'), 2))
    };
    let mut v = value;
    // `skip(value, prefix)` for a one-byte prefix.
    let skip = |v: &mut &[u8], c: u8| -> Result<(), String> {
        if v.first() == Some(&c) {
            *v = &v[1..];
            Ok(())
        } else {
            Err(elem_err(std::str::from_utf8(&[c]).unwrap_or(""), v))
        }
    };
    // 2006: four bytes, the first a digit, `atoi`.
    if v.len() < 4 || !digit(v, 0) {
        return Err(elem_err("2006", v));
    }
    let year = match std::str::from_utf8(&v[..4])
        .ok()
        .filter(|p| p.bytes().all(|c| c.is_ascii_digit()))
    {
        Some(p) => p.parse::<i64>().map_err(|_| elem_err("2006", v))?,
        None => return Err(elem_err("2006", v)),
    };
    v = &v[4..];
    skip(&mut v, b'-')?;
    let Some((month, n)) = getnum(v, true) else {
        return Err(elem_err("01", v));
    };
    v = &v[n..];
    if !(1..=12).contains(&month) {
        return Err(msg_err(": month out of range"));
    }
    skip(&mut v, b'-')?;
    let Some((day, n)) = getnum(v, true) else {
        return Err(elem_err("02", v));
    };
    v = &v[n..];
    skip(&mut v, b'T')?;
    let Some((hour, n)) = getnum(v, false) else {
        return Err(elem_err("15", v));
    };
    v = &v[n..];
    if hour >= 24 {
        return Err(msg_err(": hour out of range"));
    }
    skip(&mut v, b':')?;
    let Some((min, n)) = getnum(v, true) else {
        return Err(elem_err("04", v));
    };
    v = &v[n..];
    if min >= 60 {
        return Err(msg_err(": minute out of range"));
    }
    skip(&mut v, b':')?;
    let Some((sec, n)) = getnum(v, true) else {
        return Err(elem_err("05", v));
    };
    v = &v[n..];
    if sec >= 60 {
        return Err(msg_err(": second out of range"));
    }
    // A fractional second the layout does not name (`parseNanoseconds`:
    // at most nine digits are read, all are skipped).
    let mut nsec: i64 = 0;
    if v.len() >= 2 && (v[0] == b'.' || v[0] == b',') && digit(v, 1) {
        let mut n = 2;
        while digit(v, n) {
            n += 1;
        }
        let nb = n.min(10);
        let digits = std::str::from_utf8(&v[1..nb]).unwrap_or("0");
        nsec = digits.parse::<i64>().unwrap_or(0);
        for _ in 0..10 - nb {
            nsec *= 10;
        }
        v = &v[n..];
    }
    // Z07:00.
    let hold = v;
    let offset: i64 = if v.first() == Some(&b'Z') {
        v = &v[1..];
        0
    } else {
        if v.len() < 6 || v[3] != b':' {
            return Err(elem_err("Z07:00", hold));
        }
        // `getnum(hour, true)`, then the minute only if the hour parsed;
        // a range error is reported before a parse error (minute last).
        let hr = getnum(&v[1..3], true).map(|(n, _)| n);
        let mm = hr.and_then(|_| getnum(&v[4..6], true).map(|(n, _)| n));
        let sign = v[0];
        v = &v[6..];
        let range = if mm.unwrap_or(0) > 60 {
            Some("minute")
        } else if hr.unwrap_or(0) > 24 {
            Some("hour")
        } else {
            None
        };
        if let Some(what) = range {
            return Err(format!(
                "parsing time {}: time zone offset {what} out of range",
                go_quote(value)
            ));
        }
        let (Some(hr), Some(mm)) = (hr, mm) else {
            return Err(elem_err("Z07:00", hold));
        };
        let off = (hr * 60 + mm) * 60;
        match sign {
            b'+' => off,
            b'-' => -off,
            _ => return Err(elem_err("Z07:00", hold)),
        }
    };
    if !v.is_empty() {
        return Err(msg_err(&format!(": extra text: {}", go_quote(v))));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (m, d) = (month as u32, day as u32);
    if day < 1 || d > days_in(year, m) {
        return Err(msg_err(": day out of range"));
    }
    let secs = days_from_civil(year, m, d) * 86_400 + hour * 3600 + min * 60 + sec - offset;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok((secs, nsec as u32))
}

/// Go `time.Time.UnmarshalJSON` of a quoted string (go-amino checked the
/// quotes and the final `Z`): Go 1.26's `parseStrictRFC3339` has its strict
/// checks disabled, so this is `time.Parse(time.RFC3339, ...)` of the bytes
/// between the quotes (not JSON-unescaped). Unix seconds and nanoseconds.
pub(crate) fn parse_rfc3339(quoted: &[u8]) -> Result<(i64, u32), String> {
    go_parse_rfc3339(&quoted[1..quoted.len() - 1])
}
