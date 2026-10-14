//! The Go JSON behaviors PFM's memo handling depends on: `encoding/json`
//! decoding of the memo into `PacketMetadata` (case-insensitive field
//! matching, type errors, the custom `Duration` decoder), the
//! `iancoleman/orderedmap`-backed `JSONObject` that re-encodes `next` (key
//! order preserved, duplicates moved to the end, numbers as `float64`), and
//! `json.Marshal`'s compaction of a marshaler's output.

use x_ibc::gojson::{escape_string, format_float};

/// A parsed JSON value with its exact source text.
#[derive(Debug, Clone, PartialEq)]
pub struct Val {
    /// The decoded value.
    pub kind: Kind,
    /// The value's source text.
    pub raw: String,
}

/// A JSON value; objects keep every member, duplicates included.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// `null`.
    Null,
    /// `true` / `false`.
    Bool(bool),
    /// A number literal.
    Num(String),
    /// A string.
    Str(String),
    /// An array.
    Arr(Vec<Val>),
    /// An object, members in source order.
    Obj(Vec<(String, Val)>),
}

impl Kind {
    /// The JSON type name Go's `UnmarshalTypeError` reports.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
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

/// Go's `quoteChar` for syntax errors.
fn quote_char(c: char) -> String {
    match c {
        '\'' => "'\\''".into(),
        '"' => "'\"'".into(),
        _ => {
            let q = format!("{c:?}");
            format!("'{}'", &q[1..q.len() - 1])
        }
    }
}

struct Parser<'a> {
    s: &'a str,
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s[self.i..].chars().next()
    }

    fn ws(&mut self) {
        while let Some(c) = self.peek() {
            if matches!(c, ' ' | '\t' | '\n' | '\r') {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    fn err(&self, ctx: &str) -> String {
        self.peek().map_or_else(
            || "unexpected end of JSON input".to_owned(),
            |c| format!("invalid character {} {ctx}", quote_char(c)),
        )
    }

    fn value(&mut self) -> Result<Val, String> {
        self.ws();
        let start = self.i;
        let kind = match self.peek() {
            Some('{') => self.object()?,
            Some('[') => self.array()?,
            Some('"') => Kind::Str(self.string()?),
            Some('t') => self.literal("true", Kind::Bool(true))?,
            Some('f') => self.literal("false", Kind::Bool(false))?,
            Some('n') => self.literal("null", Kind::Null)?,
            Some(c) if c == '-' || c.is_ascii_digit() => self.number()?,
            _ => return Err(self.err("looking for beginning of value")),
        };
        Ok(Val {
            kind,
            raw: self.s[start..self.i].to_owned(),
        })
    }

    fn literal(&mut self, lit: &str, k: Kind) -> Result<Kind, String> {
        for (n, want) in lit.chars().enumerate() {
            match self.peek() {
                Some(c) if c == want => self.i += 1,
                _ if n == 0 => return Err(self.err("looking for beginning of value")),
                _ => return Err(self.err(&format!("in literal {lit} (expecting '{want}')"))),
            }
        }
        Ok(k)
    }

    fn digits(&mut self) -> usize {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        self.i - start
    }

    fn number(&mut self) -> Result<Kind, String> {
        let start = self.i;
        if self.peek() == Some('-') {
            self.i += 1;
        }
        match self.peek() {
            Some('0') => self.i += 1,
            Some(c) if c.is_ascii_digit() => {
                self.digits();
            }
            _ => return Err(self.err("in numeric literal")),
        }
        if self.peek() == Some('.') {
            self.i += 1;
            if self.digits() == 0 {
                return Err(self.err("after decimal point in numeric literal"));
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.i += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.i += 1;
            }
            if self.digits() == 0 {
                return Err(self.err("in exponent of numeric literal"));
            }
        }
        Ok(Kind::Num(self.s[start..self.i].to_owned()))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let mut v = 0;
        for _ in 0..4 {
            match self.peek().and_then(|c| c.to_digit(16)) {
                Some(d) => {
                    v = v * 16 + d;
                    self.i += 1;
                }
                None => return Err(self.err("in \\u hexadecimal character escape")),
            }
        }
        Ok(v)
    }

    fn string(&mut self) -> Result<String, String> {
        self.i += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err("unexpected end of JSON input".into());
            };
            self.i += c.len_utf8();
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let Some(e) = self.peek() else {
                        return Err("unexpected end of JSON input".into());
                    };
                    self.i += 1;
                    match e {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let r = self.hex4()?;
                            if (0xD800..0xDC00).contains(&r) && self.s[self.i..].starts_with("\\u")
                            {
                                let save = self.i;
                                self.i += 2;
                                let r2 = self.hex4()?;
                                if (0xDC00..0xE000).contains(&r2) {
                                    let cp = 0x10000 + ((r - 0xD800) << 10) + (r2 - 0xDC00);
                                    out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                                    continue;
                                }
                                self.i = save;
                                out.push('\u{FFFD}');
                                continue;
                            }
                            out.push(char::from_u32(r).unwrap_or('\u{FFFD}'));
                        }
                        _ => {
                            self.i -= 1;
                            return Err(self.err("in string escape code"));
                        }
                    }
                }
                c if (c as u32) < 0x20 => {
                    self.i -= 1;
                    return Err(self.err("in string literal"));
                }
                c => out.push(c),
            }
        }
    }

    fn array(&mut self) -> Result<Kind, String> {
        self.i += 1;
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(']') {
            self.i += 1;
            return Ok(Kind::Arr(items));
        }
        loop {
            items.push(self.value()?);
            self.ws();
            match self.peek() {
                Some(',') => self.i += 1,
                Some(']') => {
                    self.i += 1;
                    return Ok(Kind::Arr(items));
                }
                _ => return Err(self.err("after array element")),
            }
        }
    }

    fn object(&mut self) -> Result<Kind, String> {
        self.i += 1;
        let mut members = Vec::new();
        self.ws();
        if self.peek() == Some('}') {
            self.i += 1;
            return Ok(Kind::Obj(members));
        }
        loop {
            self.ws();
            if self.peek() != Some('"') {
                return Err(self.err("looking for beginning of object key string"));
            }
            let k = self.string()?;
            self.ws();
            if self.peek() != Some(':') {
                return Err(self.err("after object key"));
            }
            self.i += 1;
            let v = self.value()?;
            members.push((k, v));
            self.ws();
            match self.peek() {
                Some(',') => self.i += 1,
                Some('}') => {
                    self.i += 1;
                    return Ok(Kind::Obj(members));
                }
                _ => return Err(self.err("after object key:value pair")),
            }
        }
    }
}

