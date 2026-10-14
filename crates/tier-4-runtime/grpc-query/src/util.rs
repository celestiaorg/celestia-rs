//! Shared helpers: Go-exact address codecs, denoms, store decoding.

use celestia_proto::cosmos::base::query::v1beta1::PageResponse;
use prost::Message;
use sdk::Context;
use sdk::error::sdk as errs;

use crate::error::{QResult, QueryError, invalid_arg};

/// Account address prefix.
pub const ACC: &str = "celestia";
/// Validator operator prefix.
pub const VALOPER: &str = "celestiavaloper";
/// Consensus address prefix.
pub const VALCONS: &str = "celestiavalcons";

const CHARSET: &[u8; 32] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";

fn polymod(values: impl IntoIterator<Item = u8>) -> u32 {
    const GEN: [u32; 5] = [
        0x3b6a_57b2,
        0x2650_8e6d,
        0x1ea1_19fa,
        0x3d42_33dd,
        0x2a14_62b3,
    ];
    let mut chk: u32 = 1;
    for v in values {
        let b = chk >> 25;
        chk = ((chk & 0x01ff_ffff) << 5) ^ u32::from(v);
        for (i, g) in GEN.iter().enumerate() {
            if (b >> i) & 1 == 1 {
                chk ^= g;
            }
        }
    }
    chk
}

fn hrp_expand(hrp: &str) -> Vec<u8> {
    let b = hrp.as_bytes();
    let mut v: Vec<u8> = b.iter().map(|c| c >> 5).collect();
    v.push(0);
    v.extend(b.iter().map(|c| c & 31));
    v
}

fn checksum(hrp: &str, data: &[u8]) -> String {
    let mut v = hrp_expand(hrp);
    v.extend_from_slice(data);
    v.extend_from_slice(&[0; 6]);
    let pm = polymod(v) ^ 1;
    (0..6)
        .map(|i| CHARSET[((pm >> (5 * (5 - i))) & 31) as usize] as char)
        .collect()
}

