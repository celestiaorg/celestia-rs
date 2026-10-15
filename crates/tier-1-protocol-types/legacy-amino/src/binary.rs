//! go-amino binary: port of `binary-decode.go` / `binary-encode.go`
//! (including the consumed-byte counts its error messages report).

use num_bigint::BigInt;

use crate::schema::{Kind, Schema, Special};
use crate::time::{MAX_SECS, MIN_SECS};
use crate::{AnyV, Error, Res, V, upper_hex};

const VARINT: u8 = 0;
const BYTE_LENGTH: u8 = 2;

fn typ3_str(t: u8) -> String {
    match t {
        0 => "(U)Varint".into(),
        1 => "8Byte".into(),
        2 => "ByteLength".into(),
        5 => "4Byte".into(),
        _ => format!("<Invalid Typ3 {t:X}>"),
    }
}

fn go_err(s: impl Into<String>) -> Error {
    Error::Go(s.into())
}

/// Go `binary.Uvarint`: value and byte count (0: too short, <0: overflow).
fn uvarint(buf: &[u8]) -> (u64, isize) {
    let mut x = 0u64;
    let mut s = 0u32;
    for (i, &b) in buf.iter().enumerate() {
        if i == 10 {
            return (0, -11);
        }
        if b < 0x80 {
            if i == 9 && b > 1 {
                return (0, -10);
            }
            return (x | u64::from(b) << s, isize::try_from(i + 1).unwrap_or(0));
        }
        x |= u64::from(b & 0x7f) << s;
        s += 7;
    }
    (0, 0)
}

/// go-amino `DecodeUvarint`.
fn decode_uvarint(bz: &[u8]) -> (u64, usize, Option<Error>) {
    let (u, n) = uvarint(bz);
    match n.cmp(&0) {
        std::cmp::Ordering::Equal => (u, 0, Some(go_err("buffer too small"))),
        std::cmp::Ordering::Less => (u, n.unsigned_abs(), Some(go_err("EOF decoding uvarint"))),
        std::cmp::Ordering::Greater => (u, n.unsigned_abs(), None),
    }
}

fn uvarint_size(u: u64) -> usize {
    if u == 0 {
        1
    } else {
        (64 - u.leading_zeros() as usize).div_ceil(7)
    }
}

pub(crate) fn put_uvarint(out: &mut Vec<u8>, mut u: u64) {
    while u >= 0x80 {
        out.push((u as u8) | 0x80);
        u >>= 7;
    }
    out.push(u as u8);
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    put_uvarint(out, b.len() as u64);
    out.extend_from_slice(b);
}

fn put_key(out: &mut Vec<u8>, num: u32, typ: u8) {
    put_uvarint(out, u64::from(num) << 3 | u64::from(typ));
}

/// go-amino `DecodeByteSlice`.
fn decode_byte_slice(bz: &[u8]) -> (Vec<u8>, usize, Option<Error>) {
    let (count, n0, e) = decode_uvarint(bz);
    if let Some(e) = e {
        return (Vec::new(), n0, Some(e));
    }
    let rest = &bz[n0..];
    if i64::try_from(count).is_err() {
        return (
            Vec::new(),
            n0,
            Some(go_err(format!(
                "invalid negative length {count} decoding []byte"
            ))),
        );
    }
    let Some(b) = usize::try_from(count).ok().and_then(|c| rest.get(..c)) else {
        return (
            Vec::new(),
            n0,
            Some(go_err(format!(
                "insufficient bytes decoding []byte of length {count}"
            ))),
        );
    };
    (b.to_vec(), n0 + b.len(), None)
}

/// go-amino `decodeFieldNumberAndTyp3`.
fn decode_key(bz: &[u8]) -> (u32, u8, usize, Option<Error>) {
    let (v, n, e) = decode_uvarint(bz);
    if e.is_some() {
        return (0, 0, n, e);
    }
    let typ = (v & 7) as u8;
    let num = v >> 3;
    if num > (1 << 29) - 1 {
        return (0, typ, n, Some(go_err(format!("invalid field num {num}"))));
    }
    (u32::try_from(num).unwrap_or(0), typ, n, None)
}

