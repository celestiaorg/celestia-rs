//! `LegacyDec`: port of Go `cosmossdk.io/math.LegacyDec` (`legacy_dec.go`).

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;
use std::sync::LazyLock;

use num_bigint::{BigInt, Sign};
use num_traits::{Signed, ToPrimitive, Zero};

use crate::int::big_int_overflows;
use crate::parse::{Base, parse_go_big_int};
use crate::{Int, MathError, Result, json_string, to_json_string};

/// Number of decimal places of [`LegacyDec`] (Go `LegacyPrecision`).
pub const LEGACY_PRECISION: u32 = 18;

/// Go `maxApproxRootIterations`.
const MAX_APPROX_ROOT_ITERATIONS: usize = 300;

/// `10^18` (Go `precisionReuse`).
const PRECISION: u64 = 1_000_000_000_000_000_000;
/// `10^18 / 2` (Go `fivePrecision`).
const FIVE_PRECISION: u64 = PRECISION / 2;

fn precision() -> BigInt {
    BigInt::from(PRECISION)
}

/// `2^256 * 10^18 - 1` (Go `upperLimit`); the range is symmetric.
static UPPER_LIMIT: LazyLock<BigInt> =
    LazyLock::new(|| (BigInt::from(1) << 256usize) * precision() - 1);

/// `10^(18 - prec)` (Go `precisionMultiplier`).
fn precision_multiplier(prec: i64) -> Result<BigInt> {
    let zeros = u32::try_from(prec)
        .ok()
        .filter(|p| *p <= LEGACY_PRECISION)
        .map(|p| LEGACY_PRECISION - p)
        .ok_or(MathError::InvalidPrecision(prec))?;
    Ok(BigInt::from(10u64.pow(zeros)))
}

/// 18-decimal fixed-point number backed by an integer scaled by `10^18`,
/// matching Go `math.LegacyDec`.
///
/// Values are bounded to `±(2^256 * 10^18 - 1)` scaled units (Go
/// `IsInValidRange`). Where Go panics on overflow or division by zero the
/// `checked_*` methods return a [`MathError`].
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LegacyDec(BigInt);

impl LegacyDec {
    /// Zero (Go `LegacyZeroDec`).
    #[must_use]
    pub fn zero() -> Self {
        Self(BigInt::zero())
    }

    /// One (Go `LegacyOneDec`).
    #[must_use]
    pub fn one() -> Self {
        Self(precision())
    }

    /// `10^-18` (Go `LegacySmallestDec`).
    #[must_use]
    pub fn smallest() -> Self {
        Self(BigInt::from(1))
    }

    /// The largest representable value, `2^256 - 10^-18`.
    #[must_use]
    pub fn max_value() -> Self {
        Self(UPPER_LIMIT.clone())
    }

    /// Whole number `i` (Go `LegacyNewDec`).
    #[must_use]
    pub fn new(i: i64) -> Self {
        Self(BigInt::from(i) * precision())
    }

    /// `i * 10^-prec` (Go `LegacyNewDecWithPrec`). `prec` must be in
    /// `0..=18` (Go panics otherwise).
    pub fn new_with_prec(i: i64, prec: i64) -> Result<Self> {
        Ok(Self(BigInt::from(i) * precision_multiplier(prec)?))
    }

    /// Whole number from an [`Int`] (Go `LegacyNewDecFromInt`).
    #[must_use]
    pub fn from_int(i: &Int) -> Self {
        Self(i.as_bigint() * precision())
    }

    /// `i * 10^-prec` from an [`Int`] (Go `LegacyNewDecFromIntWithPrec`).
    pub fn from_int_with_prec(i: &Int, prec: i64) -> Result<Self> {
        Ok(Self(i.as_bigint() * precision_multiplier(prec)?))
    }