/// `bech32.DecodeAndConvert` (cosmos btcutil v1.0.5, limit 1023) with Go's
/// error strings.
pub fn bech32_decode(input: &str) -> Result<(String, Vec<u8>), String> {
    let fail = |e: String| Err(format!("decoding bech32 failed: {e}"));
    if input.len() > 1023 || input.len() < 8 {
        return fail(format!("invalid bech32 string length {}", input.len()));
    }
    let mut has_lower = false;
    let mut has_upper = false;
    for &c in input.as_bytes() {
        if !(33..=126).contains(&c) {
            return fail(format!("invalid character in string: '{}'", c as char));
        }
        has_lower |= c.is_ascii_lowercase();
        has_upper |= c.is_ascii_uppercase();
        if has_lower && has_upper {
            return fail("string not all lowercase or all uppercase".into());
        }
    }
    let bech = input.to_ascii_lowercase();
    let one = match bech.rfind('1') {
        Some(i) if i >= 1 && i + 7 <= bech.len() => i,
        Some(i) => return fail(format!("invalid separator index {i}")),
        None => return fail("invalid separator index -1".into()),
    };
    let hrp = &bech[..one];
    let mut decoded = Vec::with_capacity(bech.len() - one - 1);
    for &c in &bech.as_bytes()[one + 1..] {
        match CHARSET.iter().position(|x| *x == c) {
            Some(i) => decoded.push(u8::try_from(i).unwrap_or_default()),
            None => return fail(format!("invalid character not part of charset: {c}")),
        }
    }
    let (values, check) = decoded.split_at(decoded.len() - 6);
    let mut all = hrp_expand(hrp);
    all.extend_from_slice(values);
    all.extend_from_slice(check);
    if polymod(all) != 1 {
        return fail(format!(
            "invalid checksum (expected {} got {})",
            checksum(hrp, values),
            &bech[bech.len() - 6..]
        ));
    }
    // ConvertBits(data, 5, 8, false).
    let mut out = Vec::new();
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for v in values {
        acc = (acc << 5) | u32::from(*v);
        bits += 5;
        while bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    if bits > 4 || (acc & ((1 << bits) - 1)) != 0 {
        return fail("invalid incomplete group".into());
    }
    Ok((hrp.to_owned(), out))
}

/// `address.Bech32Codec.StringToBytes` for `prefix`.
pub fn codec_to_bytes(s: &str, prefix: &str) -> QResult<Vec<u8>> {
    if s.trim().is_empty() {
        return Err(QueryError::Plain(
            "empty address string is not allowed".into(),
        ));
    }
    let (hrp, bz) = bech32_decode(s).map_err(QueryError::Plain)?;
    if hrp != prefix {
        return Err(QueryError::Sdk(errs::logic(format!(
            "hrp does not match bech32 prefix: expected '{prefix}' got '{hrp}'"
        ))));
    }
    verify_address(&bz)?;
    Ok(bz)
}

/// `sdk.VerifyAddressFormat`.
pub fn verify_address(bz: &[u8]) -> QResult<()> {
    if bz.is_empty() {
        return Err(QueryError::Sdk(errs::unknown_address(
            "addresses cannot be empty",
        )));
    }
    if bz.len() > 255 {
        return Err(QueryError::Sdk(errs::unknown_address(
            "address max length is 255",
        )));
    }
    Ok(())
}

/// `sdk.AccAddressFromBech32` / `ValAddressFromBech32` (`GetFromBech32`):
/// plain errors with the legacy messages.
pub fn from_bech32_legacy(s: &str, prefix: &str) -> QResult<Vec<u8>> {
    if s.trim().is_empty() {
        return Err(QueryError::Plain(
            "empty address string is not allowed".into(),
        ));
    }
    let (hrp, bz) = bech32_decode(s).map_err(QueryError::Plain)?;
    if hrp != prefix {
        return Err(QueryError::Plain(format!(
            "invalid Bech32 prefix; expected {prefix}, got {hrp}"
        )));
    }
    verify_address(&bz)?;
    Ok(bz)
}

/// Bech32 string of `bz` (`""` for empty input).
#[must_use]
pub fn bech32(prefix: &str, bz: &[u8]) -> String {
    sdk::address::to_bech32(prefix, bz)
}

/// Go `hex.DecodeString` with its error strings.
pub fn go_hex_decode(s: &str) -> Result<Vec<u8>, String> {
    fn nib(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }
    let invalid = |c: u8| {
        let ch = char::from(c);
        format!("encoding/hex: invalid byte: U+{:04X} '{ch}'", u32::from(ch))
    };
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len() / 2);
    let mut j = 1;
    while j < b.len() {
        let hi = nib(b[j - 1]).ok_or_else(|| invalid(b[j - 1]))?;
        let lo = nib(b[j]).ok_or_else(|| invalid(b[j]))?;
        out.push(hi << 4 | lo);
        j += 2;
    }
    if b.len() % 2 == 1 {
        if nib(b[j - 1]).is_none() {
            return Err(invalid(b[j - 1]));
        }
        return Err("encoding/hex: odd length hex string".into());
    }
    Ok(out)
}

/// `sdk.ValidateDenom` as a `codes.InvalidArgument` status.
pub fn validate_denom(denom: &str) -> QResult<()> {
    if sdk::coins::is_valid_denom(denom) {
        Ok(())
    } else {
        Err(invalid_arg(format!("invalid denom: {denom}")))
    }
}

/// Decodes a stored protobuf value (`cdc.MustUnmarshal`).
pub fn decode<M: Message + Default>(b: &[u8]) -> QResult<M> {
    M::decode(b).map_err(|e| QueryError::Plain(e.to_string()))
}

/// Reads and decodes `key` of `store`.
pub fn get<M: Message + Default>(
    ctx: &mut Context<'_>,
    store: &str,
    key: &[u8],
) -> QResult<Option<M>> {
    ctx.get(store, key)?.map(|b| decode(&b)).transpose()
}

/// `Some(page)` (Go always returns a non-nil page response).
#[must_use]
pub fn page(p: PageResponse) -> Option<PageResponse> {
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bech32_go_errors() {
        let ok = "celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7";
        assert_eq!(
            bech32_decode(ok).unwrap().1,
            sdk::address::module_address("gov")
        );
        let bad = "celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v8";
        assert_eq!(
            bech32_decode(bad).unwrap_err(),
            "decoding bech32 failed: invalid checksum (expected tgz4v7 got tgz4v8)"
        );
        assert_eq!(
            bech32_decode("abc").unwrap_err(),
            "decoding bech32 failed: invalid bech32 string length 3"
        );
        assert_eq!(
            bech32_decode("celestia1Abcdefgh").unwrap_err(),
            "decoding bech32 failed: string not all lowercase or all uppercase"
        );
    }
}
