//! Message signers from the `cosmos.msg.v1.signer` proto option
//! (`codec.GetMsgV2Signers`).

/// A signer field value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signer {
    /// A bech32 address string (account or validator prefix).
    Str(String),
    /// Raw address bytes.
    Raw(Vec<u8>),
}

include!("gen/signers.rs");

/// protobuf-go `protowire.ConsumeVarint`: at most 10 bytes, the 10th at
/// most 1 (gogoproto accepts any 10th byte).
fn strict_varint(b: &mut &[u8]) -> Option<u64> {
    let mut v = 0u64;
    for i in 0..10 {
        let (&x, rest) = b.split_first()?;
        *b = rest;
        if i == 9 && x > 1 {
            return None;
        }
        v |= u64::from(x & 0x7f) << (7 * i);
        if x < 0x80 {
            return Some(v);
        }
    }
    None
}

/// What protobuf-go `dynamicpb` decoding rejects in bytes gogoproto
/// accepted, for message `msg` ([`crate::getsigners_schema`]): invalid
/// UTF-8 in a proto3 string field and varints protobuf-go cannot parse
/// (also inside packed repeated scalars), depth first in wire order.
fn dynamicpb_check(msg: &str, mut b: &[u8], depth: usize) -> Result<(), String> {
    use crate::getsigners_schema::{MESSAGE_FIELDS, PACKED_FIELDS, STRING_FIELDS};
    const WIRE: &str = "proto: cannot parse invalid wire-format data";
    // protobuf-go's recursion limit.
    if depth > 10_000 {
        return Err("exceeded maximum recursion depth".into());
    }
    while !b.is_empty() {
        let tag = strict_varint(&mut b).ok_or(WIRE)?;
        let num = u32::try_from(tag >> 3).map_err(|_| WIRE)?;
        if num == 0 || num > (1 << 29) - 1 {
            return Err(WIRE.into());
        }
        match tag & 7 {
            0 => {
                strict_varint(&mut b).ok_or(WIRE)?;
            }
            1 | 5 => {
                let n = if tag & 7 == 1 { 8 } else { 4 };
                b = b.get(n..).ok_or(WIRE)?;
            }
            2 => {
                let len = strict_varint(&mut b)
                    .and_then(|l| usize::try_from(l).ok())
                    .ok_or(WIRE)?;
                let payload = b.get(..len).ok_or(WIRE)?;
                b = &b[len..];
                let key = |&(m, n, _): &(&str, u32, &str)| (m, n).cmp(&(msg, num));
                if let Ok(i) = STRING_FIELDS.binary_search_by(key) {
                    if std::str::from_utf8(payload).is_err() {
                        return Err(format!(
                            "proto: field {msg}.{} contains invalid UTF-8",
                            STRING_FIELDS[i].2
                        ));
                    }
                } else if let Ok(i) = MESSAGE_FIELDS.binary_search_by(key) {
                    dynamicpb_check(MESSAGE_FIELDS[i].2, payload, depth + 1)?;
                } else if let Ok(i) =
                    PACKED_FIELDS.binary_search_by(|&(m, n, _)| (m, n).cmp(&(msg, num)))
                {
                    match PACKED_FIELDS[i].2 {
                        0 => {
                            let mut p = payload;
                            while !p.is_empty() {
                                strict_varint(&mut p).ok_or(WIRE)?;
                            }
                        }
                        size if payload.len() % size != 0 => return Err(WIRE.into()),
                        _ => {}
                    }
                }
            }
            3 => skip_group(&mut b, num, depth).ok_or(WIRE)?,
            _ => return Err(WIRE.into()),
        }
    }
    Ok(())
}

/// protobuf-go `ConsumeFieldValue` of a group's contents (after its start
/// tag) through the matching end tag.
fn skip_group(b: &mut &[u8], num: u32, depth: usize) -> Option<()> {
    if depth > 10_000 {
        return None;
    }
    loop {
        let tag = strict_varint(b)?;
        // protowire `ConsumeTag` (inside a group): field numbers up to
        // `math.MaxInt32`, unlike the 2^29 - 1 of a message's own fields.
        let n = u32::try_from(tag >> 3).ok()?;
        if n == 0 || n > i32::MAX as u32 {
            return None;
        }
        match tag & 7 {
            0 => {
                strict_varint(b)?;
            }
            1 => *b = b.get(8..)?,
            5 => *b = b.get(4..)?,
            2 => {
                let len = usize::try_from(strict_varint(b)?).ok()?;
                *b = b.get(len..)?;
            }
            3 => skip_group(b, n, depth + 1)?,
            4 if n == num => return Some(()),
            _ => return None,
        }
    }
}

