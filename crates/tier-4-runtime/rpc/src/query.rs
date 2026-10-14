//! Port of `libs/pubsub/query` (+ `syntax`): the event query language
//! (`tm.event = 'Tx' AND tx.height > 5 AND transfer.amount CONTAINS 'utia'`),
//! with Go's parse error messages and matching semantics (exact decimal
//! comparison of numbers, RFC 3339 / date comparison of timestamps).

use std::cmp::Ordering;
use std::collections::BTreeMap;

use ctypes::proto::abci;

/// A query token (`syntax.Token`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    /// Invalid.
    Invalid,
    /// Tag `x.y`.
    Tag,
    /// `'string'`.
    String,
    /// Number.
    Number,
    /// `TIME …`.
    Time,
    /// `DATE …`.
    Date,
    /// `AND`.
    And,
    /// `CONTAINS`.
    Contains,
    /// `EXISTS`.
    Exists,
    /// `=`.
    Eq,
    /// `<`.
    Lt,
    /// `<=`.
    Leq,
    /// `>`.
    Gt,
    /// `>=`.
    Geq,
}

impl Token {
    fn label(self) -> &'static str {
        match self {
            Self::Invalid => "invalid token",
            Self::Tag => "tag",
            Self::String => "string",
            Self::Number => "number",
            Self::Time => "timestamp",
            Self::Date => "datestamp",
            Self::And => "AND operator",
            Self::Contains => "CONTAINS operator",
            Self::Exists => "EXISTS operator",
            Self::Eq => "= operator",
            Self::Lt => "< operator",
            Self::Leq => "<= operator",
            Self::Gt => "> operator",
            Self::Geq => ">= operator",
        }
    }
}

/// A condition argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arg {
    /// Argument type.
    pub kind: Token,
    /// Text (without quotes / the `TIME`/`DATE` label).
    pub text: String,
}

impl Arg {
    /// `Arg.String()`.
    #[must_use]
    pub fn to_go_string(&self) -> String {
        match self.kind {
            Token::String => format!("'{}'", self.text),
            Token::Time => format!("TIME {}", self.text),
            Token::Date => format!("DATE {}", self.text),
            _ => self.text.clone(),
        }
    }

    /// `Arg.Number()`.
    #[must_use]
    pub fn number(&self) -> Option<Dec> {
        Dec::parse_float(&self.text)
    }

    /// `Arg.Time()` as Unix nanoseconds (zero time on error).
    #[must_use]
    pub fn time(&self) -> i128 {
        match self.kind {
            Token::Date => parse_date(&self.text),
            Token::Time => parse_rfc3339(&self.text),
            _ => None,
        }
        .unwrap_or(GO_ZERO_NANOS)
    }
}

/// A query condition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    /// Composite tag (`type.key`).
    pub tag: String,
    /// Operator.
    pub op: Token,
    /// Argument (none for `EXISTS`).
    pub arg: Option<Arg>,
    op_text: String,
}

impl Condition {
    /// `Condition.String()`.
    #[must_use]
    pub fn to_go_string(&self) -> String {
        let s = format!("{} {}", self.tag, self.op_text);
        match &self.arg {
            Some(a) => format!("{s} {}", a.to_go_string()),
            None => s,
        }
    }

    /// The argument text (`Arg.Value()`; empty without argument).
    #[must_use]
    pub fn value(&self) -> &str {
        self.arg.as_ref().map_or("", |a| a.text.as_str())
    }
}

// --- scanner ---------------------------------------------------------------

struct Scanner<'a> {
    chars: Vec<(usize, char)>,
    src: &'a str,
    idx: usize,
    buf: String,
    tok: Token,
    pos: usize,
    end: usize,
    last: usize,
}

fn is_tag_rune(c: char) -> bool {
    c == '.' || c == '_' || c == '-' || c.is_alphabetic() || is_go_digit(c)
}

fn is_first_tag_rune(c: char) -> bool {
    c == '_' || c.is_alphabetic() || is_go_digit(c)
}

/// `unicode.IsDigit` (Nd).
fn is_go_digit(c: char) -> bool {
    c.is_ascii_digit() || (!c.is_ascii() && unicode_nd(c))
}