    /// `i * 10^-prec` from a [`BigInt`] (Go `LegacyNewDecFromBigIntWithPrec`),
    /// range checked.
    pub fn from_bigint_with_prec(i: &BigInt, prec: i64) -> Result<Self> {
        Self::checked(i * precision_multiplier(prec)?)
    }

    /// Wraps an already-scaled (`x 10^18`) integer, range checked.
    pub fn from_scaled(i: BigInt) -> Result<Self> {
        Self::checked(i)
    }

    /// Parses a decimal string exactly like Go `LegacyNewDecFromStr`:
    /// `(-)whole(.fraction)` with at most 18 fractional digits.
    ///
    /// Go quirks are preserved: after stripping one leading `-` the digit run
    /// is parsed with `big.Int.SetString(_, 10)`, so `"+.5"` and `"--5"` are
    /// accepted (the latter as `5`).
    pub fn from_str_go(s: &str) -> Result<Self> {
        let (neg, s) = match s.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, s),
        };
        if s.is_empty() {
            return Err(MathError::EmptyDecimalStr);
        }
        let parts: Vec<&str> = s.split('.').collect();
        let mut combined = String::from(parts[0]);
        let mut len_decs = 0usize;
        match parts.len() {
            1 => {}
            2 => {
                len_decs = parts[1].len();
                if len_decs == 0 || combined.is_empty() {
                    return Err(MathError::InvalidDecimalLength);
                }
                combined.push_str(parts[1]);
            }
            _ => return Err(MathError::InvalidDecimalStr),
        }
        let prec = LEGACY_PRECISION as usize;
        if len_decs > prec {
            return Err(MathError::ExcessPrecision(len_decs));
        }
        combined.extend(std::iter::repeat_n('0', prec - len_decs));
        let mut v = parse_go_big_int(combined.as_bytes(), Base::Ten)
            .ok_or(MathError::InvalidDecimalDigits(combined))?;
        if neg {
            v = -v;
        }
        Self::checked(v).map_err(|_| MathError::InvalidDecimalStr)
    }

    /// Borrows the underlying `10^18`-scaled integer.
    #[must_use]
    pub fn as_scaled(&self) -> &BigInt {
        &self.0
    }

    fn in_range(v: &BigInt) -> bool {
        v.magnitude() <= UPPER_LIMIT.magnitude()
    }

    /// Go `assertInValidRange`.
    fn checked(v: BigInt) -> Result<Self> {
        if Self::in_range(&v) {
            Ok(Self(v))
        } else {
            Err(MathError::IntOverflow)
        }
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

    /// Go `IsInteger`: the fractional part is zero.
    #[must_use]
    pub fn is_integer(&self) -> bool {
        (&self.0 % precision()).is_zero()
    }

    /// Go `Neg`.
    #[must_use]
    pub fn neg(&self) -> Self {
        Self(-&self.0)
    }

    /// Go `Abs`.
    #[must_use]
    pub fn abs(&self) -> Self {
        Self(self.0.abs())
    }

    /// Go `Add`.
    pub fn checked_add(&self, other: &Self) -> Result<Self> {
        Self::checked(&self.0 + &other.0)
    }

    /// Go `Sub`.
    pub fn checked_sub(&self, other: &Self) -> Result<Self> {
        Self::checked(&self.0 - &other.0)
    }

    /// Go `Mul`: banker's rounding of the 36-decimal product.
    pub fn checked_mul(&self, other: &Self) -> Result<Self> {
        Self::checked(chop_precision_and_round(&self.0 * &other.0))
    }

    /// Go `MulTruncate`: the product truncated toward zero.
    pub fn checked_mul_truncate(&self, other: &Self) -> Result<Self> {
        Self::checked(&self.0 * &other.0 / precision())
    }

    /// Go `MulRoundUp`: positive products round up (away from zero),
    /// negative products truncate (toward zero).
    pub fn checked_mul_round_up(&self, other: &Self) -> Result<Self> {
        Self::checked(chop_precision_and_round_up(&(&self.0 * &other.0)))
    }

    /// Go `MulInt`.
    pub fn checked_mul_int(&self, i: &Int) -> Result<Self> {
        Self::checked(&self.0 * i.as_bigint())
    }

    /// Go `MulInt64`.
    pub fn checked_mul_int64(&self, i: i64) -> Result<Self> {
        Self::checked(&self.0 * BigInt::from(i))
    }

    /// Go `Quo`: `self * 10^36 / other` truncated, then banker's rounding.
    pub fn checked_quo(&self, other: &Self) -> Result<Self> {
        if other.is_zero() {
            return Err(MathError::DivideByZero);
        }
        let p = precision();
        let q = &self.0 * &p * &p / &other.0;
        Self::checked(chop_precision_and_round(q))
    }

    /// Go `QuoTruncate`: quotient truncated toward zero.
    pub fn checked_quo_truncate(&self, other: &Self) -> Result<Self> {
        if other.is_zero() {
            return Err(MathError::DivideByZero);
        }
        Self::checked(&self.0 * precision() / &other.0)
    }

    /// Go `QuoRoundUp`: `self * 10^18 / other`, adding one unit when the
    /// remainder is non-zero and the result should round up. As in Go, the
    /// sign test inspects the truncated quotient (not the dividend).
    pub fn checked_quo_round_up(&self, other: &Self) -> Result<Self> {
        if other.is_zero() {
            return Err(MathError::DivideByZero);
        }
        let num = &self.0 * precision();
        let mut q = &num / &other.0;
        let rem = &num % &other.0;
        let q_neg = q.is_negative();
        let d2_neg = other.is_negative();
        if (rem.is_positive() && q_neg == d2_neg) || (rem.is_negative() && q_neg != d2_neg) {
            q += 1;
        }
        Self::checked(q)
    }

    /// Go `QuoInt`: truncated division by an integer (no range check needed).
    pub fn checked_quo_int(&self, i: &Int) -> Result<Self> {
        if i.is_zero() {
            return Err(MathError::DivideByZero);
        }
        Ok(Self(&self.0 / i.as_bigint()))
    }

    /// Go `QuoInt64`.
    pub fn checked_quo_int64(&self, i: i64) -> Result<Self> {
        if i == 0 {
            return Err(MathError::DivideByZero);
        }
        Ok(Self(&self.0 / BigInt::from(i)))
    }

    /// Go `Power`: exponentiation by squaring with banker's rounding (and a
    /// range check) after every multiplication, in Go's exact order.
    pub fn checked_power(&self, power: u64) -> Result<Self> {
        if power == 0 {
            return Ok(Self::one());
        }
        let mut d = self.clone();
        let mut tmp = Self::one();
        let mut i = power;
        while i > 1 {
            if !i.is_multiple_of(2) {
                tmp = tmp.checked_mul(&d)?;
            }
            i /= 2;
            d = d.checked_mul(&d)?;
        }
        d.checked_mul(&tmp)
    }

    /// Go `ApproxRoot`: Newton's method for the `root`-th root, returning
    /// `-(|d|.ApproxRoot())` for negative input and 1 for `root == 0`.
    /// Go's recovered overflow panics are returned as errors.
    pub fn approx_root(&self, root: u64) -> Result<Self> {
        if root == 0 {
            return Ok(Self::one());
        }
        if self.is_negative() {
            return Ok(self.neg().approx_root(root)?.neg());
        }
        let one = Self::one();
        if root == 1 || self.is_zero() || *self == one {
            return Ok(self.clone());
        }
        let smallest = Self::smallest();
        // Go converts with `int64(root)`, wrapping above i64::MAX.
        #[allow(clippy::cast_possible_wrap)]
        let root_i64 = root as i64;
        let mut guess = one;
        for _ in 0..MAX_APPROX_ROOT_ITERATIONS {
            let mut prev = guess.checked_power(root - 1)?;
            if prev.is_zero() {
                prev = smallest.clone();
            }
            let delta = self
                .checked_quo(&prev)?
                .checked_sub(&guess)?
                .checked_quo_int64(root_i64)?;
            guess = guess.checked_add(&delta)?;
            if delta.abs() <= smallest {
                break;
            }
        }
        Ok(guess)
    }

    /// Go `ApproxSqrt`.
    pub fn approx_sqrt(&self) -> Result<Self> {
        self.approx_root(2)
    }

    /// Go `Ceil`: smallest integer value `>=` self.
    pub fn ceil(&self) -> Result<Self> {
        let p = precision();
        let mut q = &self.0 / &p;
        if (&self.0 % &p).is_positive() {
            q += 1;
        }
        Self::checked(q * p)
    }

    /// Go `TruncateDec`: integer part, as a decimal.
    #[must_use]
    pub fn truncate_dec(&self) -> Self {
        Self(&self.0 / precision() * precision())
    }

    /// Go `TruncateInt`: integer part (truncated toward zero).
    #[must_use]
    pub fn truncate_int(&self) -> Int {
        // |self| < 2^256 * 10^18, so the quotient always fits in 256 bits.
        Int::from_bigint(&self.0 / precision()).unwrap_or_default()
    }

    /// Go `TruncateInt64`.
    pub fn truncate_int64(&self) -> Result<i64> {
        (&self.0 / precision())
            .to_i64()
            .ok_or(MathError::ConversionOutOfRange)
    }

    /// Go `RoundInt`: banker's rounding to an integer. Fails when the result
    /// exceeds 256 bits (Go panics).
    pub fn round_int(&self) -> Result<Int> {
        let r = chop_precision_and_round(self.0.clone());
        if big_int_overflows(&r) {
            return Err(MathError::IntOutOfRange);
        }
        Int::from_bigint(r)
    }

    /// Go `RoundInt64`.
    pub fn round_int64(&self) -> Result<i64> {
        chop_precision_and_round(self.0.clone())
            .to_i64()
            .ok_or(MathError::ConversionOutOfRange)
    }

    /// Gogoproto `customtype` binary form (Go `Marshal`): decimal ASCII of the
    /// `10^18`-scaled integer, e.g. `0.001` -> `b"1000000000000000"`.
    #[must_use]
    pub fn marshal(&self) -> Vec<u8> {
        self.0.to_string().into_bytes()
    }

    /// Decodes the gogoproto binary form (Go `Unmarshal`): parsed with
    /// `big.Int.UnmarshalText` (Go literal syntax) and range checked. An empty
    /// slice yields zero (Go leaves the value nil, which marshals identically).
    pub fn unmarshal(data: &[u8]) -> Result<Self> {
        if data.is_empty() {
            return Ok(Self::zero());
        }
        let v = parse_go_big_int(data, Base::Auto)
            .ok_or_else(|| MathError::InvalidInt(String::from_utf8_lossy(data).into_owned()))?;
        if !Self::in_range(&v) {
            return Err(MathError::DecOutOfRange);
        }
        Ok(Self(v))
    }

    /// JSON encoding (Go `MarshalJSON`): the [`Display`](fmt::Display) form as
    /// a JSON string.
    #[must_use]
    pub fn to_json(&self) -> String {
        to_json_string(&self.to_string())
    }

    /// JSON decoding (Go `UnmarshalJSON`): a JSON string parsed with
    /// [`LegacyDec::from_str_go`].
    pub fn from_json(bz: &[u8]) -> Result<Self> {
        Self::from_str_go(&json_string(bz)?)
    }

    /// Go `LegacyMinDec`.
    #[must_use]
    pub fn min_dec(a: &Self, b: &Self) -> Self {
        if a < b { a.clone() } else { b.clone() }
    }

    /// Go `LegacyMaxDec`.
    #[must_use]
    pub fn max_dec(a: &Self, b: &Self) -> Self {
        if a < b { b.clone() } else { a.clone() }
    }
}

