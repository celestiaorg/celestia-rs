//! go-amino JSON: port of `json-encode.go` / `json-decode.go` plus the Go
//! JSON (un)marshalers of the overriding types.

use base64::Engine as _;
use num_bigint::BigInt;

use crate::gojson::{
    array_items, check_valid, encode_string, go_quote, kind_name, object_entries, trim, type_error,
    unmarshal_bool, unmarshal_bytes, unmarshal_int, unmarshal_string,
};
use crate::schema::{Kind, Schema, Special};
use crate::{AnyV, Error, Res, V, err};

fn b64(out: &mut Vec<u8>, b: &[u8]) {
    out.push(b'"');
    out.extend_from_slice(
        base64::engine::general_purpose::STANDARD
            .encode(b)
            .as_bytes(),
    );
    out.push(b'"');
}

fn dec_string(i: Option<&BigInt>) -> String {
    i.map_or_else(
        || "0".to_owned(),
        |i| {
            math::LegacyDec::from_scaled(i.clone())
                .map_or_else(|_| i.to_string(), |d| d.to_string())
        },
    )
}

impl Schema {
    /// `Codec.MarshalJSON(o)` (`o` passed by value: not addressable).
    pub(crate) fn marshal_json(&self, ty: usize, v: &V) -> Res<Vec<u8>> {
        let mut out = Vec::new();
        let reg = self.t(ty).reg.as_ref();
        if let Some(r) = reg {
            out.extend_from_slice(format!("{{\"type\":\"{}\",\"value\":", r.name).as_bytes());
        }
        self.enc_json(&mut out, ty, v, false)?;
        if reg.is_some() {
            out.push(b'}');
        }
        Ok(out)
    }

    /// The interface wrapper of a registered concrete value.
    fn enc_concrete(&self, out: &mut Vec<u8>, c: usize, v: &V) -> Res<()> {
        let ct = self.t(c);
        let Some(r) = &ct.reg else {
            return err(format!(
                "Cannot encode unregistered concrete type {}.",
                ct.go
            ));
        };
        out.extend_from_slice(format!("{{\"type\":\"{}\",\"value\":", r.name).as_bytes());
        self.enc_json(out, c, v, r.ptr_preferred)?;
        out.push(b'}');
        Ok(())
    }