fn unicode_nd(c: char) -> bool {
    // Decimal digits outside ASCII: char::is_numeric covers Nd, Nl and No;
    // approximate Nd by requiring a contiguous 0-9 block.
    if !c.is_numeric() {
        return false;
    }
    let cp = u32::from(c);
    let zero = cp - (cp % 16) % 10;
    (0..10).all(|d| char::from_u32(zero + d).is_some_and(char::is_numeric))
}

fn is_go_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{85}' | '\u{a0}'
    ) || (!c.is_ascii() && c.is_whitespace())
}

impl<'a> Scanner<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            chars: src.char_indices().collect(),
            src,
            idx: 0,
            buf: String::new(),
            tok: Token::Invalid,
            pos: 0,
            end: 0,
            last: 0,
        }
    }

    fn rune(&mut self) -> Option<char> {
        if let Some(&(_, c)) = self.chars.get(self.idx) {
            self.idx += 1;
            self.last = c.len_utf8();
            self.end += self.last;
            Some(c)
        } else {
            self.last = 0;
            None
        }
    }

    fn unrune(&mut self) {
        if self.last > 0 {
            self.idx -= 1;
            self.end -= self.last;
            self.last = 0;
        }
    }

    /// `Next`: `Ok(false)` at EOF.
    fn next(&mut self) -> Result<bool, String> {
        self.buf.clear();
        self.pos = self.end;
        self.tok = Token::Invalid;
        loop {
            let Some(ch) = self.rune() else {
                return Ok(false);
            };
            if is_go_space(ch) {
                self.pos = self.end;
                continue;
            }
            if ch.is_ascii_digit() {
                self.scan_number(ch);
                return Ok(true);
            } else if is_first_tag_rune(ch) {
                self.scan_tag_like(ch)?;
                return Ok(true);
            }
            return match ch {
                '\'' => self.scan_string(ch).map(|()| true),
                '<' | '>' | '=' => {
                    self.scan_compare(ch);
                    Ok(true)
                }
                _ => Err(format!("invalid input {ch} at offset {}", self.end)),
            };
        }
    }

    fn scan_while(&mut self, ok: impl Fn(char) -> bool) {
        loop {
            match self.rune() {
                None => return,
                Some(c) if ok(c) => self.buf.push(c),
                Some(_) => {
                    self.unrune();
                    return;
                }
            }
        }
    }

    fn scan_number(&mut self, first: char) {
        self.buf.push(first);
        self.scan_while(|c| c.is_ascii_digit());
        match self.rune() {
            Some('.') => {
                self.buf.push('.');
                self.scan_while(|c| c.is_ascii_digit());
            }
            Some(_) => self.unrune(),
            None => {}
        }
        self.tok = Token::Number;
    }

    fn scan_string(&mut self, first: char) -> Result<(), String> {
        loop {
            match self.rune() {
                None => return Err("EOF".into()),
                Some(c) if c == first => {
                    self.tok = Token::String;
                    return Ok(());
                }
                Some(c) => self.buf.push(c),
            }
        }
    }

    fn scan_compare(&mut self, first: char) {
        self.buf.push(first);
        self.tok = match first {
            '=' => {
                self.tok = Token::Eq;
                return;
            }
            '<' => Token::Lt,
            _ => Token::Gt,
        };
        match self.rune() {
            None => {}
            Some('=') => {
                self.buf.push('=');
                self.tok = if self.tok == Token::Lt {
                    Token::Leq
                } else {
                    Token::Geq
                };
            }
            Some(_) => self.unrune(),
        }
    }

    fn scan_tag_like(&mut self, first: char) -> Result<(), String> {
        self.buf.push(first);
        let mut has_space = false;
        loop {
            match self.rune() {
                None => break,
                Some(c) if is_tag_rune(c) => self.buf.push(c),
                Some(c) => {
                    has_space = c == ' ';
                    break;
                }
            }
        }
        match self.buf.as_str() {
            "TIME" if has_space => return self.scan_timestamp(),
            "DATE" if has_space => return self.scan_datestamp(),
            "AND" => self.tok = Token::And,
            "EXISTS" => self.tok = Token::Exists,
            "CONTAINS" => self.tok = Token::Contains,
            _ => self.tok = Token::Tag,
        }
        self.unrune();
        Ok(())
    }

    fn scan_timestamp(&mut self) -> Result<(), String> {
        self.buf.clear();
        self.scan_while(|c| "-T:+Z".contains(c) || c.is_ascii_digit());
        let s = self.buf.clone();
        match parse_rfc3339_detailed(&s) {
            Err(e) => Err(format!("invalid TIME value: {e}")),
            Ok((y, _)) if !(1900..=2999).contains(&y) => {
                Err(format!("timestamp year {y} out of range"))
            }
            Ok(_) => {
                self.tok = Token::Time;
                Ok(())
            }
        }
    }

    fn scan_datestamp(&mut self) -> Result<(), String> {
        self.buf.clear();
        self.scan_while(|c| c.is_ascii_digit() || c == '-');
        let s = self.buf.clone();
        match parse_date_detailed(&s) {
            Err(e) => Err(format!("invalid DATE value: {e}")),
            Ok((y, _)) if !(1900..=2999).contains(&y) => {
                Err(format!("datestamp year {y} out of range"))
            }
            Ok(_) => {
                self.tok = Token::Date;
                Ok(())
            }
        }
    }
}