/// Parses one complete JSON document (`json.Unmarshal`'s syntax check).
pub fn parse(s: &str) -> Result<Val, String> {
    let mut p = Parser { s, i: 0 };
    let v = p.value()?;
    p.ws();
    if p.i < s.len() {
        return Err(p.err("after top-level value"));
    }
    Ok(v)
}

/// `orderedmap` normalization: last value wins, a repeated key moves to the
/// end.
fn dedup(members: &[(String, Val)]) -> Vec<(&str, &Val)> {
    let mut out: Vec<(&str, &Val)> = Vec::new();
    for (k, v) in members {
        if let Some(pos) = out.iter().position(|(ek, _)| ek == k) {
            out.remove(pos);
        }
        out.push((k, v));
    }
    out
}

/// `json.Marshal` of the `interface{}` / `OrderedMap` tree; `None` when a
/// number does not fit a `float64` (Go's decode fails).
fn marshal_ordered(v: &Val) -> Option<String> {
    Some(match &v.kind {
        Kind::Null => "null".into(),
        Kind::Bool(b) => b.to_string(),
        Kind::Num(lit) => {
            let f: f64 = lit.parse().ok()?;
            if !f.is_finite() {
                return None;
            }
            format_float(f)
        }
        Kind::Str(s) => escape_string(s),
        Kind::Arr(items) => {
            let parts: Option<Vec<String>> = items.iter().map(marshal_ordered).collect();
            format!("[{}]", parts?.join(","))
        }
        Kind::Obj(members) => {
            let parts: Option<Vec<String>> = dedup(members)
                .into_iter()
                .map(|(k, v)| marshal_ordered(v).map(|s| format!("{}:{s}", escape_string(k))))
                .collect();
            format!("{{{}}}", parts?.join(","))
        }
    })
}

/// `json.Marshal`'s `appendCompact` (HTML escaping on) of a marshaler's
/// output.
pub fn compact(raw: &str) -> Result<String, String> {
    parse(raw)?;
    let mut out = String::with_capacity(raw.len());
    let mut in_str = false;
    let mut esc = false;
    for c in raw.chars() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
        } else if matches!(c, ' ' | '\t' | '\n' | '\r') {
            continue;
        } else if c == '"' {
            in_str = true;
        }
        match c {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c => out.push(c),
        }
    }
    Ok(out)
}

