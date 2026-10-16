//! Go `encoding/json` decoding (Go 1.26, the default v1 package) as the
//! consensus paths of celestia-app use it: the validity scanner with its
//! limit of 10 000 nested arrays and objects (`maxNestingDepth`, counted
//! over both kinds together), `json.Unmarshal`'s whole-input check
//! (`checkValid`), `json.Decoder`'s first value (gogoproto `jsonpb`),
//! string unquoting (invalid UTF-8 and lone surrogates become U+FFFD) and a
//! value tree.
//!
//! Nothing here recurses: the scanner keeps an explicit stack, the tree is
//! built and dropped iteratively. serde_json stops at 128 levels (and its
//! `IgnoredAny` never stops), so it cannot decide what Go accepts.

use std::ops::Range;

/// Go `encoding/json` `maxNestingDepth`: the 10 001st open array or object
/// fails with `exceeded max depth`.
pub const MAX_NESTING_DEPTH: usize = 10_000;

/// Go's `quoteChar` for scanner errors.
fn quote_char(c: u8) -> String {
    match c {
        b'\'' => "'\\''".into(),
        b'"' => "'\"'".into(),
        _ => {
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
                _ => char::from(c).to_string(),
            };
            format!("'{s}'")
        }
    }
}

fn syntax(c: u8, ctx: &str) -> String {
    format!("invalid character {} {ctx}", quote_char(c))
}

/// `checkValid`'s error at the end of the input.
pub const UNEXPECTED_END: &str = "unexpected end of JSON input";

fn ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

#[derive(Clone, Copy)]
enum Ctx {
    Top,
    ObjKey,
    ObjVal,
    Arr,
}

struct Scan<'a> {
    s: &'a [u8],
    i: usize,
    depth: usize,
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
        let c = self.peek().ok_or_else(|| UNEXPECTED_END.to_owned())?;
        self.i += 1;
        Ok(c)
    }

    /// Scans one value with an explicit stack of open containers, failing
    /// like Go's scanner when the 10 001st container opens.
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
                    self.depth = self.depth.max(open.len() + 1);
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
                _ => Err(UNEXPECTED_END.into()),
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

    /// The rest of a string literal (any byte from 0x20 up is accepted,
    /// valid UTF-8 or not).
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

/// A scanned value: where it is and how deeply it nests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scanned {
    /// The value's bytes (no surrounding whitespace).
    pub span: Range<usize>,
    /// The deepest nesting of arrays and objects (0 for a scalar).
    pub depth: usize,
}

/// `json.Unmarshal`'s `checkValid`: `b` holds exactly one value (and
/// whitespace), nested at most [`MAX_NESTING_DEPTH`] levels.
pub fn check_valid(b: &[u8]) -> Result<Scanned, String> {
    let mut sc = Scan {
        s: b,
        i: 0,
        depth: 0,
    };
    sc.skip_ws();
    let start = sc.i;
    sc.value()?;
    let end = sc.i;
    sc.end_value(Ctx::Top)?;
    Ok(Scanned {
        span: start..end,
        depth: sc.depth,
    })
}

/// `json.Decoder.Decode` of the first value of `b` (what follows it is not
/// read): `EOF` for an empty (or all-whitespace) input, `unexpected EOF`
/// for a truncated value, the scanner's error otherwise.
pub fn first_value(b: &[u8]) -> Result<Scanned, String> {
    let mut sc = Scan {
        s: b,
        i: 0,
        depth: 0,
    };
    sc.skip_ws();
    if sc.peek().is_none() {
        return Err("EOF".into());
    }
    let start = sc.i;
    match sc.value() {
        Ok(()) => Ok(Scanned {
            span: start..sc.i,
            depth: sc.depth,
        }),
        Err(e) if e == UNEXPECTED_END => Err("unexpected EOF".into()),
        Err(e) => Err(e),
    }
}

fn getu4(s: &[u8]) -> Option<u32> {
    if s.len() < 6 || s[0] != b'\\' || s[1] != b'u' {
        return None;
    }
    let mut r = 0u32;
    for &c in &s[2..6] {
        r = r * 16 + char::from(c).to_digit(16)?;
    }
    Some(r)
}

