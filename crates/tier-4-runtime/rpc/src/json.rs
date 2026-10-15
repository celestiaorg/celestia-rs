//! Go `encoding/json`-compatible JSON: an output tree written with Go's
//! escaping rules (HTML-safe, `\u2028`/`\u2029`, Go 1.22+ `\b`/`\f`),
//! `json.Compact` for raw messages, and a parser that reports Go's syntax
//! error messages (`invalid character 'x' looking for beginning of value`,
//! `unexpected end of JSON input`, …) and keeps every value's raw text (for
//! `json.RawMessage` semantics).

use std::fmt::Write as _;

/// A JSON value to be written exactly as Go would.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum J {
    /// `null`.
    Null,
    /// `true` / `false`.
    Bool(bool),
    /// A number, written verbatim.
    Num(String),
    /// A string (escaped on output).
    Str(String),
    /// Pre-encoded JSON, written verbatim (must already be compact and
    /// Go-escaped; see [`compact`]).
    Raw(String),
    /// An array.
    Arr(Vec<J>),
    /// An object; keys in output order.
    Obj(Vec<(String, J)>),
}

impl J {
    /// Go `int64`/`uint64` under cmtjson / jsonpb: a decimal string.
    #[must_use]
    pub fn i64s(v: i64) -> Self {
        Self::Str(v.to_string())
    }

    /// `uint64` as a decimal string.
    #[must_use]
    pub fn u64s(v: u64) -> Self {
        Self::Str(v.to_string())
    }

    /// A JSON number.
    #[must_use]
    pub fn num(v: impl std::fmt::Display) -> Self {
        Self::Num(v.to_string())
    }

    /// A string.
    #[must_use]
    pub fn str(s: impl Into<String>) -> Self {
        Self::Str(s.into())
    }

    /// `cmtbytes.HexBytes`: upper-case hex string (`""` when empty).
    #[must_use]
    pub fn hex(b: &[u8]) -> Self {
        Self::Str(hex::encode_upper(b))
    }

    /// `[]byte` that is non-nil: base64 string.
    #[must_use]
    pub fn b64(b: &[u8]) -> Self {
        use base64::Engine as _;
        Self::Str(base64::engine::general_purpose::STANDARD.encode(b))
    }

    /// `[]byte` decoded from protobuf: nil (`null`) when empty.
    #[must_use]
    pub fn b64n(b: &[u8]) -> Self {
        if b.is_empty() {
            Self::Null
        } else {
            Self::b64(b)
        }
    }

    /// `time.Time` (RFC 3339 with nanoseconds, UTC).
    #[must_use]
    pub fn time(t: &ctypes::Time) -> Self {
        Self::Str(t.canonical_string())
    }

