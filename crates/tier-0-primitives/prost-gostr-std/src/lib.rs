//! [`prost_gostr`] for schemas whose `google.protobuf.Timestamp` and
//! `Duration` fields are all gogoproto `stdtime` / `stdduration` (the
//! celestia-core protos: 38 of 38 such fields).
//!
//! gogoproto unmarshals such a field with `StdTimeUnmarshal` /
//! `StdDurationUnmarshal` into a fresh value, so a field that occurs twice
//! takes the last occurrence whole; prost merges the occurrences (a `nanos`
//! of the first survives a second without one). A vote or commit signature
//! re-encoded that way verifies in one and not in the other. Here
//! [`encoding::message::merge`] clears a `Timestamp` or `Duration` before
//! each occurrence and then validates it as `StdTimeUnmarshal` /
//! `StdDurationUnmarshal` do (a vote with a year-10000 timestamp does not
//! decode in Go), and [`encoding::skip_field`] skips unknown fields like
//! the gogoproto generated `skip*` functions (an end group closes any open
//! group, whatever its field number), and varint scalars decode like the
//! generated `Unmarshal` (up to ten bytes, bits past 64 dropped, then the
//! low bits of the field's type; prost rejects such a varint, so a block Go
//! accepts would not decode). Everything else is [`prost_gostr`].

pub use prost_gostr::*;

/// [`prost_gostr::encoding`] with stdtime / stdduration message merging.
#[doc(hidden)]
pub mod encoding {
    pub use prost_gostr::encoding::*;

    use prost_gostr::DecodeError;
    use prost_gostr::bytes::Buf;

    /// The gogoproto generated `skip<File>`: skips the unknown field whose
    /// key was just read. Field numbers inside a group are not checked (an
    /// end group closes the innermost open group, whatever its number, and
    /// field number 0 is skipped too), varints are skipped by their
    /// continuation bits only, and a length is not checked against the
    /// buffer until the end (an overrun is an error either way).
    pub fn skip_field(
        wire_type: WireType,
        _tag: u32,
        buf: &mut impl Buf,
        _ctx: DecodeContext,
    ) -> Result<(), DecodeError> {
        // `true` for 64 continuation bits (`ErrIntOverflow`).
        fn skip_varint(buf: &mut impl Buf) -> Result<u64, DecodeError> {
            let mut v = 0u64;
            let mut shift = 0u32;
            loop {
                if shift >= 64 {
                    return Err(DecodeError::new("proto: integer overflow"));
                }
                if !buf.has_remaining() {
                    return Err(DecodeError::new("unexpected EOF"));
                }
                let b = buf.get_u8();
                v |= u64::from(b & 0x7f).wrapping_shl(shift);
                if b < 0x80 {
                    return Ok(v);
                }
                shift += 7;
            }
        }
        fn advance(buf: &mut impl Buf, n: u64) -> Result<(), DecodeError> {
            if (buf.remaining() as u64) < n {
                return Err(DecodeError::new("unexpected EOF"));
            }
            #[allow(clippy::cast_possible_truncation)]
            buf.advance(n as usize);
            Ok(())
        }
        let mut depth = 0u64;
        let mut wt = wire_type as u64;
        loop {
            match wt {
                0 => {
                    skip_varint(buf)?;
                }
                1 => advance(buf, 8)?,
                2 => {
                    // Go reads the length as an `int`: one that wraps
                    // negative is `ErrInvalidLength`.
                    let n = skip_varint(buf)?;
                    if i64::try_from(n).is_err() {
                        return Err(DecodeError::new(
                            "proto: negative length found during unmarshaling",
                        ));
                    }
                    advance(buf, n)?;
                }
                3 => depth += 1,
                4 => {
                    if depth == 0 {
                        return Err(DecodeError::new("proto: unexpected end of group"));
                    }
                    depth -= 1;
                }
                5 => advance(buf, 4)?,
                w => return Err(DecodeError::new(format!("proto: illegal wireType {w}"))),
            }
            if depth == 0 {
                return Ok(());
            }
            wt = skip_varint(buf)? & 7;
        }
    }

    /// gogoproto's varint loop: up to ten bytes (`ErrIntOverflow` past
    /// shift 63), each adding its 7 bits at its shift (bits past 64 are
    /// dropped, not an error as in prost).
    pub fn gogo_varint(buf: &mut impl Buf) -> Result<u64, DecodeError> {
        let mut v = 0u64;
        let mut shift = 0u32;
        loop {
            if shift >= 64 {
                return Err(DecodeError::new("proto: integer overflow"));
            }
            if !buf.has_remaining() {
                return Err(DecodeError::new("unexpected EOF"));
            }
            let b = buf.get_u8();
            v |= u64::from(b & 0x7f).wrapping_shl(shift);
            if b < 0x80 {
                return Ok(v);
            }
            shift += 7;
        }
    }