fn tok_label(tokens: &[Token]) -> String {
    if tokens.len() == 1 {
        return tokens[0].label().to_owned();
    }
    let last = tokens.len() - 1;
    let ss: Vec<&str> = tokens[..last].iter().map(|t| t.label()).collect();
    format!("{} or {}", ss.join(", "), tokens[last].label())
}

struct Parser<'a> {
    sc: Scanner<'a>,
}

impl Parser<'_> {
    fn require(&mut self, tokens: &[Token]) -> Result<(), String> {
        match self.sc.next() {
            Err(e) => return Err(format!("offset {}: {e}", self.sc.pos)),
            Ok(false) => return Err(format!("offset {}: EOF", self.sc.pos)),
            Ok(true) => {}
        }
        let got = self.sc.tok;
        if tokens.contains(&got) {
            Ok(())
        } else {
            Err(format!(
                "offset {}: got {}, wanted {}",
                self.sc.pos,
                got.label(),
                tok_label(tokens)
            ))
        }
    }

    fn parse_cond(&mut self) -> Result<Condition, String> {
        self.require(&[Token::Tag])?;
        let tag = self.sc.buf.clone();
        self.require(&[
            Token::Leq,
            Token::Geq,
            Token::Lt,
            Token::Gt,
            Token::Eq,
            Token::Contains,
            Token::Exists,
        ])?;
        let op = self.sc.tok;
        let op_text = self.sc.buf.clone();
        match op {
            Token::Leq | Token::Geq | Token::Lt | Token::Gt => {
                self.require(&[Token::Number, Token::Time, Token::Date])?;
            }
            Token::Eq => self.require(&[Token::Number, Token::Time, Token::Date, Token::String])?,
            Token::Contains => self.require(&[Token::String])?,
            _ => {
                return Ok(Condition {
                    tag,
                    op,
                    arg: None,
                    op_text,
                });
            }
        }
        Ok(Condition {
            tag,
            op,
            arg: Some(Arg {
                kind: self.sc.tok,
                text: self.sc.buf.clone(),
            }),
            op_text,
        })
    }
}

/// `syntax.Parse`.
pub fn parse(s: &str) -> Result<Vec<Condition>, String> {
    let mut p = Parser {
        sc: Scanner::new(s),
    };
    let mut conds = vec![p.parse_cond()?];
    loop {
        if let Ok(false) = p.sc.next() {
            break;
        }
        if p.sc.tok != Token::And {
            // Go formats the untyped constant `TAnd` (6) with %v.
            return Err(format!(
                "offset {}: got {}, want 6",
                p.sc.pos,
                p.sc.tok.label()
            ));
        }
        conds.push(p.parse_cond()?);
    }
    let _ = p.sc.src;
    Ok(conds)
}

// --- compiled query --------------------------------------------------------

/// A compiled query (`query.Query`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    /// The conditions (`Syntax()`).
    pub conds: Vec<Condition>,
}

