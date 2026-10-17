//! Port of `google.golang.org/protobuf/encoding/protowire` (v1.36.12):
//! the wire primitives `unknownproto` and the ADR-027 check use.
//!
//! Functions return the consumed length, or a negative error code exactly as
//! Go does (`errCodeTruncated` = -1, … `errCodeRecursionDepth` = -6).

/// `protowire.Type`.
pub(crate) type WireType = u8;

pub(crate) const VARINT: WireType = 0;
pub(crate) const FIXED64: WireType = 1;
pub(crate) const BYTES: WireType = 2;
pub(crate) const START_GROUP: WireType = 3;
pub(crate) const END_GROUP: WireType = 4;
pub(crate) const FIXED32: WireType = 5;

pub(crate) const ERR_TRUNCATED: isize = -1;
pub(crate) const ERR_FIELD_NUMBER: isize = -2;
pub(crate) const ERR_OVERFLOW: isize = -3;
pub(crate) const ERR_RESERVED: isize = -4;
pub(crate) const ERR_END_GROUP: isize = -5;
pub(crate) const ERR_RECURSION_DEPTH: isize = -6;

/// `protowire.DefaultRecursionLimit`.
pub(crate) const DEFAULT_RECURSION_LIMIT: i32 = 10_000;

/// `protowire.ParseError` messages (for logs only).
pub(crate) fn parse_error(n: isize) -> &'static str {
    match n {
        // `io.ErrUnexpectedEOF`; the others are protobuf-go's
        // `internal/errors.New` values, prefixed `proto: `.
        ERR_TRUNCATED => "unexpected EOF",
        ERR_FIELD_NUMBER => "proto: invalid field number",
        ERR_OVERFLOW => "proto: variable length integer overflow",
        ERR_RESERVED => "proto: cannot parse reserved wire type",
        ERR_END_GROUP => "proto: mismatching end group marker",
        ERR_RECURSION_DEPTH => "proto: exceeded maximum recursion depth",
        _ => "proto: parse error",
    }
}

/// `ConsumeVarint`: at most 10 bytes, the 10th must be `< 2`.
pub(crate) fn consume_varint(b: &[u8]) -> (u64, isize) {
    let mut v: u64 = 0;
    for i in 0..10 {
        let Some(&byte) = b.get(i) else {
            return (0, ERR_TRUNCATED);
        };
        let y = u64::from(byte);
        if i == 9 {
            if y < 2 {
                return (v + (y << 63), 10);
            }
            return (0, ERR_OVERFLOW);
        }
        if y < 0x80 {
            return (v + (y << (7 * i)), i as isize + 1);
        }
        v += (y & 0x7f) << (7 * i);
    }
    (0, ERR_OVERFLOW)
}

/// `DecodeTag`: field number `-1` if it overflows int32.
pub(crate) fn decode_tag(x: u64) -> (i64, WireType) {
    if x >> 3 > i32::MAX as u64 {
        return (-1, 0);
    }
    ((x >> 3) as i64, (x & 7) as WireType)
}

/// `ConsumeTag`.
pub(crate) fn consume_tag(b: &[u8]) -> (i64, WireType, isize) {
    let (v, n) = consume_varint(b);
    if n < 0 {
        return (0, 0, n);
    }
    let (num, typ) = decode_tag(v);
    if num < 1 {
        return (0, 0, ERR_FIELD_NUMBER);
    }
    (num, typ, n)
}

/// `ConsumeBytes`: the total length of a length-prefixed value.
pub(crate) fn consume_bytes(b: &[u8]) -> (&[u8], isize) {
    let (m, n) = consume_varint(b);
    if n < 0 {
        return (&[], n);
    }
    let rest = &b[n as usize..];
    if m > rest.len() as u64 {
        return (&[], ERR_TRUNCATED);
    }
    let m = m as usize;
    (&rest[..m], n + m as isize)
}

/// `ConsumeFieldValue`.
pub(crate) fn consume_field_value(num: i64, typ: WireType, b: &[u8]) -> isize {
    consume_field_value_d(num, typ, b, DEFAULT_RECURSION_LIMIT)
}

fn consume_field_value_d(num: i64, typ: WireType, mut b: &[u8], depth: i32) -> isize {
    match typ {
        VARINT => consume_varint(b).1,
        FIXED32 => {
            if b.len() < 4 {
                ERR_TRUNCATED
            } else {
                4
            }
        }
        FIXED64 => {
            if b.len() < 8 {
                ERR_TRUNCATED
            } else {
                8
            }
        }
        BYTES => consume_bytes(b).1,
        START_GROUP => {
            if depth < 0 {
                return ERR_RECURSION_DEPTH;
            }
            let n0 = b.len();
            loop {
                let (num2, typ2, n) = consume_tag(b);
                if n < 0 {
                    return n;
                }
                b = &b[n as usize..];
                if typ2 == END_GROUP {
                    if num != num2 {
                        return ERR_END_GROUP;
                    }
                    return (n0 - b.len()) as isize;
                }
                let n = consume_field_value_d(num2, typ2, b, depth - 1);
                if n < 0 {
                    return n;
                }
                b = &b[n as usize..];
            }
        }
        END_GROUP => ERR_END_GROUP,
        _ => ERR_RESERVED,
    }
}

/// `ConsumeField`: tag plus value.
pub(crate) fn consume_field(b: &[u8]) -> (i64, WireType, isize) {
    let (num, typ, n) = consume_tag(b);
    if n < 0 {
        return (0, 0, n);
    }
    let m = consume_field_value(num, typ, &b[n as usize..]);
    if m < 0 {
        return (0, 0, m);
    }
    (num, typ, n + m)
}

/// `SizeVarint`.
pub(crate) fn size_varint(v: u64) -> usize {
    let bits = 64 - (v | 1).leading_zeros() as usize;
    bits.div_ceil(7)
}

/// `AppendVarint`.
pub(crate) fn append_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// Appends a tag.
pub(crate) fn append_tag(out: &mut Vec<u8>, num: u32, typ: WireType) {
    append_varint(out, (u64::from(num) << 3) | u64::from(typ));
}

/// Appends a length-delimited field.
pub(crate) fn append_bytes_field(out: &mut Vec<u8>, num: u32, v: &[u8]) {
    append_tag(out, num, BYTES);
    append_varint(out, v.len() as u64);
    out.extend_from_slice(v);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints() {
        assert_eq!(consume_varint(&[0x01]), (1, 1));
        assert_eq!(consume_varint(&[0x80, 0x01]), (128, 2));
        assert_eq!(consume_varint(&[0x80]), (0, ERR_TRUNCATED));
        let max = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01];
        assert_eq!(consume_varint(&max), (u64::MAX, 10));
        let over = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02];
        assert_eq!(consume_varint(&over).1, ERR_OVERFLOW);
        assert_eq!(size_varint(0), 1);
        assert_eq!(size_varint(127), 1);
        assert_eq!(size_varint(128), 2);
        assert_eq!(size_varint(u64::MAX), 10);
    }

    #[test]
    fn groups() {
        // field 1 start group, field 2 varint 5, field 1 end group.
        let b = [0x08 | 3, 0x10, 0x05, 0x08 | 4];
        assert_eq!(consume_field(&b), (1, START_GROUP, 4));
        let bad = [0x08 | 3, 0x10 | 4];
        assert_eq!(consume_field(&bad).2, ERR_END_GROUP);
    }
}
