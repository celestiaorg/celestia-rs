//! gogoproto's compact text format (`proto.CompactTextString`, gogoproto
//! v1.7.2 `text.go`): what `fmt.Sprintf("%v", msg)` prints for a generated
//! message. Go embeds it in
//! - authz `DispatchActions`: `failed to execute message; message %v`
//!   (tx log), and
//! - gov's `safeExecuteHandler`: `handling x/gov proposal msg [%s]
//!   PANICKED: %v`, stored as the proposal's `failed_reason` (app hash).
//!
//! The text is produced from the message's wire bytes as gogoproto decoded
//! them (the Go struct): fields in declaration order under their proto
//! names; proto3 zero scalars, empty strings / bytes, absent nullable
//! messages and nil customtypes skipped; non-nullable messages always
//! written (`name:<...> `); customtypes through their `Marshal` (`math.Int`
//! / `LegacyDec`: the big integer, `HexAddress`: `0x` + hex); `stdtime` /
//! `stdduration` as `Timestamp` / `Duration`; strings and bytes quoted byte
//! by byte with octal escapes; enums by name; nested `Any`s as their
//! `type_url` / `value` fields (no expansion) plus unknown fields.
//! Every value is followed by a space (the compact writer's newline).

// Short names (`w` writer, `f` field, `b` bytes, `o` occurrence) mirror
// gogoproto's `text.go`.
#![allow(clippy::many_single_char_names)]

use prost_reflect::{Cardinality, FieldDescriptor, Kind, MessageDescriptor, Value};

use crate::signing::pool;

/// One wire occurrence of a field.
struct Occ<'a> {
    wt: u8,
    data: Occd<'a>,
}

enum Occd<'a> {
    Varint(u64),
    Fixed64(u64),
    Fixed32(u32),
    Bytes(&'a [u8]),
}

fn varint(b: &mut &[u8]) -> Option<u64> {
    let mut v = 0u64;
    for i in 0..10 {
        let x = *b.get(i)?;
        v |= u64::from(x & 0x7f) << (7 * i);
        if x < 0x80 {
            *b = &b[i + 1..];
            return Some(v);
        }
    }
    None
}

/// Splits `b` into `(field number, occurrence)` pairs; `None` on malformed
/// wire data or groups.
fn fields(mut b: &[u8]) -> Option<Vec<(u64, Occ<'_>)>> {
    let mut out = Vec::new();
    while !b.is_empty() {
        let tag = varint(&mut b)?;
        let (num, wt) = (tag >> 3, (tag & 7) as u8);
        if num == 0 {
            return None;
        }
        let data = match wt {
            0 => Occd::Varint(varint(&mut b)?),
            1 => {
                let v = u64::from_le_bytes(b.get(..8)?.try_into().ok()?);
                b = &b[8..];
                Occd::Fixed64(v)
            }
            5 => {
                let v = u32::from_le_bytes(b.get(..4)?.try_into().ok()?);
                b = &b[4..];
                Occd::Fixed32(v)
            }
            2 => {
                let len = usize::try_from(varint(&mut b)?).ok()?;
                let d = b.get(..len)?;
                b = &b[len..];
                Occd::Bytes(d)
            }
            _ => return None,
        };
        out.push((num, Occ { wt, data }));
    }
    Some(out)
}

/// gogoproto field options that change the Go field type.
#[derive(Default)]
pub struct GoOpts {
    /// `(gogoproto.nullable)` (default true).
    pub nullable: bool,
    /// `(gogoproto.customtype)`.
    pub customtype: Option<String>,
    /// `(gogoproto.stdtime)`.
    pub stdtime: bool,
    /// `(gogoproto.stdduration)`.
    pub stdduration: bool,
}

/// The gogoproto options of `f`.
#[must_use]
pub fn go_opts(f: &FieldDescriptor) -> GoOpts {
    let p = pool();
    let opts = f.options();
    let ext = |name: &str| -> Option<Value> {
        let e = p.get_extension_by_name(name)?;
        opts.has_extension(&e)
            .then(|| opts.get_extension(&e).into_owned())
    };
    GoOpts {
        nullable: !matches!(ext("gogoproto.nullable"), Some(Value::Bool(false))),
        customtype: match ext("gogoproto.customtype") {
            Some(Value::String(s)) => Some(s),
            _ => None,
        },
        stdtime: matches!(ext("gogoproto.stdtime"), Some(Value::Bool(true))),
        stdduration: matches!(ext("gogoproto.stdduration"), Some(Value::Bool(true))),
    }
}

/// A string field as the text marshaler writes it ([`write_string`]):
/// `s` in its Go form (`gostr`), so invalid bytes are escaped as Go does.
#[must_use]
pub fn quote(s: &str) -> String {
    let mut w = String::new();
    write_string(&mut w, &gostr::to_go_bytes(s));
    w
}

