//! The parts of Go's `encoding/json` go-amino calls into: string encoding,
//! the validity scanner (with its error messages), `RawMessage` splitting
//! and typed `json.Unmarshal` of scalars.

/// Go `json.Marshal(string)` (HTML-escaped, invalid UTF-8 as `�`).
pub(crate) fn encode_string(out: &mut Vec<u8>, s: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push(b'"');
    let mut i = 0;
    while i < s.len() {
        let b = s[i];
        if b < 0x80 {
            match b {
                b'\\' => out.extend_from_slice(b"\\\\"),
                b'"' => out.extend_from_slice(b"\\\""),
                0x08 => out.extend_from_slice(b"\\b"),
                0x0c => out.extend_from_slice(b"\\f"),
                b'\n' => out.extend_from_slice(b"\\n"),
                b'\r' => out.extend_from_slice(b"\\r"),
                b'\t' => out.extend_from_slice(b"\\t"),
                b'<' | b'>' | b'&' | 0..0x20 => {
                    out.extend_from_slice(b"\\u00");
                    out.push(HEX[usize::from(b >> 4)]);
                    out.push(HEX[usize::from(b & 0xf)]);
                }
                _ => out.push(b),
            }
            i += 1;
            continue;
        }
        match decode_rune(&s[i..]) {
            None => {
                out.extend_from_slice(b"\\ufffd");
                i += 1;
            }
            Some((c, n)) => {
                if c == '\u{2028}' || c == '\u{2029}' {
                    out.extend_from_slice(if c == '\u{2028}' {
                        b"\\u2028"
                    } else {
                        b"\\u2029"
                    });
                } else {
                    out.extend_from_slice(&s[i..i + n]);
                }
                i += n;
            }
        }
    }
    out.push(b'"');
}

/// Go `utf8.DecodeRune` for a valid encoding (`None`: `RuneError`, size 1).
fn decode_rune(s: &[u8]) -> Option<(char, usize)> {
    let n = match s.first()? {
        0..0x80 => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return None,
    };
    let c = std::str::from_utf8(s.get(..n)?).ok()?.chars().next()?;
    Some((c, n))
}

/// Go `quoteChar` for scanner errors.
fn quote_char(c: u8) -> String {
    match c {
        b'\'' => "'\\''".into(),
        b'"' => "'\"'".into(),
        _ => {
            let r = char::from(c);
            let s = match c {
                0x07 => "\\a".into(),
                0x08 => "\\b".into(),
                0x0c => "\\f".into(),
                b'\n' => "\\n".into(),
                b'\r' => "\\r".into(),
                b'\t' => "\\t".into(),
                0x0b => "\\v".into(),
                b'\\' => "\\\\".into(),
                0..0x20 | 0x7f => format!("\\x{c:02x}"),
                0x80..=0xa0 | 0xad => format!("\\u{:04x}", u32::from(c)),
                _ => r.to_string(),
            };
            format!("'{s}'")
        }
    }
}

fn syntax(c: u8, ctx: &str) -> String {
    format!("invalid character {} {ctx}", quote_char(c))
}

const EOF: &str = "unexpected end of JSON input";

/// Go `encoding/json` `maxNestingDepth`.
const MAX_NESTING_DEPTH: usize = 10_000;

fn ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

struct Scan<'a> {
    s: &'a [u8],
    i: usize,
}

#[derive(Clone, Copy)]
enum Ctx {
    Top,
    ObjKey,
    ObjVal,
    Arr,
}

