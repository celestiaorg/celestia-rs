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
    if crate::v9() && name == V9_REMOTE_ROUTER.name {
        return Ok(&V9_REMOTE_ROUTER);
    }
    if crate::v9() && name == V9_MSG_FORWARD.name {
        return Ok(&V9_MSG_FORWARD);
    }
    wire_table::message(name)
        .ok_or_else(|| GoError::new(format!("failed to retrieve the message of type {name:?}")))
}

/// hyperlane-cosmos v1.1.0's `hyperlane.warp.v1.RemoteRouter` (celestia-app
/// v9.0.8): `receiver_contract` (2) is a plain string, not a `HexAddress`
/// customtype; the only decoding-relevant schema difference from v1.3.0.
static V9_REMOTE_ROUTER: std::sync::LazyLock<MessageInfo> = std::sync::LazyLock::new(|| {
    let base = wire_table::message("hyperlane.warp.v1.RemoteRouter");
    let fields: Vec<FieldInfo> = base
        .map(|m| m.fields.to_vec())
        .unwrap_or_default()
        .into_iter()
        .map(|mut f| {
            if f.number == 2 {
                f.custom = "";
                f.nullable = true;
            }
            f
        })
        .collect();
    MessageInfo {
        name: "hyperlane.warp.v1.RemoteRouter",
        map_entry: false,
        fields: Box::leak(fields.into_boxed_slice()),
    }
});