/// gogoproto-style varint (the pulsar generated unmarshal): up to 64 bits
/// of shift, any 10th byte.
fn lenient_varint(b: &mut &[u8]) -> Option<u64> {
    let mut v = 0u64;
    let mut shift = 0u32;
    loop {
        if shift >= 64 {
            return None;
        }
        let (&x, rest) = b.split_first()?;
        *b = rest;
        v |= u64::from(x & 0x7f).checked_shl(shift).unwrap_or(0);
        if x < 0x80 {
            return Some(v);
        }
        shift += 7;
    }
}

/// What the generated (pulsar) unmarshal of a `protoregistry.GlobalTypes`
/// type rejects in bytes gogoproto accepted. The pulsar code itself reads
/// like gogoproto (field numbers cut to `int32`, unknown fields skipped by
/// `runtime.Skip`), but it decodes every embedded message with
/// `options.Unmarshal`: a nested pulsar type the same way, a well-known
/// type (`google.protobuf.Timestamp`, `Duration`, `Any`) with protobuf-go's
/// table-driven decoder, which rejects field numbers past 2^29 - 1, varints
/// with a 10th byte above 1, groups whose end tag does not match and
/// invalid UTF-8 in `Any.type_url` ([`dynamicpb_check`]'s rules). Such a
/// well-known type keeps unknown fields in gogoproto (`XXX_unrecognized`),
/// so a non-critical unknown field numbered 2^29..2^31 in a grant's
/// expiration passes the tx decoder and fails `GetSigners`.
fn pulsar_check(msg: &str, mut b: &[u8], depth: usize) -> Result<(), String> {
    const WIRE: &str = "proto: cannot parse invalid wire-format data";
    if depth > 10_000 {
        return Err("exceeded maximum recursion depth".into());
    }
    let info = crate::wire_table::message(msg);
    while !b.is_empty() {
        let tag = lenient_varint(&mut b).ok_or(WIRE)?;
        let num = (tag >> 3) as u32 as i32;
        let wt = tag & 7;
        if wt == 4 || num <= 0 {
            return Err(WIRE.into());
        }
        let child = info
            .and_then(|m| m.field(num as u32))
            .filter(|f| f.ty == crate::wire_table::ty::MESSAGE)
            .map(|f| f.type_name);
        match wt {
            0 => {
                lenient_varint(&mut b).ok_or(WIRE)?;
            }
            1 | 5 => {
                let n = if wt == 1 { 8 } else { 4 };
                b = b.get(n..).ok_or(WIRE)?;
            }
            2 => {
                let len = lenient_varint(&mut b)
                    .and_then(|l| usize::try_from(l).ok())
                    .ok_or(WIRE)?;
                let payload = b.get(..len).ok_or(WIRE)?;
                b = &b[len..];
                match child {
                    Some(c) if c.starts_with("google.protobuf.") => {
                        dynamicpb_check(c, payload, depth + 1)?;
                    }
                    Some(c) => pulsar_check(c, payload, depth + 1)?,
                    None => {}
                }
            }
            3 => {
                // runtime.Skip: groups counted by depth, end numbers not
                // matched.
                let mut d = 1usize;
                while d > 0 {
                    let t = lenient_varint(&mut b).ok_or(WIRE)?;
                    match t & 7 {
                        0 => {
                            lenient_varint(&mut b).ok_or(WIRE)?;
                        }
                        1 => b = b.get(8..).ok_or(WIRE)?,
                        5 => b = b.get(4..).ok_or(WIRE)?,
                        2 => {
                            let len = lenient_varint(&mut b)
                                .and_then(|l| usize::try_from(l).ok())
                                .ok_or(WIRE)?;
                            b = b.get(len..).ok_or(WIRE)?;
                        }
                        3 => d += 1,
                        4 => d -= 1,
                        _ => return Err(WIRE.into()),
                    }
                }
            }
            _ => return Err(WIRE.into()),
        }
    }
    Ok(())
}

/// The check `anyutil.Unpack` makes before Go's `GetMsgV1Signers` reads
/// the signers of a message (top-level, `MsgExec` inner, gov proposal and
/// ICA messages): a type decoded with `dynamicpb` ([`getsigners_schema`])
/// must hold valid UTF-8 in its proto3 strings and varints protobuf-go can
/// parse; a pulsar type must hold well-known-type fields protobuf-go can
/// parse ([`pulsar_check`]); unregistered types pass. `Err` is Go's error
/// text.
///
/// [`getsigners_schema`]: crate::getsigners_schema
pub fn unpack_check(type_url: &str, value: &[u8]) -> Result<(), String> {
    use crate::getsigners_schema::{DYNAMIC_ROOTS, PROTOV2_TYPES};
    if let Ok(r) = DYNAMIC_ROOTS.binary_search_by(|(u, _)| (*u).cmp(type_url)) {
        dynamicpb_check(DYNAMIC_ROOTS[r].1, value, 0)
            .map_err(|e| format!("cannot unmarshal msg {type_url}: {e}"))?;
    } else if PROTOV2_TYPES.binary_search(&type_url).is_ok() {
        pulsar_check(type_url.trim_start_matches('/'), value, 0)
            .map_err(|e| format!("cannot unmarshal msg {type_url}: {e}"))?;
    }
    Ok(())
}

