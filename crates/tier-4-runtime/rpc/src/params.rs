//! Route argument decoding as `rpc/jsonrpc/server` does it: URI query
//! parameters (`httpParamsToArgs`, `nonJSONStringToArg`) and JSON-RPC
//! params (`jsonParamsToArgs`), both ending in cmtjson decoding of the Go
//! argument type, with Go's error messages.

use base64::Engine as _;

use crate::json::{self, Node, Val};

/// The Go type of a route argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgType {
    /// `*int64`.
    I64Ptr,
    /// `*int`.
    IntPtr,
    /// `int64`.
    I64,
    /// `uint64`.
    U64,
    /// `uint`.
    Uint,
    /// `bool`.
    Bool,
    /// `string`.
    Str,
    /// `bytes.HexBytes`.
    HexBytes,
    /// `[]byte` (also `types.Tx`).
    Bytes,
    /// `[][]byte`.
    BytesList,
    /// `types.Evidence` (registered interface).
    Evidence,
}

impl ArgType {
    fn go_kind(self) -> &'static str {
        match self {
            Self::I64Ptr | Self::I64 => "int64",
            Self::IntPtr => "int",
            Self::U64 => "uint64",
            Self::Uint => "uint",
            Self::Bool => "bool",
            Self::Str => "string",
            Self::HexBytes | Self::Bytes | Self::BytesList => "slice",
            Self::Evidence => "interface",
        }
    }

    fn is_int(self) -> bool {
        matches!(
            self,
            Self::I64Ptr | Self::IntPtr | Self::I64 | Self::U64 | Self::Uint
        )
    }

    fn is_byte_slice(self) -> bool {
        matches!(self, Self::HexBytes | Self::Bytes)
    }
}

/// A decoded argument (Go's value; absent pointer = `Null`).
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    /// A nil pointer / zero value of a non-pointer argument.
    Null,
    /// Integer.
    Int(i128),
    /// Bool.
    Bool(bool),
    /// String.
    Str(String),
    /// Bytes.
    Bytes(Vec<u8>),
    /// List of byte slices.
    BytesList(Vec<Vec<u8>>),
    /// Evidence (raw JSON of the registered interface value).
    Evidence(Box<ctypes::Evidence>),
    /// Evidence decoding produced a value that failed later (kept for
    /// `ValidateBasic` errors): the Go zero-value `DuplicateVoteEvidence`.
    EmptyDuplicateVote,
}

impl Arg {
    /// Integer value (`0` for `Null`).
    #[must_use]
    pub fn int(&self) -> i128 {
        match self {
            Self::Int(i) => *i,
            _ => 0,
        }
    }

    /// Optional integer (pointer arguments).
    #[must_use]
    pub fn opt_int(&self) -> Option<i128> {
        match self {
            Self::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Bool (`false` for `Null`).
    #[must_use]
    pub fn bool(&self) -> bool {
        matches!(self, Self::Bool(true))
    }

    /// String (empty for `Null`).
    #[must_use]
    pub fn str(&self) -> &str {
        match self {
            Self::Str(s) => s,
            _ => "",
        }
    }

    /// Bytes (empty for `Null`).
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::Bytes(b) => b,
            _ => &[],
        }
    }

    /// Whether the value is Go's zero value (for `Cacheable`).
    #[must_use]
    pub fn is_zero(&self) -> bool {
        match self {
            Self::Null | Self::EmptyDuplicateVote => true,
            Self::Int(i) => *i == 0,
            Self::Bool(b) => !b,
            Self::Str(s) => s.is_empty(),
            Self::Bytes(b) => b.is_empty(),
            Self::BytesList(l) => l.is_empty(),
            Self::Evidence(_) => false,
        }
    }
}

