//! A schema-driven port of gogoproto-generated `Unmarshal`/`Marshal` code.
//!
//! Every gogoproto message in the celestia-app v10 dependency tree has
//! generated (un)marshal methods with identical structure; this module runs
//! that structure generically over [`celestia_proto::wire_table`]:
//!
//! * varints accumulate up to 64 bits without the protobuf-go overflow check
//!   (a 10th byte may carry any value, an 11th is `ErrIntOverflow`), field
//!   numbers are truncated to int32, `wireType == 4` and non-positive field
//!   numbers are errors;
//! * a known field with an unexpected wire type is an error (repeated
//!   numerics accept both packed and unpacked forms); packed elements are
//!   bounds-checked against the end of the *enclosing message*, not the packed
//!   run (so a run may over-read into the following bytes, like Go);
//! * unknown fields are skipped with `skipXxx` (groups tracked by depth only)
//!   and retained only for types with `XXX_unrecognized`;
//! * singular embedded messages merge across repeated occurrences, oneof
//!   members replace each other, scalars are last-wins, repeated fields
//!   append;
//! * `customtype`, `stdtime` and `stdduration` fields run the custom
//!   `Unmarshal` (see [`crate::custom`]).
//!
//! The decoded tree ([`Dyn`]) can be re-encoded exactly as gogoproto's
//! `Marshal` would ([`Mode::Marshal`]) or into bytes prost can decode into
//! the equivalent Rust struct ([`Mode::Struct`]).

use std::collections::BTreeMap;

use celestia_proto::wire_table::{self, FieldInfo, MessageInfo, ty};

use crate::custom;
use crate::protowire::{self, append_tag, append_varint};

/// A decode failure (Go returns an `error`; the text is for logs only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GoError(pub String);

impl GoError {
    pub(crate) fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

pub(crate) type GoResult<T> = Result<T, GoError>;

fn eof() -> GoError {
    GoError::new("unexpected EOF")
}
fn overflow() -> GoError {
    GoError::new("proto: integer overflow")
}
fn invalid_length() -> GoError {
    GoError::new("proto: negative length found during unmarshaling")
}

/// Message types whose Go struct has `XXX_unrecognized` (unknown fields are
/// kept and re-marshaled). Checked against the Go schema dump in tests.
pub(crate) const RETAINS_UNKNOWN: &[&str] = &[
    "google.protobuf.Any",
    "google.protobuf.Duration",
    "google.protobuf.Timestamp",
];

/// A decoded field value.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Val {
    /// A numeric scalar: the Go value converted to `uint64` (signed values
    /// sign-extended, bools 0/1, fixed/float types as raw bits).
    Num(u64),
    /// A string or bytes value.
    Bytes(Vec<u8>),
    /// An embedded message.
    Msg(Box<Dyn>),
    /// A `customtype` value: its canonical `Marshal` bytes, `None` when the
    /// Go value is nil (e.g. a `math.Int` that was never set).
    Custom(Option<Vec<u8>>),
    /// A `stdtime` value (seconds, nanos).
    Time(i64, i32),
    /// A `stdduration` value (seconds, nanos).
    Dur(i64, i32),
    /// A repeated field.
    List(Vec<Val>),
}

/// A decoded message (the state of the Go struct after `Unmarshal`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Dyn {
    pub info: &'static MessageInfo,
    pub fields: BTreeMap<u32, Val>,
    /// Oneof index -> the member field number currently set.
    pub oneofs: BTreeMap<i32, u32>,
    /// `XXX_unrecognized` bytes.
    pub unknown: Vec<u8>,
    /// For `google.protobuf.Any`: the concrete message after `UnpackAny`
    /// (Go's `cachedValue`).
    pub cached: Option<Box<Dyn>>,
}

/// Looks up a message type in the schema.
pub(crate) fn info(name: &str) -> GoResult<&'static MessageInfo> {
    wire_table::message(name)
        .ok_or_else(|| GoError::new(format!("failed to retrieve the message of type {name:?}")))
}

impl Dyn {
    pub(crate) fn new(info: &'static MessageInfo) -> Self {
        Self {
            info,
            fields: BTreeMap::new(),
            oneofs: BTreeMap::new(),
            unknown: Vec::new(),
            cached: None,
        }
    }