impl Scan<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(ws) {
            self.i += 1;
        }
    }

    fn next(&mut self) -> Result<u8, String> {
        let c = self.peek().ok_or_else(|| EOF.to_owned())?;
        self.i += 1;
        Ok(c)
    }

    /// Scans one value without recursion (an explicit stack of open
    /// containers), failing like Go's scanner at the 10 001st nested
    /// container (`exceeded max depth`). The recursive scanner overflowed
    /// the stack on a deeply nested `TxEncodeAmino` request.
    fn value(&mut self) -> Result<(), String> {
        let mut open: Vec<u8> = Vec::new();
        loop {
            self.skip_ws();
            let c = self.next()?;
            match c {
                b'{' | b'[' => {
                    if open.len() >= MAX_NESTING_DEPTH {
                        return Err(syntax(c, "exceeded max depth"));
                    }
                    self.skip_ws();
                    if c == b'[' {
                        if self.peek() == Some(b']') {
                            self.i += 1;
                        } else {
                            open.push(b'[');
                            continue;
                        }
                    } else if self.peek() == Some(b'}') {
                        self.i += 1;
                    } else {
                        self.object_key()?;
                        open.push(b'{');
                        continue;
                    }
                }
                b'"' => self.string()?,
                b'-' | b'0'..=b'9' => self.number(c)?,
                b't' => self.literal(b"rue", "true")?,
                b'f' => self.literal(b"alse", "false")?,
                b'n' => self.literal(b"ull", "null")?,
                _ => return Err(syntax(c, "looking for beginning of value")),
            }
            // A value ended: close the containers that end with it.
            loop {
                match open.last() {
                    None => return Ok(()),
                    Some(b'[') => {
                        if self.end_value(Ctx::Arr)? != Some(b']') {
                            break;
                        }
                    }
                    Some(_) => {
                        if self.end_value(Ctx::ObjVal)? != Some(b'}') {
                            self.object_key()?;
                            break;
                        }
                    }
                }
                open.pop();
            }
        }
    }

    /// An object key and its colon.
    fn object_key(&mut self) -> Result<(), String> {
        self.skip_ws();
        let c = self.next()?;
        if c != b'"' {
            return Err(syntax(c, "looking for beginning of object key string"));
        }
        self.string()?;
        self.end_value(Ctx::ObjKey).map(|_| ())
    }

    fn end_value(&mut self, ctx: Ctx) -> Result<Option<u8>, String> {
        self.skip_ws();
        let Some(c) = self.peek() else {
            return match ctx {
                Ctx::Top => Ok(None),
                _ => Err(EOF.into()),
            };
        };
        let want: &[u8] = match ctx {
            Ctx::Top => b"",
            Ctx::ObjKey => b":",
            Ctx::ObjVal => b",}",
            Ctx::Arr => b",]",
        };
        if want.contains(&c) {
            self.i += 1;
            return Ok(Some(c));
        }
        Err(syntax(
            c,
            match ctx {
                Ctx::Top => "after top-level value",
                Ctx::ObjKey => "after object key",
                Ctx::ObjVal => "after object key:value pair",
                Ctx::Arr => "after array element",
            },
        ))
    }

    fn string(&mut self) -> Result<(), String> {
        loop {
            let c = self.next()?;
            match c {
                b'"' => return Ok(()),
                b'\\' => {
                    let e = self.next()?;
                    match e {
                        b'b' | b'f' | b'n' | b'r' | b't' | b'\\' | b'/' | b'"' => {}
                        b'u' => {
                            for _ in 0..4 {
                                let h = self.next()?;
                                if !h.is_ascii_hexdigit() {
                                    return Err(syntax(h, "in \\u hexadecimal character escape"));
                                }
                            }
                        }
                        _ => return Err(syntax(e, "in string escape code")),
                    }
                }
                0..0x20 => return Err(syntax(c, "in string literal")),
                _ => {}
            }
        }
    }

    fn digits(&mut self) {
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
    }

    fn number(&mut self, first: u8) -> Result<(), String> {
        let mut c = first;
        if c == b'-' {
            c = self.next()?;
            if !c.is_ascii_digit() {
                return Err(syntax(c, "in numeric literal"));
            }
        }
        if c != b'0' {
            self.digits();
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            let d = self.next()?;
            if !d.is_ascii_digit() {
                return Err(syntax(d, "after decimal point in numeric literal"));
            }
            self.digits();
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.i += 1;
            let mut d = self.next()?;
            if d == b'+' || d == b'-' {
                d = self.next()?;
            }
            if !d.is_ascii_digit() {
                return Err(syntax(d, "in exponent of numeric literal"));
            }
            self.digits();
        }
        Ok(())
    }

    fn literal(&mut self, rest: &[u8], name: &str) -> Result<(), String> {
        for &want in rest {
            let c = self.next()?;
            if c != want {
                return Err(syntax(
                    c,
                    &format!("in literal {name} (expecting {})", quote_char(want)),
                ));
            }
        }
        Ok(())
    }
}

/// Go `checkValid`: the scanner error for invalid JSON.
pub(crate) fn check_valid(s: &[u8]) -> Result<(), String> {
    let mut sc = Scan { s, i: 0 };
    sc.value()?;
    sc.end_value(Ctx::Top).map(|_| ())
}

