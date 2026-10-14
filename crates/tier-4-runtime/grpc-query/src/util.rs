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

/// `bech32.DecodeAndConvert` (cosmos btcutil v1.0.5, limit 1023) with Go's
/// error strings.
pub fn bech32_decode(input: &str) -> Result<(String, Vec<u8>), String> {
    encoding::bech32::decode_and_convert(input)
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
