//! `prost`, except that protobuf `string` fields decode like gogoproto:
//! any bytes are accepted and kept in Go form ([`gostr`]), and encoding
//! writes the Go bytes back.
//!
//! The generated protobuf crates depend on this crate under the name
//! `prost` (`prost = { package = "prost-gostr", ... }`), so the code
//! `#[derive(prost::Message)]` expands to (`::prost::encoding::string::*`)
//! resolves to [`encoding::string`] here. Everything else is prost itself:
//! the `Message` trait, errors and all other encoders are the same items.

pub use prost::*;

/// `prost::encoding` with gogoproto's `string` codec.
#[doc(hidden)]
pub mod encoding {
    pub use prost::encoding::*;

    /// Protobuf `string` fields as gogoproto handles them: no UTF-8 check
    /// on decode (bytes kept in Go form), Go bytes on encode.
    #[allow(clippy::ptr_arg)]
    pub mod string {
        use prost::DecodeError;
        use prost::bytes::{Buf, BufMut};
        use prost::encoding::{
            DecodeContext, WireType, check_wire_type, decode_varint, encode_key, encode_varint,
            encoded_len_varint, key_len,
        };

        /// Encodes one string field.
        pub fn encode(tag: u32, value: &String, buf: &mut impl BufMut) {
            let b = gostr::to_go_bytes(value);
            encode_key(tag, WireType::LengthDelimited, buf);
            encode_varint(b.len() as u64, buf);
            buf.put_slice(&b);
        }

        /// Decodes one string field (replacing `value`), accepting any bytes.
        pub fn merge(
            wire_type: WireType,
            value: &mut String,
            buf: &mut impl Buf,
            _ctx: DecodeContext,
        ) -> Result<(), DecodeError> {
            check_wire_type(WireType::LengthDelimited, wire_type)?;
            let len = decode_varint(buf)?;
            if len > buf.remaining() as u64 {
                value.clear();
                return Err(DecodeError::new("buffer underflow"));
            }
            let len = usize::try_from(len).map_err(|_| DecodeError::new("buffer underflow"))?;
            let mut bytes = vec![0u8; len];
            buf.copy_to_slice(&mut bytes);
            *value = gostr::from_go_vec(bytes);
            Ok(())
        }

        /// Encodes a repeated string field.
        pub fn encode_repeated(tag: u32, values: &[String], buf: &mut impl BufMut) {
            for value in values {
                encode(tag, value, buf);
            }
        }

        /// Decodes one element of a repeated string field.
        pub fn merge_repeated(
            wire_type: WireType,
            values: &mut Vec<String>,
            buf: &mut impl Buf,
            ctx: DecodeContext,
        ) -> Result<(), DecodeError> {
            check_wire_type(WireType::LengthDelimited, wire_type)?;
            let mut value = String::new();
            merge(wire_type, &mut value, buf, ctx)?;
            values.push(value);
            Ok(())
        }

        /// Encoded length of one string field.
        #[inline]
        #[must_use]
        pub fn encoded_len(tag: u32, value: &String) -> usize {
            let n = gostr::go_len(value);
            key_len(tag) + encoded_len_varint(n as u64) + n
        }

        /// Encoded length of a repeated string field.
        #[inline]
        #[must_use]
        pub fn encoded_len_repeated(tag: u32, values: &[String]) -> usize {
            key_len(tag) * values.len()
                + values
                    .iter()
                    .map(|v| {
                        let n = gostr::go_len(v);
                        encoded_len_varint(n as u64) + n
                    })
                    .sum::<usize>()
        }
    }
}

/// A oneof's decoding as gogoproto generates it: each occurrence of one of
/// its fields replaces the oneof with a fresh value (prost merges an
/// occurrence into a value of the same variant, so a message whose oneof
/// field occurs twice decoded to another value than in Go). The generated
/// code names it in the parent's `#[prost(oneof = "...")]` attribute
/// (`GogoOneof::<m::Sum>`), see [`gogo_oneof!`].
pub struct GogoOneof<T>(core::marker::PhantomData<T>);

/// The `#[derive(Oneof)]` functions [`GogoOneof`] wraps.
pub trait GogoOneofField: Sized {
    /// `Oneof::merge`.
    fn merge_field<B: bytes::Buf>(
        field: &mut Option<Self>,
        tag: u32,
        wire_type: encoding::WireType,
        buf: &mut B,
        ctx: encoding::DecodeContext,
    ) -> Result<(), DecodeError>;
    /// `Oneof::encoded_len`.
    fn field_len(&self) -> usize;
}

impl<T: GogoOneofField> GogoOneof<T> {
    /// Decodes one occurrence into a fresh value that replaces `field`.
    pub fn merge<B: bytes::Buf>(
        field: &mut Option<T>,
        tag: u32,
        wire_type: encoding::WireType,
        buf: &mut B,
        ctx: encoding::DecodeContext,
    ) -> Result<(), DecodeError> {
        let mut fresh = None;
        T::merge_field(&mut fresh, tag, wire_type, buf, ctx)?;
        *field = fresh;
        Ok(())
    }

    /// The oneof's encoded length.
    pub fn encoded_len(v: &T) -> usize {
        v.field_len()
    }
}

/// Implements [`GogoOneofField`] for a `#[derive(Oneof)]` enum (generated
/// next to it by `cargo xtask gen-proto`).
#[macro_export]
macro_rules! gogo_oneof {
    ($e:ident) => {
        impl $crate::GogoOneofField for $e {
            fn merge_field<B: $crate::bytes::Buf>(
                field: &mut ::core::option::Option<Self>,
                tag: u32,
                wire_type: $crate::encoding::WireType,
                buf: &mut B,
                ctx: $crate::encoding::DecodeContext,
            ) -> ::core::result::Result<(), $crate::DecodeError> {
                $e::merge(field, tag, wire_type, buf, ctx)
            }
            fn field_len(&self) -> usize {
                self.encoded_len()
            }
        }
    };
}
