//! celestia-app v11's encoding rule for txs (`canonicalTxDecoder` in
//! `app/encoding/canonical_tx_decoder.go`), applied from app version 11.
//!
//! A tx decodes only if every message in it (`TxRaw`, `TxBody`, `AuthInfo`
//! and the message in every `Any`, at any depth) uses an encoding that all
//! protobuf decoders read the same way: no unknown fields (non-critical ones
//! included); fields in ascending order and not repeated when singular; one
//! member per oneof; minimal varints, tags and length prefixes; the wire type
//! the schema expects (packed versus unpacked repeated scalars included);
//! integers in range for their type; valid UTF-8 strings; and `math.Int` /
//! `LegacyDec` values present when not nullable and in canonical decimal
//! form. Proto3 defaults written out and empty messages left out stay valid.
//!
//! Error texts match Go's (they reach the tx log only).

use celestia_proto::wire_table::{FieldInfo, MessageInfo, ty};

use crate::gogo::{self, GoError};
use crate::protowire::{self, WireType};

/// Go's `maxCanonicalDepth`.
const MAX_DEPTH: usize = 100;

const ANY: &str = "google.protobuf.Any";

fn not_canonical(detail: &str) -> GoError {
    GoError::new(format!("tx is not canonically encoded: {detail}"))
}

fn in_message(info: &MessageInfo, detail: &str) -> GoError {
    not_canonical(&format!("{}: {detail}", info.name))
}

/// `checkCanonicalTx`: the encoding of `TxRaw` bytes and of the `TxBody` and
/// `AuthInfo` they hold.
pub(crate) fn check_tx(tx: &[u8]) -> Result<(), GoError> {
    check_message(tx, gogo::info("cosmos.tx.v1beta1.TxRaw")?, 0)?;
    // `check_message` accepted the bytes, so each field below is well formed
    // and appears at most once.
    check_message(
        field_value(tx, 1),
        gogo::info("cosmos.tx.v1beta1.TxBody")?,
        1,
    )?;
    check_message(
        field_value(tx, 2),
        gogo::info("cosmos.tx.v1beta1.AuthInfo")?,
        1,
    )
}

fn is_decimal(custom: &str) -> bool {
    custom.ends_with("math.Int") || custom.ends_with("math.LegacyDec")
}

fn wire_type(t: u8) -> Option<WireType> {
    match t {
        ty::BOOL
        | ty::ENUM
        | ty::INT32
        | ty::SINT32
        | ty::UINT32
        | ty::INT64
        | ty::SINT64
        | ty::UINT64 => Some(protowire::VARINT),
        ty::FIXED32 | ty::SFIXED32 | ty::FLOAT => Some(protowire::FIXED32),
        ty::FIXED64 | ty::SFIXED64 | ty::DOUBLE => Some(protowire::FIXED64),
        ty::STRING | ty::BYTES | ty::MESSAGE => Some(protowire::BYTES),
        _ => None,
    }
}

/// `varintInRange`: values decoders cannot truncate differently.
fn varint_in_range(v: u64, t: u8) -> bool {
    match t {
        ty::BOOL => v <= 1,
        ty::INT32 | ty::ENUM => v as i64 == i64::from(v as i32),
        ty::UINT32 | ty::SINT32 => u32::try_from(v).is_ok(),
        _ => true,
    }
}

/// `canonicalDecimal`: an optional minus, digits without leading zeros, not
/// `-0`.
fn canonical_decimal(mut v: &[u8]) -> bool {
    if let Some(rest) = v.strip_prefix(b"-") {
        if rest == b"0" {
            return false;
        }
        v = rest;
    }
    if v.is_empty() || (v[0] == b'0' && v.len() > 1) {
        return false;
    }
    v.iter().all(u8::is_ascii_digit)
}

/// `consumeBytes`: a length-prefixed value with a minimal length prefix.
fn consume_bytes(b: &[u8]) -> Result<(&[u8], usize), &'static str> {
    let (v, n) = protowire::consume_bytes(b);
    if n < 0 {
        return Err("invalid length");
    }
    let n = n as usize;
    if n - v.len() != protowire::size_varint(v.len() as u64) {
        return Err("length prefix is not minimal");
    }
    Ok((v, n))
}

