//! `Int`: port of Go `cosmossdk.io/math.Int` (`int.go`).

use std::fmt;
use std::str::FromStr;

use num_bigint::{BigInt, Sign};
use num_traits::{Signed, ToPrimitive, Zero};

use crate::parse::{Base, parse_go_big_int};
use crate::{LegacyDec, MathError, Result, json_string, to_json_string};

/// Maximum bit length of [`Int`] and [`crate::Uint`] values (Go `MaxBitLen`).
pub const MAX_BIT_LEN: u64 = 256;

/// Returns true when `i` does not fit in [`MAX_BIT_LEN`] bits (Go
/// `bigIntOverflows`).
pub(crate) fn big_int_overflows(i: &BigInt) -> bool {
    i.bits() > MAX_BIT_LEN
}

/// Arbitrary-precision signed integer bounded to `±(2^256 - 1)`, matching Go
/// `math.Int`.
///
/// Arithmetic is checked: where Go panics (or its `Safe*` variant returns an
/// error) the `checked_*` methods return a [`MathError`].
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Int(BigInt);

impl Int {
    /// Zero (Go `ZeroInt`).
    #[must_use]
    pub fn zero() -> Self {
        Self(BigInt::zero())
    }

    /// One (Go `OneInt`).
    #[must_use]
    pub fn one() -> Self {
        Self(BigInt::from(1))
    }

    /// Constructs from an `i64` (Go `NewInt`).
    #[must_use]
    pub fn from_i64(n: i64) -> Self {
        Self(BigInt::from(n))
    }

    /// Constructs from a `u64` (Go `NewIntFromUint64`).
    #[must_use]
    pub fn from_u64(n: u64) -> Self {
        Self(BigInt::from(n))
    }

    /// Constructs from a [`BigInt`], failing if it exceeds 256 bits (Go
    /// `NewIntFromBigInt`, which panics instead).
    pub fn from_bigint(i: BigInt) -> Result<Self> {
        if big_int_overflows(&i) {
            return Err(MathError::IntOutOfRange);
        }
        Ok(Self(i))
    }

    /// Constructs `n * 10^dec` (Go `NewIntWithDecimal`).
    pub fn new_with_decimal(n: i64, dec: u32) -> Result<Self> {
        Self::from_bigint(BigInt::from(n) * num_traits::pow(BigInt::from(10), dec as usize))
    }

    /// Parses a Go integer literal (Go `NewIntFromString`, i.e.
    /// `big.Int.SetString(s, 0)` plus the 256-bit range check). Accepts an
    /// optional sign, `0x`/`0b`/`0o`/`0` prefixes and `_` separators.
    pub fn parse(s: &str) -> Result<Self> {
        let i = parse_go_big_int(s.as_bytes(), Base::Auto)
            .ok_or_else(|| MathError::InvalidInt(s.to_owned()))?;
        Self::from_bigint(i)
    }

    /// Borrows the underlying [`BigInt`].
    #[must_use]
    pub fn as_bigint(&self) -> &BigInt {
        &self.0
    }

    /// Consumes `self`, returning the underlying [`BigInt`].
    #[must_use]
    pub fn into_bigint(self) -> BigInt {
        self.0
    }

    /// Converts to [`LegacyDec`] (Go `ToLegacyDec`).
    #[must_use]
    pub fn to_legacy_dec(&self) -> LegacyDec {
        LegacyDec::from_int(self)
    }

    /// Converts to `i64`, or `None` when out of range (Go `Int64` panics).
    #[must_use]
    pub fn to_i64(&self) -> Option<i64> {
        self.0.to_i64()
    }

    /// Converts to `u64`, or `None` when out of range (Go `Uint64` panics).
    #[must_use]
    pub fn to_u64(&self) -> Option<u64> {
        self.0.to_u64()
    }

    /// Go `IsInt64`.
    #[must_use]
    pub fn is_i64(&self) -> bool {
        self.to_i64().is_some()
    }

    /// Go `IsUint64`.
    #[must_use]
    pub fn is_u64(&self) -> bool {
        self.to_u64().is_some()
    }