    /// An object from `(key, value)` pairs.
    #[must_use]
    pub fn obj<const N: usize>(fields: [(&str, J); N]) -> Self {
        Self::Obj(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
    }

    /// Appends a field to an object (no-op on other values).
    pub fn push(&mut self, k: &str, v: J) {
        if let Self::Obj(f) = self {
            f.push((k.to_owned(), v));
        }
    }

    /// The compact encoding.
    #[must_use]
    pub fn encode(&self) -> String {
        let mut s = String::new();
        self.write(&mut s);
        s
    }

    /// Writes the compact encoding.
    pub fn write(&self, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Self::Num(n) | Self::Raw(n) => out.push_str(n),
            Self::Str(s) => quote_into(s, out),
            Self::Arr(a) => {
                out.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Self::Obj(f) => {
                out.push('{');
                for (i, (k, v)) in f.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    quote_into(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

/// Go `encoding/json` string encoding (with HTML escaping).
#[must_use]
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    quote_into(s, &mut out);
    out
}

fn quote_into(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '<' | '>' | '&' | '\u{2028}' | '\u{2029}' => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Go's lossy view of bytes as a string (invalid UTF-8 → U+FFFD per byte).
#[must_use]
pub fn go_string(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len());
    let mut rest = b;
    loop {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                out.push_str(s);
                return out;
            }
            Err(e) => {
                let (ok, bad) = rest.split_at(e.valid_up_to());
                out.push_str(std::str::from_utf8(ok).unwrap_or_default());
                // Go's utf8.DecodeRune consumes one byte of an invalid
                // sequence at a time.
                out.push('\u{fffd}');
                rest = &bad[1..];
            }
        }
    }
}

/// `json.Compact` with HTML escaping (what `json.Marshal` does to a
/// `json.RawMessage` / `MarshalJSON` output). The input must be valid
/// JSON.
#[must_use]
pub fn compact(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_str = false;
    let mut esc = false;
    for c in raw.chars() {
        if in_str {
            match c {
                '<' | '>' | '&' | '\u{2028}' | '\u{2029}' => {
                    let _ = write!(out, "\\u{:04x}", u32::from(c));
                }
                _ => out.push(c),
            }
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            ' ' | '\t' | '\n' | '\r' => {}
            '"' => {
                in_str = true;
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// A parsed JSON value with its raw text.
#[derive(Debug, Clone, PartialEq)]
pub struct Node<'a> {
    /// The value's text in the input (without surrounding whitespace).
    pub raw: &'a str,
    /// The value.
    pub v: Val<'a>,
}

/// A parsed JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Val<'a> {
    /// `null`.
    Null,
    /// A boolean.
    Bool(bool),
    /// A number (text).
    Num(&'a str),
    /// A string (unescaped).
    Str(String),
    /// An array.
    Arr(Vec<Node<'a>>),
    /// An object (keys in input order; duplicates kept).
    Obj(Vec<(String, Node<'a>)>),
}

impl<'a> Node<'a> {
    /// Moves the value out (a `Node` cannot be destructured: it implements
    /// `Drop`).
    #[must_use]
    pub fn into_val(mut self) -> Val<'a> {
        std::mem::replace(&mut self.v, Val::Null)
    }
}

impl Drop for Node<'_> {
    /// Drops nested containers iteratively: a tree as deep as Go accepts
    /// (10 000 levels) must not overflow a worker thread's stack.
    fn drop(&mut self) {
        if !matches!(self.v, Val::Arr(_) | Val::Obj(_)) {
            return;
        }
        let mut stack = vec![std::mem::replace(&mut self.v, Val::Null)];
        while let Some(mut v) = stack.pop() {
            match &mut v {
                Val::Arr(items) => {
                    for n in items {
                        stack.push(std::mem::replace(&mut n.v, Val::Null));
                    }
                }
                Val::Obj(fields) => {
                    for (_, n) in fields {
                        stack.push(std::mem::replace(&mut n.v, Val::Null));
                    }
                }
                _ => {}
            }
        }
    }
}

impl Val<'_> {
    /// Go's name of the JSON value kind in `UnmarshalTypeError`.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "bool",
            Self::Num(_) => "number",
            Self::Str(_) => "string",
            Self::Arr(_) => "array",
            Self::Obj(_) => "object",
        }
    }
}

impl<'a> Node<'a> {
    /// Object field lookup, last occurrence wins (Go map semantics).
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Node<'a>> {
        match &self.v {
            Val::Obj(f) => f.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The string value, if a string.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match &self.v {
            Val::Str(s) => Some(s),
            _ => None,
        }
    }
}

/// Go `quoteChar`.
#[must_use]
pub fn quote_char(c: u8) -> String {
    match c {
        b'\'' => "'\\''".to_owned(),
        b'"' => "'\"'".to_owned(),
        _ => {
            let ch = char::from(c);
            let inner = match ch {
                '\u{7}' => "\\a".to_owned(),
                '\u{8}' => "\\b".to_owned(),
                '\u{c}' => "\\f".to_owned(),
                '\n' => "\\n".to_owned(),
                '\r' => "\\r".to_owned(),
                '\t' => "\\t".to_owned(),
                '\u{b}' => "\\v".to_owned(),
                '\\' => "\\\\".to_owned(),
                c if (c as u32) < 0x20 || c as u32 == 0x7f => format!("\\x{:02x}", c as u32),
                c if (0x80..0xa0).contains(&(c as u32)) => format!("\\u{:04x}", c as u32),
                c if c as u32 == 0xad => "\\u00ad".to_owned(),
                c => c.to_string(),
            };
            format!("'{inner}'")
        }
    }
}

struct Parser<'a> {
    s: &'a str,
    b: &'a [u8],
    i: usize,
}

const EOF: &str = "unexpected end of JSON input";

/// Go `encoding/json` `maxNestingDepth`.
const MAX_NESTING_DEPTH: usize = 10_000;

fn invalid(c: u8, ctx: &str) -> String {
    format!("invalid character {} {ctx}", quote_char(c))
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    /// Parses one value without recursion (an explicit stack of open
    /// containers), enforcing Go's `maxNestingDepth`: the 10001st open
    /// array or object fails with `exceeded max depth`, exactly where Go's
    /// scanner stops. Recursion here let a JSON-RPC body of a million `[`
    /// overflow the stack and abort the node.
    fn value(&mut self) -> Result<Node<'a>, String> {
        enum Open<'a> {
            Arr(usize, Vec<Node<'a>>),
            Obj(usize, Vec<(String, Node<'a>)>, String),
        }
        let mut stack: Vec<Open<'a>> = Vec::new();
        'value: loop {
            self.ws();
            let start = self.i;
            let c = self.peek().ok_or(EOF)?;
            let mut node = match c {
                b'{' | b'[' => {
                    if stack.len() >= MAX_NESTING_DEPTH {
                        return Err(invalid(c, "exceeded max depth"));
                    }
                    self.i += 1;
                    self.ws();
                    if c == b'[' {
                        if self.peek() == Some(b']') {
                            self.i += 1;
                            Node {
                                raw: &self.s[start..self.i],
                                v: Val::Arr(Vec::new()),
                            }
                        } else {
                            stack.push(Open::Arr(start, Vec::new()));
                            continue 'value;
                        }
                    } else if self.peek() == Some(b'}') {
                        self.i += 1;
                        Node {
                            raw: &self.s[start..self.i],
                            v: Val::Obj(Vec::new()),
                        }
                    } else {
                        let k = self.object_key()?;
                        stack.push(Open::Obj(start, Vec::new(), k));
                        continue 'value;
                    }
                }
                b'"' => Node {
                    v: Val::Str(self.string()?),
                    raw: "",
                },
                b'-' | b'0'..=b'9' => {
                    self.number()?;
                    Node {
                        raw: "",
                        v: Val::Num(&self.s[start..self.i]),
                    }
                }
                b't' => {
                    self.literal("true")?;
                    Node {
                        raw: "",
                        v: Val::Bool(true),
                    }
                }
                b'f' => {
                    self.literal("false")?;
                    Node {
                        raw: "",
                        v: Val::Bool(false),
                    }
                }
                b'n' => {
                    self.literal("null")?;
                    Node {
                        raw: "",
                        v: Val::Null,
                    }
                }
                c => return Err(invalid(c, "looking for beginning of value")),
            };
            if node.raw.is_empty() {
                node.raw = &self.s[start..self.i];
            }
            // Attach the finished value to the open containers, closing
            // those that end here.
            loop {
                match stack.last_mut() {
                    None => return Ok(node),
                    Some(Open::Arr(_, items)) => {
                        items.push(node);
                        self.ws();
                        match self.peek() {
                            None => return Err(EOF.into()),
                            Some(b',') => {
                                self.i += 1;
                                continue 'value;
                            }
                            Some(b']') => self.i += 1,
                            Some(c) => return Err(invalid(c, "after array element")),
                        }
                    }
                    Some(Open::Obj(_, fields, key)) => {
                        fields.push((std::mem::take(key), node));
                        self.ws();
                        match self.peek() {
                            None => return Err(EOF.into()),
                            Some(b',') => {
                                self.i += 1;
                                *key = self.object_key()?;
                                continue 'value;
                            }
                            Some(b'}') => self.i += 1,
                            Some(c) => return Err(invalid(c, "after object key:value pair")),
                        }
                    }
                }
                node = match stack.pop() {
                    Some(Open::Arr(start, items)) => Node {
                        raw: &self.s[start..self.i],
                        v: Val::Arr(items),
                    },
                    Some(Open::Obj(start, fields, _)) => Node {
                        raw: &self.s[start..self.i],
                        v: Val::Obj(fields),
                    },
                    None => return Err(EOF.into()),
                };
            }
        }
    }

    /// An object key and its colon (after `{` or `,`).
    fn object_key(&mut self) -> Result<String, String> {
        self.ws();
        match self.peek() {
            None => return Err(EOF.into()),
            Some(b'"') => {}
            Some(c) => {
                return Err(invalid(c, "looking for beginning of object key string"));
            }
        }
        let k = self.string()?;
        self.ws();
        match self.peek() {
            None => Err(EOF.into()),
            Some(b':') => {
                self.i += 1;
                Ok(k)
            }
            Some(c) => Err(invalid(c, "after object key")),
        }
    }

    fn literal(&mut self, lit: &str) -> Result<(), String> {
        let lb = lit.as_bytes();
        self.i += 1;
        for &want in &lb[1..] {
            match self.peek() {
                None => return Err(EOF.into()),
                Some(c) if c == want => self.i += 1,
                Some(c) => {
                    return Err(invalid(
                        c,
                        &format!("in literal {lit} (expecting {})", quote_char(want)),
                    ));
                }
            }
        }
        Ok(())
    }

    fn digits(&mut self) {
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.i += 1;
        }
    }

    fn number(&mut self) -> Result<(), String> {
        if self.peek() == Some(b'-') {
            self.i += 1;
            match self.peek() {
                None => return Err(EOF.into()),
                Some(b'0'..=b'9') => {}
                Some(c) => return Err(invalid(c, "in numeric literal")),
            }
        }
        if self.peek() == Some(b'0') {
            self.i += 1;
        } else {
            self.digits();
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            match self.peek() {
                None => return Err(EOF.into()),
                Some(b'0'..=b'9') => self.digits(),
                Some(c) => return Err(invalid(c, "after decimal point in numeric literal")),
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            match self.peek() {
                None => return Err(EOF.into()),
                Some(b'0'..=b'9') => self.digits(),
                Some(c) => return Err(invalid(c, "in exponent of numeric literal")),
            }
        }
        Ok(())
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let mut v = 0u32;
        for _ in 0..4 {
            let c = self.peek().ok_or(EOF)?;
            let d = char::from(c)
                .to_digit(16)
                .ok_or_else(|| invalid(c, "in \\u hexadecimal character escape"))?;
            v = v * 16 + d;
            self.i += 1;
        }
        Ok(v)
    }

    fn string(&mut self) -> Result<String, String> {
        self.i += 1; // opening quote
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = self.peek().ok_or(EOF)?;
            match c {
                b'"' => {
                    self.i += 1;
                    return Ok(go_string(&out));
                }
                b'\\' => {
                    self.i += 1;
                    let e = self.peek().ok_or(EOF)?;
                    self.i += 1;
                    match e {
                        b'"' | b'\\' | b'/' => out.push(e),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let mut r = self.hex4()?;
                            if (0xd800..0xdc00).contains(&r)
                                && self.b.get(self.i) == Some(&b'\\')
                                && self.b.get(self.i + 1) == Some(&b'u')
                            {
                                let save = self.i;
                                self.i += 2;
                                let r2 = self.hex4()?;
                                if (0xdc00..0xe000).contains(&r2) {
                                    r = 0x10000 + ((r - 0xd800) << 10) + (r2 - 0xdc00);
                                } else {
                                    self.i = save;
                                    r = 0xfffd;
                                }
                            } else if (0xd800..0xe000).contains(&r) {
                                r = 0xfffd;
                            }
                            let ch = char::from_u32(r).unwrap_or('\u{fffd}');
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        c => return Err(invalid(c, "in string escape code")),
                    }
                }
                c if c < 0x20 => return Err(invalid(c, "in string literal")),
                c => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
    }
}

/// Parses one JSON document (Go `json.Unmarshal` syntax checking).
pub fn parse(s: &str) -> Result<Node<'_>, String> {
    let mut p = Parser {
        s,
        b: s.as_bytes(),
        i: 0,
    };
    let n = p.value()?;
    p.ws();
    if let Some(c) = p.peek() {
        return Err(invalid(c, "after top-level value"));
    }
    Ok(n)
}

/// Validates without building a tree (Go `json.Valid` / `checkValid`).
pub fn check_valid(s: &str) -> Result<(), String> {
    parse(s).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_matches_go() {
        assert_eq!(
            quote("a<b>&\"\\\n\u{1}\u{8}\u{2028}é"),
            r#""a\u003cb\u003e\u0026\"\\\n\u0001\b\u2028é""#
        );
    }

    /// Regression (fuzzing): a request body of nested arrays overflowed the
    /// stack of the recursive parser and aborted the node. Go accepts
    /// 10 000 levels and fails the next with `exceeded max depth`.
    #[test]
    fn nesting_depth_matches_go_without_recursion() {
        let run = || {
            let ok = format!("{}{}", "[".repeat(10_000), "]".repeat(10_000));
            assert!(parse(&ok).is_ok());
            let obj = format!("{}1{}", "{\"a\":".repeat(10_000), "}".repeat(10_000));
            assert!(parse(&obj).is_ok());
            assert_eq!(
                parse(&"[".repeat(10_001)).err().as_deref(),
                Some("invalid character '[' exceeded max depth")
            );
            assert_eq!(
                parse(&format!("{}{{}}", "[".repeat(10_000)))
                    .err()
                    .as_deref(),
                Some("invalid character '{' exceeded max depth")
            );
            assert_eq!(
                parse(&"[".repeat(1_000_000)).err().as_deref(),
                Some("invalid character '[' exceeded max depth")
            );
            // An earlier syntax error wins, as in Go's scanner.
            assert_eq!(
                parse(&format!("[x{}", "[".repeat(20_000))).err().as_deref(),
                Some("invalid character 'x' looking for beginning of value")
            );
        };
        // A tokio worker's stack size.
        std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(run)
            .map(|h| h.join().is_ok())
            .map_or_else(|_| panic!("spawn"), |ok| assert!(ok));
    }

    #[test]
    fn errors_match_go() {
        assert_eq!(
            parse("{bad json").unwrap_err(),
            "invalid character 'b' looking for beginning of object key string"
        );
        assert_eq!(parse("").unwrap_err(), "unexpected end of JSON input");
        assert_eq!(
            parse("[1,]").unwrap_err(),
            "invalid character ']' looking for beginning of value"
        );
        assert_eq!(
            parse("{\"a\" 1}").unwrap_err(),
            "invalid character '1' after object key"
        );
        assert_eq!(
            parse("1 2").unwrap_err(),
            "invalid character '2' after top-level value"
        );
        assert_eq!(parse("tru").unwrap_err(), "unexpected end of JSON input");
        assert_eq!(
            parse("trx").unwrap_err(),
            "invalid character 'x' in literal true (expecting 'u')"
        );
        assert_eq!(
            parse("-a").unwrap_err(),
            "invalid character 'a' in numeric literal"
        );
        assert_eq!(
            parse("\"\\x\"").unwrap_err(),
            "invalid character 'x' in string escape code"
        );
        assert_eq!(
            parse("\"\n\"").unwrap_err(),
            "invalid character '\\n' in string literal"
        );
    }

    #[test]
    fn compact_raw() {
        assert_eq!(
            compact("{ \"a\" : [1, 2], \"b\": \"x <y>\" }"),
            r#"{"a":[1,2],"b":"x \u003cy\u003e"}"#
        );
    }

    #[test]
    fn raw_spans() {
        let n = parse(r#" {"a": {"b": [1, "x"]}, "c": 5} "#).unwrap();
        assert_eq!(n.get("a").unwrap().raw, r#"{"b": [1, "x"]}"#);
        assert_eq!(n.get("c").unwrap().raw, "5");
    }
}