    macro_rules! gogo_scalar {
        ($m:ident, $ty:ty, $conv:expr) => {
            /// gogoproto's decoding of this varint scalar.
            pub mod $m {
                pub use prost_gostr::encoding::$m::*;

                use super::gogo_varint;
                use prost_gostr::DecodeError;
                use prost_gostr::bytes::Buf;
                use prost_gostr::encoding::{DecodeContext, WireType, check_wire_type};

                /// One occurrence (the last one wins).
                pub fn merge(
                    wire_type: WireType,
                    value: &mut $ty,
                    buf: &mut impl Buf,
                    _ctx: DecodeContext,
                ) -> Result<(), DecodeError> {
                    check_wire_type(WireType::Varint, wire_type)?;
                    *value = $conv(gogo_varint(buf)?);
                    Ok(())
                }

                /// One element, or a packed run whose last element may read
                /// past the run's end (gogoproto bounds it by the message
                /// only, and parsing continues after it).
                pub fn merge_repeated(
                    wire_type: WireType,
                    values: &mut Vec<$ty>,
                    buf: &mut impl Buf,
                    _ctx: DecodeContext,
                ) -> Result<(), DecodeError> {
                    if wire_type == WireType::LengthDelimited {
                        let len = gogo_varint(buf)?;
                        if i64::try_from(len).is_err() {
                            return Err(DecodeError::new(
                                "proto: negative length found during unmarshaling",
                            ));
                        }
                        let rem = buf.remaining() as u64;
                        if len > rem {
                            return Err(DecodeError::new("unexpected EOF"));
                        }
                        #[allow(clippy::cast_possible_truncation)]
                        let end = (rem - len) as usize;
                        while buf.remaining() > end {
                            values.push($conv(gogo_varint(buf)?));
                        }
                        return Ok(());
                    }
                    check_wire_type(WireType::Varint, wire_type)?;
                    values.push($conv(gogo_varint(buf)?));
                    Ok(())
                }
            }
        };
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    mod conv {
        pub(super) fn i32(v: u64) -> i32 {
            v as i32
        }
        pub(super) fn i64(v: u64) -> i64 {
            v as i64
        }
        pub(super) fn u32(v: u64) -> u32 {
            v as u32
        }
        pub(super) fn u64(v: u64) -> u64 {
            v
        }
        pub(super) fn bool(v: u64) -> bool {
            v != 0
        }
    }

    gogo_scalar!(int32, i32, super::conv::i32);
    gogo_scalar!(int64, i64, super::conv::i64);
    gogo_scalar!(uint32, u32, super::conv::u32);
    gogo_scalar!(uint64, u64, super::conv::u64);
    gogo_scalar!(bool, bool, super::conv::bool);

    /// `buf` with a length prefix read as gogoproto does (ten-byte varints
    /// drop their bits past 64) and re-encoded canonically, for the prost
    /// `bytes` / `string` decoders.
    fn canon_len<B: Buf>(buf: &mut B) -> Result<impl Buf + '_, DecodeError> {
        let len = gogo_len(buf)?;
        let mut prefix = Vec::with_capacity(10);
        encode_varint(len as u64, &mut prefix);
        Ok(prost_gostr::bytes::Bytes::from(prefix).chain(buf.take(len)))
    }

    /// `bytes` fields with gogoproto's length prefix.
    pub mod bytes {
        pub use prost_gostr::encoding::bytes::*;

        use prost_gostr::DecodeError;
        use prost_gostr::bytes::Buf;
        use prost_gostr::encoding::{BytesAdapter, DecodeContext, WireType, check_wire_type};

        /// One occurrence (the last one wins).
        pub fn merge(
            wire_type: WireType,
            value: &mut impl BytesAdapter,
            buf: &mut impl Buf,
            ctx: DecodeContext,
        ) -> Result<(), DecodeError> {
            check_wire_type(WireType::LengthDelimited, wire_type)?;
            prost_gostr::encoding::bytes::merge(wire_type, value, &mut super::canon_len(buf)?, ctx)
        }

        /// One element of a repeated field.
        pub fn merge_repeated<T: BytesAdapter + Default>(
            wire_type: WireType,
            values: &mut Vec<T>,
            buf: &mut impl Buf,
            ctx: DecodeContext,
        ) -> Result<(), DecodeError> {
            let mut v = T::default();
            merge(wire_type, &mut v, buf, ctx)?;
            values.push(v);
            Ok(())
        }
    }