/// Go's `codec.GetMsgV1Signers` (re-packed message, `anyutil.Unpack`, then
/// the signer option): [`unpack_check`] before [`get_signers`].
pub fn get_signers_go(type_url: &str, bytes: &[u8]) -> Option<Result<Vec<Signer>, String>> {
    if let Err(e) = unpack_check(type_url, bytes) {
        return Some(Err(e));
    }
    get_signers(type_url, bytes).map(|r| r.map_err(|e| e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    /// A pulsar type's nested `google.protobuf.Timestamp` is decoded by
    /// protobuf-go's table-driven decoder: a (non-critical, so past the tx
    /// decoder) unknown field numbered 2^31 - 1 fails `GetSigners`; the
    /// same field in the pulsar message itself is skipped.
    #[test]
    fn pulsar_nested_well_known_types_are_strict() {
        fn varint(mut v: u64, out: &mut Vec<u8>) {
            while v >= 0x80 {
                out.push((v as u8) | 0x80);
                v >>= 7;
            }
            out.push(v as u8);
        }
        fn bytes_field(n: u64, v: &[u8]) -> Vec<u8> {
            let mut o = Vec::new();
            varint(n << 3 | 2, &mut o);
            varint(v.len() as u64, &mut o);
            o.extend_from_slice(v);
            o
        }
        let mut unknown = Vec::new();
        varint(((1u64 << 31) - 1) << 3, &mut unknown);
        unknown.push(1);
        let url = "/cosmos.authz.v1beta1.MsgGrant";
        let grant = |exp: &[u8]| {
            let g = [bytes_field(2, exp)].concat();
            [
                bytes_field(1, b"g"),
                bytes_field(2, b"h"),
                bytes_field(3, &g),
            ]
            .concat()
        };
        assert!(unpack_check(url, &grant(&[0x08, 0x01])).is_ok());
        let bad = grant(&[[0x08, 0x01].as_slice(), &unknown].concat());
        assert_eq!(
            unpack_check(url, &bad).unwrap_err(),
            "cannot unmarshal msg /cosmos.authz.v1beta1.MsgGrant: proto: cannot parse invalid wire-format data"
        );
        let top = [grant(&[0x08, 0x01]), unknown].concat();
        assert!(unpack_check(url, &top).is_ok());
    }

    #[test]
    fn bank_send_and_multisend() {
        let m = crate::cosmos::bank::v1beta1::MsgSend {
            from_address: "a".into(),
            ..Default::default()
        };
        let s = get_signers("/cosmos.bank.v1beta1.MsgSend", &m.encode_to_vec())
            .unwrap()
            .unwrap();
        assert_eq!(s, vec![Signer::Str("a".into())]);
        let mm = crate::cosmos::bank::v1beta1::MsgMultiSend {
            inputs: vec![crate::cosmos::bank::v1beta1::Input {
                address: "x".into(),
                coins: vec![],
            }],
            outputs: vec![],
        };
        let s = get_signers("/cosmos.bank.v1beta1.MsgMultiSend", &mm.encode_to_vec())
            .unwrap()
            .unwrap();
        assert_eq!(s, vec![Signer::Str("x".into())]);
        assert!(get_signers("/cosmos.bank.v1beta1.Params", &[]).is_none());
    }

    /// protobuf-go's `ConsumeFieldValue` reads a group's inner tags with
    /// `ConsumeTag` (field numbers up to 2^31 - 1), not the 2^29 - 1 limit
    /// of a message's own fields.
    #[test]
    fn group_inner_fields_up_to_max_int32() {
        let url = "/celestia.blob.v1.MsgPayForBlobs";
        // signer, then non-critical field 1024 as a group holding `inner`.
        let msg =
            |inner: &[u8]| [&[0x0a, 0x01, b's', 0x83, 0x40][..], inner, &[0x84, 0x40]].concat();
        // Field 2^29, varint 0.
        assert!(unpack_check(url, &msg(&[0x80, 0x80, 0x80, 0x80, 0x10, 0x00])).is_ok());
        // Field 2^31 - 1 is the largest.
        assert!(unpack_check(url, &msg(&[0xf8, 0xff, 0xff, 0xff, 0x3f, 0x00])).is_ok());
        // Field 2^31 overflows int32.
        assert!(unpack_check(url, &msg(&[0x80, 0x80, 0x80, 0x80, 0x40, 0x00])).is_err());
    }
}