/// `writeString`: quoted, byte by byte, `\n` `\r` `\t` `\"` `\\` escaped,
/// other bytes outside 0x20..0x7f as three-digit octal.
fn write_string(w: &mut String, s: &[u8]) {
    w.push('"');
    for &c in s {
        match c {
            b'\n' => w.push_str("\\n"),
            b'\r' => w.push_str("\\r"),
            b'\t' => w.push_str("\\t"),
            b'"' => w.push_str("\\\""),
            b'\\' => w.push_str("\\\\"),
            0x20..0x7f => w.push(char::from(c)),
            _ => {
                w.push('\\');
                w.push(char::from(b'0' + (c >> 6)));
                w.push(char::from(b'0' + ((c >> 3) & 7)));
                w.push(char::from(b'0' + (c & 7)));
            }
        }
    }
    w.push('"');
}

/// The rendering context: app version (v9 schema differences).
#[derive(Clone, Copy)]
struct Cx {
    app_version: u64,
}

/// Renders the scalar `o` of field kind `k` (`fmt.Fprint` of the Go value).
fn scalar(w: &mut String, k: &Kind, o: &Occd<'_>) -> Option<()> {
    use std::fmt::Write as _;
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    let s = match (k, o) {
        (Kind::Int32 | Kind::Enum(_), Occd::Varint(v)) => (*v as i32).to_string(),
        (Kind::Int64, Occd::Varint(v)) => (*v as i64).to_string(),
        (Kind::Uint32, Occd::Varint(v)) => (*v as u32).to_string(),
        (Kind::Uint64, Occd::Varint(v)) => v.to_string(),
        (Kind::Sint32, Occd::Varint(v)) => {
            let v = *v as u32;
            (((v >> 1) as i32) ^ -((v & 1) as i32)).to_string()
        }
        (Kind::Sint64, Occd::Varint(v)) => (((v >> 1) as i64) ^ -((v & 1) as i64)).to_string(),
        (Kind::Bool, Occd::Varint(v)) => (*v != 0).to_string(),
        (Kind::Fixed32, Occd::Fixed32(v)) => v.to_string(),
        (Kind::Sfixed32, Occd::Fixed32(v)) => (*v as i32).to_string(),
        (Kind::Fixed64, Occd::Fixed64(v)) => v.to_string(),
        (Kind::Sfixed64, Occd::Fixed64(v)) => (*v as i64).to_string(),
        // No float fields in messages (Go's %v float form is not ported).
        _ => return None,
    };
    let _ = write!(w, "{s}");
    Some(())
}

fn is_zero(o: &Occd<'_>) -> bool {
    match o {
        Occd::Varint(v) | Occd::Fixed64(v) => *v == 0,
        Occd::Fixed32(v) => *v == 0,
        Occd::Bytes(b) => b.is_empty(),
    }
}

/// Scalars of a repeated field (packed or not), in order.
fn repeated_scalars<'a>(k: &Kind, occs: &[&Occ<'a>]) -> Option<Vec<Occd<'a>>> {
    let mut out = Vec::new();
    let fixed = match k {
        Kind::Fixed32 | Kind::Sfixed32 | Kind::Float => Some(5u8),
        Kind::Fixed64 | Kind::Sfixed64 | Kind::Double => Some(1u8),
        _ => None,
    };
    for o in occs {
        match (&o.data, fixed) {
            (Occd::Bytes(b), _) => {
                let mut b = *b;
                while !b.is_empty() {
                    out.push(match fixed {
                        None => Occd::Varint(varint(&mut b)?),
                        Some(1) => {
                            let v = u64::from_le_bytes(b.get(..8)?.try_into().ok()?);
                            b = &b[8..];
                            Occd::Fixed64(v)
                        }
                        Some(_) => {
                            let v = u32::from_le_bytes(b.get(..4)?.try_into().ok()?);
                            b = &b[4..];
                            Occd::Fixed32(v)
                        }
                    });
                }
            }
            (Occd::Varint(v), None) => out.push(Occd::Varint(*v)),
            (Occd::Fixed64(v), Some(1)) => out.push(Occd::Fixed64(*v)),
            (Occd::Fixed32(v), Some(5)) => out.push(Occd::Fixed32(*v)),
            _ => return None,
        }
    }
    Some(out)
}

/// The name of an enum value (`enumStringMaps`); `None` for other kinds
/// and unknown values (written as numbers).
fn enum_of(k: &Kind, o: &Occd<'_>) -> Option<String> {
    let (Kind::Enum(e), Occd::Varint(v)) = (k, o) else {
        return None;
    };
    #[allow(clippy::cast_possible_truncation)]
    let v = *v as i32;
    e.get_value(v).map(|ev| ev.name().to_owned())
}

