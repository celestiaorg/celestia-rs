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
        let n = u32::try_from(tag >> 3).ok()?;
        if n == 0 || n > (1 << 29) - 1 {
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

/// The check `anyutil.Unpack` makes before Go's `GetMsgV1Signers` reads
/// the signers of a message (top-level, `MsgExec` inner, gov proposal and
/// ICA messages): a type decoded with `dynamicpb` ([`getsigners_schema`])
/// must hold valid UTF-8 in its proto3 strings and varints protobuf-go can
/// parse; pulsar and unregistered types pass. `Err` is Go's error text.
///
/// [`getsigners_schema`]: crate::getsigners_schema
pub fn unpack_check(type_url: &str, value: &[u8]) -> Result<(), String> {
    use crate::getsigners_schema::DYNAMIC_ROOTS;
    if let Ok(r) = DYNAMIC_ROOTS.binary_search_by(|(u, _)| (*u).cmp(type_url)) {
        dynamicpb_check(DYNAMIC_ROOTS[r].1, value, 0)
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
}