/// End of the (valid) value starting at `i` (after whitespace).
fn value_end(s: &[u8], i: usize) -> usize {
    let mut sc = Scan { s, i };
    let _ = sc.value();
    sc.i
}

fn skip(s: &[u8], mut i: usize) -> usize {
    while s.get(i).copied().is_some_and(ws) {
        i += 1;
    }
    i
}

/// Trims whitespace around a (valid) value.
pub(crate) fn trim(s: &[u8]) -> &[u8] {
    let a = skip(s, 0);
    let b = value_end(s, a);
    &s[a..b]
}

/// The JSON kind Go names in `UnmarshalTypeError` (`None` for null).
pub(crate) fn kind_name(raw: &[u8]) -> Option<&'static str> {
    match raw.first() {
        Some(b'{') => Some("object"),
        Some(b'[') => Some("array"),
        Some(b'"') => Some("string"),
        Some(b't' | b'f') => Some("bool"),
        Some(b'n') => None,
        _ => Some("number"),
    }
}

pub(crate) fn type_error(kind: &str, go: &str) -> String {
    format!("json: cannot unmarshal {kind} into Go value of type {go}")
}

/// Members of a (valid, trimmed) object: decoded key and raw value.
pub(crate) fn object_entries(raw: &[u8]) -> Vec<(Vec<u8>, &[u8])> {
    let mut out = Vec::new();
    let mut i = skip(raw, 1);
    if raw.get(i) == Some(&b'}') {
        return out;
    }
    loop {
        let ke = value_end(raw, i);
        let key = unquote(&raw[i..ke]);
        i = skip(raw, ke) + 1; // ':'
        let vs = skip(raw, i);
        let ve = value_end(raw, vs);
        out.push((key, &raw[vs..ve]));
        i = skip(raw, ve);
        if raw.get(i) != Some(&b',') {
            return out;
        }
        i = skip(raw, i + 1);
    }
}

/// Elements of a (valid, trimmed) array.
pub(crate) fn array_items(raw: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut i = skip(raw, 1);
    if raw.get(i) == Some(&b']') {
        return out;
    }
    loop {
        let ve = value_end(raw, i);
        out.push(&raw[i..ve]);
        i = skip(raw, ve);
        if raw.get(i) != Some(&b',') {
            return out;
        }
        i = skip(raw, i + 1);
    }
}

fn hex4(s: &[u8]) -> Option<u32> {
    u32::from_str_radix(std::str::from_utf8(s.get(..4)?).ok()?, 16).ok()
}

fn push_char(out: &mut Vec<u8>, c: char) {
    let mut b = [0u8; 4];
    out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
}

/// Go `unquote` of a (valid) string literal.
pub(crate) fn unquote(raw: &[u8]) -> Vec<u8> {
    let s = &raw[1..raw.len() - 1];
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if c == b'\\' {
            let e = s.get(i + 1).copied().unwrap_or(b'\\');
            match e {
                b'u' => {
                    let r = hex4(&s[i + 2..]).unwrap_or(0xfffd);
                    i += 6;
                    if (0xd800..0xe000).contains(&r) {
                        let pair = (s.get(i) == Some(&b'\\') && s.get(i + 1) == Some(&b'u'))
                            .then(|| hex4(&s[i + 2..]))
                            .flatten()
                            .filter(|lo| r < 0xdc00 && (0xdc00..0xe000).contains(lo));
                        if let Some(lo) = pair {
                            let c = 0x10000 + ((r - 0xd800) << 10) + (lo - 0xdc00);
                            push_char(&mut out, char::from_u32(c).unwrap_or('\u{fffd}'));
                            i += 6;
                        } else {
                            push_char(&mut out, '\u{fffd}');
                        }
                    } else {
                        push_char(&mut out, char::from_u32(r).unwrap_or('\u{fffd}'));
                    }
                    continue;
                }
                b'b' => out.push(0x08),
                b'f' => out.push(0x0c),
                b'n' => out.push(b'\n'),
                b'r' => out.push(b'\r'),
                b't' => out.push(b'\t'),
                other => out.push(other),
            }
            i += 2;
        } else if c < 0x80 {
            out.push(c);
            i += 1;
        } else if let Some((_, n)) = decode_rune(&s[i..]) {
            out.extend_from_slice(&s[i..i + n]);
            i += n;
        } else {
            push_char(&mut out, '\u{fffd}');
            i += 1;
        }
    }
    out
}