    /// A singular bytes/string field (empty if unset).
    pub(crate) fn bytes(&self, num: u32) -> &[u8] {
        match self.fields.get(&num) {
            Some(Val::Bytes(b)) => b,
            _ => &[],
        }
    }

    /// A repeated bytes/string field.
    pub(crate) fn bytes_list(&self, num: u32) -> Vec<Vec<u8>> {
        match self.fields.get(&num) {
            Some(Val::List(l)) => l
                .iter()
                .filter_map(|v| match v {
                    Val::Bytes(b) => Some(b.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// A repeated numeric field.
    pub(crate) fn num_list(&self, num: u32) -> Vec<u64> {
        match self.fields.get(&num) {
            Some(Val::List(l)) => l
                .iter()
                .filter_map(|v| match v {
                    Val::Num(n) => Some(*n),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// A singular message field if set.
    pub(crate) fn msg_mut(&mut self, num: u32) -> Option<&mut Dyn> {
        match self.fields.get_mut(&num) {
            Some(Val::Msg(m)) => Some(m),
            _ => None,
        }
    }

    /// A non-nullable message field: Go always has the (possibly zero)
    /// value, so materialize it.
    pub(crate) fn msg_value_mut(&mut self, num: u32) -> GoResult<&mut Dyn> {
        if !matches!(self.fields.get(&num), Some(Val::Msg(_))) {
            let f = self
                .info
                .field(num)
                .ok_or_else(|| GoError::new("no such field"))?;
            self.fields
                .insert(num, Val::Msg(Box::new(Dyn::new(info(f.type_name)?))));
        }
        match self.fields.get_mut(&num) {
            Some(Val::Msg(m)) => Ok(m),
            _ => Err(GoError::new("unreachable")),
        }
    }

    /// The messages of a repeated message field.
    pub(crate) fn msg_list_mut(&mut self, num: u32) -> Vec<&mut Dyn> {
        match self.fields.get_mut(&num) {
            Some(Val::List(l)) => l
                .iter_mut()
                .filter_map(|v| match v {
                    Val::Msg(m) => Some(m.as_mut()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn msg_list(&self, num: u32) -> Vec<&Dyn> {
        match self.fields.get(&num) {
            Some(Val::List(l)) => l
                .iter()
                .filter_map(|v| match v {
                    Val::Msg(m) => Some(m.as_ref()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    fn set(&mut self, f: &FieldInfo, v: Val) {
        if f.oneof >= 0
            && let Some(prev) = self.oneofs.insert(f.oneof, f.number)
            && prev != f.number
        {
            self.fields.remove(&prev);
        }
        self.fields.insert(f.number, v);
    }

    fn push(&mut self, f: &FieldInfo, v: Val) {
        match self
            .fields
            .entry(f.number)
            .or_insert_with(|| Val::List(Vec::new()))
        {
            Val::List(l) => l.push(v),
            other => *other = Val::List(vec![v]),
        }
    }
}

/// Go `int` arithmetic helpers (64-bit two's complement).
fn go_add(a: i64, b: i64) -> i64 {
    a.wrapping_add(b)
}

/// Reads a varint the way gogoproto-generated code does: up to 64 bits of
/// shift, bounds-checked against `l` (the enclosing message length).
fn read_varint(data: &[u8], i: &mut i64, l: i64) -> GoResult<u64> {
    let mut v: u64 = 0;
    let mut shift: u32 = 0;
    loop {
        if shift >= 64 {
            return Err(overflow());
        }
        if *i >= l {
            return Err(eof());
        }
        let b = data[*i as usize];
        *i += 1;
        v |= u64::from(b & 0x7f) << shift;
        if b < 0x80 {
            return Ok(v);
        }
        shift += 7;
    }
}

/// Reads a length prefix and returns `(start, end)` of the value, applying
/// gogoproto's `ErrInvalidLength` / `io.ErrUnexpectedEOF` checks.
fn read_len(data: &[u8], i: &mut i64, l: i64) -> GoResult<(i64, i64)> {
    let len = read_varint(data, i, l)? as i64;
    if len < 0 {
        return Err(invalid_length());
    }
    let post = go_add(*i, len);
    if post < 0 {
        return Err(invalid_length());
    }
    if post > l {
        return Err(eof());
    }
    Ok((*i, post))
}

/// `skipXxx`: the length of the unknown field starting at `data[0]`.
pub(crate) fn skip(data: &[u8]) -> GoResult<i64> {
    let l = data.len() as i64;
    let mut i: i64 = 0;
    let mut depth = 0i64;
    while i < l {
        let wire = read_varint(data, &mut i, l)?;
        match wire & 7 {
            0 => {
                let mut shift = 0u32;
                loop {
                    if shift >= 64 {
                        return Err(overflow());
                    }
                    if i >= l {
                        return Err(eof());
                    }
                    i += 1;
                    if data[(i - 1) as usize] < 0x80 {
                        break;
                    }
                    shift += 7;
                }
            }
            1 => i = go_add(i, 8),
            2 => {
                let len = read_varint(data, &mut i, l)? as i64;
                if len < 0 {
                    return Err(invalid_length());
                }
                i = go_add(i, len);
            }
            3 => depth += 1,
            4 => {
                if depth == 0 {
                    return Err(GoError::new("proto: unexpected end of group"));
                }
                depth -= 1;
            }
            5 => i = go_add(i, 4),
            w => return Err(GoError::new(format!("proto: illegal wireType {w}"))),
        }
        if i < 0 {
            return Err(invalid_length());
        }
        if depth == 0 {
            return Ok(i);
        }
    }
    Err(eof())
}

fn is_varint_type(t: u8) -> bool {
    matches!(
        t,
        ty::INT32
            | ty::INT64
            | ty::UINT32
            | ty::UINT64
            | ty::SINT32
            | ty::SINT64
            | ty::BOOL
            | ty::ENUM
    )
}

fn is_fixed32_type(t: u8) -> bool {
    matches!(t, ty::FIXED32 | ty::SFIXED32 | ty::FLOAT)
}

fn is_fixed64_type(t: u8) -> bool {
    matches!(t, ty::FIXED64 | ty::SFIXED64 | ty::DOUBLE)
}

/// Converts an accumulated varint to the Go field value (as `uint64`).
fn varint_value(t: u8, v: u64) -> u64 {
    match t {
        ty::INT32 | ty::ENUM => i64::from(v as u32 as i32) as u64,
        ty::UINT32 => u64::from(v as u32),
        ty::BOOL => u64::from(v != 0),
        ty::SINT32 => {
            let x = v as u32;
            let d = ((x >> 1) ^ (x & 1).wrapping_neg()) as i32;
            i64::from(d) as u64
        }
        ty::SINT64 => (v >> 1) ^ (v & 1).wrapping_neg(),
        _ => v,
    }
}

fn read_fixed(data: &[u8], i: &mut i64, l: i64, n: i64) -> GoResult<u64> {
    if go_add(*i, n) > l {
        return Err(eof());
    }
    let mut v = 0u64;
    for k in 0..n {
        v |= u64::from(data[(*i + k) as usize]) << (8 * k);
    }
    *i += n;
    Ok(v)
}

/// `proto.Unmarshal` into a fresh message of type `name`.
pub(crate) fn unmarshal(name: &str, data: &[u8]) -> GoResult<Dyn> {
    let mut d = Dyn::new(info(name)?);
    unmarshal_into(&mut d, data)?;
    Ok(d)
}

/// The generated `Unmarshal` method (merging into `d`).
pub(crate) fn unmarshal_into(d: &mut Dyn, data: &[u8]) -> GoResult<()> {
    let l = data.len() as i64;
    let mut i: i64 = 0;
    let retains = RETAINS_UNKNOWN.contains(&d.info.name);
    while i < l {
        let pre = i;
        let wire = read_varint(data, &mut i, l)?;
        let field_num = (wire >> 3) as u32 as i32;
        let wire_type = (wire & 7) as u8;
        if wire_type == 4 {
            return Err(GoError::new(format!(
                "proto: {}: wiretype end group for non-group",
                d.info.name
            )));
        }
        if field_num <= 0 {
            return Err(GoError::new(format!(
                "proto: {}: illegal tag {field_num} (wire type {wire})",
                d.info.name
            )));
        }
        match d.info.field(field_num as u32) {
            Some(f) if !d.info.map_entry => decode_field(d, f, wire_type, data, &mut i, l)?,
            Some(_) => return Err(GoError::new("map fields are not supported")),
            None => {
                let n = skip(&data[pre as usize..])?;
                if n < 0 || go_add(pre, n) < 0 {
                    return Err(invalid_length());
                }
                if go_add(pre, n) > l {
                    return Err(eof());
                }
                if retains {
                    d.unknown
                        .extend_from_slice(&data[pre as usize..(pre + n) as usize]);
                }
                i = pre + n;
            }
        }
    }
    if i > l {
        return Err(eof());
    }
    Ok(())
}

fn wrong_wire(f: &FieldInfo, wt: u8) -> GoError {
    GoError::new(format!(
        "proto: wrong wireType = {wt} for field {}",
        f.number
    ))
}

#[allow(clippy::too_many_lines)]
fn decode_field(
    d: &mut Dyn,
    f: &'static FieldInfo,
    wt: u8,
    data: &[u8],
    i: &mut i64,
    l: i64,
) -> GoResult<()> {
    let t = f.ty;
    if is_varint_type(t) || is_fixed32_type(t) || is_fixed64_type(t) {
        let (elem_wt, width) = if is_varint_type(t) {
            (protowire::VARINT, 0)
        } else if is_fixed32_type(t) {
            (protowire::FIXED32, 4)
        } else {
            (protowire::FIXED64, 8)
        };
        let read_one = |i: &mut i64| -> GoResult<u64> {
            if width == 0 {
                Ok(varint_value(t, read_varint(data, i, l)?))
            } else {
                read_fixed(data, i, l, width)
            }
        };
        if !f.repeated {
            if wt != elem_wt {
                return Err(wrong_wire(f, wt));
            }
            let v = read_one(i)?;
            d.set(f, Val::Num(v));
        } else if wt == elem_wt {
            let v = read_one(i)?;
            d.push(f, Val::Num(v));
        } else if wt == protowire::BYTES {
            let (_, post) = read_len(data, i, l)?;
            d.fields
                .entry(f.number)
                .or_insert_with(|| Val::List(Vec::new()));
            while *i < post {
                let v = read_one(i)?;
                d.push(f, Val::Num(v));
            }
        } else {
            return Err(wrong_wire(f, wt));
        }
        return Ok(());
    }
    if wt != protowire::BYTES {
        return Err(wrong_wire(f, wt));
    }
    let (start, post) = read_len(data, i, l)?;
    let value = &data[start as usize..post as usize];
    *i = post;
    match t {
        ty::STRING | ty::BYTES => {
            if !f.custom.is_empty() {
                let prev = if f.nullable || f.repeated {
                    None
                } else {
                    match d.fields.get(&f.number) {
                        Some(Val::Custom(c)) => c.clone(),
                        _ => None,
                    }
                };
                let v = custom::unmarshal(f.custom, value, prev)?;
                if f.repeated {
                    d.push(f, Val::Custom(v));
                } else {
                    d.set(f, Val::Custom(v));
                }
            } else if f.repeated {
                d.push(f, Val::Bytes(value.to_vec()));
            } else {
                d.set(f, Val::Bytes(value.to_vec()));
            }
        }
        ty::MESSAGE => {
            if f.std_time || f.std_duration {
                let ts = unmarshal(f.type_name, value)?;
                let secs = match ts.fields.get(&1) {
                    Some(Val::Num(n)) => *n as i64,
                    _ => 0,
                };
                let nanos = match ts.fields.get(&2) {
                    Some(Val::Num(n)) => *n as i64 as i32,
                    _ => 0,
                };
                let v = if f.std_time {
                    custom::validate_timestamp(secs, nanos)?;
                    Val::Time(secs, nanos)
                } else {
                    custom::validate_duration(secs, nanos)?;
                    Val::Dur(secs, nanos)
                };
                if f.repeated {
                    d.push(f, v);
                } else {
                    d.set(f, v);
                }
            } else if f.repeated || f.oneof >= 0 {
                let m = unmarshal(f.type_name, value)?;
                if f.repeated {
                    d.push(f, Val::Msg(Box::new(m)));
                } else {
                    d.set(f, Val::Msg(Box::new(m)));
                }
            } else {
                let inner = info(f.type_name)?;
                let entry = d
                    .fields
                    .entry(f.number)
                    .or_insert_with(|| Val::Msg(Box::new(Dyn::new(inner))));
                if let Val::Msg(m) = entry {
                    unmarshal_into(m, value)?;
                }
            }
        }
        _ => return Err(GoError::new(format!("unsupported field type {t}"))),
    }
    Ok(())
}

/// How [`marshal`] renders values that Go and prost represent differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Exactly gogoproto's `Marshal` output.
    Marshal,
    /// Bytes that prost decodes into the Rust equivalent of the Go struct:
    /// strings are made valid UTF-8 (lossy), nil customtype values are
    /// omitted, and unpacked `Any` values carry their normalized concrete
    /// message.
    Struct,
}

/// gogoproto `Marshal`.
pub(crate) fn marshal(d: &Dyn, mode: Mode) -> Vec<u8> {
    let mut out = Vec::new();
    marshal_into(d, mode, &mut out);
    out
}

fn marshal_into(d: &Dyn, mode: Mode, out: &mut Vec<u8>) {
    if mode == Mode::Struct
        && d.info.name == "google.protobuf.Any"
        && let Some(cached) = &d.cached
    {
        let type_url = d.bytes(1);
        if !type_url.is_empty() {
            protowire::append_bytes_field(out, 1, type_url);
        }
        let value = marshal(cached, mode);
        if !value.is_empty() {
            protowire::append_bytes_field(out, 2, &value);
        }
        out.extend_from_slice(&d.unknown);
        return;
    }
    // gogoproto emits a oneof at the position of its highest-numbered member.
    let mut oneof_last: BTreeMap<i32, u32> = BTreeMap::new();
    for f in d.info.fields {
        if f.oneof >= 0 {
            oneof_last.insert(f.oneof, f.number);
        }
    }
    for f in d.info.fields {
        if f.oneof >= 0 {
            if oneof_last.get(&f.oneof) != Some(&f.number) {
                continue;
            }
            if let Some(set) = d.oneofs.get(&f.oneof)
                && let (Some(mf), Some(v)) = (d.info.field(*set), d.fields.get(set))
            {
                marshal_value(mf, v, mode, true, out);
            }
            continue;
        }
        match d.fields.get(&f.number) {
            Some(v) => marshal_value(f, v, mode, false, out),
            None => marshal_absent(f, mode, out),
        }
    }
    out.extend_from_slice(&d.unknown);
}

/// Fields Go always marshals even when they were never decoded.
fn marshal_absent(f: &FieldInfo, mode: Mode, out: &mut Vec<u8>) {
    if f.repeated || f.nullable {
        return;
    }
    if f.ty == ty::MESSAGE {
        if f.std_time {
            marshal_value(
                f,
                &Val::Time(custom::ZERO_TIME_SECONDS, 0),
                mode,
                false,
                out,
            );
        } else if f.std_duration {
            marshal_value(f, &Val::Dur(0, 0), mode, false, out);
        } else if let Ok(inner) = info(f.type_name) {
            marshal_value(f, &Val::Msg(Box::new(Dyn::new(inner))), mode, false, out);
        }
    } else if !f.custom.is_empty() {
        marshal_value(f, &Val::Custom(None), mode, false, out);
    }
}

fn float_is_zero(f: &FieldInfo, v: u64) -> bool {
    match f.ty {
        ty::FLOAT => f32::from_bits(v as u32) == 0.0,
        ty::DOUBLE => f64::from_bits(v) == 0.0,
        _ => v == 0,
    }
}

fn encode_num(f: &FieldInfo, v: u64, out: &mut Vec<u8>) {
    match f.ty {
        ty::SINT32 => {
            let x = v as i32;
            append_varint(out, u64::from(((x << 1) ^ (x >> 31)) as u32));
        }
        ty::SINT64 => {
            let x = v as i64;
            append_varint(out, ((x << 1) ^ (x >> 63)) as u64);
        }
        ty::FIXED32 | ty::SFIXED32 | ty::FLOAT => out.extend_from_slice(&(v as u32).to_le_bytes()),
        ty::FIXED64 | ty::SFIXED64 | ty::DOUBLE => out.extend_from_slice(&v.to_le_bytes()),
        _ => append_varint(out, v),
    }
}

fn num_wire_type(f: &FieldInfo) -> u8 {
    if is_fixed32_type(f.ty) {
        protowire::FIXED32
    } else if is_fixed64_type(f.ty) {
        protowire::FIXED64
    } else {
        protowire::VARINT
    }
}

fn string_bytes(f: &FieldInfo, b: &[u8], mode: Mode) -> Vec<u8> {
    if mode == Mode::Struct && f.ty == ty::STRING {
        String::from_utf8_lossy(b).into_owned().into_bytes()
    } else {
        b.to_vec()
    }
}

fn encode_timestamp(secs: i64, nanos: i32) -> Vec<u8> {
    let mut b = Vec::new();
    if secs != 0 {
        append_tag(&mut b, 1, protowire::VARINT);
        append_varint(&mut b, secs as u64);
    }
    if nanos != 0 {
        append_tag(&mut b, 2, protowire::VARINT);
        append_varint(&mut b, i64::from(nanos) as u64);
    }
    b
}

fn marshal_value(f: &FieldInfo, v: &Val, mode: Mode, oneof: bool, out: &mut Vec<u8>) {
    match v {
        Val::Num(n) => {
            if oneof || !float_is_zero(f, *n) {
                append_tag(out, f.number, num_wire_type(f));
                encode_num(f, *n, out);
            }
        }
        Val::Bytes(b) => {
            if oneof || !b.is_empty() || f.repeated {
                protowire::append_bytes_field(out, f.number, &string_bytes(f, b, mode));
            }
        }
        Val::Msg(m) => protowire::append_bytes_field(out, f.number, &marshal(m, mode)),
        Val::Custom(c) => match (c, mode) {
            (Some(b), _) => protowire::append_bytes_field(out, f.number, b),
            (None, Mode::Marshal) => {
                protowire::append_bytes_field(out, f.number, &custom::nil_marshal(f.custom));
            }
            (None, Mode::Struct) => {
                if f.custom.ends_with("HexAddress") {
                    protowire::append_bytes_field(out, f.number, &custom::nil_marshal(f.custom));
                }
            }
        },
        Val::Time(s, n) | Val::Dur(s, n) => {
            protowire::append_bytes_field(out, f.number, &encode_timestamp(*s, *n));
        }
        Val::List(items) => {
            let numeric = is_varint_type(f.ty) || is_fixed32_type(f.ty) || is_fixed64_type(f.ty);
            if numeric && f.packed {
                if items.is_empty() {
                    return;
                }
                let mut body = Vec::new();
                for it in items {
                    if let Val::Num(n) = it {
                        encode_num(f, *n, &mut body);
                    }
                }
                protowire::append_bytes_field(out, f.number, &body);
            } else {
                for it in items {
                    marshal_value(f, it, mode, true, out);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coin_roundtrip_and_customtype() {
        // Coin{denom: "utia", amount: "0x10"} -> Go holds Int(16).
        let mut b = Vec::new();
        protowire::append_bytes_field(&mut b, 1, b"utia");
        protowire::append_bytes_field(&mut b, 2, b"0x10");
        let d = unmarshal("cosmos.base.v1beta1.Coin", &b).unwrap();
        let mut want = Vec::new();
        protowire::append_bytes_field(&mut want, 1, b"utia");
        protowire::append_bytes_field(&mut want, 2, b"16");
        assert_eq!(marshal(&d, Mode::Marshal), want);
        // Missing amount marshals as "0" (non-nullable customtype).
        let d = unmarshal("cosmos.base.v1beta1.Coin", &b[..6]).unwrap();
        let mut want = Vec::new();
        protowire::append_bytes_field(&mut want, 1, b"utia");
        protowire::append_bytes_field(&mut want, 2, b"0");
        assert_eq!(marshal(&d, Mode::Marshal), want);
        assert_eq!(marshal(&d, Mode::Struct), b[..6].to_vec());
        // Invalid amounts fail to decode.
        let mut bad = Vec::new();
        protowire::append_bytes_field(&mut bad, 2, b"abc");
        assert!(unmarshal("cosmos.base.v1beta1.Coin", &bad).is_err());
    }

    #[test]
    fn wire_type_errors_and_skips() {
        // Fee.gas_limit (uint64) sent as bytes: gogoproto rejects it.
        let b = [0x12, 0x00];
        assert!(unmarshal("cosmos.tx.v1beta1.Fee", &b).is_err());
        // Unknown field skipped.
        let b = [0x48, 0x01, 0x10, 0x05];
        let d = unmarshal("cosmos.tx.v1beta1.Fee", &b).unwrap();
        assert_eq!(d.fields.get(&2), Some(&Val::Num(5)));
        // End group at top level.
        assert!(unmarshal("cosmos.tx.v1beta1.Fee", &[0x0c]).is_err());
        // Field number 0.
        assert!(unmarshal("cosmos.tx.v1beta1.Fee", &[0x00, 0x00]).is_err());
    }
}