type Dec = (usize, Res<V>);

impl Schema {
    /// go-amino `typeToTyp3` of the dereferenced type `i`.
    fn typ3(&self, i: usize) -> u8 {
        match self.t(i).kind {
            Kind::Interface | Kind::Array | Kind::Slice | Kind::String | Kind::Struct => {
                BYTE_LENGTH
            }
            _ => VARINT,
        }
    }

    /// go-amino `isDefaultValue`: the dereferenced value, or `None` when
    /// it is the default.
    fn non_default<'a>(&self, mut ty: usize, mut v: &'a V) -> Option<(usize, &'a V)> {
        while self.t(ty).kind == Kind::Ptr {
            match v {
                V::Ptr(inner) => v = inner,
                _ => return None,
            }
            ty = self.t(ty).elem;
        }
        let default = match (self.t(ty).kind, v) {
            (_, V::Nil) => true,
            (Kind::Int32 | Kind::Int64, V::Int(x)) => *x == 0,
            (Kind::Uint | Kind::Uint8 | Kind::Uint32 | Kind::Uint64, V::Uint(x)) => *x == 0,
            (Kind::String, V::Str(s)) => s.is_empty(),
            (Kind::Slice, V::List(l)) => l.is_empty(),
            (Kind::Slice, V::Bytes(b)) => b.is_empty(),
            _ => false,
        };
        (!default).then_some((ty, v))
    }

    /// `UnmarshalBinaryBare(bz, *T)`.
    pub(crate) fn unmarshal_binary_bare(&self, bz: &[u8], ty: usize) -> Res<V> {
        let t = self.t(ty);
        let mut bz = bz;
        if let Some(r) = &t.reg {
            if bz.len() < 4 {
                return Err(go_err(format!(
                    "UnmarshalBinaryBare expected to read prefix bytes {} (since it is registered concrete) but got {}",
                    upper_hex(&r.prefix),
                    upper_hex(bz)
                )));
            }
            if bz[..4] != r.prefix {
                return Err(go_err(format!(
                    "UnmarshalBinaryBare expected to read prefix bytes {} (since it is registered concrete) but got {}...",
                    upper_hex(&r.prefix),
                    upper_hex(&bz[..4])
                )));
            }
            bz = &bz[4..];
        }
        let (n, r) = self.dec(bz, ty, 1, true);
        let v = r.map_err(|e| {
            go_err(format!(
                "unmarshal to {} failed after {n} bytes ({e}): {}",
                t.go,
                upper_hex(bz)
            ))
        })?;
        if n != bz.len() {
            return Err(go_err(format!(
                "unmarshal to {} didn't read all bytes. Expected to read {}, only read {n}: {}",
                t.go,
                bz.len(),
                upper_hex(bz)
            )));
        }
        Ok(v)
    }

    /// `decodeReflectBinary`.
    fn dec(&self, bz: &[u8], ty: usize, fnum: u32, bare: bool) -> Dec {
        if self.t(ty).kind == Kind::Ptr {
            let (n, r) = self.dec(bz, self.t(ty).elem, fnum, bare);
            return (n, r.map(|v| V::Ptr(Box::new(v))));
        }
        let t = self.t(ty);
        match t.special {
            Special::Int | Special::Dec | Special::Any | Special::Key(..) => {
                let (bz2, n, e) = decode_byte_slice(bz);
                if let Some(e) = e {
                    return (n, Err(e));
                }
                return (n, self.unmarshal_amino(ty, bz2));
            }
            _ => {}
        }
        match t.kind {
            Kind::Interface => self.dec_iface(bz, ty, fnum, bare),
            Kind::Array => {
                if t.len > bz.len() {
                    return (
                        0,
                        Err(go_err(format!(
                            "Insufficient bytes to decode [{}]byte.",
                            t.len
                        ))),
                    );
                }
                let (b, n, e) = decode_byte_slice(bz);
                if let Some(e) = e {
                    return (n, Err(e));
                }
                if b.len() != t.len {
                    return (
                        n,
                        Err(go_err(format!(
                            "Mismatched byte array length: Expected {}, got {}",
                            t.len,
                            b.len()
                        ))),
                    );
                }
                (n, Ok(V::Bytes(b)))
            }
            Kind::Slice if self.is_bytes(ty) => {
                let (b, n, e) = decode_byte_slice(bz);
                if let Some(e) = e {
                    return (n, Err(e));
                }
                (n, Ok(if b.is_empty() { V::Nil } else { V::Bytes(b) }))
            }
            Kind::Slice => self.dec_slice(bz, ty, fnum, bare),
            Kind::Struct => self.dec_struct(bz, ty, bare),
            Kind::Int64 | Kind::Int32 | Kind::Uint | Kind::Uint32 | Kind::Uint64 => {
                let (u, n, e) = decode_uvarint(bz);
                if let Some(e) = e {
                    return (n, Err(e));
                }
                let v = match t.kind {
                    Kind::Int64 => V::Int(u as i64),
                    Kind::Int32 => {
                        let x = u as i64;
                        if x > i64::from(i32::MAX) || x < i64::from(i32::MIN) {
                            return (n, Err(go_err("encoded integer value overflows int(32)")));
                        }
                        V::Int(x)
                    }
                    _ => V::Uint(u),
                };
                (n, Ok(v))
            }
            Kind::Uint8 => {
                let (u, n, e) = decode_uvarint(bz);
                if let Some(e) = e {
                    return (n, Err(e));
                }
                if u > 255 {
                    return (n, Err(go_err("EOF decoding uint8")));
                }
                (n, Ok(V::Uint(u)))
            }
            Kind::Bool => match bz.first() {
                None => (0, Err(go_err("EOF decoding bool"))),
                Some(0) => (1, Ok(V::Bool(false))),
                Some(1) => (1, Ok(V::Bool(true))),
                Some(_) => (1, Err(go_err("invalid bool"))),
            },
            Kind::String => {
                let (b, n, e) = decode_byte_slice(bz);
                (n, e.map_or(Ok(V::Str(b)), Err))
            }
            Kind::Ptr => (0, Err(go_err("unreachable"))),
        }
    }

    /// `UnmarshalAmino` of the types overriding go-amino binary.
    fn unmarshal_amino(&self, ty: usize, bz: Vec<u8>) -> Res<V> {
        match self.t(ty).special {
            Special::Any => Ok(V::Any(AnyV::Amino(bz))),
            Special::Int => {
                if bz.is_empty() {
                    return Ok(V::BigInt(None));
                }
                let i = parse_text(&bz)?;
                if i.bits() > 256 {
                    return Err(go_err(format!(
                        "integer out of range; got: {}, max: 256",
                        i.bits()
                    )));
                }
                Ok(V::BigInt(Some(i)))
            }
            Special::Dec => {
                if bz.is_empty() {
                    return Ok(V::Dec(None));
                }
                let i = parse_text(&bz)?;
                math::LegacyDec::from_scaled(i.clone())
                    .map_err(|_| go_err("decimal out of range"))?;
                Ok(V::Dec(Some(i)))
            }
            Special::Key(size, pubkey) => {
                if bz.len() != size {
                    return Err(if pubkey {
                        Error::Sdk(8, "invalid pubkey size: invalid pubkey".into())
                    } else {
                        Error::Go("invalid privkey size".into())
                    });
                }
                Ok(V::Struct(vec![V::Bytes(bz)]))
            }
            _ => Err(go_err("unreachable")),
        }
    }

    fn dec_iface(&self, bz: &[u8], ty: usize, fnum: u32, bare: bool) -> Dec {
        let mut n = 0;
        let buf;
        let mut bz = bz;
        if !bare {
            let (b, _, e) = decode_byte_slice(bz);
            if let Some(e) = e {
                return (0, Err(e));
            }
            n += uvarint_size(b.len() as u64);
            buf = b;
            bz = &buf;
        }
        if bz.len() < 4 {
            return (n, Err(go_err("EOF while reading prefix bytes.")));
        }
        let cty = if bz[0] == 0 {
            if bz.len() < 8 {
                return (n, Err(go_err("EOF while reading disamb bytes.")));
            }
            let mut df = [0u8; 7];
            df.copy_from_slice(&bz[1..8]);
            bz = &bz[8..];
            n += 8;
            match self.by_disfix.get(&df) {
                Some(c) => *c,
                None => {
                    return (
                        n,
                        Err(go_err(format!(
                            "unrecognized disambiguation+prefix bytes {}",
                            upper_hex(&df)
                        ))),
                    );
                }
            }
        } else {
            let mut pb = [0u8; 4];
            pb.copy_from_slice(&bz[..4]);
            bz = &bz[4..];
            n += 4;
            match self.t(ty).implementers.get(&pb).map(Vec::as_slice) {
                None | Some([]) => {
                    return (
                        n,
                        Err(go_err(format!(
                            "unrecognized prefix bytes {}",
                            upper_hex(&pb)
                        ))),
                    );
                }
                Some([c]) => *c,
                Some([a, b, ..]) => {
                    return (
                        n,
                        Err(go_err(format!(
                            "conflicting concrete types registered for {}: e.g. {} and {}",
                            upper_hex(&pb),
                            self.t(*a).go,
                            self.t(*b).go
                        ))),
                    );
                }
            }
        };
        let (m, r) = self.dec(bz, cty, fnum, true);
        n += m;
        let v = match r {
            Ok(v) => v,
            Err(e) => return (n, Err(e)),
        };
        if m < bz.len() {
            return (
                n,
                Err(go_err("bytes left over after reading interface contents")),
            );
        }
        (n, Ok(V::Iface(cty, Box::new(v))))
    }

    fn dec_slice(&self, bz: &[u8], ty: usize, fnum: u32, bare: bool) -> Dec {
        let ert = self.t(ty).elem;
        let einfo = self.deref(ert);
        let mut n = 0;
        let buf;
        let mut bz = bz;
        if !bare {
            let (b, _, e) = decode_byte_slice(bz);
            if let Some(e) = e {
                return (0, Err(e));
            }
            n += uvarint_size(b.len() as u64);
            buf = b;
            bz = &buf;
        }
        let mut out = Vec::new();
        if self.typ3(einfo) == BYTE_LENGTH {
            loop {
                if bz.is_empty() {
                    break;
                }
                let (num, typ, kn, _) = decode_key(bz);
                if num < fnum {
                    return (
                        n,
                        Err(go_err(format!(
                            "expected repeated field number {fnum} or greater, got {num}"
                        ))),
                    );
                }
                if num > fnum {
                    break;
                }
                if typ != BYTE_LENGTH {
                    return (
                        n,
                        Err(go_err(format!(
                            "expected repeated field type {}, got {}",
                            typ3_str(BYTE_LENGTH),
                            typ3_str(typ)
                        ))),
                    );
                }
                bz = &bz[kn..];
                n += kn;
                if bz.first() == Some(&0) {
                    bz = &bz[1..];
                    n += 1;
                    out.push(self.default_value(ert));
                    continue;
                }
                let (m, r) = self.dec(bz, ert, 1, false);
                bz = &bz[m..];
                n += m;
                match r {
                    Ok(v) => out.push(v),
                    Err(e) => {
                        return (n, Err(go_err(format!("error reading array contents: {e}"))));
                    }
                }
            }
        } else {
            while !bz.is_empty() {
                let (m, r) = self.dec(bz, ert, fnum, false);
                bz = &bz[m..];
                n += m;
                match r {
                    Ok(v) => {
                        let default =
                            self.t(ert).kind == Kind::Ptr && self.non_default(ert, &v).is_none();
                        out.push(if default { V::Nil } else { v });
                    }
                    Err(e) => {
                        return (n, Err(go_err(format!("error reading array contents: {e}"))));
                    }
                }
            }
        }
        (n, Ok(if out.is_empty() { V::Nil } else { V::List(out) }))
    }

    fn dec_struct(&self, bz: &[u8], ty: usize, bare: bool) -> Dec {
        let t = self.t(ty);
        let mut n = 0;
        let buf;
        let mut bz = bz;
        if !bare {
            let (b, _, e) = decode_byte_slice(bz);
            if let Some(e) = e {
                return (0, Err(e));
            }
            n += uvarint_size(b.len() as u64);
            buf = b;
            bz = &buf;
        }
        if t.special == Special::Time {
            let (secs, nanos, m, e) = decode_time(bz);
            n += m;
            return (n, e.map_or(Ok(V::Time(secs, nanos)), Err));
        }
        let mut last = 0u32;
        let mut vals = Vec::with_capacity(t.fields.len());
        for (i, f) in t.fields.iter().enumerate() {
            let num = u32::try_from(i + 1).unwrap_or(0);
            if bz.is_empty() {
                vals.push(self.default_value(f.ty));
                continue;
            }
            if f.unpacked {
                let (m, r) = self.dec(bz, f.ty, num, true);
                bz = &bz[m..];
                n += m;
                match r {
                    Ok(v) => vals.push(v),
                    Err(e) => return (n, Err(e)),
                }
                continue;
            }
            let (fnum, typ, kn, e) = decode_key(bz);
            if num < fnum {
                vals.push(self.default_value(f.ty));
                continue;
            }
            if fnum <= last {
                return (
                    n,
                    Err(go_err(format!(
                        "encountered fieldnNum: {fnum}, but we have already seen fnum: {last}\nbytes:{}",
                        upper_hex(bz)
                    ))),
                );
            }
            last = fnum;
            bz = &bz[kn..];
            n += kn;
            if let Some(e) = e {
                return (n, Err(e));
            }
            if num != fnum {
                return (
                    n,
                    Err(go_err(format!(
                        "expected field # {num} of {}, got {fnum}",
                        t.go
                    ))),
                );
            }
            let want = self.typ3(self.deref(f.ty));
            if typ != want {
                return (
                    n,
                    Err(go_err(format!(
                        "expected field type {} for # {fnum} of {}, got {}",
                        typ3_str(want),
                        t.go,
                        typ3_str(typ)
                    ))),
                );
            }
            let (m, r) = self.dec(bz, f.ty, num, false);
            bz = &bz[m..];
            n += m;
            match r {
                Ok(v) => vals.push(v),
                Err(e) => return (n, Err(e)),
            }
        }
        while !bz.is_empty() {
            let (fnum, typ, kn, e) = decode_key(bz);
            bz = &bz[kn..];
            n += kn;
            if let Some(e) = e {
                return (n, Err(e));
            }
            if fnum <= last {
                return (
                    n,
                    Err(go_err(format!(
                        "encountered fieldnNum: {fnum}, but we have already seen fnum: {last}\nbytes:{}",
                        upper_hex(bz)
                    ))),
                );
            }
            last = fnum;
            let r = match typ {
                0 => {
                    let (_, k) = uvarint(bz);
                    match k.cmp(&0) {
                        std::cmp::Ordering::Equal => Err(go_err("buffer too small")),
                        std::cmp::Ordering::Less => Err(go_err("EOF decoding varint")),
                        std::cmp::Ordering::Greater => Ok(k.unsigned_abs()),
                    }
                }
                1 => {
                    if bz.len() < 8 {
                        Err(go_err("EOF decoding int64"))
                    } else {
                        Ok(8)
                    }
                }
                2 => {
                    let (_, k, e) = decode_byte_slice(bz);
                    e.map_or(Ok(k), Err)
                }
                5 => {
                    if bz.len() < 4 {
                        Err(go_err("EOF decoding int32"))
                    } else {
                        Ok(4)
                    }
                }
                _ => Err(go_err(format!("invalid typ3 bytes {}", typ3_str(typ)))),
            };
            match r {
                Ok(k) => {
                    bz = &bz[k..];
                    n += k;
                }
                Err(e) => return (n, Err(e)),
            }
        }
        (n, Ok(V::Struct(vals)))
    }

    /// `MarshalBinaryBare(o)`.
    pub(crate) fn marshal_binary_bare(&self, ty: usize, v: &V) -> Res<Vec<u8>> {
        let mut out = Vec::new();
        if let Some(r) = &self.t(ty).reg {
            out.extend_from_slice(&r.prefix);
        }
        self.enc(&mut out, ty, v, 1, true)?;
        Ok(out)
    }

    /// `encodeReflectBinary` (`ty` is not a pointer type).
    fn enc(&self, out: &mut Vec<u8>, ty: usize, v: &V, fnum: u32, bare: bool) -> Res<()> {
        let t = self.t(ty);
        match (t.special, v) {
            (Special::Int, V::BigInt(i)) | (Special::Dec, V::Dec(i)) => {
                let text = i.as_ref().map_or_else(|| "0".to_owned(), BigInt::to_string);
                put_bytes(out, text.as_bytes());
                return Ok(());
            }
            (Special::Any, V::Any(a)) => {
                let bz = match a {
                    AnyV::Value(c, cv) => self.marshal_binary_bare(*c, cv)?,
                    AnyV::Amino(b) => b.clone(),
                    AnyV::Json(_) => Vec::new(),
                    AnyV::Zero => {
                        return Err(go_err(crate::unpack::any_compat_error(
                            "amino binary marshal",
                            "{TypeUrl: Value:[] XXX_unrecognized:[] XXX_sizecache:0}",
                        )));
                    }
                };
                put_bytes(out, &bz);
                return Ok(());
            }
            (Special::Key(..), V::Struct(f)) => {
                let key = match f.first() {
                    Some(V::Bytes(b)) => b.as_slice(),
                    _ => &[],
                };
                put_bytes(out, key);
                return Ok(());
            }
            _ => {}
        }
        match t.kind {
            Kind::Interface => {
                let V::Iface(c, cv) = v else {
                    out.push(0);
                    return Ok(());
                };
                let ct = self.t(*c);
                let Some(r) = &ct.reg else {
                    return Err(go_err(format!(
                        "Cannot encode unregistered concrete type {}.",
                        ct.go
                    )));
                };
                let mut buf = Vec::new();
                let need_disamb =
                    t.always_disamb || t.implementers.get(&r.prefix).is_some_and(|l| l.len() > 1);
                if need_disamb {
                    buf.push(0);
                    buf.extend_from_slice(&r.disamb);
                }
                buf.extend_from_slice(&r.prefix);
                self.enc(&mut buf, *c, cv, fnum, true)?;
                if bare {
                    out.extend_from_slice(&buf);
                } else {
                    put_bytes(out, &buf);
                }
            }
            Kind::Array | Kind::Slice if self.is_bytes(ty) => match v {
                V::Bytes(b) => put_bytes(out, b),
                _ => put_bytes(out, &[]),
            },
            Kind::Array | Kind::Slice => self.enc_list(out, ty, v, fnum, bare)?,
            Kind::Struct => {
                let mut buf = Vec::new();
                if let V::Time(s, ns) = v {
                    encode_time(&mut buf, *s, *ns)?;
                } else if let V::Struct(vals) = v {
                    for (i, f) in t.fields.iter().enumerate() {
                        let num = u32::try_from(i + 1).unwrap_or(0);
                        let Some(fv) = vals.get(i) else { continue };
                        let Some((dty, dv)) = self.non_default(f.ty, fv) else {
                            continue;
                        };
                        if f.unpacked {
                            self.enc_list(&mut buf, dty, dv, num, true)?;
                            continue;
                        }
                        let before_key = buf.len();
                        put_key(&mut buf, num, self.typ3(dty));
                        let before_val = buf.len();
                        self.enc(&mut buf, dty, dv, num, false)?;
                        let is_ptr = self.t(f.ty).kind == Kind::Ptr;
                        if !is_ptr && buf.len() == before_val + 1 && buf.last() == Some(&0) {
                            buf.truncate(before_key);
                        }
                    }
                }
                if bare {
                    out.extend_from_slice(&buf);
                } else {
                    put_bytes(out, &buf);
                }
            }
            Kind::Int32 | Kind::Int64 => {
                put_uvarint(out, if let V::Int(x) = v { *x as u64 } else { 0 });
            }
            Kind::Uint | Kind::Uint8 | Kind::Uint32 | Kind::Uint64 => {
                put_uvarint(out, if let V::Uint(x) = v { *x } else { 0 });
            }
            Kind::Bool => out.push(u8::from(matches!(v, V::Bool(true)))),
            Kind::String => put_bytes(out, if let V::Str(s) = v { s } else { &[] }),
            Kind::Ptr => return Err(go_err("unreachable")),
        }
        Ok(())
    }

    fn enc_list(&self, out: &mut Vec<u8>, ty: usize, v: &V, fnum: u32, bare: bool) -> Res<()> {
        let ert = self.t(ty).elem;
        let einfo = self.deref(ert);
        let empty = Vec::new();
        let items = if let V::List(l) = v { l } else { &empty };
        let mut buf = Vec::new();
        if self.typ3(einfo) == BYTE_LENGTH {
            for e in items {
                put_key(&mut buf, fnum, BYTE_LENGTH);
                match self.non_default(ert, e) {
                    None => buf.push(0),
                    Some((dty, dv)) => self.enc(&mut buf, dty, dv, 1, false)?,
                }
            }
        } else {
            for e in items {
                let mut d = e;
                while let V::Ptr(inner) = d {
                    d = inner;
                }
                let zero;
                if *d == V::Nil {
                    zero = self.zero(einfo);
                    d = &zero;
                }
                self.enc(&mut buf, einfo, d, fnum, false)?;
            }
        }
        if bare {
            out.extend_from_slice(&buf);
        } else {
            put_bytes(out, &buf);
        }
        Ok(())
    }
}

