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
