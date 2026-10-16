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

/// Go's recovered nil-pointer panic: a method of a nil `math.Int` (a coin
/// whose `amount` field was empty or absent) dereferences it.
fn nil_amount_panic() -> crate::SdkError {
    crate::error::go_panic("runtime error: invalid memory address or nil pointer dereference")
}

/// `coin.IsPositive()` on the coin as Go decoded it (`""` is a nil amount:
/// `Err` is Go's nil-pointer panic).
pub fn raw_is_positive(c: &pb::Coin) -> Result<bool> {
    if c.amount.is_empty() {
        return Err(nil_amount_panic());
    }
    Ok(Amount::parse(&c.amount).is_ok_and(|a| a.is_positive()))
}

/// `coin.IsNegative()` on the coin as Go decoded it (`Err`: the panic of a
/// nil amount).
pub fn raw_is_negative(c: &pb::Coin) -> Result<bool> {
    if c.amount.is_empty() {
        return Err(nil_amount_panic());
    }
    Ok(Amount::parse(&c.amount).is_ok_and(|a| a.is_negative()))
}

/// `sdk.Coins.Validate()` of coins as Go decoded them: the inner `Err` is
/// Go's error text, the outer one the panic Go raises on a nil amount it
/// reaches.
pub fn raw_coins_validate(coins: &[pb::Coin]) -> Result<std::result::Result<(), String>> {
    let Some((first, rest)) = coins.split_first() else {
        return Ok(Ok(()));
    };
    if !is_valid_denom(&first.denom) {
        return Ok(Err(format!("invalid denom: {}", first.denom)));
    }
    if !raw_is_positive(first)? {
        return Ok(Err(format!(
            "coin {} amount is not positive",
            raw_coins_string(std::slice::from_ref(first))
        )));
    }
    let mut low = first.denom.as_str();
    for c in rest {
        if !is_valid_denom(&c.denom) {
            return Ok(Err(format!("invalid denom: {}", c.denom)));
        }
        if c.denom.as_str() < low {
            return Ok(Err(format!("denomination {} is not sorted", c.denom)));
        }
        if c.denom == low {
            return Ok(Err(format!("duplicate denomination {}", c.denom)));
        }
        if !raw_is_positive(c)? {
            return Ok(Err(format!("coin {} amount is not positive", c.denom)));
        }
        low = &c.denom;
    }
    Ok(Ok(()))
}

/// `sdk.Coins.IsValid()` of coins as Go decoded them (unsanitized, a nil
/// amount for an empty `amount`): `Err` is the panic Go raises on a nil
/// amount it reaches.
pub fn raw_coins_is_valid(coins: &[pb::Coin]) -> Result<bool> {
    let Some((first, rest)) = coins.split_first() else {
        return Ok(true);
    };
    if !is_valid_denom(&first.denom) || !raw_is_positive(first)? {
        return Ok(false);
    }
    let mut low = first.denom.as_str();
    for c in rest {
        if !is_valid_denom(&c.denom) || c.denom.as_str() <= low || !raw_is_positive(c)? {
            return Ok(false);
        }
        low = &c.denom;
    }
    Ok(true)
}

/// `sdk.Coins.String()` of coins as Go decoded them (`<nil>` for a nil
/// amount).
#[must_use]
pub fn raw_coins_string(coins: &[pb::Coin]) -> String {
    coins
        .iter()
        .map(|c| {
            let amount = if c.amount.is_empty() {
                "<nil>".to_owned()
            } else {
                Amount::parse(&c.amount).map_or_else(|_| c.amount.clone(), |a| a.to_string())
            };
            format!("{amount}{}", c.denom)
        })
        .collect::<Vec<_>>()
        .join(",")
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

    /// Found by the execution differential (`fuzz-diff exec`): Go validates
    /// the decoded coins as they are, and a nil amount panics once reached.
    #[test]
    fn raw_coins_like_go() {
        let c = |d: &str, a: &str| pb::Coin {
            denom: gostr::from_go_bytes(d.as_bytes()).into_owned(),
            amount: a.into(),
        };
        assert!(raw_coins_is_valid(&[]).unwrap_or(false));
        assert!(raw_coins_is_valid(&[c("atom", "1"), c("utia", "2")]).unwrap_or(false));
        // unsorted, zero, invalid denom: invalid (no panic)
        assert!(!raw_coins_is_valid(&[c("utia", "2"), c("atom", "1")]).unwrap_or(true));
        assert!(!raw_coins_is_valid(&[c("utia", "0")]).unwrap_or(true));
        assert!(!raw_coins_is_valid(&[c("u", "")]).unwrap_or(true));
        // nil amount reached: panic (ErrPanic)
        assert_eq!(
            raw_coins_is_valid(&[c("utia", "")]).map_err(|e| e.code),
            Err(111_222)
        );
        let bad = pb::Coin {
            denom: gostr::from_go_bytes(b"x\xff").into_owned(),
            amount: "007".into(),
        };
        assert_eq!(
            gostr::to_go_bytes(&raw_coins_string(&[c("utia", ""), bad])).as_ref(),
            b"<nil>utia,7x\xff"
        );
    }

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
