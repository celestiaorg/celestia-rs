//! go-amino v0.16 `UnmarshalJSON` of a parameter value into its key-table
//! type, as cosmos-sdk v0.46 `Subspace.Update` runs it, and amino's
//! `MarshalJSON` of the result (what `Set` stores).
//!
//! `Update` decodes into the stored value (`GetIfExists`), so an
//! `omitempty` struct field the JSON leaves out keeps it; any other
//! missing field becomes zero. `null` sets the zero value. 64-bit
//! integers (and `time.Duration`) must be quoted: amino strips the quotes
//! and hands the rest to `encoding/json`. Other scalars go to
//! `encoding/json` as they are; `sdk.Dec` and `sdk.Int` to their
//! `UnmarshalJSON`. Errors are Go's texts.

use std::fmt::Write as _;

use legacy_amino::gojson_scalar as gj;
use math::LegacyDec;
use x_params::v046::{Object, json_string};

/// A key-table Go type.
#[derive(Debug, Clone, Copy)]
pub enum Ty {
    /// `uint64` / `int64` / `time.Duration`: (Go type name, signed).
    Int64(&'static str, bool),
    /// `uint32`.
    Uint32,
    /// `bool`.
    Bool,
    /// `string`.
    Str,
    /// `sdk.Dec`.
    Dec,
    /// `sdk.Int`.
    Int,
    /// `[]string`.
    Strings,
    /// A struct (fields in Go order).
    Struct(&'static [Field]),
    /// A slice of a struct (`[]types.Coin`).
    Slice(&'static [Field]),
}

/// One struct field.
#[derive(Debug, Clone, Copy)]
pub struct Field {
    /// JSON name.
    pub json: &'static str,
    /// Go type.
    pub ty: Ty,
    /// `omitempty` JSON tag.
    pub omitempty: bool,
    /// A pointer (`*time.Duration`): nil is `None`.
    pub ptr: bool,
}

const fn f(json: &'static str, ty: Ty, omitempty: bool, ptr: bool) -> Field {
    Field {
        json,
        ty,
        omitempty,
        ptr,
    }
}

const I64: Ty = Ty::Int64("int64", true);
const U64: Ty = Ty::Int64("uint64", false);
const DUR: Ty = Ty::Int64("time.Duration", true);

/// `types.Coin`.
pub const COIN: &[Field] = &[
    f("denom", Ty::Str, false, false),
    f("amount", Ty::Int, false, false),
];
/// `abci.BlockParams`.
pub const BLOCK_PARAMS: &[Field] = &[
    f("max_bytes", I64, true, false),
    f("max_gas", I64, true, false),
];
/// `types.EvidenceParams`.
pub const EVIDENCE_PARAMS: &[Field] = &[
    f("max_age_num_blocks", I64, true, false),
    f("max_age_duration", DUR, false, false),
    f("max_bytes", I64, true, false),
];
/// `types.VersionParams`.
pub const VERSION_PARAMS: &[Field] = &[f("app_version", U64, true, false)];
/// gov `v1.DepositParams`.
pub const DEPOSIT_PARAMS: &[Field] = &[
    f("min_deposit", Ty::Slice(COIN), true, false),
    f("max_deposit_period", DUR, true, true),
];
/// gov `v1.VotingParams`.
pub const VOTING_PARAMS: &[Field] = &[f("voting_period", DUR, true, true)];
/// gov `v1.TallyParams`.
pub const TALLY_PARAMS: &[Field] = &[
    f("quorum", Ty::Str, true, false),
    f("threshold", Ty::Str, true, false),
    f("veto_threshold", Ty::Str, true, false),
];

/// A decoded value.
#[derive(Debug, Clone, PartialEq)]
pub enum D {
    /// 64-bit integer (as `i128` to hold both signs).
    Int64(i128),
    /// `uint32`.
    Uint32(u32),
    /// `bool`.
    Bool(bool),
    /// `string`.
    Str(String),
    /// `sdk.Dec`; `None` is nil.
    Dec(Option<LegacyDec>),
    /// `sdk.Int` as its decimal text; `None` is nil.
    Int(Option<String>),
    /// `[]string`; `None` is nil.
    Strings(Option<Vec<String>>),
    /// Struct fields; a `None` field is a nil pointer.
    Struct(Vec<Option<D>>),
    /// Slice of structs; `None` is nil.
    Slice(Option<Vec<D>>),
}

/// The zero value of `ty`.
#[must_use]
pub fn zero(ty: Ty) -> D {
    match ty {
        Ty::Int64(..) => D::Int64(0),
        Ty::Uint32 => D::Uint32(0),
        Ty::Bool => D::Bool(false),
        Ty::Str => D::Str(String::new()),
        Ty::Dec => D::Dec(None),
        Ty::Int => D::Int(None),
        Ty::Strings => D::Strings(None),
        Ty::Struct(fields) => D::Struct(
            fields
                .iter()
                .map(|f| (!f.ptr).then(|| zero(f.ty)))
                .collect(),
        ),
        Ty::Slice(_) => D::Slice(None),
    }
}

type R<T> = std::result::Result<T, String>;

/// A Go runtime panic during decoding (amino slices a lone `"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Panic(pub String);

/// `UnmarshalJSON(bz, &dest)` with `dest` holding `prev`: the decoded
/// value, Go's error, or a panic.
pub fn decode(bz: &[u8], ty: Ty, prev: D) -> std::result::Result<R<D>, Panic> {
    if bz.is_empty() {
        return Ok(Err("UnmarshalJSON cannot decode empty bytes".to_owned()));
    }
    dec(bz, ty, prev)
}

fn stdlib<T>(bz: &[u8], f: impl FnOnce(&[u8]) -> R<Option<T>>) -> R<Option<T>> {
    gj::check_valid(bz)?;
    f(gj::trim(bz))
}

fn dec(bz: &[u8], ty: Ty, prev: D) -> std::result::Result<R<D>, Panic> {
    if bz == b"null" {
        return Ok(Ok(zero(ty)));
    }
    Ok(match ty {
        Ty::Int64(go, signed) => {
            if bz[0] != b'"' || bz[bz.len() - 1] != b'"' {
                return Ok(Err(format!(
                    "invalid character -- Amino:JSON int/int64/uint/uint64 expects quoted values for javascript numeric support, got: {}.",
                    String::from_utf8_lossy(bz)
                )));
            }
            if bz.len() == 1 {
                return Err(Panic(
                    "runtime error: slice bounds out of range [1:0]".to_owned(),
                ));
            }
            stdlib(&bz[1..bz.len() - 1], |raw| {
                gj::unmarshal_int(raw, go, signed, 64)
            })
            .map(|v| D::Int64(v.unwrap_or(0)))
        }
        Ty::Uint32 => stdlib(bz, |raw| gj::unmarshal_int(raw, "uint32", false, 32))
            .map(|v| D::Uint32(v.map_or(0, |n| u32::try_from(n).unwrap_or(0)))),
        Ty::Bool => {
            stdlib(bz, |raw| gj::unmarshal_bool(raw, "bool")).map(|v| D::Bool(v.unwrap_or(false)))
        }
        Ty::Str => stdlib(bz, |raw| gj::unmarshal_string(raw, "string"))
            .map(|v| D::Str(String::from_utf8_lossy(&v.unwrap_or_default()).into_owned())),
        // `Dec.UnmarshalJSON` and `Int.UnmarshalJSON` read the JSON into a
        // `string` (a padded null leaves it empty), then parse it.
        Ty::Dec => text(bz).and_then(|s| {
            LegacyDec::from_str_go(&s.unwrap_or_default())
                .map(|d| D::Dec(Some(d)))
                .map_err(|e| e.to_string())
        }),
        Ty::Int => text(bz).and_then(|s| {
            let s = s.unwrap_or_default();
            match math::parse_big_int_text(s.as_bytes()) {
                None => Err(format!(
                    "math/big: cannot unmarshal {} into a *big.Int",
                    go_quote(&s)
                )),
                Some(i) if i.bits() > 256 => Err(format!("integer out of range: {s}")),
                Some(i) => Ok(D::Int(Some(i.to_string()))),
            }
        }),
        Ty::Strings => return slice(bz, Ty::Str),
        Ty::Slice(fields) => return slice(bz, Ty::Struct(fields)),
        Ty::Struct(fields) => return structure(bz, fields, prev),
    })
}

/// amino `decodeReflectJSONSlice`: an empty slice is nil.
fn slice(bz: &[u8], elem: Ty) -> std::result::Result<R<D>, Panic> {
    let nil = || match elem {
        Ty::Str => D::Strings(None),
        _ => D::Slice(None),
    };
    let items = match raw_items(bz, "[]json.RawMessage") {
        Err(e) => return Ok(Err(e)),
        Ok(None) => return Ok(Ok(nil())),
        Ok(Some(items)) if items.is_empty() => return Ok(Ok(nil())),
        Ok(Some(items)) => items,
    };
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        match dec(it, elem, zero(elem))? {
            Ok(v) => out.push(v),
            Err(e) => return Ok(Err(e)),
        }
    }
    Ok(Ok(match elem {
        Ty::Str => D::Strings(Some(
            out.into_iter()
                .map(|v| match v {
                    D::Str(s) => s,
                    _ => String::new(),
                })
                .collect(),
        )),
        _ => D::Slice(Some(out)),
    }))
}

/// amino `decodeReflectJSONStruct` into `prev`.
fn structure(bz: &[u8], fields: &'static [Field], prev: D) -> std::result::Result<R<D>, Panic> {
    if let Err(e) = gj::check_valid(bz) {
        return Ok(Err(e));
    }
    let raw = gj::trim(bz);
    // A JSON null into the map leaves it nil: every field is then missing.
    if let Some(k) = gj::kind_name(raw).filter(|k| *k != "object") {
        return Ok(Err(gj::type_error(k, "map[string]json.RawMessage")));
    }
    let entries = if raw.first() == Some(&b'{') {
        gj::object_entries(raw)
    } else {
        Vec::new()
    };
    let mut prev = match prev {
        D::Struct(v) => v,
        _ => fields
            .iter()
            .map(|f| (!f.ptr).then(|| zero(f.ty)))
            .collect(),
    };
    for (i, fd) in fields.iter().enumerate() {
        // The last duplicate key wins in a Go map.
        let v = entries
            .iter()
            .rev()
            .find(|(k, _)| k.as_slice() == fd.json.as_bytes())
            .map(|(_, v)| *v)
            .filter(|v| !v.is_empty());
        let Some(v) = v else {
            if !fd.omitempty {
                prev[i] = (!fd.ptr).then(|| zero(fd.ty));
            }
            continue;
        };
        if fd.ptr && v == b"null" {
            prev[i] = None;
            continue;
        }
        let before = prev[i].take().unwrap_or_else(|| zero(fd.ty));
        match dec(v, fd.ty, before)? {
            Ok(d) => prev[i] = Some(d),
            Err(e) => return Ok(Err(e)),
        }
    }
    Ok(Ok(D::Struct(prev)))
}

/// `json.Unmarshal(bz, &text)` for a `string`.
fn text(bz: &[u8]) -> R<Option<String>> {
    stdlib(bz, |raw| gj::unmarshal_string(raw, "string"))
        .map(|v| v.map(|b| String::from_utf8_lossy(&b).into_owned()))
}

/// `json.Unmarshal(bz, &[]json.RawMessage)`: `None` for null.
fn raw_items<'a>(bz: &'a [u8], go: &str) -> R<Option<Vec<&'a [u8]>>> {
    gj::check_valid(bz)?;
    let raw = gj::trim(bz);
    match gj::kind_name(raw) {
        None => Ok(None),
        Some("array") => Ok(Some(gj::array_items(raw))),
        Some(k) => Err(gj::type_error(k, go)),
    }
}