/// celestia-app v9.0.8's `celestia.forwarding.v1.MsgForward`: fields 1-6
/// (v10 added `custom_hook_id` (7) and `custom_hook_metadata` (8)), so a
/// message carrying them (an ICA host tx: no unknown-field check) holds
/// neither at app version 9.
static V9_MSG_FORWARD: std::sync::LazyLock<MessageInfo> = std::sync::LazyLock::new(|| {
    let base = wire_table::message("celestia.forwarding.v1.MsgForward");
    let fields: Vec<FieldInfo> = base
        .map(|m| m.fields.to_vec())
        .unwrap_or_default()
        .into_iter()
        .filter(|f| f.number <= 6)
        .collect();
    MessageInfo {
        name: "celestia.forwarding.v1.MsgForward",
        map_entry: false,
        fields: Box::leak(fields.into_boxed_slice()),
    }
});

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

    pub(crate) fn set(&mut self, f: &FieldInfo, v: Val) {
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

/// The Go struct name the generated code prints in its errors
/// (`proto: CosmosTx: ...`, `ModeInfo_Single` for a nested message).
fn go_struct_name(full_name: &str) -> String {
    let t = crate::go_types::go_type_name(full_name);
    t.rsplit('.').next().unwrap_or_default().to_owned()
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
                go_struct_name(d.info.name)
            )));
        }
        if field_num <= 0 {
            return Err(GoError::new(format!(
                "proto: {}: illegal tag {field_num} (wire type {wire})",
                go_struct_name(d.info.name)
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

/// The side effect of gogoproto's `Marshal` on the Go struct: `math.Int`'s
/// `MarshalTo` (pointer receiver) sets a nil value to zero in place, for
/// every `Int` it reaches (non-nullable fields, set nullable pointers,
/// repeated elements, nested messages; not the cached values of `Any`s,
/// whose `Marshal` writes their bytes). `LegacyDec.MarshalTo` copies and
/// leaves a nil value nil.
pub(crate) fn zero_nil_ints(d: &mut Dyn) {
    for f in d.info.fields {
        if f.is_any() || f.std_time || f.std_duration {
            continue;
        }
        if f.oneof >= 0 && d.oneofs.get(&f.oneof) != Some(&f.number) {
            continue;
        }
        if f.custom == custom::INT {
            match d.fields.get_mut(&f.number) {
                Some(Val::List(items)) => {
                    for it in items {
                        if let Val::Custom(c @ None) = it {
                            *c = Some(custom::nil_marshal(f.custom));
                        }
                    }
                }
                Some(Val::Custom(c @ None)) => *c = Some(custom::nil_marshal(f.custom)),
                None if !f.nullable && !f.repeated => {
                    d.fields
                        .insert(f.number, Val::Custom(Some(custom::nil_marshal(f.custom))));
                }
                _ => {}
            }
        } else if f.ty == ty::MESSAGE {
            match d.fields.get_mut(&f.number) {
                Some(Val::List(items)) => {
                    for it in items {
                        if let Val::Msg(m) = it {
                            zero_nil_ints(m);
                        }
                    }
                }
                Some(Val::Msg(m)) => zero_nil_ints(m),
                None if !f.nullable && !f.repeated && f.oneof < 0 => {
                    if let Ok(inner) = info(f.type_name) {
                        let mut m = Dyn::new(inner);
                        zero_nil_ints(&mut m);
                        d.fields.insert(f.number, Val::Msg(Box::new(m)));
                    }
                }
                _ => {}
            }
        }
    }
}

/// How [`marshal`] renders values that Go and prost represent differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Exactly gogoproto's `Marshal` output.
    Marshal,
    /// Bytes that prost decodes into the Rust equivalent of the Go struct:
    /// strings keep their bytes (the protobuf crates decode any bytes, see
    /// `gostr`), nil customtype values are
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
        out.extend_from_slice(&unknown_for(d, mode));
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
        let normalized;
        let v = match d.fields.get(&f.number) {
            Some(v) if mode == Mode::Struct && f.is_any() && !keeps_raw_anys(d.info.name) => {
                normalized = normalize_anys(v);
                Some(normalized.as_ref().unwrap_or(v))
            }
            v => v,
        };
        match v {
            Some(v) => marshal_value(f, v, mode, false, out),
            None => marshal_absent(f, mode, out),
        }
    }
    out.extend_from_slice(&unknown_for(d, mode));
}

/// Messages whose `Any` fields [`Mode::Struct`] keeps as received even
/// when prost cannot decode them: their consumers decode the inner value
/// the gogoproto way themselves (the router's `txdecode::prost_compatible`
/// for messages, the fee-grant module for allowances, the ante handler for
/// multisig sub-keys), and Go keeps the bytes as received where it stores
/// them (an `AllowedMsgAllowance`'s inner allowance, the messages nested in
/// a proposal's messages, the sub-keys of an account's multisig key, a solo
/// machine's public key, which `Initialize` also compares with
/// `reflect.DeepEqual`).
fn keeps_raw_anys(name: &str) -> bool {
    matches!(
        name,
        "cosmos.crypto.multisig.LegacyAminoPubKey"
            | "cosmos.feegrant.v1beta1.AllowedMsgAllowance"
            | "cosmos.feegrant.v1beta1.MsgGrantAllowance"
            | "cosmos.authz.v1beta1.MsgExec"
            | "cosmos.gov.v1.MsgSubmitProposal"
            | "ibc.applications.interchain_accounts.v1.CosmosTx"
            | "ibc.lightclients.solomachine.v3.ConsensusState"
            | "ibc.lightclients.solomachine.v3.Header"
    )
}

/// An `Any` value (or a list of them) with the value replaced by
/// gogoproto's decoding of the registered type re-encoded
/// ([`Mode::Struct`]: what prost decodes into the struct Go holds); `None`
/// when the type is unknown or does not decode.
fn normalize_anys(v: &Val) -> Option<Val> {
    match v {
        Val::List(items) => {
            let out: Vec<Option<Val>> = items.iter().map(normalize_anys).collect();
            out.iter().any(Option::is_some).then(|| {
                Val::List(
                    out.into_iter()
                        .zip(items)
                        .map(|(n, v)| n.unwrap_or_else(|| v.clone()))
                        .collect(),
                )
            })
        }
        Val::Msg(any) => {
            let raw = any.bytes(2);
            let info = crate::registry::resolve(any.bytes(1)).and_then(|i| info(i.name).ok())?;
            let mut inner = Dyn::new(info);
            unmarshal_into(&mut inner, raw).ok()?;
            let mut any = any.clone();
            any.fields
                .insert(2, Val::Bytes(marshal(&inner, Mode::Struct)));
            Some(Val::Msg(any))
        }
        _ => None,
    }
}

/// `XXX_unrecognized` as [`marshal`] writes it: in [`Mode::Struct`]
/// without the fields prost cannot skip but gogoproto did (prost drops
/// unknown fields anyway): keys past `u32::MAX` (gogoproto truncates the
/// field number to `int32`), groups (gogoproto counts start and end group
/// tags without matching their numbers) and 10-byte varints whose last
/// byte is above 1.
fn unknown_for(d: &Dyn, mode: Mode) -> std::borrow::Cow<'_, [u8]> {
    if mode == Mode::Marshal {
        return std::borrow::Cow::Borrowed(&d.unknown);
    }
    std::borrow::Cow::Owned(strip_prost_incompatible(&d.unknown))
}

fn strip_prost_incompatible(b: &[u8]) -> Vec<u8> {
    /// A varint and its length; `bad`: 10 bytes with a last byte above 1.
    fn varint(b: &[u8]) -> Option<(u64, usize, bool)> {
        let mut v = 0u64;
        for (i, &x) in b.iter().enumerate().take(10) {
            v |= u64::from(x & 0x7f).checked_shl(7 * i as u32).unwrap_or(0);
            if x < 0x80 {
                return Some((v, i + 1, i == 9 && x > 1));
            }
        }
        None
    }
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    // Depth of open groups (gogoproto `skip`: numbers are not matched).
    let mut depth = 0usize;
    let mut drop_from: Option<usize> = None;
    while i < b.len() {
        let start = i;
        let Some((key, n, bad_key)) = varint(&b[i..]) else {
            break;
        };
        i += n;
        let (len, bad) = match key & 7 {
            0 => match varint(&b[i..]) {
                Some((_, n, bad)) => (n, bad),
                None => break,
            },
            1 => (8, false),
            5 => (4, false),
            2 => match varint(&b[i..]) {
                Some((l, n, bad)) => match usize::try_from(l).ok().and_then(|l| l.checked_add(n)) {
                    Some(t) => (t, bad),
                    None => break,
                },
                None => break,
            },
            3 => {
                if depth == 0 {
                    drop_from = Some(start);
                }
                depth += 1;
                continue;
            }
            4 => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    drop_from = None;
                }
                continue;
            }
            _ => break,
        };
        if i + len > b.len() {
            break;
        }
        i += len;
        if depth == 0 && drop_from.is_none() && !bad_key && !bad && u32::try_from(key).is_ok() {
            out.extend_from_slice(&b[start..i]);
        }
    }
    out
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
                protowire::append_bytes_field(out, f.number, b);
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
                } else if f.nullable && !f.repeated {
                    // A non-nil pointer to a nil value (the field was
                    // present but empty): an empty occurrence, which prost
                    // reads as absent but handlers can tell apart on the
                    // wire (`MsgEditValidator`).
                    protowire::append_bytes_field(out, f.number, &[]);
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
    fn marshal_zeroes_nil_ints_only() {
        // MsgCreateValidator{commission: {rate: ""}, value: {denom}}: after
        // a Marshal the nil Ints (min_self_delegation, value.amount) are
        // zero, the nil LegacyDecs (commission rates) still nil.
        let name = "cosmos.staking.v1beta1.MsgCreateValidator";
        let mut b = Vec::new();
        protowire::append_bytes_field(&mut b, 2, &[0x0a, 0x00]);
        protowire::append_bytes_field(&mut b, 7, &[0x0a, 0x04, b'u', b't', b'i', b'a']);
        let mut d = unmarshal(name, &b).unwrap();
        zero_nil_ints(&mut d);
        let mut want = Vec::new();
        protowire::append_bytes_field(&mut want, 1, &[]);
        protowire::append_bytes_field(&mut want, 2, &[]);
        protowire::append_bytes_field(&mut want, 3, b"0");
        protowire::append_bytes_field(
            &mut want,
            7,
            &[0x0a, 0x04, b'u', b't', b'i', b'a', 0x12, 0x01, b'0'],
        );
        assert_eq!(marshal(&d, Mode::Struct), want);
    }

    #[test]
    fn nullable_customtype_present_but_empty() {
        // MsgEditValidator.commission_rate (nullable LegacyDec): absent is
        // a nil pointer, an empty occurrence a pointer to a nil value
        // (marshaled as "0"); Struct keeps the two apart.
        let name = "cosmos.staking.v1beta1.MsgEditValidator";
        let d = unmarshal(name, &[]).unwrap();
        assert_eq!(marshal(&d, Mode::Struct), [0x0a, 0x00]);
        assert_eq!(marshal(&d, Mode::Marshal), [0x0a, 0x00]);
        let d = unmarshal(name, &[0x1a, 0x00]).unwrap();
        assert_eq!(marshal(&d, Mode::Struct), [0x0a, 0x00, 0x1a, 0x00]);
        assert_eq!(marshal(&d, Mode::Marshal), [0x0a, 0x00, 0x1a, 0x01, b'0']);
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