/// Go `chopPrecisionAndRound`: divide by `10^18` with banker's rounding
/// (ties to even), symmetric around zero.
fn chop_precision_and_round(d: BigInt) -> BigInt {
    if d.is_negative() {
        return -chop_precision_and_round(-d);
    }
    let p = precision();
    let quo = &d / &p;
    let rem = &d % &p;
    if rem.is_zero() {
        return quo;
    }
    match rem.cmp(&BigInt::from(FIVE_PRECISION)) {
        Ordering::Less => quo,
        Ordering::Greater => quo + 1,
        Ordering::Equal => {
            if quo.bit(0) {
                quo + 1
            } else {
                quo
            }
        }
    }
}

/// Go `chopPrecisionAndRoundUp`: positive values round up, negative values
/// truncate toward zero.
fn chop_precision_and_round_up(d: &BigInt) -> BigInt {
    let p = precision();
    if d.is_negative() {
        return d / &p;
    }
    let quo = d / &p;
    if (d % &p).is_zero() { quo } else { quo + 1 }
}

impl fmt::Display for LegacyDec {
    /// Go `String`: always 18 fractional digits, e.g. `0.001000000000000000`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let digits = self.0.magnitude().to_string();
        let prec = LEGACY_PRECISION as usize;
        if self.is_negative() {
            f.write_str("-")?;
        }
        if digits.len() <= prec {
            f.write_str("0.")?;
            for _ in 0..prec - digits.len() {
                f.write_str("0")?;
            }
            f.write_str(&digits)
        } else {
            let (int_part, frac) = digits.split_at(digits.len() - prec);
            write!(f, "{int_part}.{frac}")
        }
    }
}