    /// `encodeReflectJSON`.
    fn enc_json(&self, out: &mut Vec<u8>, ty: usize, v: &V, addr: bool) -> Res<()> {
        let (mut ty, mut v, mut addr) = (ty, v, addr);
        while self.t(ty).kind == Kind::Ptr {
            let V::Ptr(inner) = v else {
                out.extend_from_slice(b"null");
                return Ok(());
            };
            v = inner;
            addr = true;
            ty = self.t(ty).elem;
        }
        let t = self.t(ty);
        match (t.special, v) {
            (Special::Time, V::Time(s, ns)) => {
                let Some(f) = crate::time::format_rfc3339_nano(*s, *ns) else {
                    return err("Time.MarshalJSON: year outside of range [0,9999]");
                };
                out.extend_from_slice(format!("\"{f}\"").as_bytes());
                return Ok(());
            }
            (Special::Key(..), V::Struct(f)) => {
                match f.first() {
                    Some(V::Bytes(b)) => b64(out, b),
                    _ => out.extend_from_slice(b"null"),
                }
                return Ok(());
            }
            (Special::Multisig, V::Struct(f)) => {
                let k = match f.first() {
                    Some(V::Uint(k)) => *k,
                    _ => 0,
                };
                let mut pks = Vec::new();
                if let Some(V::List(l)) = f.get(1) {
                    for a in l {
                        match a {
                            V::Ptr(b) => match b.as_ref() {
                                V::Any(AnyV::Value(c, cv)) => pks.push(V::Iface(*c, cv.clone())),
                                _ => {
                                    return Err(Error::Sdk(
                                        29,
                                        "expected <nil>, got <nil>: invalid type".into(),
                                    ));
                                }
                            },
                            _ => {
                                return Err(Error::Panic(
                                    "runtime error: invalid memory address or nil pointer dereference".into(),
                                ));
                            }
                        }
                    }
                }
                let repr = V::Struct(vec![V::Uint(k), V::List(pks)]);
                return self.enc_json(out, self.multisig_repr, &repr, false);
            }
            (Special::Int, V::BigInt(i)) => {
                let s = i.as_ref().map_or_else(|| "0".to_owned(), BigInt::to_string);
                out.extend_from_slice(format!("\"{s}\"").as_bytes());
                return Ok(());
            }
            (Special::Dec, V::Dec(i)) => {
                out.extend_from_slice(format!("\"{}\"", dec_string(i.as_ref())).as_bytes());
                return Ok(());
            }
            (Special::HexAddress, V::Bytes(b)) => {
                let s: String = std::iter::once("0x".to_owned())
                    .chain(b.iter().map(|x| format!("{x:02x}")))
                    .collect();
                out.extend_from_slice(format!("\"{s}\"").as_bytes());
                return Ok(());
            }
            (Special::Coins, _) => {
                out.push(b'[');
                if let V::List(l) = v {
                    for (i, c) in l.iter().enumerate() {
                        if i > 0 {
                            out.push(b',');
                        }
                        let V::Struct(f) = c else { continue };
                        out.push(b'{');
                        if let Some(V::Str(d)) = f.first()
                            && !d.is_empty()
                        {
                            out.extend_from_slice(b"\"denom\":");
                            encode_string(out, d);
                            out.push(b',');
                        }
                        out.extend_from_slice(b"\"amount\":");
                        let amt = match f.get(1) {
                            Some(V::BigInt(Some(i))) => i.to_string(),
                            _ => "0".into(),
                        };
                        out.extend_from_slice(format!("\"{amt}\"").as_bytes());
                        out.push(b'}');
                    }
                }
                out.push(b']');
                return Ok(());
            }
            (Special::Any, V::Any(a)) => {
                if addr {
                    return match a {
                        AnyV::Value(c, cv) => self.enc_concrete(out, *c, cv),
                        AnyV::Json(raw) => {
                            out.extend_from_slice(raw);
                            Ok(())
                        }
                        AnyV::Amino(_) => Ok(()),
                        AnyV::Zero => err(crate::unpack::any_compat_error(
                            "JSON marshal",
                            "&{TypeUrl: Value:[] XXX_unrecognized:[] XXX_sizecache:0}",
                        )),
                    };
                }
                match a {
                    AnyV::Amino(b) => b64(out, b),
                    AnyV::Zero => {
                        return err(crate::unpack::any_compat_error(
                            "amino binary marshal",
                            "{TypeUrl: Value:[] XXX_unrecognized:[] XXX_sizecache:0}",
                        ));
                    }
                    _ => out.extend_from_slice(b"null"),
                }
                return Ok(());
            }
            (Special::Unsupported, _) => {
                return err(format!("amino JSON of {} is not supported", t.go));
            }
            _ => {}
        }
        match t.kind {
            Kind::Interface => match v {
                V::Iface(c, cv) => self.enc_concrete(out, *c, cv)?,
                _ => out.extend_from_slice(b"null"),
            },
            Kind::Slice | Kind::Array => {
                if t.kind == Kind::Slice && *v == V::Nil {
                    out.extend_from_slice(b"null");
                } else if self.is_bytes(ty) {
                    match v {
                        V::Bytes(b) => b64(out, b),
                        _ => b64(out, &[]),
                    }
                } else {
                    out.push(b'[');
                    if let V::List(l) = v {
                        for (i, e) in l.iter().enumerate() {
                            if i > 0 {
                                out.push(b',');
                            }
                            self.enc_json(out, t.elem, e, t.kind == Kind::Slice || addr)?;
                        }
                    }
                    out.push(b']');
                }
            }
            Kind::Struct => {
                out.push(b'{');
                let empty = Vec::new();
                let vals = if let V::Struct(f) = v { f } else { &empty };
                let mut comma = false;
                for (f, fv) in t.fields.iter().zip(vals) {
                    if f.omitempty && self.is_empty(f.ty, fv) {
                        continue;
                    }
                    if comma {
                        out.push(b',');
                    }
                    encode_string(out, f.json.as_bytes());
                    out.push(b':');
                    self.enc_json(out, f.ty, fv, addr)?;
                    comma = true;
                }
                out.push(b'}');
            }
            Kind::Int64 => out.extend_from_slice(
                format!("\"{}\"", if let V::Int(x) = v { *x } else { 0 }).as_bytes(),
            ),
            Kind::Uint | Kind::Uint64 => {
                out.extend_from_slice(
                    format!("\"{}\"", if let V::Uint(x) = v { *x } else { 0 }).as_bytes(),
                );
            }
            Kind::Int32 => out.extend_from_slice(
                (if let V::Int(x) = v { *x } else { 0 })
                    .to_string()
                    .as_bytes(),
            ),
            Kind::Uint8 | Kind::Uint32 => {
                out.extend_from_slice(
                    (if let V::Uint(x) = v { *x } else { 0 })
                        .to_string()
                        .as_bytes(),
                );
            }
            Kind::Bool => out.extend_from_slice(if matches!(v, V::Bool(true)) {
                b"true"
            } else {
                b"false"
            }),
            Kind::String => encode_string(out, if let V::Str(s) = v { s } else { &[] }),
            Kind::Ptr => {}
        }
        Ok(())
    }