/// `checkPacked`.
fn check_packed(mut b: &[u8], t: u8) -> Result<(), &'static str> {
    match wire_type(t) {
        Some(protowire::FIXED32) if !b.len().is_multiple_of(4) => Err("packed fixed32 length"),
        Some(protowire::FIXED64) if !b.len().is_multiple_of(8) => Err("packed fixed64 length"),
        Some(protowire::FIXED32 | protowire::FIXED64) => Ok(()),
        Some(protowire::VARINT) => {
            while !b.is_empty() {
                let (v, n) = protowire::consume_varint(b);
                if n < 0 || n as usize != protowire::size_varint(v) {
                    return Err("packed varint is not minimal");
                }
                if !varint_in_range(v, t) {
                    return Err("packed value is out of range");
                }
                b = &b[n as usize..];
            }
            Ok(())
        }
        _ => Err("field cannot be packed"),
    }
}

/// `checkMessage`.
fn check_message(mut b: &[u8], info: &MessageInfo, depth: usize) -> Result<(), GoError> {
    if depth > MAX_DEPTH {
        return Err(in_message(info, &format!("nested deeper than {MAX_DEPTH}")));
    }
    let is_required = |f: &FieldInfo| is_decimal(f.custom) && !f.nullable && !f.repeated;
    let required = info.fields.iter().filter(|f| is_required(f)).count();
    let mut present = 0usize;
    let mut prev: i64 = 0;
    let mut oneofs_seen: u64 = 0;
    let mut type_url: &[u8] = &[];
    let mut value: &[u8] = &[];
    while !b.is_empty() {
        let (num, typ, n) = protowire::consume_tag(b);
        if n < 0 {
            return Err(in_message(info, "invalid tag"));
        }
        if n as usize != protowire::size_varint((num as u64) << 3) {
            return Err(in_message(info, &format!("field {num} tag is not minimal")));
        }
        b = &b[n as usize..];
        let Some(f) = info.field(num as u32) else {
            return Err(in_message(info, &format!("unknown field {num}")));
        };
        let repeats = f.repeated && !f.packed;
        if num < prev || (num == prev && !repeats) {
            return Err(in_message(
                info,
                &format!("field {num} is out of order or repeated"),
            ));
        }
        if f.oneof >= 0 && num != prev {
            if f.oneof >= 64 || oneofs_seen & (1 << f.oneof) != 0 {
                return Err(in_message(info, &format!("field {num} sets a oneof twice")));
            }
            oneofs_seen |= 1 << f.oneof;
        }
        prev = num;
        // A required field is singular, so it is counted once.
        present += usize::from(is_required(f));

        if f.packed {
            if typ != protowire::BYTES {
                return Err(in_message(info, &format!("field {num} is not packed")));
            }
            let (v, m) =
                consume_bytes(b).map_err(|e| in_message(info, &format!("field {num}: {e}")))?;
            check_packed(v, f.ty).map_err(|e| in_message(info, &format!("field {num}: {e}")))?;
            b = &b[m..];
            continue;
        }
        if Some(typ) != wire_type(f.ty) {
            return Err(in_message(
                info,
                &format!("field {num} has wire type {typ}"),
            ));
        }
        let m = match typ {
            protowire::VARINT => {
                let (v, k) = protowire::consume_varint(b);
                if k < 0 || k as usize != protowire::size_varint(v) {
                    return Err(in_message(
                        info,
                        &format!("field {num} varint is not minimal"),
                    ));
                }
                if !varint_in_range(v, f.ty) {
                    return Err(in_message(
                        info,
                        &format!("field {num} value is out of range"),
                    ));
                }
                k as usize
            }
            protowire::FIXED32 | protowire::FIXED64 => {
                let size = if typ == protowire::FIXED32 { 4 } else { 8 };
                if b.len() < size {
                    return Err(in_message(info, &format!("field {num} is truncated")));
                }
                size
            }
            _ => {
                let (v, k) =
                    consume_bytes(b).map_err(|e| in_message(info, &format!("field {num}: {e}")))?;
                check_bytes_value(v, f, info, num, depth)?;
                if info.name == ANY {
                    match num {
                        1 => type_url = v,
                        2 => value = v,
                        _ => {}
                    }
                }
                k
            }
        };
        b = &b[m..];
    }
    if present != required {
        return Err(in_message(info, "a non-nullable decimal field is missing"));
    }
    if info.name == ANY {
        return check_any_value(type_url, value, depth);
    }
    Ok(())
}