    /// Go `IsZero`.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0.is_zero()
    }

    /// Go `IsNegative`.
    #[must_use]
    pub fn is_negative(&self) -> bool {
        self.0.sign() == Sign::Minus
    }

    /// Go `IsPositive`.
    #[must_use]
    pub fn is_positive(&self) -> bool {
        self.0.sign() == Sign::Plus
    }

    /// Go `Sign`: -1, 0 or 1.
    #[must_use]
    pub fn sign(&self) -> i32 {
        match self.0.sign() {
            Sign::Minus => -1,
            Sign::NoSign => 0,
            Sign::Plus => 1,
        }
    }

    fn checked(i: BigInt) -> Result<Self> {
        if big_int_overflows(&i) {
            return Err(MathError::IntOverflow);
        }
        Ok(Self(i))
    }

    /// Go `SafeAdd` / `Add`.
    pub fn checked_add(&self, other: &Self) -> Result<Self> {
        Self::checked(&self.0 + &other.0)
    }

    /// Go `SafeSub` / `Sub`.
    pub fn checked_sub(&self, other: &Self) -> Result<Self> {
        Self::checked(&self.0 - &other.0)
    }

    /// Go `SafeMul` / `Mul`.
    pub fn checked_mul(&self, other: &Self) -> Result<Self> {
        Self::checked(&self.0 * &other.0)
    }

    /// Go `SafeQuo` / `Quo`: quotient truncated toward zero.
    pub fn checked_quo(&self, other: &Self) -> Result<Self> {
        if other.is_zero() {
            return Err(MathError::DivideByZero);
        }
        Ok(Self(&self.0 / &other.0))
    }

    /// Go `SafeMod` / `Mod`: Euclidean modulus (`0 <= r < |other|`), as Go's
    /// `big.Int.Mod`.
    pub fn checked_mod(&self, other: &Self) -> Result<Self> {
        if other.is_zero() {
            return Err(MathError::DivideByZero);
        }
        Ok(Self(euclid_mod(&self.0, &other.0)))
    }

    /// Go `AddRaw`.
    pub fn checked_add_raw(&self, other: i64) -> Result<Self> {
        self.checked_add(&Self::from_i64(other))
    }

    /// Go `SubRaw`.
    pub fn checked_sub_raw(&self, other: i64) -> Result<Self> {
        self.checked_sub(&Self::from_i64(other))
    }

    /// Go `MulRaw`.
    pub fn checked_mul_raw(&self, other: i64) -> Result<Self> {
        self.checked_mul(&Self::from_i64(other))
    }

    /// Go `QuoRaw`.
    pub fn checked_quo_raw(&self, other: i64) -> Result<Self> {
        self.checked_quo(&Self::from_i64(other))
    }

    /// Go `ModRaw`.
    pub fn checked_mod_raw(&self, other: i64) -> Result<Self> {
        self.checked_mod(&Self::from_i64(other))
    }

    /// Go `Neg` (the range is symmetric, so this cannot overflow).
    #[must_use]
    pub fn neg(&self) -> Self {
        Self(-&self.0)
    }

    /// Go `Abs`.
    #[must_use]
    pub fn abs(&self) -> Self {
        Self(self.0.abs())
    }

    /// Gogoproto `customtype` binary form (Go `Marshal`): the decimal ASCII
    /// representation (`big.Int.MarshalText`), `"0"` for zero.
    #[must_use]
    pub fn marshal(&self) -> Vec<u8> {
        self.0.to_string().into_bytes()
    }

    /// Decodes the gogoproto `customtype` binary form (Go `Unmarshal`).
    ///
    /// Like Go this parses with `big.Int.UnmarshalText` (Go literal syntax)
    /// and enforces the 256-bit bound. An empty slice yields zero (Go leaves
    /// the value untouched, i.e. nil, which marshals identically).
    pub fn unmarshal(data: &[u8]) -> Result<Self> {
        if data.is_empty() {
            return Ok(Self::zero());
        }
        let i = parse_go_big_int(data, Base::Auto)
            .ok_or_else(|| MathError::InvalidInt(String::from_utf8_lossy(data).into_owned()))?;
        Self::from_bigint(i)
    }

    /// The bit length of a non-empty `data` that [`Int::unmarshal`] rejects
    /// as out of range (Go's `integer out of range; got: %d, max: 256`);
    /// `None` when it parses within range or does not parse.
    #[must_use]
    pub fn unmarshal_overflow_bits(data: &[u8]) -> Option<u64> {
        let i = parse_go_big_int(data, Base::Auto)?;
        big_int_overflows(&i).then(|| i.bits())
    }

    /// JSON encoding (Go `MarshalJSON`): the decimal string as a JSON string.
    #[must_use]
    pub fn to_json(&self) -> String {
        to_json_string(&self.to_string())
    }

    /// JSON decoding (Go `UnmarshalJSON`): the input must be a JSON string
    /// holding a Go integer literal within range.
    pub fn from_json(bz: &[u8]) -> Result<Self> {
        Self::parse(&json_string(bz)?)
    }
}

/// Go `big.Int.Mod`: Euclidean modulus.
fn euclid_mod(x: &BigInt, y: &BigInt) -> BigInt {
    let m = x % y; // truncated, sign of x
    if m.is_negative() { m + y.abs() } else { m }
}

impl fmt::Display for Int {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl FromStr for Int {
    type Err = MathError;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

impl TryFrom<BigInt> for Int {
    type Error = MathError;

    fn try_from(i: BigInt) -> Result<Self> {
        Self::from_bigint(i)
    }
}

impl From<i64> for Int {
    fn from(n: i64) -> Self {
        Self::from_i64(n)
    }
}

impl From<u64> for Int {
    fn from(n: u64) -> Self {
        Self::from_u64(n)
    }
}

impl serde::Serialize for Int {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for Int {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}