/// `json.Marshal(metadata.Next)` for a `JSONObject` decoded from `v`.
pub fn marshal_next(v: &Val) -> Result<String, String> {
    let primitive = match &v.kind {
        Kind::Obj(_) => match marshal_ordered(v) {
            Some(s) => return Ok(s),
            None => v.raw.clone(),
        },
        Kind::Str(s) => s.clone(),
        _ => v.raw.clone(),
    };
    compact(&primitive)
        .map_err(|e| format!("json: error calling MarshalJSON for type *types.JSONObject: {e}"))
}

/// Go's `time.quote`.
fn time_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        if (c as u32) >= 0x80 || c < ' ' {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                let _ = core::fmt::Write::write_fmt(&mut out, format_args!("\\x{b:02x}"));
            }
        } else {
            if c == '"' || c == '\\' {
                out.push('\\');
            }
            out.push(c);
        }
    }
    out.push('"');
    out
}

/// Go's `time.ParseDuration` (nanoseconds).
#[allow(clippy::too_many_lines, clippy::many_single_char_names)]
pub fn parse_duration(orig: &str) -> Result<i64, String> {
    const LIMIT: u64 = 1 << 63;
    let invalid = || format!("time: invalid duration {}", time_quote(orig));
    let mut s = orig;
    let mut neg = false;
    if let Some(c) = s.chars().next() {
        if c == '-' || c == '+' {
            neg = c == '-';
            s = &s[1..];
        }
    }
    if s == "0" {
        return Ok(0);
    }
    if s.is_empty() {
        return Err(invalid());
    }
    let mut d: u64 = 0;
    while !s.is_empty() {
        let b0 = s.as_bytes()[0];
        if !(b0 == b'.' || b0.is_ascii_digit()) {
            return Err(invalid());
        }
        // leadingInt
        let pl = s.len();
        let mut v: u64 = 0;
        let mut i = 0;
        let bytes = s.as_bytes();
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            if v > LIMIT / 10 {
                return Err(invalid());
            }
            v = v * 10 + u64::from(bytes[i] - b'0');
            if v > LIMIT {
                return Err(invalid());
            }
            i += 1;
        }
        s = &s[i..];
        let pre = pl != s.len();
        let mut post = false;
        let mut f: u64 = 0;
        let mut scale = 1f64;
        if s.as_bytes().first() == Some(&b'.') {
            s = &s[1..];
            let pl = s.len();
            let bytes = s.as_bytes();
            let mut i = 0;
            let mut overflow = false;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                if !overflow {
                    if f > (LIMIT - 1) / 10 {
                        overflow = true;
                    } else {
                        let y = f * 10 + u64::from(bytes[i] - b'0');
                        if y > LIMIT {
                            overflow = true;
                        } else {
                            f = y;
                            scale *= 10.0;
                        }
                    }
                }
                i += 1;
            }
            s = &s[i..];
            post = pl != s.len();
        }
        if !pre && !post {
            return Err(invalid());
        }
        let i = s
            .bytes()
            .position(|c| c == b'.' || c.is_ascii_digit())
            .unwrap_or(s.len());
        if i == 0 {
            return Err(format!(
                "time: missing unit in duration {}",
                time_quote(orig)
            ));
        }
        let u = &s[..i];
        s = &s[i..];
        let unit: u64 = match u {
            "ns" => 1,
            "us" | "\u{b5}s" | "\u{3bc}s" => 1_000,
            "ms" => 1_000_000,
            "s" => 1_000_000_000,
            "m" => 60_000_000_000,
            "h" => 3_600_000_000_000,
            _ => {
                return Err(format!(
                    "time: unknown unit {} in duration {}",
                    time_quote(u),
                    time_quote(orig)
                ));
            }
        };
        if v > LIMIT / unit {
            return Err(invalid());
        }
        v *= unit;
        if f > 0 {
            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss
            )]
            let add = (f as f64 * (unit as f64 / scale)) as u64;
            v = v.wrapping_add(add);
            if v > LIMIT {
                return Err(invalid());
            }
        }
        d = d.wrapping_add(v);
        if d > LIMIT {
            return Err(invalid());
        }
    }
    if neg {
        #[allow(clippy::cast_possible_wrap)]
        return Ok((d as i64).wrapping_neg());
    }
    if d > LIMIT - 1 {
        return Err(invalid());
    }
    #[allow(clippy::cast_possible_wrap)]
    Ok(d as i64)
}

/// Go's `int64(f)` on amd64 (out of range and NaN give `MinInt64`).
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
pub fn go_f64_to_i64(f: f64) -> i64 {
    if f.is_nan() || f >= 9_223_372_036_854_775_808.0 || f < -9_223_372_036_854_775_808.0 {
        i64::MIN
    } else {
        f.trunc() as i64
    }
}