/// The checks of a length-delimited value of field `num` of `info`.
fn check_bytes_value(
    v: &[u8],
    f: &FieldInfo,
    info: &MessageInfo,
    num: i64,
    depth: usize,
) -> Result<(), GoError> {
    if f.ty == ty::STRING && std::str::from_utf8(v).is_err() {
        return Err(in_message(info, &format!("field {num} is not valid UTF-8")));
    }
    if is_decimal(f.custom) && !canonical_decimal(v) {
        return Err(in_message(
            info,
            &format!("field {num} is not a canonical decimal"),
        ));
    }
    if f.ty == ty::MESSAGE {
        let sub = gogo::info(f.type_name)
            .map_err(|e| in_message(info, &format!("field {num}: {}", e.0)))?;
        check_message(v, sub, depth + 1)?;
    }
    Ok(())
}

/// `checkAnyValue`: an `Any` without a type URL must be empty.
fn check_any_value(type_url: &[u8], value: &[u8], depth: usize) -> Result<(), GoError> {
    if type_url.is_empty() {
        if !value.is_empty() {
            return Err(not_canonical("Any has a value but no type URL"));
        }
        return Ok(());
    }
    // The type URL passed the UTF-8 check above.
    let url = String::from_utf8_lossy(type_url);
    let name = url.rsplit('/').next().unwrap_or_default();
    let info = celestia_proto::wire_table::message(name)
        .ok_or_else(|| not_canonical(&format!("Any {url}: unknown message type {name}")))?;
    check_message(value, info, depth + 1)
}

/// `fieldValue`: the value of length-delimited field `num` in bytes that
/// [`check_message`] accepted.
fn field_value(mut b: &[u8], num: i64) -> &[u8] {
    while !b.is_empty() {
        let (n, typ, k) = protowire::consume_tag(b);
        if k < 0 {
            return &[];
        }
        b = &b[k as usize..];
        let m = protowire::consume_field_value(n, typ, b);
        if m < 0 {
            return &[];
        }
        if n == num && typ == protowire::BYTES {
            return protowire::consume_bytes(b).0;
        }
        b = &b[m as usize..];
    }
    &[]
}

#[cfg(test)]
mod tests {
    use crate::protowire::{BYTES, VARINT, append_bytes_field, append_tag, append_varint};

    const SEND: &str = "/cosmos.bank.v1beta1.MsgSend";
    const EXEC: &str = "/cosmos.authz.v1beta1.MsgExec";
    const NON_CRITICAL: u32 = 1025;

    fn field(num: u32, v: &[u8]) -> Vec<u8> {
        let mut b = Vec::new();
        append_bytes_field(&mut b, num, v);
        b
    }

    fn varint_field(num: u32, v: u64) -> Vec<u8> {
        let mut b = Vec::new();
        append_tag(&mut b, num, VARINT);
        append_varint(&mut b, v);
        b
    }

    fn cat(parts: &[&[u8]]) -> Vec<u8> {
        parts.concat()
    }

    fn any(url: &str, value: &[u8]) -> Vec<u8> {
        let mut b = field(1, url.as_bytes());
        if !value.is_empty() {
            b.extend(field(2, value));
        }
        b
    }

    fn coin(denom: &[u8], amount: Option<&[u8]>) -> Vec<u8> {
        let mut b = field(1, denom);
        if let Some(a) = amount {
            b.extend(field(2, a));
        }
        b
    }

    fn send(to: &[u8], c: &[u8]) -> Vec<u8> {
        cat(&[&field(1, b"celestia1from"), &field(2, to), &field(3, c)])
    }

    fn canonical_send() -> Vec<u8> {
        send(b"celestia1to", &coin(b"utia", Some(b"1")))
    }

    fn exec(msgs: &[Vec<u8>]) -> Vec<u8> {
        let mut b = field(1, b"celestia1grantee");
        for m in msgs {
            b.extend(field(2, m));
        }
        b
    }

    fn body(memo: &[u8], msgs: &[Vec<u8>]) -> Vec<u8> {
        let mut b = Vec::new();
        for m in msgs {
            b.extend(field(1, m));
        }
        if !memo.is_empty() {
            b.extend(field(2, memo));
        }
        b
    }

    fn mode_info(mode: u64) -> Vec<u8> {
        field(1, &varint_field(1, mode))
    }