/// A customtype's `Marshal()` text for the field bytes `b` (`None`: nil /
/// absent value).
fn customtype(w: &mut String, ct: &str, b: Option<&[u8]>) -> Option<()> {
    if ct.ends_with("math.Int") || ct.ends_with("math.Uint") {
        let v = math::Int::unmarshal(b.unwrap_or_default()).ok()?;
        write_string(w, &v.marshal());
    } else if ct.ends_with("math.LegacyDec") {
        let v = math::LegacyDec::unmarshal(b.unwrap_or_default()).ok()?;
        write_string(w, &v.marshal());
    } else if ct.ends_with("util.HexAddress") {
        let bytes = match b {
            None => [0u8; 32],
            Some(b) => {
                let s = std::str::from_utf8(b).ok()?;
                let h = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
                let v = hex::decode(h).ok()?;
                <[u8; 32]>::try_from(v.as_slice()).ok()?
            }
        };
        write_string(w, format!("0x{}", hex::encode(bytes)).as_bytes());
    } else {
        return None;
    }
    Some(())
}

/// `<seconds:N nanos:N > ` of a Timestamp / Duration value.
fn seconds_nanos(w: &mut String, secs: i64, nanos: i32) {
    use std::fmt::Write as _;
    w.push('<');
    if secs != 0 {
        let _ = write!(w, "seconds:{secs} ");
    }
    if nanos != 0 {
        let _ = write!(w, "nanos:{nanos} ");
    }
    w.push('>');
}

/// Seconds and nanos of a merged `Timestamp` / `Duration` encoding.
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
fn read_seconds_nanos(b: &[u8]) -> Option<(i64, i32)> {
    let (mut s, mut n) = (0i64, 0i32);
    for (num, o) in fields(b)? {
        match (num, o.data) {
            (1, Occd::Varint(v)) => s = v as i64,
            (2, Occd::Varint(v)) => n = v as i32,
            (1 | 2, _) => return None,
            _ => {}
        }
    }
    Some((s, n))
}

/// The Go field name in the text (`props.OrigName`) and `:`.
fn name(w: &mut String, f: &FieldDescriptor) {
    w.push_str(f.name());
    w.push(':');
}

/// A field whose Go type is a nested message (`<...>`).
fn message_value(w: &mut String, cx: Cx, md: &MessageDescriptor, b: &[u8]) -> Option<()> {
    w.push('<');
    write_struct(w, cx, md, b)?;
    w.push('>');
    Some(())
}