/// `types.ForwardMetadata`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ForwardMetadata {
    /// `receiver`.
    pub receiver: String,
    /// `port`.
    pub port: String,
    /// `channel`.
    pub channel: String,
    /// `timeout` (nanoseconds).
    pub timeout: i64,
    /// `retries`.
    pub retries: Option<u8>,
    /// `next`.
    pub next: Option<Val>,
}

fn type_error(value: &str, strukt: &str, field: &str, ty: &str) -> String {
    format!("json: cannot unmarshal {value} into Go struct field {strukt}.{field} of type {ty}")
}

/// Go's struct field matching: exact name, else ASCII case-insensitive.
fn field_is(key: &str, name: &str) -> bool {
    key == name || key.eq_ignore_ascii_case(name)
}

fn decode_forward(
    m: &mut ForwardMetadata,
    members: &[(String, Val)],
    saved: &mut Option<String>,
) -> Result<(), String> {
    let mut save = |e: String| {
        if saved.is_none() {
            *saved = Some(e);
        }
    };
    for (k, v) in members {
        let string_field =
            |dst: &mut String, name: &str, save: &mut dyn FnMut(String)| match &v.kind {
                Kind::Str(s) => dst.clone_from(s),
                Kind::Null => {}
                other => save(type_error(
                    other.type_name(),
                    "ForwardMetadata",
                    &format!("forward.{name}"),
                    "string",
                )),
            };
        if field_is(k, "receiver") {
            string_field(&mut m.receiver, "receiver", &mut save);
        } else if field_is(k, "port") {
            string_field(&mut m.port, "port", &mut save);
        } else if field_is(k, "channel") {
            string_field(&mut m.channel, "channel", &mut save);
        } else if field_is(k, "timeout") {
            m.timeout = match &v.kind {
                Kind::Num(lit) => {
                    let f: f64 = lit.parse().unwrap_or(f64::INFINITY);
                    if !f.is_finite() {
                        return Err(format!(
                            "json: cannot unmarshal number {lit} into Go value of type float64"
                        ));
                    }
                    go_f64_to_i64(f)
                }
                Kind::Str(s) => parse_duration(s)?,
                _ => return Err("invalid duration".into()),
            };
        } else if field_is(k, "retries") {
            match &v.kind {
                Kind::Null => m.retries = None,
                Kind::Num(lit) => match lit.parse::<u64>() {
                    Ok(n) if n <= 255 => m.retries = Some(u8::try_from(n).unwrap_or_default()),
                    _ => {
                        m.retries.get_or_insert(0);
                        save(type_error(
                            &format!("number {lit}"),
                            "ForwardMetadata",
                            "forward.retries",
                            "uint8",
                        ));
                    }
                },
                other => {
                    m.retries.get_or_insert(0);
                    save(type_error(
                        other.type_name(),
                        "ForwardMetadata",
                        "forward.retries",
                        "uint8",
                    ));
                }
            }
        } else if field_is(k, "next") {
            m.next = match &v.kind {
                Kind::Null => None,
                _ => Some(v.clone()),
            };
        }
    }
    Ok(())
}

/// The memo's routing decision.
#[derive(Debug, Clone, PartialEq)]
pub enum Memo {
    /// Not a forward (`json.Unmarshal` into a map failed or `forward` is
    /// missing / null): the packet goes to the transfer app.
    NotForward,
    /// `json.Unmarshal(memo, &PacketMetadata{})` failed.
    ParseError(String),
    /// `forward` decoded to a nil pointer (Go then panics in `Validate`).
    NilForward,
    /// The forward metadata.
    Forward(ForwardMetadata),
}

/// Classifies a transfer memo as PFM's `OnRecvPacket` does.
#[must_use]
pub fn classify(memo: &str) -> Memo {
    let Ok(root) = parse(memo) else {
        return Memo::NotForward;
    };
    let Kind::Obj(members) = &root.kind else {
        return Memo::NotForward;
    };
    // `d["forward"] == nil` on the map decode (last duplicate wins).
    match members.iter().rev().find(|(k, _)| k == "forward") {
        None => return Memo::NotForward,
        Some((_, v)) if v.kind == Kind::Null => return Memo::NotForward,
        Some(_) => {}
    }
    let mut fwd: Option<ForwardMetadata> = None;
    let mut saved: Option<String> = None;
    for (k, v) in members {
        if !field_is(k, "forward") {
            continue;
        }
        match &v.kind {
            Kind::Null => fwd = None,
            Kind::Obj(fields) => {
                let mut m = fwd.take().unwrap_or_default();
                if let Err(e) = decode_forward(&mut m, fields, &mut saved) {
                    return Memo::ParseError(e);
                }
                fwd = Some(m);
            }
            other => {
                fwd.get_or_insert_with(ForwardMetadata::default);
                if saved.is_none() {
                    saved = Some(type_error(
                        other.type_name(),
                        "PacketMetadata",
                        "forward",
                        "types.ForwardMetadata",
                    ));
                }
            }
        }
    }
    if let Some(e) = saved {
        return Memo::ParseError(e);
    }
    fwd.map_or(Memo::NilForward, Memo::Forward)
}

