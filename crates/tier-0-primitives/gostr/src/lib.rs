//! Go strings (arbitrary bytes) carried in Rust `String`s.
//!
//! A Go `string` is a byte sequence: gogoproto unmarshals a protobuf
//! `string` field without checking UTF-8, the state machine stores and
//! emits those bytes as they are, and `len` counts bytes. A Rust `String`
//! must hold UTF-8, so the protobuf crates (through `prost-gostr`) decode a
//! string field `b` to the *Go form* `E(b)` and encode `D(s)`:
//!
//! - every char of `b`'s valid UTF-8 runs is kept, except the 128 chars
//!   U+10FF80..=U+10FFFF (the reserved set R, private use / noncharacters);
//! - every byte of an invalid run, and every byte of the UTF-8 encoding of
//!   a char in R, becomes the R char `U+10FF00 + byte`.
//!
//! `D` maps each R char back to its byte and every other char to its
//! UTF-8, so `D(E(b)) == b` for all `b`, and `E(s) == s` for every valid
//! string without R chars (all practical input). Equality of Go strings is
//! equality of their Go forms. What differs from the Go bytes is the
//! encoded length (`go_len`), byte order between strings with invalid
//! bytes, and the text JSON encoders write ([`json_escape_invalid`]).
//! See `docs/adr/0005-go-strings.md`.

use std::borrow::Cow;

mod fold;

/// First char of the reserved set R; byte `b` (`0x80..=0xFF`) is
/// `char::from_u32(R_BASE + b)`.
pub const R_BASE: u32 = 0x10_FF00;

/// The R char standing for byte `b` (`b >= 0x80`).
#[inline]
fn r_char(b: u8) -> char {
    char::from_u32(R_BASE + u32::from(b)).unwrap_or(char::REPLACEMENT_CHARACTER)
}

/// The byte an R char stands for, if `c` is in R.
#[inline]
#[must_use]
pub fn r_byte(c: char) -> Option<u8> {
    let v = u32::from(c);
    if (R_BASE + 0x80..=R_BASE + 0xFF).contains(&v) {
        u8::try_from(v - R_BASE).ok()
    } else {
        None
    }
}

/// True when `s` (valid UTF-8 bytes) contains a char in R: its UTF-8 is
/// `F4 8F BE xx` or `F4 8F BF xx`.
#[inline]
fn has_r_bytes(b: &[u8]) -> bool {
    b.windows(3)
        .any(|w| w[0] == 0xF4 && w[1] == 0x8F && (w[2] == 0xBE || w[2] == 0xBF))
}

fn push_bytes_as_r(out: &mut String, bytes: &[u8]) {
    for &b in bytes {
        out.push(r_char(b));
    }
}

/// `E`: the Go form of the bytes `b` (lossless).
#[must_use]
pub fn from_go_bytes(b: &[u8]) -> Cow<'_, str> {
    if let Ok(s) = std::str::from_utf8(b)
        && !has_r_bytes(b)
    {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(b.len() + 8);
    for chunk in b.utf8_chunks() {
        for c in chunk.valid().chars() {
            if r_byte(c).is_some() {
                let mut buf = [0u8; 4];
                push_bytes_as_r(&mut out, c.encode_utf8(&mut buf).as_bytes());
            } else {
                out.push(c);
            }
        }
        push_bytes_as_r(&mut out, chunk.invalid());
    }
    Cow::Owned(out)
}

/// `E` on an owned buffer: no copy when the bytes are already a Go form.
#[must_use]
pub fn from_go_vec(b: Vec<u8>) -> String {
    match String::from_utf8(b) {
        Ok(s) if !has_r_bytes(s.as_bytes()) => s,
        Ok(s) => from_go_bytes(s.as_bytes()).into_owned(),
        Err(e) => from_go_bytes(e.as_bytes()).into_owned(),
    }
}

/// `D`: the Go bytes of the Go form `s`.
#[must_use]
pub fn to_go_bytes(s: &str) -> Cow<'_, [u8]> {
    if !has_r_bytes(s.as_bytes()) {
        return Cow::Borrowed(s.as_bytes());
    }
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        if let Some(b) = r_byte(c) {
            out.push(b);
        } else {
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
    }
    Cow::Owned(out)
}

/// Go's `len(s)`: the byte length of `D(s)`.
#[must_use]
pub fn go_len(s: &str) -> usize {
    if !has_r_bytes(s.as_bytes()) {
        return s.len();
    }
    s.chars()
        .map(|c| if r_byte(c).is_some() { 1 } else { c.len_utf8() })
        .sum()
}

/// Whether the Go string is valid UTF-8 (Go's `utf8.ValidString`).
#[must_use]
pub fn is_valid_utf8(s: &str) -> bool {
    !has_r_bytes(s.as_bytes()) || std::str::from_utf8(&to_go_bytes(s)).is_ok()
}