impl Query {
    /// `query.New`.
    pub fn new(s: &str) -> Result<Self, String> {
        let conds = parse(s)?;
        for c in &conds {
            if c.op == Token::Exists {
                continue;
            }
            let Some(a) = &c.arg else {
                return Err(format!(
                    "compile {}: missing argument for {}",
                    c.to_go_string(),
                    c.op.label()
                ));
            };
            let ok = matches!(
                (c.op, a.kind),
                (Token::Contains, Token::String)
                    | (
                        Token::Eq,
                        Token::String | Token::Number | Token::Date | Token::Time
                    )
                    | (
                        Token::Lt | Token::Leq | Token::Gt | Token::Geq,
                        Token::Number | Token::Date | Token::Time
                    )
            );
            if !ok {
                return Err(format!(
                    "compile {}: invalid op/arg combination ({}, {})",
                    c.to_go_string(),
                    c.op.label(),
                    a.kind.label()
                ));
            }
        }
        Ok(Self { conds })
    }

    /// `Query.String()` (the canonical form used as subscription key).
    #[must_use]
    pub fn to_go_string(&self) -> String {
        self.conds
            .iter()
            .map(Condition::to_go_string)
            .collect::<Vec<_>>()
            .join(" AND ")
    }

    /// `Query.Matches` on a flattened event map (`type.key → values`).
    #[must_use]
    pub fn matches(&self, events: &BTreeMap<String, Vec<String>>) -> bool {
        let expanded = expand_events(events);
        for c in &self.conds {
            if !expanded.iter().any(|e| cond_matches_event(c, e)) {
                return false;
            }
        }
        !expanded.is_empty()
    }
}

/// `ExpandEvents`: `"a.b.c" → Event{type: "a.b", attrs: [{key: "c"}]}`.
fn expand_events(events: &BTreeMap<String, Vec<String>>) -> Vec<abci::Event> {
    events
        .iter()
        .map(|(composite, values)| {
            let (typ, key) = composite
                .rsplit_once('.')
                .unwrap_or(("", composite.as_str()));
            abci::Event {
                r#type: typ.to_owned(),
                attributes: values
                    .iter()
                    .map(|v| abci::EventAttribute {
                        key: key.to_owned(),
                        value: v.clone(),
                        index: false,
                    })
                    .collect(),
            }
        })
        .collect()
}

fn cond_matches_event(c: &Condition, e: &abci::Event) -> bool {
    if !c.tag.starts_with(&e.r#type) {
        return false;
    }
    if c.tag.len() == e.r#type.len() {
        return value_matches(c, "");
    }
    let mut found = false;
    for a in &e.attributes {
        if format!("{}.{}", e.r#type, a.key) == c.tag {
            found = true;
            if value_matches(c, &a.value) {
                return true;
            }
        }
    }
    let _ = found;
    false
}

/// Whether event value `s` satisfies the condition's operator/argument.
#[must_use]
pub fn value_matches(c: &Condition, s: &str) -> bool {
    let Some(a) = &c.arg else {
        return true; // EXISTS
    };
    match (c.op, a.kind) {
        (Token::Contains, _) => s.contains(a.text.as_str()),
        (Token::Eq, Token::String) => s == a.text,
        (op, Token::Number) => {
            let (Some(w), Some(v)) = (parse_number(s), a.number()) else {
                return false;
            };
            cmp_op(op, w.cmp(&v))
        }
        (op, Token::Date) => match parse_date(s) {
            Some(t) => cmp_op(op, t.cmp(&a.time())),
            None => false,
        },
        (op, Token::Time) => match parse_rfc3339(s) {
            Some(t) => cmp_op(op, t.cmp(&a.time())),
            None => false,
        },
        _ => false,
    }
}

fn cmp_op(op: Token, o: Ordering) -> bool {
    match op {
        Token::Eq => o == Ordering::Equal,
        Token::Lt => o == Ordering::Less,
        Token::Leq => o != Ordering::Greater,
        Token::Gt => o == Ordering::Greater,
        Token::Geq => o != Ordering::Less,
        _ => false,
    }
}

// --- numbers ---------------------------------------------------------------

/// An exact decimal (`big.Float` stand-in): `(-1)^neg · digits · 10^exp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dec {
    neg: bool,
    /// Significant digits without leading/trailing zeros (empty = 0).
    digits: Vec<u8>,
    /// Decimal exponent of the last digit.
    exp: i64,
}

impl Dec {
    /// Zero.
    #[must_use]
    pub fn zero() -> Self {
        Self {
            neg: false,
            digits: Vec::new(),
            exp: 0,
        }
    }

    /// An integer.
    #[must_use]
    pub fn from_i64(v: i64) -> Self {
        Self::parse_float(&v.to_string()).unwrap_or_else(Self::zero)
    }