/// Concatenated bytes of every occurrence: gogoproto merges repeated
/// occurrences of a singular message field.
fn merged(occs: &[&Occ<'_>]) -> Option<Vec<u8>> {
    let mut v = Vec::new();
    for o in occs {
        let Occd::Bytes(b) = o.data else { return None };
        v.extend_from_slice(b);
    }
    Some(v)
}

fn write_field(w: &mut String, cx: Cx, f: &FieldDescriptor, occs: &[&Occ<'_>]) -> Option<()> {
    let k = f.kind();
    let opts = go_opts(f);
    let v9_string =
        cx.app_version < 10 && f.full_name() == "hyperlane.warp.v1.RemoteRouter.receiver_contract";
    let ct = if v9_string {
        None
    } else {
        opts.customtype.as_deref()
    };
    if f.is_map() {
        return None;
    }
    if f.cardinality() == Cardinality::Repeated {
        if let Some(ct) = ct {
            for o in occs {
                let Occd::Bytes(b) = o.data else { return None };
                name(w, f);
                customtype(w, ct, Some(b))?;
                w.push(' ');
            }
            return Some(());
        }
        match &k {
            Kind::Message(md) => {
                for o in occs {
                    let Occd::Bytes(b) = o.data else { return None };
                    name(w, f);
                    if opts.stdtime || opts.stdduration {
                        let (s, n) = read_seconds_nanos(b)?;
                        seconds_nanos(w, s, n);
                    } else {
                        message_value(w, cx, md, b)?;
                    }
                    w.push(' ');
                }
            }
            Kind::String | Kind::Bytes => {
                for o in occs {
                    let Occd::Bytes(b) = o.data else { return None };
                    name(w, f);
                    write_string(w, b);
                    w.push(' ');
                }
            }
            _ => {
                for v in repeated_scalars(&k, occs)? {
                    name(w, f);
                    if let Some(n) = enum_of(&k, &v) {
                        w.push_str(&n);
                    } else {
                        scalar(w, &k, &v)?;
                    }
                    w.push(' ');
                }
            }
        }
        return Some(());
    }
    // Singular.
    if let Some(ct) = ct {
        let last = occs.last().map(|o| match o.data {
            Occd::Bytes(b) => Some(b),
            _ => None,
        });
        if opts.nullable && last.is_none() {
            return Some(());
        }
        name(w, f);
        customtype(w, ct, last.flatten())?;
        w.push(' ');
        return Some(());
    }
    match &k {
        Kind::Message(md) => {
            if occs.is_empty() && opts.nullable {
                return Some(());
            }
            let b = merged(occs)?;
            name(w, f);
            if opts.stdtime {
                let (s, n) = if occs.is_empty() {
                    (-62_135_596_800, 0)
                } else {
                    read_seconds_nanos(&b)?
                };
                seconds_nanos(w, s, n);
            } else if opts.stdduration {
                let (s, n) = read_seconds_nanos(&b)?;
                seconds_nanos(w, s, n);
            } else {
                message_value(w, cx, md, &b)?;
            }
            w.push(' ');
        }
        Kind::String | Kind::Bytes => {
            let Some(o) = occs.last() else {
                return Some(());
            };
            let Occd::Bytes(b) = o.data else { return None };
            if b.is_empty() {
                return Some(());
            }
            name(w, f);
            write_string(w, b);
            w.push(' ');
        }
        _ => {
            let Some(o) = occs.last() else {
                return Some(());
            };
            if o.wt == 2 {
                // A packed singular scalar is a decode error in gogoproto.
                return None;
            }
            if is_zero(&o.data) {
                return Some(());
            }
            name(w, f);
            if let Some(n) = enum_of(&k, &o.data) {
                w.push_str(&n);
            } else {
                scalar(w, &k, &o.data)?;
            }
            w.push(' ');
        }
    }
    Some(())
}

/// `writeUnknownStruct` in compact mode (an `Any`'s `XXX_unrecognized`).
fn write_unknown(w: &mut String, occs: &[(u64, Occ<'_>)]) {
    use std::fmt::Write as _;
    for (num, o) in occs {
        let _ = write!(w, "{num}:");
        match o.data {
            Occd::Bytes(b) => w.push_str(&gostr::quote(&gostr::from_go_bytes(b))),
            Occd::Varint(v) | Occd::Fixed64(v) => {
                let _ = write!(w, "{v}");
            }
            Occd::Fixed32(v) => {
                let _ = write!(w, "{v}");
            }
        }
        w.push(' ');
    }
}

fn write_struct(w: &mut String, cx: Cx, md: &MessageDescriptor, b: &[u8]) -> Option<()> {
    let occs = fields(b)?;
    let is_any = md.full_name() == "google.protobuf.Any";
    for f in md.fields() {
        if cx.app_version < 10
            && md.full_name() == "celestia.forwarding.v1.MsgForward"
            && f.number() > 6
        {
            continue;
        }
        if f.containing_oneof().is_some() && !f.field_descriptor_proto().proto3_optional() {
            // No message has a oneof; the interface field is not ported.
            return None;
        }
        let mine: Vec<&Occ<'_>> = occs
            .iter()
            .filter(|(n, _)| *n == u64::from(f.number()))
            .map(|(_, o)| o)
            .collect();
        write_field(w, cx, &f, &mine)?;
    }
    if is_any {
        let unknown: Vec<(u64, Occ<'_>)> = fields(b)?.into_iter().filter(|(n, _)| *n > 2).collect();
        write_unknown(w, &unknown);
    }
    Some(())
}

/// `fmt.Sprintf("%v", msg)` of the message `value` of type `type_url`
/// (gogoproto `CompactTextString`), at `app_version`. `None` when the type
/// is unknown or the bytes are not what gogoproto decodes.
#[must_use]
pub fn compact_text(type_url: &str, value: &[u8], app_version: u64) -> Option<String> {
    let name = type_url.rsplit('/').next()?;
    let md = pool().get_message_by_name(name)?;
    let mut w = String::new();
    write_struct(&mut w, Cx { app_version }, &md, value)?;
    Some(w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celestia_proto::cosmos::bank::v1beta1::MsgSend;
    use celestia_proto::cosmos::base::v1beta1::Coin;
    use prost::Message as _;

    #[test]
    fn msg_send() {
        let m = MsgSend {
            from_address: "a".into(),
            to_address: String::new(),
            amount: vec![
                Coin {
                    denom: "utia".into(),
                    amount: "0010".into(),
                },
                Coin {
                    denom: "x\n\u{e9}".into(),
                    amount: String::new(),
                },
            ],
        };
        assert_eq!(
            compact_text("/cosmos.bank.v1beta1.MsgSend", &m.encode_to_vec(), 10).as_deref(),
            Some(
                "from_address:\"a\" amount:<denom:\"utia\" amount:\"8\" > \
                 amount:<denom:\"x\\n\\303\\251\" amount:\"0\" > "
            )
        );
    }
}