impl FromStr for LegacyDec {
    type Err = MathError;

    fn from_str(s: &str) -> Result<Self> {
        Self::from_str_go(s)
    }
}

impl From<i64> for LegacyDec {
    fn from(i: i64) -> Self {
        Self::new(i)
    }
}

impl From<&Int> for LegacyDec {
    fn from(i: &Int) -> Self {
        Self::from_int(i)
    }
}

impl serde::Serialize for LegacyDec {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for LegacyDec {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        Self::from_str_go(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::LegacyDec;

    fn d(s: &str) -> LegacyDec {
        LegacyDec::from_str_go(s).unwrap()
    }

    // Ported from Go TestDecString / TestNewDecFromStr / TestArithmetic.
    #[test]
    fn go_table_tests() {
        assert_eq!(d("0.001").to_string(), "0.001000000000000000");
        assert_eq!(d("-1.5").to_string(), "-1.500000000000000000");
        assert_eq!(d("0.001").marshal(), b"1000000000000000");
        assert_eq!(LegacyDec::zero().marshal(), b"0");
        assert!(LegacyDec::from_str_go("").is_err());
        assert!(LegacyDec::from_str_go("0.0000000000000000001").is_err());
        assert!(LegacyDec::from_str_go("1.").is_err());
        assert!(LegacyDec::from_str_go(".1").is_err());
        assert_eq!(d("--5"), d("5"));
        // banker's rounding
        let half = d("0.5");
        let tiny = LegacyDec::smallest();
        assert_eq!(half.checked_mul(&tiny).unwrap(), LegacyDec::zero());
        assert_eq!(
            d("1.5").checked_mul(&tiny).unwrap(),
            LegacyDec::from_scaled(2.into()).unwrap()
        );
        assert_eq!(
            d("-1.5").checked_mul(&tiny).unwrap(),
            LegacyDec::from_scaled((-2).into()).unwrap()
        );
        assert_eq!(d("2").approx_sqrt().unwrap(), d("1.414213562373095049"));
        assert_eq!(d("3").checked_power(2).unwrap(), d("9"));
        assert!(LegacyDec::max_value().ceil().is_err());
        assert!(LegacyDec::max_value().round_int().is_err());
    }
}