/// `ForwardMetadata.Validate`.
pub fn validate(m: &ForwardMetadata) -> Result<(), String> {
    if m.receiver.is_empty() {
        return Err("failed to validate metadata. receiver cannot be empty".into());
    }
    x_ibc::host::port_identifier_validator(&m.port)
        .map_err(|e| format!("failed to validate metadata: {}", e.log))?;
    x_ibc::host::channel_identifier_validator(&m.channel)
        .map_err(|e| format!("failed to validate metadata: {}", e.log))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_memos() {
        assert_eq!(classify(""), Memo::NotForward);
        assert_eq!(classify("hello"), Memo::NotForward);
        assert_eq!(classify("[1]"), Memo::NotForward);
        assert_eq!(classify(r#"{"forward":null}"#), Memo::NotForward);
        assert_eq!(classify(r#"{"Forward":{}}"#), Memo::NotForward);
        let Memo::Forward(m) = classify(
            r#"{"forward":{"receiver":"r","port":"transfer","channel":"channel-0","timeout":"1m","retries":2,"next":{"b":1,"a":"<x>","b":2}}}"#,
        ) else {
            panic!("expected forward");
        };
        assert_eq!(m.timeout, 60_000_000_000);
        assert_eq!(m.retries, Some(2));
        assert_eq!(
            marshal_next(m.next.as_ref().unwrap()).unwrap(),
            "{\"a\":\"\\u003cx\\u003e\",\"b\":2}"
        );
        assert_eq!(
            classify(r#"{"forward":{"receiver":1}}"#),
            Memo::ParseError(
                "json: cannot unmarshal number into Go struct field ForwardMetadata.forward.receiver of type string".into()
            )
        );
        assert_eq!(
            classify(r#"{"forward":{"retries":300}}"#),
            Memo::ParseError(
                "json: cannot unmarshal number 300 into Go struct field ForwardMetadata.forward.retries of type uint8".into()
            )
        );
        assert_eq!(
            classify(r#"{"forward":"x"}"#),
            Memo::ParseError(
                "json: cannot unmarshal string into Go struct field PacketMetadata.forward of type types.ForwardMetadata".into()
            )
        );
        assert_eq!(
            classify(r#"{"forward":{"timeout":null}}"#),
            Memo::ParseError("invalid duration".into())
        );
    }

    #[test]
    fn next_primitives() {
        let v = parse(r#""{\"a\": [1, 2]}""#).unwrap();
        assert_eq!(marshal_next(&v).unwrap(), r#"{"a":[1,2]}"#);
        let v = parse("[1.50, 2]").unwrap();
        assert_eq!(marshal_next(&v).unwrap(), "[1.50,2]");
        let v = parse(r#""abc""#).unwrap();
        assert_eq!(
            marshal_next(&v).unwrap_err(),
            "json: error calling MarshalJSON for type *types.JSONObject: invalid character 'a' looking for beginning of value"
        );
        let v = parse(r#"{"x":1.0,"y":{"z":1e21}}"#).unwrap();
        assert_eq!(marshal_next(&v).unwrap(), r#"{"x":1,"y":{"z":1e+21}}"#);
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("1h30m"), Ok(5_400_000_000_000));
        assert_eq!(parse_duration("1.5s"), Ok(1_500_000_000));
        assert_eq!(parse_duration("-2ms"), Ok(-2_000_000));
        assert_eq!(parse_duration("0"), Ok(0));
        assert_eq!(
            parse_duration(""),
            Err("time: invalid duration \"\"".into())
        );
        assert_eq!(
            parse_duration("1"),
            Err("time: missing unit in duration \"1\"".into())
        );
        assert_eq!(
            parse_duration("1x"),
            Err("time: unknown unit \"x\" in duration \"1x\"".into())
        );
        assert_eq!(go_f64_to_i64(1.9), 1);
        assert_eq!(go_f64_to_i64(1e30), i64::MIN);
    }
}
