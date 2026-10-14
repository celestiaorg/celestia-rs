//! `Uint`: port of Go `cosmossdk.io/math.Uint` (`uint.go`).

use std::fmt;
use std::str::FromStr;

use num_bigint::{BigInt, BigUint, Sign};
use num_traits::{ToPrimitive, Zero};

use crate::int::MAX_BIT_LEN;
use crate::parse::{Base, parse_go_big_int};
use crate::{MathError, Result, json_string, to_json_string};

/// Arbitrary-precision unsigned integer bounded to `2^256 - 1`, matching Go
/// `math.Uint`. Go panics on overflow/underflow; the `checked_*` methods here
/// return a [`MathError`] instead.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uint(BigUint);

/// Go `UintOverflow`.
fn check(i: BigInt) -> Result<Uint> {
    let (sign, magnitude) = i.into_parts();
    if sign == Sign::Minus {
        return Err(MathError::NegativeUint);
    }
    Uint::from_biguint(magnitude)
}

impl Uint {
    /// Zero (Go `ZeroUint`).
    #[must_use]
    pub fn zero() -> Self {
        Self(BigUint::zero())
    }

    /// One (Go `OneUint`).
    #[must_use]
    pub fn one() -> Self {
        Self(BigUint::from(1u8))
    }

    /// Constructs from a `u64` (Go `NewUint`).
    #[must_use]
    pub fn from_u64(n: u64) -> Self {
        Self(BigUint::from(n))
    }

    /// Constructs from a [`BigUint`], failing above 256 bits (Go
    /// `NewUintFromBigInt`, which panics instead).
    pub fn from_biguint(i: BigUint) -> Result<Self> {
        if i.bits() > MAX_BIT_LEN {
            return Err(MathError::IntOutOfRange);
        }
        Ok(Self(i))
    }

    /// Parses a Go integer literal (Go `ParseUint`: `SetString(s, 0)` then
    /// the sign and 256-bit checks).
    pub fn parse(s: &str) -> Result<Self> {
        let i = parse_go_big_int(s.as_bytes(), Base::Auto)
            .ok_or_else(|| MathError::InvalidInt(s.to_owned()))?;
        check(i)
    }

    /// Borrows the underlying [`BigUint`].
    #[must_use]
    pub fn as_biguint(&self) -> &BigUint {
        &self.0
    }

    /// Converts to `u64`, or `None` when out of range (Go `Uint64` panics).
    #[must_use]
    pub fn to_u64(&self) -> Option<u64> {
        self.0.to_u64()
    }

    /// Go `IsZero`.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0.is_zero()
    }

    fn bounded(i: BigUint) -> Result<Self> {
        Self::from_biguint(i).map_err(|_| MathError::IntOverflow)
    }

    /// Go `Add`.
    pub fn checked_add(&self, other: &Self) -> Result<Self> {
        Self::bounded(&self.0 + &other.0)
    }

    /// Go `Sub` (errors on underflow).
    pub fn checked_sub(&self, other: &Self) -> Result<Self> {
        if other.0 > self.0 {
            return Err(MathError::NegativeUint);
        }
        Ok(Self(&self.0 - &other.0))
    }

    /// Go `Mul`.
    pub fn checked_mul(&self, other: &Self) -> Result<Self> {
        Self::bounded(&self.0 * &other.0)
    }

    /// Go `Quo`.
    pub fn checked_quo(&self, other: &Self) -> Result<Self> {
        if other.is_zero() {
            return Err(MathError::DivideByZero);
        }
        Ok(Self(&self.0 / &other.0))
    }

    /// Go `Mod`.
    pub fn checked_mod(&self, other: &Self) -> Result<Self> {
        if other.is_zero() {
            return Err(MathError::DivideByZero);
        }
        Ok(Self(&self.0 % &other.0))
    }

    /// Go `RelativePow`: `x^n` in fixed point with base `b`, rounding half up
    /// at each step.
    pub fn relative_pow(x: &Self, n: &Self, b: &Self) -> Result<Self> {
        if x.is_zero() {
            return Ok(if n.is_zero() {
                Self::one()
            } else {
                Self::zero()
            });
        }
        let two = Self::from_u64(2);
        let mut x = x.clone();
        let mut z = if n.checked_mod(&two)?.is_zero() {
            b.clone()
        } else {
            x.clone()
        };
        let half_of_b = b.checked_quo(&two)?;
        let mut n = n.checked_quo(&two)?;
        while !n.is_zero() {
            x = x.checked_mul(&x)?.checked_add(&half_of_b)?.checked_quo(b)?;
            if n.checked_mod(&two)? == Self::one() {
                z = z.checked_mul(&x)?.checked_add(&half_of_b)?.checked_quo(b)?;
            }
            n = n.checked_quo(&two)?;
        }
        Ok(z)
    }

    /// Gogoproto `customtype` binary form (Go `Marshal`): decimal ASCII.
    #[must_use]
    pub fn marshal(&self) -> Vec<u8> {
        self.0.to_string().into_bytes()
    }

    /// Decodes the gogoproto binary form (Go `Unmarshal`). An empty slice
    /// yields zero (Go leaves the value nil, which marshals identically).
    pub fn unmarshal(data: &[u8]) -> Result<Self> {
        if data.is_empty() {
            return Ok(Self::zero());
        }
        let i = parse_go_big_int(data, Base::Auto)
            .ok_or_else(|| MathError::InvalidInt(String::from_utf8_lossy(data).into_owned()))?;
        check(i)
    }

    /// JSON encoding (Go `MarshalJSON`).
    #[must_use]
    pub fn to_json(&self) -> String {
        to_json_string(&self.to_string())
    }

    /// JSON decoding (Go `UnmarshalJSON`).
    pub fn from_json(bz: &[u8]) -> Result<Self> {
        Self::parse(&json_string(bz)?)
    }
}

impl fmt::Display for Uint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl FromStr for Uint {
    type Err = MathError;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

impl From<u64> for Uint {
    fn from(n: u64) -> Self {
        Self::from_u64(n)
    }
}

impl serde::Serialize for Uint {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for Uint {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}