    /// go-amino `isEmpty(derefPointers(field), field.ZeroValue)`.
    fn is_empty(&self, ty: usize, v: &V) -> bool {
        let mut d = v;
        let mut dty = ty;
        while self.t(dty).kind == Kind::Ptr {
            match d {
                V::Ptr(inner) => d = inner,
                _ => return true,
            }
            dty = self.t(dty).elem;
        }
        if self.t(ty).kind != Kind::Ptr && self.is_zero(ty, d) {
            return true;
        }
        match (self.t(dty).kind, d) {
            (Kind::Slice | Kind::Array, V::List(l)) => l.is_empty(),
            (Kind::Slice | Kind::Array, V::Bytes(b)) => b.is_empty(),
            (Kind::Slice, V::Nil) => true,
            (Kind::String, V::Str(s)) => s.is_empty(),
            _ => false,
        }
    }

    /// `Codec.UnmarshalJSON(bz, *T)`.
    pub(crate) fn unmarshal_json(&self, bz: &[u8], ty: usize) -> Res<V> {
        if bz.is_empty() {
            return err("UnmarshalJSON cannot decode empty bytes");
        }
        let t = self.t(ty);
        let mut bz = bz.to_vec();
        if let Some(r) = &t.reg {
            let (name, data) = decode_interface_json(&bz)?;
            if name != r.name.as_bytes() {
                return err(format!(
                    "UnmarshalJSON wants to decode a {} but found a {}",
                    r.name,
                    crate::go_lossy(&name)
                ));
            }
            bz = data;
        }
        self.dec_json(&bz, ty)
    }

