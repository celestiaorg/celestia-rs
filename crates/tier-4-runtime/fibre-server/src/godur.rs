//! Go `time.ParseDuration` and `time.Duration.String`, for the `[health]`
//! durations of the configuration and the texts of health messages.

// A line-by-line port of Go's parser and formatter, with Go's names.
#![allow(clippy::many_single_char_names, clippy::too_many_lines)]

use std::fmt::Write as _;
use std::time::Duration;

/// Go `time.ParseDuration`: a signed sequence of decimal numbers with
/// units (`ns`, `us`, `µs`, `μs`, `ms`, `s`, `m`, `h`), in nanoseconds.
/// The error is Go's text.
pub fn parse(s: &str) -> Result<i64, String> {
    let orig = s;
    let quoted = gostr::quote(orig);
    let invalid = || format!("time: invalid duration {quoted}");
    let mut s = s;
    let mut neg = false;
    if let Some(c) = s.chars().next()
        && (c == '-' || c == '+')
    {
        neg = c == '-';
        s = &s[1..];
    }
    if s == "0" {
        return Ok(0);
    }
    if s.is_empty() {
        return Err(invalid());
    }
    let mut d: u64 = 0;
    while !s.is_empty() {
        let b = s.as_bytes();
        if !(b[0] == b'.' || b[0].is_ascii_digit()) {
            return Err(invalid());
        }
        // Integer part.
        let int_len = b.iter().take_while(|c| c.is_ascii_digit()).count();
        let pre = int_len > 0;
        let mut v: u64 = 0;
        for &c in &b[..int_len] {
            v = v
                .checked_mul(10)
                .and_then(|v| v.checked_add(u64::from(c - b'0')))
                .ok_or_else(invalid)?;
            if v > 1 << 63 {
                return Err(invalid());
            }
        }
        s = &s[int_len..];
        // Fraction.
        let mut f: u64 = 0;
        let mut scale: f64 = 1.0;
        let mut post = false;
        if s.starts_with('.') {
            s = &s[1..];
            let b = s.as_bytes();
            let frac_len = b.iter().take_while(|c| c.is_ascii_digit()).count();
            post = frac_len > 0;
            let mut overflow = false;
            for &c in &b[..frac_len] {
                if overflow {
                    continue;
                }
                if f > (1u64 << 63) / 10 {
                    overflow = true;
                    continue;
                }
                let y = f * 10 + u64::from(c - b'0');
                if y > 1 << 63 {
                    overflow = true;
                    continue;
                }
                f = y;
                scale *= 10.0;
            }
            s = &s[frac_len..];
        }
        if !pre && !post {
            return Err(invalid());
        }
        // Unit.
        let unit_len = s
            .char_indices()
            .find(|&(_, c)| c == '.' || c.is_ascii_digit())
            .map_or(s.len(), |(i, _)| i);
        if unit_len == 0 {
            return Err(format!("time: missing unit in duration {quoted}"));
        }
        let unit = &s[..unit_len];
        s = &s[unit_len..];
        let unit_nanos: u64 = match unit {
            "ns" => 1,
            "us" | "µs" | "μs" => 1_000,
            "ms" => 1_000_000,
            "s" => 1_000_000_000,
            "m" => 60_000_000_000,
            "h" => 3_600_000_000_000,
            _ => {
                return Err(format!(
                    "time: unknown unit {} in duration {quoted}",
                    gostr::quote(unit)
                ));
            }
        };
        if v > (1u64 << 63) / unit_nanos {
            return Err(invalid());
        }
        v *= unit_nanos;
        if f > 0 {
            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss
            )]
            let frac = (f as f64 * (unit_nanos as f64 / scale)) as u64;
            v = v.checked_add(frac).ok_or_else(invalid)?;
            if v > 1 << 63 {
                return Err(invalid());
            }
        }
        d = d.checked_add(v).ok_or_else(invalid)?;
        if d > 1 << 63 {
            return Err(invalid());
        }
    }
    if neg {
        #[allow(clippy::cast_possible_wrap)]
        return Ok((d as i64).wrapping_neg());
    }
    if d > (1u64 << 63) - 1 {
        return Err(invalid());
    }
    #[allow(clippy::cast_possible_wrap)]
    Ok(d as i64)
}

/// Go `time.Duration.String` of a non-negative duration (`2m0s`, `1.5s`,
/// `300ms`).
#[must_use]
pub fn format(d: Duration) -> String {
    let u = u64::try_from(d.as_nanos()).unwrap_or(u64::MAX);
    if u < 1_000_000_000 {
        if u == 0 {
            return "0s".into();
        }
        let (prec, unit) = if u < 1_000 {
            (0, "ns")
        } else if u < 1_000_000 {
            (3, "µs")
        } else {
            (6, "ms")
        };
        return format!("{}{unit}", fmt_frac(u, prec));
    }
    let secs_total = u / 1_000_000_000;
    let frac = u % 1_000_000_000;
    let mut out = String::new();
    let h = secs_total / 3600;
    let m = (secs_total / 60) % 60;
    let s = secs_total % 60;
    if h > 0 {
        let _ = write!(out, "{h}h");
    }
    if h > 0 || m > 0 {
        let _ = write!(out, "{m}m");
    }
    out.push_str(&fmt_frac(s * 1_000_000_000 + frac, 9));
    out.push('s');
    out
}

/// `v / 10^prec` with the fraction's trailing zeros (and a bare dot)
/// removed, as Go's `fmtFrac` + `fmtInt`.
fn fmt_frac(v: u64, prec: u32) -> String {
    let p = 10u64.pow(prec);
    let int = v / p;
    let frac = v % p;
    if frac == 0 {
        return int.to_string();
    }
    let mut digits = format!("{frac:0width$}", width = prec as usize);
    while digits.ends_with('0') {
        digits.pop();
    }
    format!("{int}.{digits}")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn parse_matches_go() {
        assert_eq!(parse("10s").unwrap(), 10_000_000_000);
        assert_eq!(parse("2m").unwrap(), 120_000_000_000);
        assert_eq!(parse("1.5s").unwrap(), 1_500_000_000);
        assert_eq!(parse("1h30m").unwrap(), 5_400_000_000_000);
        assert_eq!(parse("500ms").unwrap(), 500_000_000);
        assert_eq!(parse("0").unwrap(), 0);
        assert_eq!(parse("-3s").unwrap(), -3_000_000_000);
        assert_eq!(parse("1µs").unwrap(), 1_000);
        assert_eq!(parse("").unwrap_err(), "time: invalid duration \"\"");
        assert_eq!(
            parse("10").unwrap_err(),
            "time: missing unit in duration \"10\""
        );
        assert_eq!(
            parse("3x").unwrap_err(),
            "time: unknown unit \"x\" in duration \"3x\""
        );
        assert_eq!(parse(".s").unwrap_err(), "time: invalid duration \".s\"");
    }

    #[test]
    fn format_matches_go() {
        assert_eq!(format(Duration::from_secs(120)), "2m0s");
        assert_eq!(format(Duration::from_secs(3600)), "1h0m0s");
        assert_eq!(format(Duration::from_secs(10)), "10s");
        assert_eq!(format(Duration::from_millis(1500)), "1.5s");
        assert_eq!(format(Duration::from_millis(300)), "300ms");
        assert_eq!(format(Duration::from_micros(1)), "1µs");
        assert_eq!(format(Duration::from_nanos(5)), "5ns");
        assert_eq!(format(Duration::ZERO), "0s");
        assert_eq!(format(Duration::from_secs(3661)), "1h1m1s");
    }
}
