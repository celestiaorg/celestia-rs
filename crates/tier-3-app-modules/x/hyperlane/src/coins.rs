//! Minimal port of `sdk.Coin` / `sdk.Coins` (cosmos-sdk `types/coin.go`)
//! covering what hyperlane uses.

use std::fmt;

use celestia_proto::cosmos::base::v1beta1::Coin as ProtoCoin;
use math::Int;

use crate::errors::panic_err;
use sdk::Result;

/// `sdk.ValidateDenom`: `[a-zA-Z][a-zA-Z0-9/:._-]{2,127}`.
pub fn validate_denom(denom: &str) -> std::result::Result<(), String> {
    let b = denom.as_bytes();
    let ok = (3..=128).contains(&b.len())
        && b[0].is_ascii_alphabetic()
        && b[1..]
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'/' | b':' | b'.' | b'_' | b'-'));
    if ok {
        Ok(())
    } else {
        Err(format!("invalid denom: {denom}"))
    }
}

/// `sdk.Coin`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Coin {
    /// Denomination.
    pub denom: String,
    /// Amount.
    pub amount: Int,
}

impl Coin {
    /// `sdk.NewCoin`: validates (Go panics on an invalid coin).
    pub fn new(denom: impl Into<String>, amount: Int) -> Result<Self> {
        let c = Self {
            denom: denom.into(),
            amount,
        };
        c.validate().map_err(panic_err)?;
        Ok(c)
    }

    /// `Coin.Validate`.
    pub fn validate(&self) -> std::result::Result<(), String> {
        validate_denom(&self.denom)?;
        if self.amount.is_negative() {
            return Err(format!("negative coin amount: {}", self.amount));
        }
        Ok(())
    }

    /// From the protobuf form (amount is the gogoproto `math.Int` customtype).
    pub fn from_proto(c: &ProtoCoin) -> Result<Self> {
        Ok(Self {
            denom: c.denom.clone(),
            amount: Int::unmarshal(c.amount.as_bytes())
                .map_err(|e| sdk::error::sdk::tx_decode(e.to_string()))?,
        })
    }

    /// To the protobuf form.
    #[must_use]
    pub fn to_proto(&self) -> ProtoCoin {
        ProtoCoin {
            denom: self.denom.clone(),
            amount: self.amount.to_string(),
        }
    }
}

impl fmt::Display for Coin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.amount, self.denom)
    }
}

/// `sdk.Coins`: sorted by denom, no zero amounts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Coins(pub Vec<Coin>);

impl Coins {
    /// `sdk.NewCoins`: removes zero coins, sorts, validates (Go panics on an
    /// invalid set).
    pub fn new(coins: impl IntoIterator<Item = Coin>) -> Result<Self> {
        let mut v: Vec<Coin> = coins.into_iter().filter(|c| !c.amount.is_zero()).collect();
        v.sort_by(|a, b| a.denom.cmp(&b.denom));
        let c = Self(v);
        c.validate()
            .map_err(|e| panic_err(format!("invalid coin set {c}: {e}")))?;
        Ok(c)
    }

    /// `Coins.Validate`.
    pub fn validate(&self) -> std::result::Result<(), String> {
        let mut prev: Option<&str> = None;
        for c in &self.0 {
            validate_denom(&c.denom)?;
            if let Some(p) = prev {
                if c.denom.as_str() < p {
                    return Err(format!("denomination {} is not sorted", c.denom));
                }
                if c.denom == p {
                    return Err(format!("duplicate denomination {}", c.denom));
                }
            }
            if !c.amount.is_positive() {
                return Err(format!("coin {} amount is not positive", c.denom));
            }
            prev = Some(&c.denom);
        }
        Ok(())
    }

    /// From protobuf coins as stored (no re-validation, like gogoproto).
    pub fn from_proto(v: &[ProtoCoin]) -> Result<Self> {
        v.iter()
            .map(Coin::from_proto)
            .collect::<Result<_>>()
            .map(Self)
    }

    /// To protobuf coins.
    #[must_use]
    pub fn to_proto(&self) -> Vec<ProtoCoin> {
        self.0.iter().map(Coin::to_proto).collect()
    }