fn parse_text(bz: &[u8]) -> Res<BigInt> {
    math::parse_big_int_text(bz).ok_or_else(|| {
        go_err(format!(
            "math/big: cannot unmarshal {} into a *big.Int",
            crate::gojson::go_quote(bz)
        ))
    })
}

/// go-amino `DecodeTime`: seconds, nanos, bytes consumed, error.
fn decode_time(bz: &[u8]) -> (i64, u32, usize, Option<Error>) {
    let mut bz = bz;
    let mut n = 0;
    let mut sec = 0i64;
    let mut nsec = 0u32;
    if !bz.is_empty() {
        let (num, typ, kn, e) = decode_key(bz);
        if let Some(e) = e {
            return (0, 0, 0, Some(e));
        }
        if num == 1 && typ == VARINT {
            bz = &bz[kn..];
            n = kn;
            let (u, m, e) = decode_uvarint(bz);
            bz = &bz[m..];
            n += m;
            if let Some(e) = e {
                return (0, 0, n, Some(e));
            }
            let res = u as i64;
            if !(MIN_SECS..MAX_SECS).contains(&res) {
                return (
                    0,
                    0,
                    n,
                    Some(go_err(format!(
                        "invalid time: seconds have to be > {MIN_SECS} and < {MAX_SECS}, got: {res}"
                    ))),
                );
            }
            sec = res;
        } else if !(num == 2 && typ == VARINT) {
            return (
                0,
                0,
                0,
                Some(go_err(format!(
                    "expected field number 1 <Varint> or field number 2 <Varint> , got {num}"
                ))),
            );
        }
    }
    if !bz.is_empty() {
        let (num, typ, kn, e) = decode_key(bz);
        if let Some(e) = e {
            return (0, 0, n, Some(e));
        }
        if num == 2 && typ == VARINT {
            bz = &bz[kn..];
            n += kn;
            let (u, m, e) = decode_uvarint(bz);
            n += m;
            if let Some(e) = e {
                return (0, 0, n, Some(e));
            }
            if u > 999_999_999 {
                return (
                    0,
                    0,
                    n,
                    Some(go_err(format!(
                        "invalid time: nanoseconds not in interval [0, 999999999] {u}"
                    ))),
                );
            }
            nsec = u32::try_from(u).unwrap_or(0);
        }
    }
    (sec, nsec, n, None)
}

/// go-amino `EncodeTime`.
fn encode_time(out: &mut Vec<u8>, s: i64, ns: u32) -> Res<()> {
    if s != 0 {
        if !(MIN_SECS..MAX_SECS).contains(&s) {
            return Err(go_err(format!(
                "invalid time: seconds have to be >= {MIN_SECS} and < {MAX_SECS}, got: {s}"
            )));
        }
        put_key(out, 1, VARINT);
        put_uvarint(out, s as u64);
    }
    if ns != 0 {
        put_key(out, 2, VARINT);
        put_uvarint(out, u64::from(ns));
    }
    Ok(())
}