    /// `big.ParseFloat(s, 10, …)`: `[+-]digits[.digits][e[+-]digits]`,
    /// `Inf` / `inf` accepted as infinity are not supported (rejected).
    #[must_use]
    pub fn parse_float(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        let mut i = 0;
        let mut neg = false;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            neg = b[i] == b'-';
            i += 1;
        }
        let mut digits = Vec::new();
        let mut exp: i64 = 0;
        let mut any = false;
        while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'_') {
            if b[i] == b'_' {
                return None;
            }
            digits.push(b[i] - b'0');
            any = true;
            i += 1;
        }
        if i < b.len() && b[i] == b'.' {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                digits.push(b[i] - b'0');
                exp -= 1;
                any = true;
                i += 1;
            }
        }
        if !any {
            return None;
        }
        if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
            i += 1;
            let mut eneg = false;
            if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
                eneg = b[i] == b'-';
                i += 1;
            }
            let start = i;
            let mut e: i64 = 0;
            while i < b.len() && b[i].is_ascii_digit() {
                e = e.saturating_mul(10).saturating_add(i64::from(b[i] - b'0'));
                i += 1;
            }
            if i == start {
                return None;
            }
            exp = exp.saturating_add(if eneg { -e } else { e });
        }
        if i != b.len() {
            return None;
        }
        Some(Self::normalize(neg, digits, exp))
    }

    fn normalize(neg: bool, mut digits: Vec<u8>, mut exp: i64) -> Self {
        let lead = digits.iter().take_while(|&&d| d == 0).count();
        digits.drain(..lead);
        while digits.last() == Some(&0) {
            digits.pop();
            exp += 1;
        }
        if digits.is_empty() {
            return Self::zero();
        }
        Self { neg, digits, exp }
    }

    fn cmp_abs(&self, o: &Self) -> Ordering {
        // Magnitude: position of the most significant digit.
        let ma = self.digits.len() as i64 + self.exp;
        let mb = o.digits.len() as i64 + o.exp;
        if ma != mb {
            return ma.cmp(&mb);
        }
        let n = self.digits.len().max(o.digits.len());
        for i in 0..n {
            let a = self.digits.get(i).copied().unwrap_or(0);
            let b = o.digits.get(i).copied().unwrap_or(0);
            if a != b {
                return a.cmp(&b);
            }
        }
        Ordering::Equal
    }

    /// Whether this is an integer value.
    #[must_use]
    pub fn is_int(&self) -> bool {
        self.exp >= 0
    }

    /// `Float.Int64()` truncation (saturating).
    #[must_use]
    pub fn to_i64(&self) -> i64 {
        if self.digits.is_empty() {
            return 0;
        }
        let keep = self.digits.len() as i64 + self.exp;
        if keep <= 0 {
            return 0;
        }
        let mut v: i64 = 0;
        for i in 0..keep {
            let d = usize::try_from(i)
                .ok()
                .and_then(|i| self.digits.get(i))
                .copied()
                .unwrap_or(0);
            v = match v.checked_mul(10).and_then(|v| v.checked_add(i64::from(d))) {
                Some(v) => v,
                None => return if self.neg { i64::MIN } else { i64::MAX },
            };
        }
        if self.neg { -v } else { v }
    }
}

impl PartialOrd for Dec {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for Dec {
    fn cmp(&self, o: &Self) -> Ordering {
        let za = self.digits.is_empty();
        let zb = o.digits.is_empty();
        match (za, zb) {
            (true, true) => return Ordering::Equal,
            (true, false) => {
                return if o.neg {
                    Ordering::Greater
                } else {
                    Ordering::Less
                };
            }
            (false, true) => {
                return if self.neg {
                    Ordering::Less
                } else {
                    Ordering::Greater
                };
            }
            _ => {}
        }
        match (self.neg, o.neg) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => self.cmp_abs(o),
            (true, true) => o.cmp_abs(self),
        }
    }
}

/// `query.parseNumber`: integer strings use the `^\d+(\.\d+)?` prefix (so a
/// signed integer does not match), others too.
#[must_use]
pub fn parse_number(s: &str) -> Option<Dec> {
    let prefix = extract_num(s);
    if prefix.is_empty() {
        return None;
    }
    Dec::parse_float(prefix)
}

