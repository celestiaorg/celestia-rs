//! gogoproto `customtype`, `stdtime` and `stdduration` decoding.
//!
//! Custom types reachable from a decoded tx (checked against the schema in
//! tests): `cosmossdk.io/math.Int`, `cosmossdk.io/math.LegacyDec`,
//! hyperlane `util.HexAddress` and the secp256r1 `ecdsaPK`.

use crate::gogo::{GoError, GoResult};

pub(crate) const INT: &str = "cosmossdk.io/math.Int";
pub(crate) const LEGACY_DEC: &str = "cosmossdk.io/math.LegacyDec";
pub(crate) const HEX_ADDRESS: &str = "github.com/bcp-innovations/hyperlane-cosmos/util.HexAddress";
pub(crate) const ECDSA_PK: &str = "ecdsaPK";

/// Custom types this module implements.
pub(crate) const SUPPORTED: &[&str] = &[INT, LEGACY_DEC, HEX_ADDRESS, ECDSA_PK];

/// `time.Time{}.Unix()`: the seconds a zero (never decoded) non-nullable
/// `stdtime` field marshals as.
pub(crate) const ZERO_TIME_SECONDS: i64 = -62_135_596_800;

/// Runs the custom type's `Unmarshal` on `data`. `prev` is the current Go
/// value (`None` = nil) for non-nullable fields, which Go unmarshals in place.
/// Returns the new value as canonical `Marshal` bytes (`None` = nil).
pub(crate) fn unmarshal(
    kind: &str,
    data: &[u8],
    prev: Option<Vec<u8>>,
) -> GoResult<Option<Vec<u8>>> {
    match kind {
        INT => {
            // `if len(data) == 0 { i = nil; return nil }` leaves the value as is.
            if data.is_empty() {
                return Ok(prev);
            }
            let v = math::Int::unmarshal(data).map_err(|e| big_err(data, &e))?;
            Ok(Some(v.marshal()))
        }
        LEGACY_DEC => {
            if data.is_empty() {
                return Ok(prev);
            }
            let v = math::LegacyDec::unmarshal(data).map_err(|e| big_err(data, &e))?;
            Ok(Some(v.marshal()))
        }
        HEX_ADDRESS => Ok(Some(hex_address(data)?)),
        ECDSA_PK => {
            ecdsa_pk(data)?;
            Ok(Some(data.to_vec()))
        }
        other => Err(GoError::new(format!("unsupported customtype {other}"))),
    }
}

/// `big.Int.UnmarshalText`'s error (`%q` of the bytes) for unparsable
/// digits; other errors (range) as they are.
fn big_err(data: &[u8], e: &math::MathError) -> GoError {
    match e {
        math::MathError::InvalidInt(_) => GoError::new(format!(
            "math/big: cannot unmarshal {} into a *big.Int",
            gostr::quote(&gostr::from_go_bytes(data))
        )),
        other => GoError::new(other.to_string()),
    }
}

/// `Marshal` of a nil / zero custom value.
pub(crate) fn nil_marshal(kind: &str) -> Vec<u8> {
    match kind {
        HEX_ADDRESS => {
            let mut s = b"0x".to_vec();
            s.extend(std::iter::repeat_n(b'0', 64));
            s
        }
        INT | LEGACY_DEC => b"0".to_vec(),
        _ => Vec::new(),
    }
}

/// hyperlane `HexAddress.Unmarshal`: exactly 66 bytes, `0x` + 64 hex digits.
fn hex_address(data: &[u8]) -> GoResult<Vec<u8>> {
    if data.len() != 66 {
        return Err(GoError::new("invalid hex address length"));
    }
    // strings.TrimPrefix(s, "0x") then a 64-character check.
    let Some(hex) = data.strip_prefix(b"0x") else {
        return Err(GoError::new("invalid hex address length"));
    };
    let mut out = b"0x".to_vec();
    for &c in hex {
        let lower = match c {
            b'0'..=b'9' | b'a'..=b'f' => c,
            b'A'..=b'F' => c + 32,
            _ => return Err(GoError::new("encoding/hex: invalid byte")),
        };
        out.push(lower);
    }
    Ok(out)
}