/// `json.Unmarshal(raw, &string)`: `Ok(None)` for null.
pub(crate) fn unmarshal_string(raw: &[u8], go: &str) -> Result<Option<Vec<u8>>, String> {
    match kind_name(raw) {
        None => Ok(None),
        Some("string") => Ok(Some(unquote(raw))),
        Some(k) => Err(type_error(k, go)),
    }
}

/// `json.Unmarshal(raw, &bool)`.
pub(crate) fn unmarshal_bool(raw: &[u8], go: &str) -> Result<Option<bool>, String> {
    match raw.first() {
        Some(b't') => Ok(Some(true)),
        Some(b'f') => Ok(Some(false)),
        Some(b'n') => Ok(None),
        _ => Err(type_error(kind_name(raw).unwrap_or("null"), go)),
    }
}

/// `json.Unmarshal(raw, &intN)` / `&uintN` (`raw` is any valid value).
pub(crate) fn unmarshal_int(
    raw: &[u8],
    go: &str,
    signed: bool,
    bits: u32,
) -> Result<Option<i128>, String> {
    match kind_name(raw) {
        None => return Ok(None),
        Some("number") => {}
        Some(k) => return Err(type_error(k, go)),
    }
    let s = std::str::from_utf8(raw).unwrap_or_default();
    let parsed: Option<i128> = if signed {
        s.parse::<i64>().ok().map(i128::from).filter(|v| {
            let lim = 1i128 << (bits - 1);
            *v >= -lim && *v < lim
        })
    } else {
        s.parse::<u64>()
            .ok()
            .map(i128::from)
            .filter(|v| bits == 64 || *v < (1i128 << bits))
    };
    // Go's ParseInt rejects a leading '+'; JSON numbers never have one.
    parsed
        .map(Some)
        .ok_or_else(|| type_error(&format!("number {s}"), go))
}

/// Go `base64.StdEncoding.Decode` (newlines skipped).
pub(crate) fn base64_decode(src: &[u8]) -> Result<Vec<u8>, String> {
    fn val(c: u8) -> Option<u32> {
        Some(u32::from(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        }))
    }
    let corrupt = |at: usize| format!("illegal base64 data at input byte {at}");
    let mut out = Vec::with_capacity(src.len() / 4 * 3);
    let mut si = 0;
    while si < src.len() {
        let mut dbuf = [0u32; 4];
        let mut dlen = 4;
        let mut trailing = None;
        let mut j = 0;
        while j < 4 {
            if si == src.len() {
                if j == 0 {
                    return Ok(out);
                }
                return Err(corrupt(si - j));
            }
            let c = src[si];
            si += 1;
            if let Some(v) = val(c) {
                dbuf[j] = v;
                j += 1;
                continue;
            }
            if c == b'\n' || c == b'\r' {
                continue;
            }
            if c != b'=' {
                return Err(corrupt(si - 1));
            }
            match j {
                0 | 1 => return Err(corrupt(si - 1)),
                2 => {
                    while si < src.len() && (src[si] == b'\n' || src[si] == b'\r') {
                        si += 1;
                    }
                    if si == src.len() {
                        return Err(corrupt(src.len()));
                    }
                    if src[si] != b'=' {
                        return Err(corrupt(si - 1));
                    }
                    si += 1;
                }
                _ => {}
            }
            while si < src.len() && (src[si] == b'\n' || src[si] == b'\r') {
                si += 1;
            }
            if si < src.len() {
                trailing = Some(si);
            }
            dlen = j;
            break;
        }
        let v = dbuf[0] << 18 | dbuf[1] << 12 | dbuf[2] << 6 | dbuf[3];
        let bytes = [(v >> 16) as u8, (v >> 8) as u8, v as u8];
        out.extend_from_slice(&bytes[..dlen - 1]);
        if let Some(at) = trailing {
            return Err(corrupt(at));
        }
        if dlen < 4 {
            return Ok(out);
        }
    }
    Ok(out)
}

/// `json.Unmarshal(raw, &[]byte)`: `Ok(None)` for null.
pub(crate) fn unmarshal_bytes(raw: &[u8], go: &str) -> Result<Option<Vec<u8>>, String> {
    match unmarshal_string(raw, go)? {
        None => Ok(None),
        Some(s) => base64_decode(&s).map(Some),
    }
}

