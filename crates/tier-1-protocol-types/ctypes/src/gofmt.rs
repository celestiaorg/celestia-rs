//! Go `fmt` behaviors that end up in error strings.

use core::fmt::Write as _;

/// `fmt.Sprintf("%X", b)`: upper-case hex.
#[must_use]
pub fn hex_upper(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        let _ = write!(s, "{x:02X}");
    }
    s
}

/// `cmtbytes.Fingerprint`: the first 6 bytes, zero padded.
#[must_use]
pub fn fingerprint(b: &[u8]) -> [u8; 6] {
    let mut out = [0u8; 6];
    let n = b.len().min(6);
    out[..n].copy_from_slice(&b[..n]);
    out
}

/// `fmt.Sprintf("%X", cmtbytes.Fingerprint(b))`.
#[must_use]
pub fn fingerprint_hex(b: &[u8]) -> String {
    hex_upper(&fingerprint(b))
}

/// `time.Duration(ns).String()`.
#[must_use]
pub fn duration_string(d: i64) -> String {
    let neg = d < 0;
    let mut u = d.unsigned_abs();
    let mut out = String::new();
    if u < 1_000_000_000 {
        if u == 0 {
            return "0s".to_owned();
        }
        let (prec, unit) = if u < 1_000 {
            (0, "ns")
        } else if u < 1_000_000 {
            (3, "\u{b5}s")
        } else {
            (6, "ms")
        };
        let (frac, rest) = fmt_frac(u, prec);
        out.push_str(&rest.to_string());
        out.push_str(&frac);
        out.push_str(unit);
    } else {
        let (frac, secs) = fmt_frac(u, 9);
        u = secs;
        let s = format!("{}{frac}s", u % 60);
        u /= 60;
        if u > 0 {
            let m = format!("{}m", u % 60);
            u /= 60;
            if u > 0 {
                let _ = write!(out, "{u}h");
            }
            out.push_str(&m);
        }
        out.push_str(&s);
    }
    if neg {
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

/// `strconv.Quote` (what `%q` prints for a string).
#[must_use]
pub fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{7}' => out.push_str("\\a"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{b}' => out.push_str("\\v"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c if c.is_control() || c.is_whitespace() && c != ' ' => {
                if (c as u32) < 0x10000 {
                    let _ = write!(out, "\\u{:04x}", c as u32);
                } else {
                    let _ = write!(out, "\\U{:08x}", c as u32);
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(duration_string(0), "0s");
        assert_eq!(duration_string(48 * 3600 * 1_000_000_000), "48h0m0s");
        assert_eq!(duration_string(1_500), "1.5\u{b5}s");
        assert_eq!(duration_string(-1), "-1ns");
        assert_eq!(duration_string(61_000_000_001), "1m1.000000001s");
        assert_eq!(duration_string(i64::MIN), "-2562047h47m16.854775808s");
    }
}