/// Go `%q` of an ASCII-printable string (others escaped as `\xNN`).
fn go_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn is_empty(d: &D) -> bool {
    match d {
        D::Int64(v) => *v == 0,
        D::Uint32(v) => *v == 0,
        D::Bool(v) => !*v,
        D::Str(s) => s.is_empty(),
        D::Strings(v) => v.as_ref().is_none_or(Vec::is_empty),
        D::Slice(v) => v.as_ref().is_none_or(Vec::is_empty),
        D::Dec(_) | D::Int(_) | D::Struct(_) => false,
    }
}

/// amino `MarshalJSON` of `d` (of type `ty`).
#[must_use]
pub fn encode(d: &D, ty: Ty) -> String {
    match (d, ty) {
        (D::Int64(v), Ty::Int64(..)) => format!("\"{v}\""),
        (D::Uint32(v), _) => v.to_string(),
        (D::Bool(v), _) => v.to_string(),
        (D::Str(s), _) => json_string(s),
        (D::Dec(v), _) => json_string(&v.clone().unwrap_or_else(LegacyDec::zero).to_string()),
        (D::Int(v), _) => json_string(v.as_deref().unwrap_or("0")),
        (D::Strings(Some(v)), _) => {
            let items: Vec<String> = v.iter().map(|s| json_string(s)).collect();
            format!("[{}]", items.join(","))
        }
        (D::Slice(Some(v)), Ty::Slice(fields)) => {
            let items: Vec<String> = v.iter().map(|x| encode(x, Ty::Struct(fields))).collect();
            format!("[{}]", items.join(","))
        }
        (D::Struct(v), Ty::Struct(fields)) => {
            let mut o = Object::new();
            for (fd, x) in fields.iter().zip(v) {
                match x {
                    None => {
                        if !fd.omitempty {
                            o.field(fd.json, "null".to_owned());
                        }
                    }
                    Some(x) if fd.omitempty && !fd.ptr && is_empty(x) => {}
                    Some(x) => {
                        o.field(fd.json, encode(x, fd.ty));
                    }
                }
            }
            o.finish()
        }
        _ => "null".to_owned(),
    }
}

/// Field `i` of a decoded struct as an integer (nil and other kinds: 0).
#[must_use]
pub fn int_field(d: &D, i: usize) -> Option<i128> {
    match d {
        D::Struct(v) => match v.get(i)? {
            Some(D::Int64(n)) => Some(*n),
            _ => None,
        },
        _ => None,
    }
}