/// Go's per-byte decoding of invalid UTF-8 (`utf8.DecodeRune` returns
/// `(RuneError, 1)`), mapping each invalid byte with `bad`, on the Go
/// bytes `b`; the result is returned in Go form.
fn map_invalid(b: &[u8], bad: &str) -> String {
    let mut out = String::with_capacity(b.len());
    for chunk in b.utf8_chunks() {
        out.push_str(&from_go_bytes(chunk.valid().as_bytes()));
        for _ in chunk.invalid() {
            out.push_str(bad);
        }
    }
    out
}

/// Go's `encoding/json` string encoding of invalid UTF-8 applied to text
/// built from Go forms: each invalid byte of `D(s)` becomes the six
/// characters `�` (`json.Marshal`, gogoproto `jsonpb`). Returned in
/// Go form (unchanged when `s` has no invalid bytes).
#[must_use]
pub fn json_escape_invalid(s: &str) -> Cow<'_, str> {
    if is_valid_utf8(s) {
        return Cow::Borrowed(s);
    }
    Cow::Owned(map_invalid(&to_go_bytes(s), "\\ufffd"))
}

/// Each invalid byte of `D(s)` replaced by U+FFFD (what `json.Unmarshal`
/// of Go-marshalled text, `strings.ToValidUTF8`-per-byte, `[]rune(s)` and
/// `strings.Map` produce). Returned in Go form.
#[must_use]
pub fn lossy(s: &str) -> Cow<'_, str> {
    if is_valid_utf8(s) {
        return Cow::Borrowed(s);
    }
    Cow::Owned(map_invalid(&to_go_bytes(s), "\u{FFFD}"))
}

/// The Go form of a string read from text that Go would read as UTF-8
/// (JSON input): expands chars in R so that `D` restores them.
#[must_use]
pub fn canonical(s: &str) -> Cow<'_, str> {
    from_go_bytes(s.as_bytes())
}

/// Final Go bytes of JSON text built from Go forms with a Rust encoder:
/// `D`, with each invalid byte inside a JSON string escaped as Go's
/// `encoding/json` does (`�`).
#[must_use]
pub fn json_bytes(text: &str) -> Cow<'_, [u8]> {
    match json_escape_invalid(text) {
        Cow::Borrowed(t) => to_go_bytes(t),
        Cow::Owned(t) => Cow::Owned(to_go_bytes(&t).into_owned()),
    }
}

/// Go's `for _, r := range s` over the Go string: `Some(c)` for each valid
/// char, `None` for each invalid byte (Go's `RuneError` of width 1).
#[must_use]
pub fn runes(s: &str) -> Vec<Option<char>> {
    let b = to_go_bytes(s);
    let mut out = Vec::with_capacity(b.len());
    for chunk in b.utf8_chunks() {
        out.extend(chunk.valid().chars().map(Some));
        out.extend(chunk.invalid().iter().map(|_| None));
    }
    out
}

/// Appends the valid char `c` to a Go form (a char in R is appended as
/// the R chars of its UTF-8 bytes).
pub fn push_char(out: &mut String, c: char) {
    if r_byte(c).is_some() {
        let mut buf = [0u8; 4];
        push_bytes_as_r(out, c.encode_utf8(&mut buf).as_bytes());
    } else {
        out.push(c);
    }
}

/// Go's `unicode.SimpleFold`.
#[must_use]
pub fn simple_fold(r: u32) -> u32 {
    match fold::SIMPLE_FOLD.binary_search_by_key(&r, |&(a, _)| a) {
        Ok(i) => fold::SIMPLE_FOLD[i].1,
        Err(_) => r,
    }
}

/// Go's `strings.EqualFold` on two Go strings (Go forms): Unicode simple
/// case folding, each invalid UTF-8 byte read as `RuneError` (so any two
/// invalid bytes are equal).
#[must_use]
pub fn equal_fold(s: &str, t: &str) -> bool {
    let rune = |r: Option<char>| r.map_or(0xFFFD, u32::from);
    let (rs, rt) = (runes(s), runes(t));
    if rs.len() != rt.len() {
        // Folding never maps between different rune counts.
        return false;
    }
    for (x, y) in rs.into_iter().zip(rt) {
        let (mut sr, mut tr) = (rune(x), rune(y));
        if sr == tr {
            continue;
        }
        if tr < sr {
            std::mem::swap(&mut sr, &mut tr);
        }
        if tr < 0x80 {
            if (u32::from(b'A')..=u32::from(b'Z')).contains(&sr) && tr == sr + 32 {
                continue;
            }
            return false;
        }
        let mut f = simple_fold(sr);
        while f != sr && f < tr {
            f = simple_fold(f);
        }
        if f != tr {
            return false;
        }
    }
    true
}

/// Go's `strconv.IsPrint`.
#[must_use]
pub fn is_print(c: char) -> bool {
    let r = u32::from(c);
    if r < 0x80 {
        return (0x20..0x7f).contains(&r);
    }
    let i = fold::PRINT_RANGES.partition_point(|&(_, hi)| hi < r);
    fold::PRINT_RANGES.get(i).is_some_and(|&(lo, _)| lo <= r)
}