fn go_quote(s: &str) -> String {
    // Go %q of a string (strconv.Quote); JSON quoting is close enough for
    // the ASCII inputs this is used with, but escape like Go.
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{7}' => out.push_str("\\a"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\u{b}' => out.push_str("\\v"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Go `encoding/json` integer decoding of a number literal into `ty`.
fn unmarshal_int(num_text: &str, ty: ArgType) -> Result<i128, String> {
    let type_name = ty.go_kind();
    let n = json::parse(num_text)?;
    match n.into_val() {
        Val::Num(t) => {
            let err =
                || format!("json: cannot unmarshal number {t} into Go value of type {type_name}");
            if t.contains(['.', 'e', 'E']) {
                return Err(err());
            }
            let v: i128 = t.parse().map_err(|_| err())?;
            let (lo, hi) = match ty {
                ArgType::U64 | ArgType::Uint => (0, i128::from(u64::MAX)),
                _ => (i128::from(i64::MIN), i128::from(i64::MAX)),
            };
            if v < lo || v > hi {
                return Err(err());
            }
            Ok(v)
        }
        other => Err(format!(
            "json: cannot unmarshal {} into Go value of type {type_name}",
            other.kind()
        )),
    }
}

/// Go `base64.StdEncoding.DecodeString` with its `CorruptInputError`
/// offsets (`\r`/`\n` ignored).
pub fn go_base64(s: &str) -> Result<Vec<u8>, String> {
    let raw: Vec<u8> = s.bytes().collect();
    let filtered: Vec<u8> = raw
        .iter()
        .copied()
        .filter(|&b| b != b'\r' && b != b'\n')
        .collect();
    if let Ok(v) = base64::engine::general_purpose::STANDARD.decode(&filtered) {
        return Ok(v);
    }
    // Locate the offending input byte like Go's decodeQuantum.
    let is_b64 = |b: u8| b.is_ascii_alphanumeric() || b == b'+' || b == b'/';
    let mut si = 0usize;
    loop {
        // One quantum of up to 4 significant characters.
        let mut j = 0;
        while j < 4 {
            while si < raw.len() && (raw[si] == b'\n' || raw[si] == b'\r') {
                si += 1;
            }
            if si == raw.len() {
                if j == 0 {
                    return Ok(Vec::new());
                }
                if j == 1 {
                    return Err(format!("illegal base64 data at input byte {}", si - j));
                }
                return Err(format!("illegal base64 data at input byte {si}"));
            }
            let c = raw[si];
            si += 1;
            if c == b'=' {
                if j < 2 {
                    return Err(format!("illegal base64 data at input byte {}", si - 1));
                }
                // Padding: must complete the quantum then end.
                if j == 2 {
                    while si < raw.len() && (raw[si] == b'\n' || raw[si] == b'\r') {
                        si += 1;
                    }
                    if si == raw.len() {
                        return Err(format!("illegal base64 data at input byte {}", raw.len()));
                    }
                    if raw[si] != b'=' {
                        return Err(format!("illegal base64 data at input byte {}", si - 1));
                    }
                    si += 1;
                }
                while si < raw.len() && (raw[si] == b'\n' || raw[si] == b'\r') {
                    si += 1;
                }
                if si < raw.len() {
                    return Err(format!("illegal base64 data at input byte {si}"));
                }
                return Err(format!(
                    "illegal base64 data at input byte {}",
                    si.saturating_sub(1)
                ));
            }
            if !is_b64(c) {
                return Err(format!("illegal base64 data at input byte {}", si - 1));
            }
            j += 1;
        }
        if si >= raw.len() {
            return Err(format!("illegal base64 data at input byte {}", raw.len()));
        }
    }
}

/// cmtjson `Unmarshal(bz, *T)`.
pub fn cmtjson_decode(bz: &str, ty: ArgType) -> Result<Arg, String> {
    if bz.is_empty() {
        return Err("cannot decode empty bytes".into());
    }
    if ty == ArgType::Evidence {
        return decode_evidence(bz);
    }
    // Go checks the whole input first (`json.Unmarshal` syntax) for the
    // stdlib paths; integers check the quoting first.
    if bz == "null" {
        return Ok(Arg::Null);
    }
    match ty {
        ArgType::I64Ptr | ArgType::IntPtr | ArgType::I64 | ArgType::U64 | ArgType::Uint => {
            let b = bz.as_bytes();
            if b[0] != b'"' || b[b.len() - 1] != b'"' || b.len() < 2 {
                return Err(format!(
                    "invalid 64-bit integer encoding {}, expected string",
                    go_quote(bz)
                ));
            }
            unmarshal_int(&bz[1..bz.len() - 1], ty).map(Arg::Int)
        }
        ArgType::Bool => {
            let n = json::parse(bz)?;
            match n.into_val() {
                Val::Bool(b) => Ok(Arg::Bool(b)),
                Val::Null => Ok(Arg::Bool(false)),
                other => Err(format!(
                    "json: cannot unmarshal {} into Go value of type bool",
                    other.kind()
                )),
            }
        }
        ArgType::Str => {
            let n = json::parse(bz)?;
            match n.into_val() {
                Val::Str(s) => Ok(Arg::Str(s)),
                Val::Null => Ok(Arg::Str(String::new())),
                other => Err(format!(
                    "json: cannot unmarshal {} into Go value of type string",
                    other.kind()
                )),
            }
        }
        ArgType::HexBytes => {
            let b = bz.as_bytes();
            if b.len() < 2 || b[0] != b'"' || b[b.len() - 1] != b'"' {
                return Err(format!("invalid hex string: {bz}"));
            }
            let inner = &bz[1..bz.len() - 1];
            hex::decode(inner)
                .map(Arg::Bytes)
                .map_err(|e| crate::indexer::go_hex_error(inner, &e))
        }
        ArgType::Bytes => decode_bytes(bz).map(|b| {
            if b.is_empty() {
                Arg::Null
            } else {
                Arg::Bytes(b)
            }
        }),
        ArgType::BytesList => {
            let n = json::parse(bz)?;
            let items = match n.into_val() {
                Val::Arr(items) => items,
                other => {
                    return Err(format!(
                        "json: cannot unmarshal {} into Go value of type []json.RawMessage",
                        other.kind()
                    ));
                }
            };
            let mut out = Vec::new();
            for it in items {
                if it.raw == "null" {
                    out.push(Vec::new());
                    continue;
                }
                out.push(decode_bytes(it.raw)?);
            }
            Ok(if out.is_empty() {
                Arg::Null
            } else {
                Arg::BytesList(out)
            })
        }
        ArgType::Evidence => unreachable_evidence(),
    }
}

fn unreachable_evidence() -> Result<Arg, String> {
    Err("unreachable".into())
}

/// `json.Unmarshal(bz, *[]byte)`.
fn decode_bytes(bz: &str) -> Result<Vec<u8>, String> {
    let n = json::parse(bz)?;
    match n.into_val() {
        Val::Str(s) => go_base64(&s),
        Val::Null => Ok(Vec::new()),
        other => Err(format!(
            "json: cannot unmarshal {} into Go value of type []uint8",
            other.kind()
        )),
    }
}

/// `nonJSONStringToArg` then `jsonStringToArg` for one URI parameter.
pub fn uri_arg(ty: ArgType, arg: &str) -> Result<Arg, String> {
    let is_int_string = {
        let b = arg.strip_prefix('-').unwrap_or(arg);
        !b.is_empty() && b.bytes().all(|c| c.is_ascii_digit())
    };
    let is_quoted = arg.starts_with('"') && arg.ends_with('"');
    let is_hex = arg
        .as_bytes()
        .get(..2)
        .is_some_and(|p| p.eq_ignore_ascii_case(b"0x"));
    if is_int_string && ty.is_int() {
        return cmtjson_decode(&format!("\"{arg}\""), ty);
    }
    if is_hex {
        if ty != ArgType::Str && !ty.is_byte_slice() {
            return Err(format!(
                "got a hex string arg, but expected '{}'",
                ty.go_kind()
            ));
        }
        let v = hex::decode(&arg[2..]).map_err(|e| crate::indexer::go_hex_error(&arg[2..], &e))?;
        if ty == ArgType::Str {
            return Ok(Arg::Str(String::from_utf8_lossy(&v).into_owned()));
        }
        return Ok(Arg::Bytes(v));
    }
    if is_quoted && ty.is_byte_slice() {
        return match cmtjson_decode(arg, ArgType::Str)? {
            Arg::Str(s) => Ok(Arg::Bytes(s.into_bytes())),
            _ => Ok(Arg::Bytes(Vec::new())),
        };
    }
    cmtjson_decode(arg, ty)
}

/// `jsonParamsToArgs`: `params` is the raw `params` member.
pub fn json_args(names: &[&str], types: &[ArgType], params: &str) -> Result<Vec<Arg>, String> {
    let n = json::parse(params)
        .map_err(|e| format!("unknown type for JSON params: {e}. Expected map or array"))?;
    match &n.v {
        Val::Null => Ok(vec![Arg::Null; names.len()]),
        Val::Obj(_) => {
            let mut out = Vec::new();
            for (name, ty) in names.iter().zip(types) {
                match n.get(name) {
                    Some(p) if !p.raw.is_empty() => out.push(cmtjson_decode(p.raw, *ty)?),
                    _ => out.push(Arg::Null),
                }
            }
            Ok(out)
        }
        Val::Arr(items) => {
            if items.len() != names.len() {
                return Err(format!(
                    "expected {} parameters ([{}]), got {} ([{}])",
                    names.len(),
                    names.join(" "),
                    items.len(),
                    items
                        .iter()
                        .map(|i| format!(
                            "[{}]",
                            i.raw
                                .bytes()
                                .map(|b| b.to_string())
                                .collect::<Vec<_>>()
                                .join(" ")
                        ))
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
            }
            items
                .iter()
                .zip(types)
                .map(|(p, ty)| cmtjson_decode(p.raw, *ty))
                .collect()
        }
        other => Err(format!(
            "unknown type for JSON params: json: cannot unmarshal {} into Go value of type []json.RawMessage. Expected map or array",
            other.kind()
        )),
    }
}

// --- evidence --------------------------------------------------------------

fn field<'a>(n: &'a Node<'a>, name: &str) -> Option<&'a Node<'a>> {
    n.get(name).filter(|v| v.raw != "null")
}

fn json_i64(n: Option<&Node<'_>>, what: &str) -> Result<i64, String> {
    let Some(n) = n else { return Ok(0) };
    let Val::Str(s) = &n.v else {
        return Err(format!(
            "invalid 64-bit integer encoding {}, expected string",
            go_quote(n.raw)
        ));
    };
    s.parse::<i64>()
        .map_err(|_| format!("json: cannot unmarshal number {s} into Go value of type {what}"))
}

fn json_num<T: TryFrom<i64>>(n: Option<&Node<'_>>, what: &str) -> Result<T, String> {
    let Some(n) = n else {
        return T::try_from(0).map_err(|_| "overflow".to_owned());
    };
    let Val::Num(t) = &n.v else {
        return Err(format!(
            "json: cannot unmarshal {} into Go value of type {what}",
            n.v.kind()
        ));
    };
    t.parse::<i64>()
        .ok()
        .and_then(|v| T::try_from(v).ok())
        .ok_or_else(|| format!("json: cannot unmarshal number {t} into Go value of type {what}"))
}

fn json_hex(n: Option<&Node<'_>>) -> Result<Vec<u8>, String> {
    match n {
        None => Ok(Vec::new()),
        Some(n) => match cmtjson_decode(n.raw, ArgType::HexBytes)? {
            Arg::Bytes(b) => Ok(b),
            _ => Ok(Vec::new()),
        },
    }
}

fn json_b64(n: Option<&Node<'_>>) -> Result<Vec<u8>, String> {
    match n {
        None => Ok(Vec::new()),
        Some(n) => decode_bytes(n.raw),
    }
}

fn json_time(n: Option<&Node<'_>>) -> Result<ctypes::Time, String> {
    let Some(n) = n else {
        return Ok(ctypes::Time::ZERO);
    };
    let b = n.raw.as_bytes();
    if b.len() < 2 || b[0] != b'"' || b[b.len() - 1] != b'"' {
        return Err(format!(
            "JSON time must be an RFC3339 string, but got {}",
            go_quote(n.raw)
        ));
    }
    if b[b.len() - 2] != b'Z' {
        return Err(format!(
            "JSON time must be UTC and end with 'Z', but got {}",
            go_quote(n.raw)
        ));
    }
    let s = n.as_str().unwrap_or_default();
    nodestore::genesis::parse_rfc3339(s).map_err(|e| e.to_string())
}

fn json_block_id(n: Option<&Node<'_>>) -> Result<ctypes::BlockId, String> {
    let Some(n) = n else {
        return Ok(ctypes::BlockId::default());
    };
    let parts = field(n, "parts");
    Ok(ctypes::BlockId {
        hash: json_hex(field(n, "hash"))?,
        part_set_header: ctypes::PartSetHeader {
            total: json_num(parts.and_then(|p| field(p, "total")), "uint32")?,
            hash: json_hex(parts.and_then(|p| field(p, "hash")))?,
        },
    })
}

fn json_vote(n: Option<&Node<'_>>) -> Result<Option<ctypes::Vote>, String> {
    let Some(n) = n else { return Ok(None) };
    Ok(Some(ctypes::Vote {
        vote_type: json_num(field(n, "type"), "types.SignedMsgType")?,
        height: json_i64(field(n, "height"), "int64")?,
        round: json_num(field(n, "round"), "int32")?,
        block_id: json_block_id(field(n, "block_id"))?,
        timestamp: json_time(field(n, "timestamp"))?,
        validator_address: json_hex(field(n, "validator_address"))?,
        validator_index: json_num(field(n, "validator_index"), "int32")?,
        signature: json_b64(field(n, "signature"))?,
        extension: json_b64(field(n, "extension"))?,
        extension_signature: json_b64(field(n, "extension_signature"))?,
    }))
}

/// cmtjson decoding of a `types.Evidence` interface value.
fn decode_evidence(bz: &str) -> Result<Arg, String> {
    let n = json::parse(bz)?;
    if n.raw == "null" {
        return Ok(Arg::Null);
    }
    let Val::Obj(_) = &n.v else {
        return Err(format!(
            "json: cannot unmarshal {} into Go value of type json.interfaceWrapper",
            n.v.kind()
        ));
    };
    let typ = n.get("type").and_then(Node::as_str).unwrap_or_default();
    if typ.is_empty() {
        return Err("interface type cannot be empty".into());
    }
    let Some(value) = n.get("value") else {
        return Err("interface value cannot be empty".into());
    };
    match typ {
        "tendermint/DuplicateVoteEvidence" => {
            let ev = ctypes::DuplicateVoteEvidence {
                vote_a: json_vote(field(value, "vote_a"))?,
                vote_b: json_vote(field(value, "vote_b"))?,
                total_voting_power: json_i64(field(value, "TotalVotingPower"), "int64")?,
                validator_power: json_i64(field(value, "ValidatorPower"), "int64")?,
                timestamp: json_time(field(value, "Timestamp"))?,
            };
            Ok(Arg::Evidence(Box::new(ctypes::Evidence::DuplicateVote(ev))))
        }
        "tendermint/LightClientAttackEvidence" => {
            Err("light client attack evidence is not accepted over JSON-RPC by this node".into())
        }
        "tendermint/PubKeyEd25519"
        | "tendermint/PubKeySecp256k1"
        | "tendermint/PrivKeyEd25519"
        | "tendermint/PrivKeySecp256k1" => Err(format!("invalid type \"{typ}\" for this value")),
        t => Err(format!("unknown type \"{t}\"")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_args_like_go() {
        assert_eq!(uri_arg(ArgType::I64Ptr, "5").unwrap(), Arg::Int(5));
        assert_eq!(
            uri_arg(ArgType::I64Ptr, "abc").unwrap_err(),
            "invalid 64-bit integer encoding \"abc\", expected string"
        );
        assert_eq!(
            uri_arg(ArgType::I64Ptr, "0x05").unwrap_err(),
            "got a hex string arg, but expected 'int64'"
        );
        assert_eq!(
            uri_arg(ArgType::Uint, "-1").unwrap_err(),
            "json: cannot unmarshal number -1 into Go value of type uint"
        );
        assert_eq!(
            uri_arg(ArgType::Bytes, "0x0a0b").unwrap(),
            Arg::Bytes(vec![10, 11])
        );
        assert_eq!(
            uri_arg(ArgType::Bytes, "zz").unwrap_err(),
            "invalid character 'z' looking for beginning of value"
        );
        assert_eq!(
            uri_arg(ArgType::Bytes, "0xzz").unwrap_err(),
            "encoding/hex: invalid byte: U+007A 'z'"
        );
        assert_eq!(
            uri_arg(ArgType::Str, "tx.height=1").unwrap_err(),
            "invalid character 'x' in literal true (expecting 'r')"
        );
        assert_eq!(
            uri_arg(ArgType::Str, "\"a\"").unwrap(),
            Arg::Str("a".into())
        );
    }

    #[test]
    fn uri_arg_non_ascii() {
        // Regression (fuzzing, rpc_request): `arg[..2]` panicked inside a
        // multi-byte character.
        for ty in [
            ArgType::Str,
            ArgType::Bytes,
            ArgType::I64,
            ArgType::HexBytes,
        ] {
            let _ = uri_arg(ty, "a\u{2b0}");
            let _ = uri_arg(ty, "\u{2b0}");
        }
        assert_eq!(
            uri_arg(ArgType::Str, "0x6869").ok(),
            Some(Arg::Str("hi".into()))
        );
    }

    #[test]
    fn json_args_like_go() {
        assert_eq!(
            json_args(&["height"], &[ArgType::I64Ptr], "[]").unwrap_err(),
            "expected 1 parameters ([height]), got 0 ([])"
        );
        assert_eq!(
            json_args(&["height"], &[ArgType::I64Ptr], "{\"height\":2}").unwrap_err(),
            "invalid 64-bit integer encoding \"2\", expected string"
        );
        assert_eq!(
            json_args(&["height"], &[ArgType::I64Ptr], "\"x\"").unwrap_err(),
            "unknown type for JSON params: json: cannot unmarshal string into Go value of type []json.RawMessage. Expected map or array"
        );
    }
}