    /// `decodeReflectJSON` into a fresh value of type `ty` (`bz` is a valid,
    /// trimmed JSON value).
    pub(crate) fn dec_json(&self, bz: &[u8], ty: usize) -> Res<V> {
        if bz == b"null" {
            return Ok(self.zero(ty));
        }
        if self.t(ty).kind == Kind::Ptr {
            return Ok(V::Ptr(Box::new(self.dec_json(bz, self.t(ty).elem)?)));
        }
        let t = self.t(ty);
        let quoted = bz.len() >= 2 && bz[0] == b'"' && bz[bz.len() - 1] == b'"';
        match t.special {
            Special::Time => {
                if !quoted {
                    return err(format!(
                        "Amino:JSON time must be an RFC3339Nano string, but got {}.",
                        crate::go_lossy(bz)
                    ));
                }
                if bz[bz.len() - 2] != b'Z' {
                    return err(format!(
                        "Amino:JSON time must be UTC and end with 'Z' but got {}.",
                        crate::go_lossy(bz)
                    ));
                }
                let (s, ns) = crate::time::parse_rfc3339(bz).map_err(Error::Go)?;
                return Ok(V::Time(s, ns));
            }
            Special::Key(size, pubkey) => {
                let b = unmarshal_bytes(bz, "[]uint8")
                    .map_err(Error::Go)?
                    .unwrap_or_default();
                if b.len() != size {
                    return Err(if pubkey {
                        Error::Sdk(8, "invalid pubkey size: invalid pubkey".into())
                    } else {
                        Error::Go("invalid privkey size".into())
                    });
                }
                return Ok(V::Struct(vec![V::Bytes(b)]));
            }
            Special::Multisig => {
                let tm = self.dec_json(bz, self.multisig_repr)?;
                let V::Struct(f) = tm else {
                    return err("unreachable");
                };
                let k = match f.first() {
                    Some(V::Uint(k)) => *k,
                    _ => 0,
                };
                let mut anys = Vec::new();
                if let Some(V::List(l)) = f.get(1) {
                    for pk in l {
                        let V::Iface(c, cv) = pk else {
                            return Err(Error::Sdk(
                                33,
                                "Expecting non nil value to create a new Any: failed packing protobuf message to Any"
                                    .into(),
                            ));
                        };
                        anys.push(V::Ptr(Box::new(V::Any(AnyV::Value(*c, cv.clone())))));
                    }
                }
                return Ok(V::Struct(vec![V::Uint(k & 0xffff_ffff), V::List(anys)]));
            }
            Special::Any => return Ok(V::Any(AnyV::Json(bz.to_vec()))),
            Special::Int => {
                let text = unmarshal_string(bz, "string")
                    .map_err(Error::Go)?
                    .unwrap_or_default();
                let Some(i) = math::parse_big_int_text(&text) else {
                    return err(format!(
                        "math/big: cannot unmarshal {} into a *big.Int",
                        go_quote(&text)
                    ));
                };
                if i.bits() > 256 {
                    return err(format!("integer out of range: {}", crate::go_lossy(&text)));
                }
                return Ok(V::BigInt(Some(i)));
            }
            Special::Dec => {
                let text = unmarshal_string(bz, "string")
                    .map_err(Error::Go)?
                    .unwrap_or_default();
                return parse_dec(&text).map(|d| V::Dec(Some(d)));
            }
            Special::HexAddress => {
                let s = unmarshal_string(bz, "string")
                    .map_err(Error::Go)?
                    .unwrap_or_default();
                let s = s.strip_prefix(b"0x").unwrap_or(&s);
                if s.len() != 64 {
                    return err("invalid hex address length");
                }
                let mut out = Vec::with_capacity(32);
                for pair in s.chunks(2) {
                    let mut byte = 0u8;
                    for &c in pair {
                        let d = match c {
                            b'0'..=b'9' => c - b'0',
                            b'a'..=b'f' => c - b'a' + 10,
                            b'A'..=b'F' => c - b'A' + 10,
                            _ => {
                                // `%#U` of `rune(byte)`: the Latin-1 code
                                // point, its char only if printable.
                                let ch = char::from(c);
                                let shown = if gostr::is_print(ch) {
                                    format!(" '{ch}'")
                                } else {
                                    String::new()
                                };
                                return err(format!(
                                    "encoding/hex: invalid byte: U+{:04X}{shown}",
                                    u32::from(c)
                                ));
                            }
                        };
                        byte = byte << 4 | d;
                    }
                    out.push(byte);
                }
                return Ok(V::Bytes(out));
            }
            Special::Unsupported => return err(format!("amino JSON of {} is not supported", t.go)),
            Special::None | Special::Coins => {}
        }
        match t.kind {
            Kind::Interface => {
                let (name, data) = decode_interface_json(bz)?;
                let Some(&c) = self.by_name.get(crate::go_lossy(&name).as_str()) else {
                    return err(format!(
                        "unrecognized concrete type name {}",
                        crate::go_lossy(&name)
                    ));
                };
                let v = self.dec_json(&data, c)?;
                if !t.implementers.values().any(|l| l.contains(&c)) {
                    return Err(Error::Panic(format!(
                        "reflect.Set: value of type {} is not assignable to type {}",
                        self.concrete_go(c),
                        t.go
                    )));
                }
                Ok(V::Iface(c, Box::new(v)))
            }
            Kind::Slice if self.is_bytes(ty) => {
                let b = unmarshal_bytes(bz, &t.go).map_err(Error::Go)?;
                Ok(match b {
                    Some(b) if !b.is_empty() => V::Bytes(b),
                    _ => V::Nil,
                })
            }
            Kind::Slice => {
                if bz[0] != b'[' {
                    return err(type_error(
                        kind_name(bz).unwrap_or("null"),
                        "[]json.RawMessage",
                    ));
                }
                let items = array_items(bz);
                if items.is_empty() {
                    return Ok(V::Nil);
                }
                let mut out = Vec::with_capacity(items.len());
                for it in items {
                    out.push(self.dec_json(it, t.elem)?);
                }
                Ok(V::List(out))
            }
            Kind::Array => err(format!("amino JSON of {} is not supported", t.go)),
            Kind::Struct => {
                if bz[0] != b'{' {
                    return err(type_error(
                        kind_name(bz).unwrap_or("null"),
                        "map[string]json.RawMessage",
                    ));
                }
                let entries = object_entries(bz);
                let mut vals = Vec::with_capacity(t.fields.len());
                for f in &t.fields {
                    let raw = entries
                        .iter()
                        .rev()
                        .find(|(k, _)| k == f.json.as_bytes())
                        .map(|(_, r)| *r);
                    match raw {
                        None => vals.push(self.zero(f.ty)),
                        Some(r) => vals.push(self.dec_json(r, f.ty)?),
                    }
                }
                Ok(V::Struct(vals))
            }
            Kind::Int64 | Kind::Uint | Kind::Uint64 => {
                if !(bz[0] == b'"' && bz[bz.len() - 1] == b'"') || bz.len() < 2 {
                    return err(format!(
                        "invalid character -- Amino:JSON int/int64/uint/uint64 expects quoted values for javascript numeric support, got: {}.",
                        crate::go_lossy(bz)
                    ));
                }
                let inner = &bz[1..bz.len() - 1];
                check_valid(inner).map_err(Error::Go)?;
                let inner = trim(inner);
                let signed = t.kind == Kind::Int64;
                let n = unmarshal_int(inner, &t.go, signed, 64)
                    .map_err(Error::Go)?
                    .unwrap_or(0);
                Ok(if signed {
                    V::Int(i64::try_from(n).unwrap_or(0))
                } else {
                    V::Uint(u64::try_from(n).unwrap_or(0))
                })
            }
            Kind::Int32 => {
                let n = unmarshal_int(bz, &t.go, true, 32)
                    .map_err(Error::Go)?
                    .unwrap_or(0);
                Ok(V::Int(i64::try_from(n).unwrap_or(0)))
            }
            Kind::Uint32 | Kind::Uint8 => {
                let bits = if t.kind == Kind::Uint8 { 8 } else { 32 };
                let n = unmarshal_int(bz, &t.go, false, bits)
                    .map_err(Error::Go)?
                    .unwrap_or(0);
                Ok(V::Uint(u64::try_from(n).unwrap_or(0)))
            }
            Kind::Bool => Ok(V::Bool(
                unmarshal_bool(bz, &t.go)
                    .map_err(Error::Go)?
                    .unwrap_or(false),
            )),
            Kind::String => Ok(V::Str(
                unmarshal_string(bz, &t.go)
                    .map_err(Error::Go)?
                    .unwrap_or_default(),
            )),
            Kind::Ptr => err("unreachable"),
        }
    }