/// Go's `strconv.Quote` (`fmt`'s `%q` of a string) of a Go string (Go
/// form); the result is a Go form.
#[must_use]
pub fn quote(s: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    let b = to_go_bytes(s);
    for chunk in b.utf8_chunks() {
        for c in chunk.valid().chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                c if is_print(c) => push_char(&mut out, c),
                '\u{7}' => out.push_str("\\a"),
                '\u{8}' => out.push_str("\\b"),
                '\u{c}' => out.push_str("\\f"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\u{b}' => out.push_str("\\v"),
                c if u32::from(c) < 0x20 || c == '\u{7f}' => {
                    let _ = write!(out, "\\x{:02x}", u32::from(c));
                }
                c if u32::from(c) < 0x10000 => {
                    let _ = write!(out, "\\u{:04x}", u32::from(c));
                }
                c => {
                    let _ = write!(out, "\\U{:08x}", u32::from(c));
                }
            }
        }
        for &x in chunk.invalid() {
            let _ = write!(out, "\\x{x:02x}");
        }
    }
    out.push('"');
    out
}

/// Go's `strings.TrimSpace` emptiness test on a Go form (invalid bytes
/// and R chars are not space, like Go's `RuneError`).
#[must_use]
pub fn is_blank(s: &str) -> bool {
    s.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(b: &[u8]) {
        let s = from_go_bytes(b);
        assert_eq!(to_go_bytes(&s).as_ref(), b, "{b:x?}");
        assert_eq!(go_len(&s), b.len());
        assert_eq!(from_go_vec(b.to_vec()), s);
    }

    #[test]
    fn lossless() {
        for b in [
            &b""[..],
            b"abc",
            b"\xff",
            b"a\xffb\xfe",
            b"\xe2\x82",
            b"\xe2\x82\xac",
            b"\xf4\x8f\xbe\x80",
            b"\xf4\x8f\xbf\xbf\xff",
            b"\xf4\x8f\xbe",
            b"\xed\xa0\x80",
            b"\xc0\xaf",
            "é€😀".as_bytes(),
        ] {
            roundtrip(b);
        }
        for i in 0..=255u8 {
            for j in 0..=255u8 {
                roundtrip(&[i, j]);
                roundtrip(&[0xF4, 0x8F, i, j]);
            }
        }
    }

    #[test]
    fn valid_strings_unchanged() {
        assert!(matches!(
            from_go_bytes("moniker é".as_bytes()),
            Cow::Borrowed(_)
        ));
        assert_eq!(from_go_bytes(b"\xf4\x8f\xbf\xbf").chars().count(), 4);
        assert!(is_valid_utf8(&from_go_bytes(b"\xf4\x8f\xbf\xbf")));
        assert!(!is_valid_utf8(&from_go_bytes(b"\xff")));
    }

    #[test]
    fn equal_fold_like_go() {
        assert!(equal_fold("celestia1ABC", "CELESTIA1abc"));
        // Kelvin sign folds to k; invalid bytes are all RuneError.
        assert!(equal_fold("\u{212a}", "k"));
        assert!(equal_fold(
            &from_go_bytes(b"a\xff"),
            &from_go_bytes(b"A\xfe")
        ));
        assert!(!equal_fold(&from_go_bytes(b"a\xff"), "a\u{fffd}x"));
        assert!(equal_fold(&from_go_bytes(b"\xff"), "\u{fffd}"));
        assert!(!equal_fold("ab", "a"));
        assert_eq!(simple_fold(u32::from('A')), u32::from('a'));
    }

    #[test]
    fn quote_like_go() {
        // strconv.Quote("bad\xff") / of a control, non-printable and astral
        assert_eq!(quote(&from_go_bytes(b"bad\xff")), "\"bad\\xff\"");
        assert_eq!(
            quote("a\"\\\n\u{7f}é\u{200b}😀"),
            "\"a\\\"\\\\\\n\\x7fé\\u200b😀\""
        );
        assert!(is_print('é') && !is_print('\u{200b}') && !is_print('\u{a0}'));
    }

    #[test]
    fn go_json_and_lossy() {
        // Go: json.Marshal("a\xe2\x82b") == `"a��b"` (per byte).
        let s = from_go_bytes(b"a\xe2\x82b");
        assert_eq!(json_escape_invalid(&s), "a\\ufffd\\ufffdb");
        assert_eq!(lossy(&s), "a\u{FFFD}\u{FFFD}b");
        let t = format!("\"{s}\"");
        assert_eq!(json_bytes(&t).as_ref(), b"\"a\\ufffd\\ufffdb\"");
        let r = from_go_bytes(b"\xf4\x8f\xbf\xbf");
        assert_eq!(json_bytes(&r).as_ref(), b"\xf4\x8f\xbf\xbf");
    }
}
