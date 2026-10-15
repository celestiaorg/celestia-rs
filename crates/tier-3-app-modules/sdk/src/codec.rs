//! `cosmossdk.io/collections` key and value codecs.
//!
//! Terminal key parts are encoded raw; non-terminal parts (all but the last
//! part of a `Pair`/`Triple`) are self-delimiting.

/// `collections.Uint64Key` / `Uint64Value` (big-endian).
#[must_use]
pub fn u64_key(v: u64) -> [u8; 8] {
    v.to_be_bytes()
}

/// `collections.Int64Key`: big-endian with the sign bit flipped so byte
/// order equals numeric order.
#[must_use]
pub fn i64_key(v: i64) -> [u8; 8] {
    let mut b = v.to_be_bytes();
    b[0] ^= 0x80;
    b
}

/// Decodes [`i64_key`].
#[must_use]
pub fn decode_i64_key(mut b: [u8; 8]) -> i64 {
    b[0] ^= 0x80;
    i64::from_be_bytes(b)
}

/// Non-terminal `StringKey`: bytes followed by a `0x00` delimiter.
#[must_use]
pub fn string_non_terminal(s: &str) -> Vec<u8> {
    let mut out = gostr::to_go_bytes(s).into_owned();
    out.push(0);
    out
}

/// Non-terminal `BytesKey` / `sdk.AccAddressKey` / `ValAddressKey`:
/// one length byte followed by the bytes (`address.MustLengthPrefix`).
///
/// Lengths above 255 are truncated to their low byte, as Go's
/// `MustLengthPrefix` would panic; addresses are at most 32 bytes.
#[must_use]
pub fn length_prefixed(b: &[u8]) -> Vec<u8> {
    let len = b.len().to_le_bytes()[0];
    let mut out = Vec::with_capacity(b.len() + 1);
    out.push(len);
    out.extend_from_slice(b);
    out
}

/// Concatenates key parts.
#[must_use]
pub fn key(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

/// `collections.Uint64Value` decode.
#[must_use]
pub fn decode_u64(b: &[u8]) -> Option<u64> {
    Some(u64::from_be_bytes(b.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int64_order_preserving() {
        let vals = [i64::MIN, -5, -1, 0, 1, 7, i64::MAX];
        let enc: Vec<_> = vals.iter().map(|v| i64_key(*v)).collect();
        assert!(enc.windows(2).all(|w| w[0] < w[1]));
        for v in vals {
            assert_eq!(decode_i64_key(i64_key(v)), v);
        }
        assert_eq!(i64_key(1), [0x80, 0, 0, 0, 0, 0, 0, 1]);
    }

    #[test]
    fn non_terminal_parts() {
        assert_eq!(string_non_terminal("ab"), b"ab\0");
        assert_eq!(length_prefixed(&[9; 20])[0], 20);
        assert_eq!(
            key(&[b"\x02", &length_prefixed(&[1, 2]), b"utia"]),
            b"\x02\x02\x01\x02utia"
        );
        assert_eq!(decode_u64(&u64_key(42)), Some(42));
    }
}
