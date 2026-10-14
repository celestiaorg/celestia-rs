//! Port of Go's `math/big` integer scanner (`(*Int).SetString`,
//! `(*Int).UnmarshalText`).
//!
//! `cosmossdk.io/math` parses integers with `SetString(s, 0)` (and
//! `UnmarshalText`, which is the same base-0 scan), so it accepts Go integer
//! literal syntax: an optional sign, the `0x`/`0X`, `0b`/`0B`, `0o`/`0O` and
//! legacy `0` (octal) prefixes, and `_` digit separators. Decimals parse their
//! digit run with base 10, which accepts only an optional sign and decimal
//! digits. Both variants must consume the entire input.

use num_bigint::{BigInt, Sign};

/// Which `SetString` base to emulate.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Base {
    /// `base == 0`: Go literal syntax (prefixes and `_` separators).
    Auto,
    /// `base == 10`: plain decimal digits.
    Ten,
}

/// Parses `s` exactly like Go's `new(big.Int).SetString(s, base)`, returning
/// `None` where Go returns `ok == false`.
pub(crate) fn parse_go_big_int(s: &[u8], base: Base) -> Option<BigInt> {
    let auto = base == Base::Auto;

    // scanSign
    let (neg, mut pos) = match s.first() {
        None => return None,
        Some(b'-') => (true, 1),
        Some(b'+') => (false, 1),
        Some(_) => (false, 0),
    };

    // nat.scan
    let mut prev = b'.';
    let mut inval_sep = false;
    let mut count = 0usize;
    let mut radix: u8 = 10;
    let mut prefix = 0u8;
    let mut ch = s.get(pos).copied();

    if auto && ch == Some(b'0') {
        prev = b'0';
        count = 1;
        pos += 1;
        ch = s.get(pos).copied();
        if let Some(c) = ch {
            (radix, prefix) = match c {
                b'b' | b'B' => (2, b'b'),
                b'o' | b'O' => (8, b'o'),
                b'x' | b'X' => (16, b'x'),
                _ => (8, b'0'),
            };
            count = 0; // prefix is not counted
            if prefix != b'0' {
                pos += 1;
                ch = s.get(pos).copied();
            }
        }
    }

    let mut digits: Vec<u8> = Vec::with_capacity(s.len());
    while let Some(c) = ch {
        if c == b'_' && auto {
            if prev != b'0' {
                inval_sep = true;
            }
            prev = b'_';
        } else {
            let d = match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'z' => c - b'a' + 10,
                b'A'..=b'Z' => c - b'A' + 10,
                _ => u8::MAX,
            };
            if d >= radix {
                // The byte does not belong to the number; Go then fails
                // because the input was not fully consumed.
                return None;
            }
            prev = b'0';
            count += 1;
            digits.push(d);
        }
        pos += 1;
        ch = s.get(pos).copied();
    }

    if inval_sep || prev == b'_' {
        return None;
    }
    if count == 0 {
        // Only the octal prefix `0` (interpreted as decimal 0) is valid.
        return (prefix == b'0').then(BigInt::default);
    }
    if digits.is_empty() {
        // The lone "0" case: the digit was consumed while probing for a prefix.
        return Some(BigInt::default());
    }
    let sign = if neg { Sign::Minus } else { Sign::Plus };
    BigInt::from_radix_be(sign, &digits, u32::from(radix))
}

#[cfg(test)]
mod tests {
    use super::{Base, parse_go_big_int};
    use num_bigint::BigInt;

    fn auto(s: &str) -> Option<String> {
        parse_go_big_int(s.as_bytes(), Base::Auto).map(|v| v.to_string())
    }

    #[test]
    fn go_literal_syntax() {
        assert_eq!(auto("0").as_deref(), Some("0"));
        assert_eq!(auto("-0").as_deref(), Some("0"));
        assert_eq!(auto("0x1F").as_deref(), Some("31"));
        assert_eq!(auto("0b101").as_deref(), Some("5"));
        assert_eq!(auto("017").as_deref(), Some("15"));
        assert_eq!(auto("0_7").as_deref(), Some("7"));
        assert_eq!(auto("1_000").as_deref(), Some("1000"));
        assert_eq!(auto("0x_1").as_deref(), Some("1"));
        assert_eq!(auto("08"), None);
        assert_eq!(auto("0_"), None);
        assert_eq!(auto("_1"), None);
        assert_eq!(auto("1__0"), None);
        assert_eq!(auto("0x"), None);
        assert_eq!(auto(""), None);
        assert_eq!(auto("-"), None);
        assert_eq!(auto(" 1"), None);
    }

    #[test]
    fn base_ten() {
        let p = |s: &str| parse_go_big_int(s.as_bytes(), Base::Ten);
        assert_eq!(p("+12"), Some(BigInt::from(12)));
        assert_eq!(p("-12"), Some(BigInt::from(-12)));
        assert_eq!(p("0x1"), None);
        assert_eq!(p("1_0"), None);
        assert_eq!(p("007"), Some(BigInt::from(7)));
    }
}