/// secp256r1 `ecdsaPK.Unmarshal`: 33 bytes, `elliptic.UnmarshalCompressed`
/// on P-256 must succeed.
fn ecdsa_pk(data: &[u8]) -> GoResult<()> {
    if data.len() != 33 {
        return Err(GoError::new("wrong ECDSA PK bytes"));
    }
    if data[0] != 2 && data[0] != 3 {
        return Err(GoError::new("wrong ECDSA PK bytes, unknown curve type"));
    }
    p256::PublicKey::from_sec1_bytes(data)
        .map(|_| ())
        .map_err(|_| GoError::new("wrong ECDSA PK bytes, unknown curve type"))
}

/// gogoproto `types.validateTimestamp`.
pub(crate) fn validate_timestamp(secs: i64, nanos: i32) -> GoResult<()> {
    if secs < ZERO_TIME_SECONDS {
        return Err(GoError::new("timestamp: before 0001-01-01"));
    }
    if secs >= 253_402_300_800 {
        return Err(GoError::new("timestamp: after 10000-01-01"));
    }
    if !(0..1_000_000_000).contains(&nanos) {
        return Err(GoError::new("timestamp: nanos not in range [0, 1e9)"));
    }
    Ok(())
}

/// gogoproto `types.DurationFromProto` (range checks plus the
/// `time.Duration` overflow checks).
pub(crate) fn validate_duration(secs: i64, nanos: i32) -> GoResult<()> {
    // int64(10000 * 365.25 * 24 * 60 * 60)
    const MAX_SECONDS: i64 = 315_576_000_000;
    if !(-MAX_SECONDS..=MAX_SECONDS).contains(&secs) {
        return Err(GoError::new("duration: seconds out of range"));
    }
    if nanos <= -1_000_000_000 || nanos >= 1_000_000_000 {
        return Err(GoError::new("duration: nanos out of range"));
    }
    if (secs < 0 && nanos > 0) || (secs > 0 && nanos < 0) {
        return Err(GoError::new(
            "duration: seconds and nanos have different signs",
        ));
    }
    let mut d = secs.wrapping_mul(1_000_000_000);
    if d / 1_000_000_000 != secs {
        return Err(GoError::new("duration: out of range for time.Duration"));
    }
    if nanos != 0 {
        d = d.wrapping_add(i64::from(nanos));
        if (d < 0) != (nanos < 0) {
            return Err(GoError::new("duration: out of range for time.Duration"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ints() {
        assert_eq!(unmarshal(INT, b"010", None).unwrap(), Some(b"8".to_vec()));
        assert_eq!(unmarshal(INT, b"", None).unwrap(), None);
        assert_eq!(
            unmarshal(INT, b"", Some(b"5".to_vec())).unwrap(),
            Some(b"5".to_vec())
        );
        assert!(unmarshal(INT, b"1.5", None).is_err());
    }

    #[test]
    fn hex_addresses() {
        let mut ok = b"0x".to_vec();
        ok.extend(std::iter::repeat_n(b'A', 64));
        let mut want = b"0x".to_vec();
        want.extend(std::iter::repeat_n(b'a', 64));
        assert_eq!(unmarshal(HEX_ADDRESS, &ok, None).unwrap(), Some(want));
        assert!(unmarshal(HEX_ADDRESS, &ok[..65], None).is_err());
        assert!(unmarshal(HEX_ADDRESS, b"", None).is_err());
    }

    #[test]
    fn durations() {
        assert!(validate_duration(315_576_000_000, 0).is_err()); // > 292 years
        assert!(validate_duration(9_223_372_036, 854_775_807).is_ok());
        assert!(validate_duration(9_223_372_036, 854_775_808).is_err());
        assert!(validate_duration(1, -1).is_err());
    }
}
