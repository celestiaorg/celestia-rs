//! `sdk.Coin` / `sdk.Coins`.
//!
//! Amounts are `math::Int` (256-bit bounded, exact Go semantics).

use std::fmt;

use celestia_proto::cosmos::base::v1beta1 as pb;

use crate::error::{Result, sdk};

/// A coin amount (`math.Int`).
pub type Amount = math::Int;

/// Maps a math error to an ABCI error.
#[must_use]
pub fn math_err(e: math::MathError) -> crate::SdkError {
    sdk::invalid_coins(e.to_string())
}

/// One coin.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Coin {
    /// Denomination.
    pub denom: String,
    /// Amount.
    pub amount: Amount,
}

/// `sdk.ValidateDenom`: `[a-zA-Z][a-zA-Z0-9/:._-]{2,127}`.
#[must_use]
pub fn is_valid_denom(d: &str) -> bool {
    let b = d.as_bytes();
    (3..=128).contains(&b.len())
        && b[0].is_ascii_alphabetic()
        && b[1..]
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || b"/:._-".contains(c))
}

impl Coin {
    /// Creates a coin.
    pub fn new(denom: impl Into<String>, amount: u64) -> Self {
        Self {
            denom: denom.into(),
            amount: Amount::from_u64(amount),
        }
    }

    /// Creates a coin from an [`Amount`].
    pub fn with_amount(denom: impl Into<String>, amount: Amount) -> Self {
        Self {
            denom: denom.into(),
            amount,
        }
    }

    /// Parses a proto coin.
    pub fn from_proto(c: &pb::Coin) -> Result<Self> {
        let amount = Amount::parse(&c.amount)
            .map_err(|_| sdk::invalid_coins(format!("invalid amount {}", c.amount)))?;
        Ok(Self {
            denom: c.denom.clone(),
            amount,
        })
    }

    /// To proto.
    #[must_use]
    pub fn to_proto(&self) -> pb::Coin {
        pb::Coin {
            denom: self.denom.clone(),
            amount: self.amount.to_string(),
        }
    }

    /// `Coin.IsValid`: valid denom and non-negative amount.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        is_valid_denom(&self.denom) && !self.amount.is_negative()
    }

    /// Zero amount.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.amount.is_zero()
    }
}

impl fmt::Display for Coin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.amount, self.denom)
    }
}

/// Sorted, de-duplicated, non-zero coins (`sdk.Coins`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Coins(pub Vec<Coin>);

impl Coins {
    /// Empty.
    #[must_use]
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// From proto coins as given (no sorting; use [`Self::is_valid`]).
    pub fn from_proto(cs: &[pb::Coin]) -> Result<Self> {
        Ok(Self(
            cs.iter().map(Coin::from_proto).collect::<Result<_>>()?,
        ))
    }

    /// `sdk.NewCoins`: sanitizes (drops zeros, sorts) and validates.
    pub fn sanitized(mut v: Vec<Coin>) -> Result<Self> {
        v.retain(|c| !c.is_zero());
        v.sort_by(|a, b| a.denom.cmp(&b.denom));
        let c = Self(v);
        if !c.is_valid() {
            return Err(sdk::invalid_coins(c.to_string()));
        }
        Ok(c)
    }

    /// To proto.
    #[must_use]
    pub fn to_proto(&self) -> Vec<pb::Coin> {
        self.0.iter().map(Coin::to_proto).collect()
    }

    /// `Coins.IsValid`: strictly ascending denoms, all valid and positive.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        match self.0.len() {
            0 => true,
            1 => is_valid_denom(&self.0[0].denom) && self.0[0].amount.is_positive(),
            _ => {
                self.0
                    .iter()
                    .all(|c| is_valid_denom(&c.denom) && c.amount.is_positive())
                    && self.0.windows(2).all(|w| w[0].denom < w[1].denom)
            }
        }
    }

    /// Amount of `denom` (zero if absent).
    #[must_use]
    pub fn amount_of(&self, denom: &str) -> Amount {
        self.0
            .iter()
            .find(|c| c.denom == denom)
            .map_or_else(Amount::zero, |c| c.amount.clone())
    }

    /// True when empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterates.
    pub fn iter(&self) -> std::slice::Iter<'_, Coin> {
        self.0.iter()
    }

    /// `Coins.Add` (both sorted): merged, zeros removed.
    pub fn add(&self, other: &Self) -> Result<Self> {
        let mut out: Vec<Coin> = Vec::new();
        let (mut i, mut j) = (0, 0);
        while i < self.0.len() || j < other.0.len() {
            let next = match (self.0.get(i), other.0.get(j)) {
                (Some(a), Some(b)) if a.denom == b.denom => {
                    i += 1;
                    j += 1;
                    Coin::with_amount(
                        a.denom.clone(),
                        a.amount.checked_add(&b.amount).map_err(math_err)?,
                    )
                }
                (Some(a), Some(b)) if a.denom < b.denom => {
                    i += 1;
                    a.clone()
                }
                (Some(_) | None, Some(b)) => {
                    j += 1;
                    b.clone()
                }
                (Some(a), None) => {
                    i += 1;
                    a.clone()
                }
                (None, None) => break,
            };
            if !next.is_zero() {
                out.push(next);
            }
        }
        Ok(Self(out))
    }
}

impl<'a> IntoIterator for &'a Coins {
    type Item = &'a Coin;
    type IntoIter = std::slice::Iter<'a, Coin>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl fmt::Display for Coins {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self.0.iter().map(ToString::to_string).collect();
        f.write_str(&parts.join(","))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_add() {
        let a = Coins(vec![Coin::new("atom", 1), Coin::new("utia", 5)]);
        let b = Coins(vec![Coin::new("utia", 2), Coin::new("zzz", 3)]);
        assert_eq!(a.add(&b).unwrap().to_string(), "1atom,7utia,3zzz");
        assert!(a.is_valid());
        assert!(!Coins(vec![Coin::new("utia", 1), Coin::new("atom", 1)]).is_valid());
        assert_eq!(Coins::new().to_string(), "");
    }

    #[test]
    fn denoms() {
        assert!(is_valid_denom("utia"));
        assert!(is_valid_denom(
            "ibc/27394FB092D2ECCD56123C74F36E4C1F926001CEADA9CA97EA622B25F41E5EB2"
        ));
        assert!(!is_valid_denom("u"));
        assert!(!is_valid_denom("1abc"));
    }
}