/// `^\d+(\.\d+)?`.
fn extract_num(s: &str) -> &str {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 {
        return "";
    }
    if i < b.len() && b[i] == b'.' {
        let mut j = i + 1;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > i + 1 {
            i = j;
        }
    }
    &s[..i]
}

/// Indexer value parsing (`big.Int.SetString` else `big.ParseFloat`).
#[must_use]
pub fn parse_index_value(s: &str) -> Option<Dec> {
    let b = s.as_bytes();
    let digits_start = usize::from(!b.is_empty() && (b[0] == b'+' || b[0] == b'-'));
    if b.len() > digits_start && b[digits_start..].iter().all(u8::is_ascii_digit) {
        return Dec::parse_float(s);
    }
    Dec::parse_float(s)
}

// --- times -----------------------------------------------------------------

/// Unix nanoseconds of Go's zero time.
pub const GO_ZERO_NANOS: i128 = -62_135_596_800 * 1_000_000_000;

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
    }
}

fn digits_n(s: &[u8], i: usize, n: usize) -> Option<i64> {
    let part = s.get(i..i + n)?;
    if !part.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(part).ok()?.parse().ok()
}

fn quote_go(s: &str) -> String {
    crate::json::quote(s)
}

/// `time.Parse("2006-01-02", s)`; returns (year, unix nanos) or Go's error
/// message.
fn parse_date_detailed(s: &str) -> Result<(i64, i128), String> {
    let b = s.as_bytes();
    let err = |elem: &str, rest: &str| {
        format!(
            "parsing time {} as \"2006-01-02\": cannot parse {} as {}",
            quote_go(s),
            quote_go(rest),
            quote_go(elem)
        )
    };
    let y = digits_n(b, 0, 4).ok_or_else(|| err("2006", s))?;
    if b.get(4) != Some(&b'-') {
        return Err(err("-", s.get(4..).unwrap_or("")));
    }
    let m = digits_n(b, 5, 2).ok_or_else(|| err("01", s.get(5..).unwrap_or("")))?;
    if !(1..=12).contains(&m) {
        return Err(format!("parsing time {}: month out of range", quote_go(s)));
    }
    if b.get(7) != Some(&b'-') {
        return Err(err("-", s.get(7..).unwrap_or("")));
    }
    let d = digits_n(b, 8, 2).ok_or_else(|| err("02", s.get(8..).unwrap_or("")))?;
    if b.len() > 10 {
        return Err(format!(
            "parsing time {}: extra text: {}",
            quote_go(s),
            quote_go(&s[10..])
        ));
    }
    if d < 1 || d > days_in_month(y, m) {
        return Err(format!("parsing time {}: day out of range", quote_go(s)));
    }
    Ok((
        y,
        i128::from(days_from_civil(y, m, d)) * 86_400 * 1_000_000_000,
    ))
}