    fn signer_info(mode: &[u8], sequence: Option<u64>) -> Vec<u8> {
        let pk = any("/cosmos.crypto.secp256k1.PubKey", &field(1, &[2; 33]));
        let mut b = cat(&[&field(1, &pk), &field(2, mode)]);
        if let Some(s) = sequence {
            b.extend(varint_field(3, s));
        }
        b
    }

    fn auth_info(signer: &[u8], fee_coin: &[u8]) -> Vec<u8> {
        let fee = cat(&[&field(1, fee_coin), &varint_field(2, 100_000)]);
        cat(&[&field(1, signer), &field(2, &fee)])
    }

    fn default_auth() -> Vec<u8> {
        auth_info(
            &signer_info(&mode_info(1), Some(1)),
            &coin(b"utia", Some(b"1000")),
        )
    }

    fn tx(body: &[u8], auth: &[u8]) -> Vec<u8> {
        cat(&[&field(1, body), &field(2, auth), &field(3, &[7; 64])])
    }

    fn send_tx() -> Vec<u8> {
        tx(
            &body(b"memo", &[any(SEND, &canonical_send())]),
            &default_auth(),
        )
    }

    /// Decodes at app version `v`.
    fn decode_at(v: u64, raw: &[u8]) -> sdk::Result<sdk::tx::Tx> {
        sdk::version::set_app_version(v);
        let out = crate::decode(raw);
        sdk::version::set_app_version(0);
        out
    }