    /// `%v` of the concrete type an interface holds (pointer if preferred).
    fn concrete_go(&self, c: usize) -> String {
        let t = self.t(c);
        if t.reg.as_ref().is_some_and(|r| r.ptr_preferred) {
            format!("*{}", t.go)
        } else {
            t.go.clone()
        }
    }
}

/// go-amino `decodeInterfaceJSON` (`json.Unmarshal` into `disfixWrapper`).
fn decode_interface_json(bz: &[u8]) -> Res<(Vec<u8>, Vec<u8>)> {
    let wrap = |e: String| Error::Go(format!("cannot parse disfix JSON wrapper: {e}"));
    check_valid(bz).map_err(wrap)?;
    let bz = trim(bz);
    let mut name = Vec::new();
    let mut data: Option<Vec<u8>> = None;
    let mut type_err = None;
    match kind_name(bz) {
        None => {}
        Some("object") => {
            for (k, raw) in object_entries(bz) {
                if k.eq_ignore_ascii_case(b"type") {
                    match kind_name(raw) {
                        None => {}
                        Some("string") => name = crate::gojson::unquote(raw),
                        Some(kind) => {
                            type_err.get_or_insert_with(|| {
                                format!(
                                    "json: cannot unmarshal {kind} into Go struct field disfixWrapper.type of type string"
                                )
                            });
                        }
                    }
                } else if k.eq_ignore_ascii_case(b"value") {
                    data = Some(raw.to_vec());
                }
            }
        }
        Some(kind) => return Err(wrap(type_error(kind, "amino.disfixWrapper"))),
    }
    if let Some(e) = type_err {
        return Err(wrap(e));
    }
    if name.is_empty() {
        return err("JSON encoding of interfaces require non-empty type field.");
    }
    match data {
        Some(d) if !d.is_empty() => Ok((name, d)),
        _ => err("interface JSON wrapper should have non-empty value field"),
    }
}

/// `math.LegacyNewDecFromStr` with Go's error texts.
fn parse_dec(text: &[u8]) -> Res<BigInt> {
    let s = crate::go_lossy(text);
    match math::LegacyDec::from_str_go(&s) {
        Ok(d) => Ok(d.as_scaled().clone()),
        // Go formats the string after stripping the sign.
        Err(math::MathError::ExcessPrecision(n)) => err(format!(
            "value '{}' exceeds max precision by {} decimal places: max precision 18",
            s.strip_prefix('-').unwrap_or(&s),
            18 - i64::try_from(n).unwrap_or(0)
        )),
        Err(math::MathError::InvalidDecimalStr) => {
            let body = s.strip_prefix('-').unwrap_or(&s);
            if body.matches('.').count() > 1 {
                err("invalid decimal string")
            } else {
                err("out of range: invalid decimal string")
            }
        }
        Err(e) => err(e.to_string()),
    }
}