/// `time.Parse(time.RFC3339, s)`: (year, unix nanos) or Go's error.
fn parse_rfc3339_detailed(s: &str) -> Result<(i64, i128), String> {
    const L: &str = "2006-01-02T15:04:05Z07:00";
    let b = s.as_bytes();
    let err = |elem: &str, rest: &str| {
        format!(
            "parsing time {} as \"{L}\": cannot parse {} as {}",
            quote_go(s),
            quote_go(rest),
            quote_go(elem)
        )
    };
    let rest = |i: usize| s.get(i..).unwrap_or("");
    let y = digits_n(b, 0, 4).ok_or_else(|| err("2006", s))?;
    if b.get(4) != Some(&b'-') {
        return Err(err("-", rest(4)));
    }
    let mo = digits_n(b, 5, 2).ok_or_else(|| err("01", rest(5)))?;
    if !(1..=12).contains(&mo) {
        return Err(format!("parsing time {}: month out of range", quote_go(s)));
    }
    if b.get(7) != Some(&b'-') {
        return Err(err("-", rest(7)));
    }
    let d = digits_n(b, 8, 2).ok_or_else(|| err("02", rest(8)))?;
    if b.get(10) != Some(&b'T') {
        return Err(err("T", rest(10)));
    }
    let hh = digits_n(b, 11, 2).ok_or_else(|| err("15", rest(11)))?;
    if hh > 23 {
        return Err(format!("parsing time {}: hour out of range", quote_go(s)));
    }
    if b.get(13) != Some(&b':') {
        return Err(err(":", rest(13)));
    }
    let mi = digits_n(b, 14, 2).ok_or_else(|| err("04", rest(14)))?;
    if mi > 59 {
        return Err(format!("parsing time {}: minute out of range", quote_go(s)));
    }
    if b.get(16) != Some(&b':') {
        return Err(err(":", rest(16)));
    }
    let ss = digits_n(b, 17, 2).ok_or_else(|| err("05", rest(17)))?;
    if ss > 59 {
        return Err(format!("parsing time {}: second out of range", quote_go(s)));
    }
    let mut i = 19;
    let mut nanos: i128 = 0;
    if b.get(i) == Some(&b'.') || b.get(i) == Some(&b',') {
        let st = i + 1;
        let mut j = st;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > st {
            let frac = &s[st..j];
            let mut f = frac.to_owned();
            f.truncate(9);
            while f.len() < 9 {
                f.push('0');
            }
            nanos = f.parse().unwrap_or(0);
            i = j;
        }
    }
    let off: i64 = match b.get(i) {
        Some(b'Z') => {
            i += 1;
            0
        }
        Some(b'+' | b'-') => {
            let sign = if b[i] == b'-' { -1 } else { 1 };
            let oh = digits_n(b, i + 1, 2).ok_or_else(|| err("Z07:00", rest(i)))?;
            if b.get(i + 3) != Some(&b':') {
                return Err(err("Z07:00", rest(i)));
            }
            let om = digits_n(b, i + 4, 2).ok_or_else(|| err("Z07:00", rest(i)))?;
            i += 6;
            sign * (oh * 3600 + om * 60)
        }
        _ => return Err(err("Z07:00", rest(i))),
    };
    if i < b.len() {
        return Err(format!(
            "parsing time {}: extra text: {}",
            quote_go(s),
            quote_go(&s[i..])
        ));
    }
    if d < 1 || d > days_in_month(y, mo) {
        return Err(format!("parsing time {}: day out of range", quote_go(s)));
    }
    let secs = days_from_civil(y, mo, d) * 86_400 + hh * 3600 + mi * 60 + ss - off;
    Ok((y, i128::from(secs) * 1_000_000_000 + nanos))
}

fn parse_date(s: &str) -> Option<i128> {
    parse_date_detailed(s).ok().map(|(_, t)| t)
}

fn parse_rfc3339(s: &str) -> Option<i128> {
    parse_rfc3339_detailed(s).ok().map(|(_, t)| t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(pairs: &[(&str, &str)]) -> BTreeMap<String, Vec<String>> {
        let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (k, v) in pairs {
            m.entry((*k).to_owned()).or_default().push((*v).to_owned());
        }
        m
    }

    #[test]
    fn parse_and_string() {
        let q = Query::new("tm.event = 'Tx' AND tx.height>=5 AND a.b CONTAINS 'x' AND c.d EXISTS")
            .unwrap();
        assert_eq!(
            q.to_go_string(),
            "tm.event = 'Tx' AND tx.height >= 5 AND a.b CONTAINS 'x' AND c.d EXISTS"
        );
        assert_eq!(Query::new("tx.height>0 AND").unwrap_err(), "offset 15: EOF");
        assert_eq!(
            Query::new("= 5").unwrap_err(),
            "offset 0: got = operator, wanted tag"
        );
        assert_eq!(
            Query::new("tx.height >< 1").unwrap_err(),
            "offset 11: got < operator, wanted number, timestamp or datestamp"
        );
        assert_eq!(
            Query::new("a.b = 'x' OR a.c = 'y'").unwrap_err(),
            "offset 10: got tag, want 6"
        );
    }

    #[test]
    fn matching() {
        let q = Query::new("tm.event='Tx' AND tx.height>3").unwrap();
        assert!(q.matches(&ev(&[("tm.event", "Tx"), ("tx.height", "5")])));
        assert!(!q.matches(&ev(&[("tm.event", "Tx"), ("tx.height", "3")])));
        let q = Query::new("a.n = 1.50").unwrap();
        assert!(q.matches(&ev(&[("a.n", "1.5utia")])));
        assert!(!q.matches(&ev(&[("a.n", "-1.5")])));
    }
}