    /// `Empty`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `IsZero`: every amount is zero (true for the empty set).
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|c| c.amount.is_zero())
    }

    /// `AmountOf`.
    #[must_use]
    pub fn amount_of(&self, denom: &str) -> Int {
        self.0
            .iter()
            .find(|c| c.denom == denom)
            .map_or_else(Int::zero, |c| c.amount.clone())
    }

    fn is_sorted(&self) -> bool {
        self.0.windows(2).all(|w| w[0].denom <= w[1].denom)
    }

    fn safe_add(&self, other: &Self) -> Result<Self> {
        if !self.is_sorted() || !other.is_sorted() {
            return Err(panic_err("Coins must be sorted"));
        }
        let mut map: std::collections::BTreeMap<&str, Int> = std::collections::BTreeMap::new();
        for c in self.0.iter().chain(&other.0) {
            let e = map.entry(c.denom.as_str()).or_insert_with(Int::zero);
            *e = e
                .checked_add(&c.amount)
                .map_err(|e| panic_err(e.to_string()))?;
        }
        Ok(Self(
            map.into_iter()
                .filter(|(_, a)| !a.is_zero())
                .map(|(d, a)| Coin {
                    denom: d.to_string(),
                    amount: a,
                })
                .collect(),
        ))
    }

    /// `Add`.
    pub fn add(&self, other: &Self) -> Result<Self> {
        self.safe_add(other)
    }

    /// `SafeSub`: returns the difference and whether any amount is negative.
    pub fn safe_sub(&self, other: &Self) -> Result<(Self, bool)> {
        let neg = Self::new(other.0.iter().cloned())?;
        let neg = Self(
            neg.0
                .into_iter()
                .map(|c| Coin {
                    denom: c.denom,
                    amount: c.amount.neg(),
                })
                .collect(),
        );
        let diff = self.safe_add(&neg)?;
        let any_neg = diff.0.iter().any(|c| c.amount.is_negative());
        Ok((diff, any_neg))
    }

    /// `IsAllGTE`.
    #[must_use]
    pub fn is_all_gte(&self, other: &Self) -> bool {
        if other.0.is_empty() {
            return true;
        }
        if self.0.is_empty() {
            return false;
        }
        other.0.iter().all(|b| b.amount <= self.amount_of(&b.denom))
    }

    /// `IsAllGT`: every denom of `other` is in `self` (`DenomsSubsetOf`)
    /// with a strictly greater amount; false for an empty `self`, true for
    /// an empty `other`.
    #[must_use]
    pub fn is_all_gt(&self, other: &Self) -> bool {
        if self.0.is_empty() {
            return false;
        }
        if other.0.is_empty() {
            return true;
        }
        if other.0.len() > self.0.len()
            || other.0.iter().any(|c| self.amount_of(&c.denom).is_zero())
        {
            return false;
        }
        other.0.iter().all(|b| self.amount_of(&b.denom) > b.amount)
    }

    /// `IsAnyGT`.
    #[must_use]
    pub fn is_any_gt(&self, other: &Self) -> bool {
        if other.0.is_empty() {
            return false;
        }
        self.0.iter().any(|c| {
            let amt = other.amount_of(&c.denom);
            c.amount > amt && !amt.is_zero()
        })
    }
}

impl fmt::Display for Coins {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, c) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(",")?;
            }
            write!(f, "{c}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(d: &str, a: i64) -> Coin {
        Coin {
            denom: d.into(),
            amount: Int::from(a),
        }
    }

    #[test]
    fn new_sorts_and_drops_zero() {
        let cs = Coins::new([c("utib", 2), c("utia", 1), c("uxyz", 0)]).unwrap();
        assert_eq!(cs.to_string(), "1utia,2utib");
        assert!(Coins::new([c("utia", 1), c("utia", 2)]).is_err());
        assert!(Coins::new([c("u", 1)]).is_err());
    }

    #[test]
    fn arithmetic() {
        let a = Coins::new([c("utia", 5)]).unwrap();
        let b = Coins::new([c("utia", 7)]).unwrap();
        assert_eq!(a.add(&b).unwrap().to_string(), "12utia");
        let (d, neg) = a.safe_sub(&b).unwrap();
        assert!(neg);
        assert_eq!(d.to_string(), "-2utia");
        let (d, neg) = b.safe_sub(&b).unwrap();
        assert!(!neg && d.is_empty());
        assert!(b.is_all_gte(&a));
        assert!(!a.is_all_gte(&b));
        assert!(b.is_any_gt(&a));
        assert!(!b.is_any_gt(&Coins::default()));
        assert!(Coins::default().is_all_gte(&Coins::default()));
        assert!(!Coins::default().is_all_gte(&a));
    }
}