    #[test]
    fn accepts_canonical_and_lenient_shapes() {
        let transfer = cat(&[
            &field(1, b"transfer"),
            &field(2, b"channel-2"),
            &field(3, &coin(b"utia", Some(b"1000"))),
            &field(4, b"celestia1from"),
            &field(5, b"osmo1to"),
            &varint_field(7, 1_794_000_000_000_000_000),
        ]);
        let hex = format!("0x{}", "AB".repeat(32));
        let remote_transfer = cat(&[
            &field(1, b"celestia1from"),
            &field(2, format!("0x{}", "00".repeat(32)).as_bytes()),
            &varint_field(3, 42_161),
            &field(4, hex.as_bytes()),
            &field(5, b"1000000"),
            &field(7, b"0"),
            &field(8, &coin(b"utia", Some(b"2000000"))),
        ]);
        let mut empty_memo = body(b"", &[any(SEND, &canonical_send())]);
        empty_memo.extend(field(2, b""));
        let cases = [
            send_tx(),
            tx(
                &body(b"", &[any(EXEC, &exec(&[any(SEND, &canonical_send())]))]),
                &default_auth(),
            ),
            tx(&empty_memo, &default_auth()),
            tx(
                &body(b"memo", &[any(SEND, &canonical_send())]),
                &auth_info(
                    &signer_info(&mode_info(1), Some(0)),
                    &coin(b"utia", Some(b"1000")),
                ),
            ),
            tx(
                &body(
                    b"",
                    &[any("/ibc.applications.transfer.v1.MsgTransfer", &transfer)],
                ),
                &default_auth(),
            ),
            tx(
                &body(
                    b"",
                    &[any(
                        "/hyperlane.warp.v1.MsgRemoteTransfer",
                        &remote_transfer,
                    )],
                ),
                &default_auth(),
            ),
        ];
        for (i, raw) in cases.iter().enumerate() {
            assert_eq!(super::check_tx(raw), Ok(()), "case {i}");
            assert!(decode_at(11, raw).is_ok(), "case {i}");
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn rejects_ambiguous_encodings_from_v11_only() {
        let send_any = any(SEND, &canonical_send());
        let mut non_minimal = body(b"memo", std::slice::from_ref(&send_any));
        append_tag(&mut non_minimal, 3, VARINT);
        non_minimal.extend([0x81, 0x00]);
        let mut unpacked_pfb = field(1, b"celestia1from");
        unpacked_pfb.extend(varint_field(3, 5));
        unpacked_pfb.extend(varint_field(3, 6));
        let two_oneofs = cat(&[&mode_info(1), &field(2, &field(1, &varint_field(1, 1)))]);
        let with_amount = |a: Option<&[u8]>| {
            tx(
                &body(b"", &[any(SEND, &send(b"celestia1to", &coin(b"utia", a)))]),
                &default_auth(),
            )
        };
        let mut deep = mode_info(1);
        for _ in 0..60 {
            let multi = cat(&[&field(1, &varint_field(1, 1)), &field(2, &deep)]);
            deep = field(2, &multi);
        }
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("non-minimal varint", tx(&non_minimal, &default_auth())),
            (
                "duplicate memo",
                tx(
                    &cat(&[
                        &body(b"a", std::slice::from_ref(&send_any)),
                        &field(2, b"b"),
                    ]),
                    &default_auth(),
                ),
            ),
            (
                "out of order",
                tx(
                    &cat(&[&field(2, b"memo"), &field(1, &send_any)]),
                    &default_auth(),
                ),
            ),
            (
                "unknown non-critical field",
                tx(
                    &cat(&[
                        &body(b"m", std::slice::from_ref(&send_any)),
                        &field(NON_CRITICAL, b"x"),
                    ]),
                    &default_auth(),
                ),
            ),
            (
                "unknown field in Any",
                tx(
                    &body(b"", &[cat(&[&send_any, &field(NON_CRITICAL, b"x")])]),
                    &default_auth(),
                ),
            ),
            (
                "nested duplicate in MsgExec",
                tx(
                    &body(
                        b"",
                        &[any(
                            EXEC,
                            &exec(&[any(SEND, &cat(&[&canonical_send(), &field(1, b"x")]))]),
                        )],
                    ),
                    &default_auth(),
                ),
            ),
            (
                "enum beyond 32 bits",
                tx(
                    &body(b"m", std::slice::from_ref(&send_any)),
                    &auth_info(
                        &signer_info(&mode_info(1 << 32 | 1), Some(1)),
                        &coin(b"utia", Some(b"1")),
                    ),
                ),
            ),
            (
                "two oneof members",
                tx(
                    &body(b"m", std::slice::from_ref(&send_any)),
                    &auth_info(
                        &signer_info(&two_oneofs, Some(1)),
                        &coin(b"utia", Some(b"1")),
                    ),
                ),
            ),
            (
                "unpacked repeated scalar",
                tx(
                    &body(
                        b"",
                        &[any("/celestia.blob.v1.MsgPayForBlobs", &unpacked_pfb)],
                    ),
                    &default_auth(),
                ),
            ),
            (
                "too deep",
                tx(
                    &body(b"m", std::slice::from_ref(&send_any)),
                    &auth_info(&signer_info(&deep, Some(1)), &coin(b"utia", Some(b"1"))),
                ),
            ),
            (
                "invalid UTF-8 memo",
                tx(
                    &body(b"\xff", std::slice::from_ref(&send_any)),
                    &default_auth(),
                ),
            ),
            (
                "invalid UTF-8 nested",
                tx(
                    &body(
                        b"",
                        &[any(
                            EXEC,
                            &exec(&[any(SEND, &send(b"to\xff", &coin(b"utia", Some(b"1"))))]),
                        )],
                    ),
                    &default_auth(),
                ),
            ),
            ("amount left out", with_amount(None)),
            ("empty amount", with_amount(Some(b""))),
            ("plus sign", with_amount(Some(b"+1"))),
            ("hex amount", with_amount(Some(b"0x10"))),
            ("underscore", with_amount(Some(b"1_0"))),
            ("leading zero", with_amount(Some(b"01"))),
            ("negative zero", with_amount(Some(b"-0"))),
        ];
        for (name, raw) in &cases {
            assert!(decode_at(10, raw).is_ok(), "{name}: v10 decodes as before");
            let err = decode_at(11, raw).expect_err(name);
            assert!(
                err.log.contains("tx is not canonically encoded"),
                "{name}: {}",
                err.log
            );
            assert_eq!((err.codespace, err.code), ("sdk", 2), "{name}");
        }
        // A non-minimal TxRaw tag is caught by the walker itself.
        let mut raw = vec![0x8a, 0x00];
        let b = body(b"m", &[send_any]);
        append_varint(&mut raw, b.len() as u64);
        raw.extend(b);
        raw.extend(field(2, &default_auth()));
        assert!(super::check_tx(&raw).is_err());
    }

    #[test]
    fn never_panics_on_prefixes_and_flips() {
        let raw = send_tx();
        for i in 0..raw.len() {
            let _ = super::check_tx(&raw[..i]);
            let mut flipped = raw.clone();
            flipped[i] ^= 0xff;
            let _ = super::check_tx(&flipped);
        }
        let _ = super::check_tx(&[BYTES; 64]);
    }
}