/// Go `strconv.Quote` (approximately: ASCII escapes, valid UTF-8 kept).
pub(crate) fn go_quote(s: &[u8]) -> String {
    let mut out = String::from("\"");
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if c < 0x80 {
            match c {
                b'"' => out.push_str("\\\""),
                b'\\' => out.push_str("\\\\"),
                0x07 => out.push_str("\\a"),
                0x08 => out.push_str("\\b"),
                0x0c => out.push_str("\\f"),
                b'\n' => out.push_str("\\n"),
                b'\r' => out.push_str("\\r"),
                b'\t' => out.push_str("\\t"),
                0x0b => out.push_str("\\v"),
                0..0x20 | 0x7f => out.push_str(&hex_escape(c)),
                _ => out.push(char::from(c)),
            }
            i += 1;
        } else if let Some((ch, n)) = decode_rune(&s[i..]) {
            out.push(ch);
            i += n;
        } else {
            out.push_str(&hex_escape(c));
            i += 1;
        }
    }
    out.push('"');
    out
}

fn hex_escape(c: u8) -> String {
    format!("\\x{c:02x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scanner_errors() {
        assert_eq!(check_valid(b""), Err(EOF.into()));
        assert_eq!(
            check_valid(b"{\"a\" 1}"),
            Err("invalid character '1' after object key".into())
        );
        assert_eq!(
            check_valid(b"[1 2]"),
            Err("invalid character '2' after array element".into())
        );
        assert_eq!(
            check_valid(b"{\"a\":1 \"b\"}"),
            Err("invalid character '\"' after object key:value pair".into())
        );
        assert_eq!(check_valid(b"tru"), Err(EOF.into()));
        assert_eq!(
            check_valid(b"trUe"),
            Err("invalid character 'U' in literal true (expecting 'u')".into())
        );
        assert_eq!(
            check_valid(b"01"),
            Err("invalid character '1' after top-level value".into())
        );
        assert_eq!(
            check_valid(b" {\"a\": [1, -0.5e+3, \"x\\u00e9\"]} "),
            Ok(())
        );
    }

    /// Regression (fuzzing): the recursive scanner overflowed the stack on
    /// a deeply nested `TxEncodeAmino` request; Go fails the 10 001st level
    /// with `exceeded max depth`.
    #[test]
    fn nesting_depth_without_recursion() {
        let run = || {
            let ok = format!("{}{}", "[".repeat(10_000), "]".repeat(10_000));
            assert_eq!(check_valid(ok.as_bytes()), Ok(()));
            let obj = format!("{}1{}", "{\"a\":".repeat(10_000), "}".repeat(10_000));
            assert_eq!(check_valid(obj.as_bytes()), Ok(()));
            assert_eq!(value_end(ok.as_bytes(), 0), ok.len());
            assert_eq!(
                check_valid("[".repeat(10_001).as_bytes()),
                Err("invalid character '[' exceeded max depth".into())
            );
            assert_eq!(
                check_valid("[".repeat(1_000_000).as_bytes()),
                Err("invalid character '[' exceeded max depth".into())
            );
            assert_eq!(
                check_valid(b"{\"a\":1,}"),
                Err("invalid character '}' looking for beginning of object key string".into())
            );
        };
        let ok = std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(run)
            .map(|h| h.join().is_ok());
        assert_eq!(ok.ok(), Some(true));
    }

    #[test]
    fn strings() {
        let mut o = Vec::new();
        encode_string(&mut o, b"a<b>&\n\x01\xff\xe2\x80\xa8\x08");
        assert_eq!(o, b"\"a\\u003cb\\u003e\\u0026\\n\\u0001\\ufffd\\u2028\\b\"");
        assert_eq!(
            unquote(b"\"a\\u00e9\\ud83d\\ude00\\ud800x\""),
            "a\u{e9}\u{1f600}\u{fffd}x".as_bytes()
        );
        assert_eq!(base64_decode(b"aGk="), Ok(b"hi".to_vec()));
        assert_eq!(
            base64_decode(b"aGk"),
            Err("illegal base64 data at input byte 0".into())
        );
        assert_eq!(
            base64_decode(b"a!=="),
            Err("illegal base64 data at input byte 1".into())
        );
    }
}
