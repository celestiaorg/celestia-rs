//! `sdk.DecCoin` / `sdk.DecCoins` (`types/dec_coin.go`).
//!
//! Operations that panic in Go return an error here.

use std::fmt;

use celestia_proto::cosmos::base::v1beta1 as pb;
use math::LegacyDec;

use crate::coins::{Coin, Coins, is_valid_denom};
use crate::error::{Result, SdkError};

fn panic_err(msg: impl Into<String>) -> SdkError {
    crate::error::go_panic(msg)
}

fn m<T>(r: math::Result<T>) -> Result<T> {
    r.map_err(|e| panic_err(e.to_string()))
}

/// One decimal coin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecCoin {
    /// Denomination.
    pub denom: String,
    /// Amount.
    pub amount: LegacyDec,
}

impl DecCoin {
    /// Creates a coin.
    pub fn new(denom: impl Into<String>, amount: LegacyDec) -> Self {
        Self {
            denom: denom.into(),
            amount,
        }
    }

    /// Zero amount.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.amount.is_zero()
    }
}

impl fmt::Display for DecCoin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.amount, self.denom)
    }
}

/// Decimal coins (sorted by denom, as produced by the Go operations).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DecCoins(pub Vec<DecCoin>);

impl DecCoins {
    /// Empty set.
    #[must_use]
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// `NewDecCoinsFromCoins` (input sanitized like `NewCoins`).
    pub fn from_coins(coins: &Coins) -> Result<Self> {
        let sanitized = Coins::sanitized(coins.0.clone())?;
        Ok(Self(
            sanitized
                .0
                .iter()
                .map(|c| DecCoin::new(c.denom.clone(), LegacyDec::from_int(&c.amount)))
                .collect(),
        ))
    }

    /// Decodes from proto as stored (`LegacyDec.Marshal` amounts).
    pub fn from_proto(cs: &[pb::DecCoin]) -> Result<Self> {
        cs.iter()
            .map(|c| {
                Ok(DecCoin::new(
                    c.denom.clone(),
                    m(LegacyDec::unmarshal(c.amount.as_bytes()))?,
                ))
            })
            .collect::<Result<_>>()
            .map(Self)
    }

    /// Decodes from proto3-JSON-decoded protos (decimal-string amounts).
    pub fn from_json_proto(cs: &[pb::DecCoin]) -> Result<Self> {
        cs.iter()
            .map(|c| {
                Ok(DecCoin::new(
                    c.denom.clone(),
                    LegacyDec::from_str_go(&c.amount)
                        .map_err(|e| crate::error::sdk::json_unmarshal(e.to_string()))?,
                ))
            })
            .collect::<Result<_>>()
            .map(Self)
    }

    /// Encodes for storage (`LegacyDec.Marshal` amounts).
    #[must_use]
    pub fn to_proto(&self) -> Vec<pb::DecCoin> {
        self.0
            .iter()
            .map(|c| pb::DecCoin {
                denom: c.denom.clone(),
                amount: String::from_utf8_lossy(&c.amount.marshal()).into_owned(),
            })
            .collect()
    }

