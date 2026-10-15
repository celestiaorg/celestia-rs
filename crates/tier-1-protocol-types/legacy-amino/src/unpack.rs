//! `codectypes.UnpackInterfaces` with the legacy amino unpackers
//! (`AminoUnpacker` after binary decoding, `AminoJSONUnpacker` after JSON
//! decoding): every `Any` field `UnpackInterfaces` visits is decoded into
//! its interface, depth first, in field order.

use crate::schema::{Kind, Schema, Special};
use crate::{AnyV, Error, Res, V, err};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Binary,
    Json,
}

/// `codectypes.anyCompatError`.
pub(crate) fn any_compat_error(what: &str, x: &str) -> String {
    format!(
        "{what} marshaling error for {x}, this is likely because amino is being used directly \
         (instead of codec.LegacyAmino which is preferred) or UnpackInterfacesMessage is not defined \
         for some type which contains a protobuf Any either directly or via one of its members. To see \
         a stacktrace of where the error is coming from, set the var Debug = true in \
         codec/types/compat.go"
    )
}

fn nil_deref() -> Error {
    Error::Panic("runtime error: invalid memory address or nil pointer dereference".into())
}

impl Schema {
    pub(crate) fn unpack(&self, ty: usize, v: &mut V, mode: Mode) -> Res<()> {
        let t = self.t(ty);
        match (t.kind, v) {
            (Kind::Ptr, V::Ptr(inner)) => self.unpack(t.elem, inner, mode),
            (Kind::Struct, V::Struct(vals))
                if t.special == Special::None || t.special == Special::Multisig =>
            {
                for (f, fv) in t.fields.iter().zip(vals.iter_mut()) {
                    match f.any_iface {
                        Some(iface) => self.unpack_any_field(f.ty, fv, iface, mode)?,
                        None => self.unpack(f.ty, fv, mode)?,
                    }
                }
                Ok(())
            }
            (Kind::Slice | Kind::Array, V::List(l)) => {
                for e in l {
                    self.unpack(t.elem, e, mode)?;
                }
                Ok(())
            }
            (Kind::Interface, V::Iface(c, cv)) => self.unpack(*c, cv, mode),
            _ => Ok(()),
        }
    }

    fn unpack_any_field(&self, ty: usize, v: &mut V, iface: usize, mode: Mode) -> Res<()> {
        let t = self.t(ty);
        match t.kind {
            Kind::Ptr => match v {
                V::Ptr(inner) => self.unpack_any_field(t.elem, inner, iface, mode),
                _ => Err(nil_deref()),
            },
            Kind::Slice | Kind::Array => {
                if let V::List(l) = v {
                    for e in l {
                        self.unpack_any_field(t.elem, e, iface, mode)?;
                    }
                }
                Ok(())
            }
            _ => {
                let V::Any(a) = v else { return Ok(()) };
                let what = match mode {
                    Mode::Binary => "amino binary unmarshal",
                    Mode::Json => "JSON unmarshal",
                };
                let decoded = match (&*a, mode) {
                    (AnyV::Value(..), _) => return Ok(()),
                    (AnyV::Amino(bz), Mode::Binary) => self.unmarshal_binary_bare(bz, iface)?,
                    (AnyV::Json(bz), Mode::Json) => {
                        if bz.is_empty() {
                            return err("UnmarshalJSON cannot decode empty bytes");
                        }
                        self.dec_json(bz, iface)?
                    }
                    _ => return err(any_compat_error(what, &self.ptr_go(iface))),
                };
                let V::Iface(c, mut cv) = decoded else {
                    return Err(Error::Panic(
                        "reflect: call of reflect.Value.Type on zero Value".into(),
                    ));
                };
                self.unpack(c, &mut cv, mode)?;
                *a = AnyV::Value(c, cv);
                Ok(())
            }
        }
    }
}