/// Go `unquote` of a string literal the scanner accepted (quotes
/// included): escapes decoded, a valid surrogate pair joined, a lone
/// surrogate and each invalid UTF-8 byte replaced by U+FFFD.
#[must_use]
pub fn unquote(raw: &[u8]) -> String {
    let s = raw.get(1..raw.len().saturating_sub(1)).unwrap_or_default();
    let mut out = String::with_capacity(s.len());
    let mut r = 0;
    while r < s.len() {
        let c = s[r];
        if c == b'\\' {
            let e = s.get(r + 1).copied().unwrap_or_default();
            r += 2;
            out.push(match e {
                b'b' => '\u{8}',
                b'f' => '\u{c}',
                b'n' => '\n',
                b'r' => '\r',
                b't' => '\t',
                b'u' => {
                    let rr = getu4(&s[r - 2..]).unwrap_or(0xfffd);
                    r += 4;
                    if (0xd800..0xe000).contains(&rr) {
                        match getu4(&s[r..]) {
                            Some(rr1)
                                if (0xd800..0xdc00).contains(&rr)
                                    && (0xdc00..0xe000).contains(&rr1) =>
                            {
                                r += 6;
                                char::from_u32(0x10000 + ((rr - 0xd800) << 10) + (rr1 - 0xdc00))
                                    .unwrap_or('\u{fffd}')
                            }
                            _ => '\u{fffd}',
                        }
                    } else {
                        char::from_u32(rr).unwrap_or('\u{fffd}')
                    }
                }
                other => char::from(other),
            });
            continue;
        }
        if c < 0x80 {
            out.push(char::from(c));
            r += 1;
            continue;
        }
        // `utf8.DecodeRune`: the longest valid prefix, else one byte.
        let n = match c {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => 0,
        };
        if let Some(ch) = s
            .get(r..r + n)
            .filter(|_| n > 0)
            .and_then(|b| std::str::from_utf8(b).ok())
        {
            out.push_str(ch);
            r += n;
        } else {
            out.push('\u{fffd}');
            r += 1;
        }
    }
    out
}

/// A decoded JSON value with its position in the input.
#[derive(Debug)]
pub struct Value {
    /// The value.
    pub kind: Kind,
    /// Its bytes in the input.
    pub span: Range<usize>,
}

/// A JSON value. Objects keep every member in input order, duplicates
/// included (Go decodes them one after the other).
#[derive(Debug)]
pub enum Kind {
    /// `null`.
    Null,
    /// `true` / `false`.
    Bool(bool),
    /// A number literal as written.
    Number(String),
    /// A string, unquoted.
    String(String),
    /// An array.
    Array(Vec<Value>),
    /// An object.
    Object(Vec<(String, Value)>),
}

impl Kind {
    /// The JSON type name Go's `UnmarshalTypeError` reports.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "bool",
            Self::Number(_) => "number",
            Self::String(_) => "string",
            Self::Array(_) => "array",
            Self::Object(_) => "object",
        }
    }
}

impl Value {
    fn take_children(&mut self, out: &mut Vec<Value>) {
        match &mut self.kind {
            Kind::Array(items) => out.append(items),
            Kind::Object(members) => out.extend(members.drain(..).map(|(_, v)| v)),
            _ => {}
        }
    }

    /// Calls `f` on this value and every value nested in it (pre-order,
    /// without recursion); stops at the first `false`.
    pub fn all(&self, mut f: impl FnMut(&Value) -> bool) -> bool {
        let mut stack: Vec<&Value> = vec![self];
        while let Some(v) = stack.pop() {
            if !f(v) {
                return false;
            }
            match &v.kind {
                Kind::Array(items) => stack.extend(items.iter().rev()),
                Kind::Object(members) => stack.extend(members.iter().rev().map(|(_, v)| v)),
                _ => {}
            }
        }
        true
    }
}

impl Drop for Value {
    fn drop(&mut self) {
        let mut stack = Vec::new();
        self.take_children(&mut stack);
        while let Some(mut v) = stack.pop() {
            v.take_children(&mut stack);
        }
    }
}