    /// `string` fields (Go form, [`prost_gostr`]) with gogoproto's length
    /// prefix.
    #[allow(clippy::ptr_arg)]
    pub mod string {
        pub use prost_gostr::encoding::string::*;

        use prost_gostr::DecodeError;
        use prost_gostr::bytes::Buf;
        use prost_gostr::encoding::{DecodeContext, WireType, check_wire_type};

        /// One occurrence (the last one wins).
        pub fn merge(
            wire_type: WireType,
            value: &mut String,
            buf: &mut impl Buf,
            ctx: DecodeContext,
        ) -> Result<(), DecodeError> {
            check_wire_type(WireType::LengthDelimited, wire_type)?;
            prost_gostr::encoding::string::merge(wire_type, value, &mut super::canon_len(buf)?, ctx)
        }

        /// One element of a repeated field.
        pub fn merge_repeated(
            wire_type: WireType,
            values: &mut Vec<String>,
            buf: &mut impl Buf,
            ctx: DecodeContext,
        ) -> Result<(), DecodeError> {
            let mut v = String::new();
            merge(wire_type, &mut v, buf, ctx)?;
            values.push(v);
            Ok(())
        }
    }

    /// The gogoproto generated `Unmarshal` loop over a message's bytes
    /// (all of `buf`): keys are gogoproto varints whose field number is
    /// `int32(key >> 3)` (must be positive), an end group is an error,
    /// known fields go to the message, unknown ones to [`skip_field`].
    pub fn gogo_merge_fields<M: prost_gostr::Message>(
        msg: &mut M,
        buf: &mut impl Buf,
        ctx: &DecodeContext,
    ) -> Result<(), DecodeError> {
        while buf.has_remaining() {
            let key = gogo_varint(buf)?;
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            let num = (key >> 3) as u32 as i32;
            let wt = key & 7;
            if wt == 4 {
                return Err(DecodeError::new("proto: wiretype end group for non-group"));
            }
            if num <= 0 {
                return Err(DecodeError::new(format!(
                    "proto: illegal tag {num} (wire type {key})"
                )));
            }
            #[allow(clippy::cast_possible_truncation)]
            let Ok(wire_type) = WireType::try_from(wt) else {
                return Err(DecodeError::new(format!("proto: illegal wireType {wt}")));
            };
            #[allow(clippy::cast_sign_loss)]
            msg.merge_field(num as u32, wire_type, buf, ctx.clone())?;
        }
        Ok(())
    }

    /// A gogoproto length prefix (`ErrInvalidLength` when it wraps
    /// negative, `io.ErrUnexpectedEOF` past the buffer).
    fn gogo_len(buf: &mut impl Buf) -> Result<usize, DecodeError> {
        let n = gogo_varint(buf)?;
        if i64::try_from(n).is_err() {
            return Err(DecodeError::new(
                "proto: negative length found during unmarshaling",
            ));
        }
        if n > buf.remaining() as u64 {
            return Err(DecodeError::new("unexpected EOF"));
        }
        #[allow(clippy::cast_possible_truncation)]
        Ok(n as usize)
    }

    /// Decodes `bz` as the gogoproto generated `Unmarshal` does (the
    /// top-level counterpart of [`message::merge`]).
    pub fn gogo_decode<M: prost_gostr::Message + Default>(mut bz: &[u8]) -> Result<M, DecodeError> {
        let mut m = M::default();
        gogo_merge_fields(&mut m, &mut bz, &DecodeContext::default())?;
        Ok(m)
    }

    /// `message` fields: decoded with gogoproto's loop, each one bounded by
    /// its length; a `Timestamp` or `Duration` occurrence replaces the
    /// field (gogoproto `stdtime` / `stdduration`) and is validated.
    pub mod message {
        pub use prost_gostr::encoding::message::*;

        use std::any::Any;

        use prost_gostr::bytes::Buf;
        use prost_gostr::encoding::{DecodeContext, WireType, check_wire_type};
        use prost_gostr::{DecodeError, Message};