    /// True when empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `IsZero`: every amount is zero.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0.iter().all(DecCoin::is_zero)
    }

    /// `IsAnyNegative`.
    #[must_use]
    pub fn is_any_negative(&self) -> bool {
        self.0.iter().any(|c| c.amount.is_negative())
    }

    /// `AmountOf`.
    #[must_use]
    pub fn amount_of(&self, denom: &str) -> LegacyDec {
        self.0
            .iter()
            .find(|c| c.denom == denom)
            .map_or_else(LegacyDec::zero, |c| c.amount.clone())
    }

    /// `Add` (`safeAdd`): merges two sorted sets, dropping zero results.
    pub fn add(&self, other: &Self) -> Result<Self> {
        let (a, b) = (&self.0, &other.0);
        let (mut i, mut j) = (0, 0);
        let mut sum = Vec::new();
        loop {
            if i == a.len() {
                sum.extend(b[j..].iter().filter(|c| !c.is_zero()).cloned());
                return Ok(Self(sum));
            }
            if j == b.len() {
                sum.extend(a[i..].iter().filter(|c| !c.is_zero()).cloned());
                return Ok(Self(sum));
            }
            let (ca, cb) = (&a[i], &b[j]);
            match ca.denom.cmp(&cb.denom) {
                std::cmp::Ordering::Less => {
                    if !ca.is_zero() {
                        sum.push(ca.clone());
                    }
                    i += 1;
                }
                std::cmp::Ordering::Equal => {
                    let res = DecCoin::new(ca.denom.clone(), m(ca.amount.checked_add(&cb.amount))?);
                    if !res.is_zero() {
                        sum.push(res);
                    }
                    i += 1;
                    j += 1;
                }
                std::cmp::Ordering::Greater => {
                    if !cb.is_zero() {
                        sum.push(cb.clone());
                    }
                    j += 1;
                }
            }
        }
    }

    /// `SafeSub`: the difference and whether any amount is negative.
    pub fn safe_sub(&self, other: &Self) -> Result<(Self, bool)> {
        let neg = Self(
            other
                .0
                .iter()
                .map(|c| DecCoin::new(c.denom.clone(), c.amount.neg()))
                .collect(),
        );
        let diff = self.add(&neg)?;
        let has_neg = diff.is_any_negative();
        Ok((diff, has_neg))
    }

    /// `Sub` (errors where Go panics on a negative result).
    pub fn sub(&self, other: &Self) -> Result<Self> {
        let (diff, neg) = self.safe_sub(other)?;
        if neg {
            return Err(panic_err("negative coin amount"));
        }
        Ok(diff)
    }

    fn map_nonzero(&self, f: impl Fn(&LegacyDec) -> math::Result<LegacyDec>) -> Result<Self> {
        let mut res = Self::new();
        for c in &self.0 {
            let p = DecCoin::new(c.denom.clone(), m(f(&c.amount))?);
            if !p.is_zero() {
                res = res.add(&Self(vec![p]))?;
            }
        }
        Ok(res)
    }

    /// `MulDec`.
    pub fn mul_dec(&self, d: &LegacyDec) -> Result<Self> {
        self.map_nonzero(|a| a.checked_mul(d))
    }

    /// `MulDecTruncate`.
    pub fn mul_dec_truncate(&self, d: &LegacyDec) -> Result<Self> {
        if d.is_zero() {
            return Ok(Self::new());
        }
        self.map_nonzero(|a| a.checked_mul_truncate(d))
    }

    /// `QuoDecTruncate`.
    pub fn quo_dec_truncate(&self, d: &LegacyDec) -> Result<Self> {
        if d.is_zero() {
            return Err(panic_err("invalid zero decimal"));
        }
        self.map_nonzero(|a| a.checked_quo_truncate(d))
    }

    /// `Intersect`: per-denom minimum with `other`, zeros removed.
    #[must_use]
    pub fn intersect(&self, other: &Self) -> Self {
        Self(
            self.0
                .iter()
                .map(|c| {
                    DecCoin::new(
                        c.denom.clone(),
                        LegacyDec::min_dec(&c.amount, &other.amount_of(&c.denom)),
                    )
                })
                .filter(|c| !c.is_zero())
                .collect(),
        )
    }

    /// `TruncateDecimal`: integral coins and the decimal change.
    pub fn truncate_decimal(&self) -> Result<(Coins, Self)> {
        let mut truncated = Coins::new();
        let mut change = Self::new();
        for c in &self.0 {
            let t = c.amount.truncate_int();
            let ch = m(c.amount.checked_sub(&LegacyDec::from_int(&t)))?;
            if !t.is_zero() {
                truncated = truncated.add(&Coins(vec![Coin::with_amount(c.denom.clone(), t)]))?;
            }
            if !ch.is_zero() {
                change = change.add(&Self(vec![DecCoin::new(c.denom.clone(), ch)]))?;
            }
        }
        Ok((truncated, change))
    }

    /// `Validate` (as a bool).
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.0
            .iter()
            .all(|c| is_valid_denom(&c.denom) && c.amount.is_positive())
            && self.0.windows(2).all(|w| w[0].denom < w[1].denom)
    }
}

impl fmt::Display for DecCoins {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self.0.iter().map(ToString::to_string).collect();
        f.write_str(&parts.join(","))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> LegacyDec {
        LegacyDec::from_str_go(s).unwrap()
    }

    #[test]
    fn arithmetic_matches_go() {
        let a = DecCoins(vec![DecCoin::new("utia", d("10.5"))]);
        let b = DecCoins(vec![
            DecCoin::new("atom", d("1")),
            DecCoin::new("utia", d("0.5")),
        ]);
        assert_eq!(
            a.add(&b).unwrap().to_string(),
            "1.000000000000000000atom,11.000000000000000000utia"
        );
        assert!(a.sub(&b).is_err());
        let (t, ch) = a.truncate_decimal().unwrap();
        assert_eq!(t.to_string(), "10utia");
        assert_eq!(ch.to_string(), "0.500000000000000000utia");
        assert_eq!(a.intersect(&b).to_string(), "0.500000000000000000utia");
        assert_eq!(
            a.mul_dec_truncate(&d("0.98")).unwrap().to_string(),
            "10.290000000000000000utia"
        );
        assert_eq!(a.to_proto()[0].amount, "10500000000000000000");
    }
}