enum Frame {
    Arr(usize, Vec<Value>),
    Obj(usize, Vec<(String, Value)>, String),
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while b.get(i).copied().is_some_and(ws) {
        i += 1;
    }
    i
}

/// The end of the string literal at `i` (a scanned input).
fn string_end(b: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    i
}

/// Builds the tree of a value the scanner accepted.
#[allow(clippy::too_many_lines)]
fn build(b: &[u8], span: Range<usize>) -> Value {
    let mut stack: Vec<Frame> = Vec::new();
    let mut i = span.start;
    loop {
        i = skip_ws(b, i);
        let start = i;
        let mut done = match b.get(i).copied().unwrap_or(b'n') {
            b'[' => {
                i = skip_ws(b, i + 1);
                if b.get(i) == Some(&b']') {
                    i += 1;
                    Value {
                        kind: Kind::Array(Vec::new()),
                        span: start..i,
                    }
                } else {
                    stack.push(Frame::Arr(start, Vec::new()));
                    continue;
                }
            }
            b'{' => {
                i = skip_ws(b, i + 1);
                if b.get(i) == Some(&b'}') {
                    i += 1;
                    Value {
                        kind: Kind::Object(Vec::new()),
                        span: start..i,
                    }
                } else {
                    let e = string_end(b, i);
                    let key = unquote(&b[i..e]);
                    i = skip_ws(b, e) + 1; // ':'
                    stack.push(Frame::Obj(start, Vec::new(), key));
                    continue;
                }
            }
            b'"' => {
                i = string_end(b, i);
                Value {
                    kind: Kind::String(unquote(&b[start..i])),
                    span: start..i,
                }
            }
            b't' => {
                i += 4;
                Value {
                    kind: Kind::Bool(true),
                    span: start..i,
                }
            }
            b'f' => {
                i += 5;
                Value {
                    kind: Kind::Bool(false),
                    span: start..i,
                }
            }
            b'n' => {
                i += 4;
                Value {
                    kind: Kind::Null,
                    span: start..i,
                }
            }
            _ => {
                while b
                    .get(i)
                    .is_some_and(|c| matches!(c, b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'))
                {
                    i += 1;
                }
                Value {
                    kind: Kind::Number(String::from_utf8_lossy(&b[start..i]).into_owned()),
                    span: start..i,
                }
            }
        };
        // Attach the finished value; close the containers it ends.
        loop {
            let Some(frame) = stack.last_mut() else {
                return done;
            };
            i = skip_ws(b, i);
            let close = b.get(i).copied().unwrap_or(b',');
            i += 1;
            match frame {
                Frame::Arr(_, items) => items.push(done),
                Frame::Obj(_, members, key) => members.push((std::mem::take(key), done)),
            }
            if close == b',' {
                if let Some(Frame::Obj(_, _, key)) = stack.last_mut() {
                    let ks = skip_ws(b, i);
                    let e = string_end(b, ks);
                    *key = unquote(&b[ks..e]);
                    i = skip_ws(b, e) + 1; // ':'
                }
                break;
            }
            done = match stack.pop() {
                Some(Frame::Arr(s, items)) => Value {
                    kind: Kind::Array(items),
                    span: s..i,
                },
                Some(Frame::Obj(s, members, _)) => Value {
                    kind: Kind::Object(members),
                    span: s..i,
                },
                None => unreachable!("a frame was just attached to"),
            };
        }
    }
}

/// `json.Unmarshal`'s view of `b`: the value tree after [`check_valid`].
pub fn parse(b: &[u8]) -> Result<Value, String> {
    let sc = check_valid(b)?;
    Ok(build(b, sc.span))
}

/// `json.Decoder.Decode`'s view of `b`: the tree of [`first_value`].
pub fn parse_first(b: &[u8]) -> Result<Value, String> {
    let sc = first_value(b)?;
    Ok(build(b, sc.span))
}

/// Inputs longer than this run [`on_large_stack`] work on its own thread.
/// Shorter ones nest at most 512 levels, which ordinary recursion handles.
pub const LARGE_INPUT: usize = 1024;

/// Runs `f` on a thread with a 256 MiB stack (reserved, touched only as
/// used) when `input_len` exceeds [`LARGE_INPUT`]: recursive code over a
/// value Go accepts (10 000 levels) needs more than a default thread stack.
pub fn on_large_stack<T: Send>(input_len: usize, f: impl FnOnce() -> T + Send) -> T {
    if input_len <= LARGE_INPUT {
        return f();
    }
    // The closure stays reachable here if the spawn fails (no thread or
    // address space left): it then runs on this stack.
    let job = std::sync::Mutex::new(Some(f));
    let run = || job.lock().ok().and_then(|mut j| j.take()).map(|f| f());
    let out = std::thread::scope(|s| {
        std::thread::Builder::new()
            .name("gojson-deep".into())
            .stack_size(256 << 20)
            .spawn_scoped(s, run)
            .ok()
            .map(|h| match h.join() {
                Ok(v) => v,
                Err(e) => std::panic::resume_unwind(e),
            })
    });
    match out.flatten() {
        Some(v) => v,
        None => match job.into_inner().ok().flatten() {
            Some(f) => f(),
            None => unreachable!("the job ran"),
        },
    }
}

/// Go `base64.StdEncoding.Decode` (padding required, `\r` / `\n` skipped,
/// trailing bits not checked).
#[must_use]
pub fn decode_base64_std(src: &[u8]) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let skip_nl = |si: &mut usize| {
        while *si < src.len() && (src[*si] == b'\n' || src[*si] == b'\r') {
            *si += 1;
        }
    };
    let mut out = Vec::with_capacity(src.len() / 4 * 3);
    let mut si = 0;
    // `decodeQuantum` until the input ends.
    loop {
        let mut dbuf = [0u8; 4];
        let mut dlen = 4;
        let mut j = 0;
        let mut done = false;
        while j < 4 {
            if si == src.len() {
                if j == 0 {
                    return Some(out);
                }
                // j == 1, or padding missing.
                return None;
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
                return None;
            }
            match j {
                0 | 1 => return None,
                2 => {
                    skip_nl(&mut si);
                    if si == src.len() || src[si] != b'=' {
                        return None;
                    }
                    si += 1;
                }
                _ => {}
            }
            skip_nl(&mut si);
            if si < src.len() {
                return None;
            }
            dlen = j;
            done = true;
            break;
        }
        let v = u32::from(dbuf[0]) << 18
            | u32::from(dbuf[1]) << 12
            | u32::from(dbuf[2]) << 6
            | u32::from(dbuf[3]);
        let b = v.to_be_bytes();
        out.extend_from_slice(&b[1..dlen]);
        if done {
            return Some(out);
        }
    }
}

/// `json.Unmarshal(raw, &b)` for a Go `[]byte`: `None` for `null` (a nil
/// slice), standard base64 for a string, one number (0..=255, `null` = 0)
/// per element for an array; `Err` otherwise.
pub fn bytes_value(v: &Value) -> Result<Option<Vec<u8>>, String> {
    match &v.kind {
        Kind::Null => Ok(None),
        Kind::String(s) => decode_base64_std(s.as_bytes())
            .map(Some)
            .ok_or_else(|| "illegal base64 data".to_owned()),
        Kind::Array(items) => items
            .iter()
            .map(|it| match &it.kind {
                Kind::Null => Ok(0),
                Kind::Number(n) => n.parse::<u8>().map_err(|_| {
                    format!("json: cannot unmarshal number {n} into Go value of type uint8")
                }),
                k => Err(format!(
                    "json: cannot unmarshal {} into Go value of type uint8",
                    k.type_name()
                )),
            })
            .collect::<Result<Vec<u8>, String>>()
            .map(Some),
        k => Err(format!(
            "json: cannot unmarshal {} into Go value of type []uint8",
            k.type_name()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested(open: &str, close: &str, n: usize, inner: &str) -> String {
        format!("{}{inner}{}", open.repeat(n), close.repeat(n))
    }

    #[test]
    fn depth_limit_is_go_s() {
        for (o, c) in [("[", "]"), ("{\"a\":", "}")] {
            assert_eq!(
                check_valid(nested(o, c, 10_000, "1").as_bytes())
                    .unwrap()
                    .depth,
                10_000
            );
            let e = check_valid(nested(o, c, 10_001, "1").as_bytes()).unwrap_err();
            assert!(e.ends_with("exceeded max depth"), "{e}");
        }
        // Arrays and objects count together; empty containers count too.
        let mixed = format!("{}[]{}", "[{\"a\":".repeat(5_000), "}]".repeat(5_000));
        assert!(check_valid(mixed.as_bytes()).is_err());
        let mixed = format!("{}{{}}{}", "[{\"a\":".repeat(4_999), "}]".repeat(4_999));
        assert_eq!(check_valid(mixed.as_bytes()).unwrap().depth, 9_999);
        // A deep tree builds and drops without recursion.
        let v = parse(nested("[", "]", 10_000, "{}").as_bytes());
        assert!(v.is_err());
        assert!(parse(nested("[", "]", 9_999, "{\"k\":[1,\"x\"]}").as_bytes()).is_err());
        let v = parse(nested("[", "]", 9_998, "{\"k\":[1,\"x\"]}").as_bytes()).unwrap();
        let mut n = 0;
        assert!(v.all(|_| {
            n += 1;
            true
        }));
        assert_eq!(n, 9_998 + 4);
    }

    #[test]
    fn decoder_first_value() {
        assert_eq!(first_value(b"  {\"a\":1} x").unwrap().span, 2..9);
        assert_eq!(first_value(b"1x").unwrap().span, 0..1);
        assert_eq!(first_value(b" \n").unwrap_err(), "EOF");
        assert_eq!(first_value(b"{\"a\":").unwrap_err(), "unexpected EOF");
        assert!(first_value(b"[1,]").is_err());
        assert!(check_valid(b"{\"a\":1} x").is_err());
    }

    #[test]
    fn tree_and_unquote() {
        let v = parse(
            br#" {"a" : [1, -2.5e3, true, null], "a":"\u00e9\ud83d\ude00\ud800x\udc00", "b":{}} "#,
        )
        .unwrap();
        let Kind::Object(m) = &v.kind else { panic!() };
        assert_eq!(m.len(), 3);
        assert_eq!(m[0].0, "a");
        let Kind::Array(items) = &m[0].1.kind else {
            panic!()
        };
        assert!(matches!(&items[1].kind, Kind::Number(n) if n == "-2.5e3"));
        assert!(matches!(&m[1].1.kind, Kind::String(s) if s == "\u{e9}\u{1f600}\u{fffd}x\u{fffd}"));
        assert_eq!(m[2].1.span, 76..78);
        assert_eq!(unquote(b"\"a\xffb\xe2\x82\""), "a\u{fffd}b\u{fffd}\u{fffd}");
        assert_eq!(unquote(b"\"\\ud800\\u0041\""), "\u{fffd}A");
    }

    #[test]
    fn large_stack() {
        assert_eq!(on_large_stack(10, || 1), 1);
        assert_eq!(on_large_stack(1 << 20, || 2), 2);
    }
    #[test]
    fn base64_like_go() {
        assert_eq!(super::decode_base64_std(b""), Some(vec![]));
        assert_eq!(super::decode_base64_std(b"AQI="), Some(vec![1, 2]));
        assert_eq!(super::decode_base64_std(b"AQ=="), Some(vec![1]));
        assert_eq!(super::decode_base64_std(b"A\nQ\r=\n="), Some(vec![1]));
        assert_eq!(super::decode_base64_std(b"AQ"), None);
        assert_eq!(super::decode_base64_std(b"AQ=="), Some(vec![1]));
        assert_eq!(super::decode_base64_std(b"AQ==AAAA"), None);
        assert_eq!(super::decode_base64_std(b"AQ=A"), None);
        assert_eq!(super::decode_base64_std(b"A==="), None);
        assert_eq!(super::decode_base64_std(b"AR=="), Some(vec![1]));
        assert_eq!(super::decode_base64_std(b"-_8="), None);
    }
}