        /// Merges one occurrence of a message field into `msg`.
        pub fn merge<M, B>(
            wire_type: WireType,
            msg: &mut M,
            buf: &mut B,
            ctx: DecodeContext,
        ) -> Result<(), DecodeError>
        where
            M: Message + 'static,
            B: Buf,
        {
            check_wire_type(WireType::LengthDelimited, wire_type)?;
            let len = super::gogo_len(buf)?;
            let mut sub = buf.take(len);
            let any = msg as &mut dyn Any;
            if let Some(ts) = any.downcast_mut::<pbjson_types::Timestamp>() {
                let (s, n) = seconds_nanos(&mut sub)?;
                *ts = pbjson_types::Timestamp {
                    seconds: s,
                    nanos: n,
                };
                return validate_timestamp(ts);
            }
            if let Some(d) = any.downcast_mut::<pbjson_types::Duration>() {
                let (s, n) = seconds_nanos(&mut sub)?;
                *d = pbjson_types::Duration {
                    seconds: s,
                    nanos: n,
                };
                return validate_duration(d);
            }
            super::gogo_merge_fields(msg, &mut sub, &ctx)
        }

        /// One element of a repeated message field (gogoproto's loop, as
        /// [`merge`]).
        pub fn merge_repeated<M, B>(
            wire_type: WireType,
            messages: &mut Vec<M>,
            buf: &mut B,
            ctx: DecodeContext,
        ) -> Result<(), DecodeError>
        where
            M: Message + Default + 'static,
            B: Buf,
        {
            check_wire_type(WireType::LengthDelimited, wire_type)?;
            let mut m = M::default();
            merge(wire_type, &mut m, buf, ctx)?;
            messages.push(m);
            Ok(())
        }

        /// gogoproto `Timestamp.Unmarshal` / `Duration.Unmarshal` of a
        /// fresh value: `seconds` (1, int64) and `nanos` (2, int32).
        fn seconds_nanos(buf: &mut impl Buf) -> Result<(i64, i32), DecodeError> {
            let (mut s, mut n) = (0i64, 0i32);
            while buf.has_remaining() {
                let key = super::gogo_varint(buf)?;
                #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                let num = (key >> 3) as u32 as i32;
                let wt = key & 7;
                if wt == 4 {
                    return Err(DecodeError::new("proto: wiretype end group for non-group"));
                }
                if num <= 0 {
                    return Err(DecodeError::new(format!(
                        "proto: illegal tag {num} (wire type {key})"
                    )));
                }
                match num {
                    1 | 2 => {
                        if wt != 0 {
                            return Err(DecodeError::new(format!(
                                "proto: wrong wireType = {wt} for field"
                            )));
                        }
                        let v = super::gogo_varint(buf)?;
                        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                        if num == 1 {
                            s = v as i64;
                        } else {
                            n = v as i32;
                        }
                    }
                    _ => {
                        let Ok(w) = WireType::try_from(wt) else {
                            return Err(DecodeError::new(format!("proto: illegal wireType {wt}")));
                        };
                        super::skip_field(w, 0, buf, DecodeContext::default())?;
                    }
                }
            }
            Ok((s, n))
        }
        /// gogoproto `types.TimestampFromProto` (`StdTimeUnmarshal`).
        fn validate_timestamp(ts: &pbjson_types::Timestamp) -> Result<(), DecodeError> {
            const MIN: i64 = -62_135_596_800;
            const MAX: i64 = 253_402_300_800;
            if ts.seconds < MIN {
                return Err(DecodeError::new("timestamp: before 0001-01-01"));
            }
            if ts.seconds >= MAX {
                return Err(DecodeError::new("timestamp: after 10000-01-01"));
            }
            if !(0..1_000_000_000).contains(&ts.nanos) {
                return Err(DecodeError::new("timestamp: nanos not in range [0, 1e9)"));
            }
            Ok(())
        }

        /// gogoproto `types.DurationFromProto` (`StdDurationUnmarshal`):
        /// the protobuf range, then a `time.Duration` (int64 nanoseconds).
        fn validate_duration(d: &pbjson_types::Duration) -> Result<(), DecodeError> {
            const MAX: i64 = 315_576_000_000;
            if !(-MAX..=MAX).contains(&d.seconds) {
                return Err(DecodeError::new("duration: seconds out of range"));
            }
            if d.nanos <= -1_000_000_000 || d.nanos >= 1_000_000_000 {
                return Err(DecodeError::new("duration: nanos out of range"));
            }
            if (d.seconds < 0 && d.nanos > 0) || (d.seconds > 0 && d.nanos < 0) {
                return Err(DecodeError::new(
                    "duration: seconds and nanos have different signs",
                ));
            }
            let out_of_range = || DecodeError::new("duration: out of range for time.Duration");
            let ns = d
                .seconds
                .checked_mul(1_000_000_000)
                .ok_or_else(out_of_range)?;
            if d.nanos != 0 {
                let total = ns.wrapping_add(i64::from(d.nanos));
                if (total < 0) != (d.nanos < 0) {
                    return Err(out_of_range());
                }
            }
            Ok(())
        }
    }
}
