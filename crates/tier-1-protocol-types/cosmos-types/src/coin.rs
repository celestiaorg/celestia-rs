//! Cosmos SDK coin domain type.

use crate::error::{Error, Result};
use crate::proto;

const MAX_AMOUNT_DIGITS: usize = 78;

/// Cosmos SDK coin with a canonical decimal integer amount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coin {
    /// Denomination.
    pub denom: String,
    /// Base-10 integer amount string.
    pub amount: String,
}

impl Coin {
    /// Create and validate a coin.
    pub fn new(denom: impl Into<String>, amount: impl Into<String>) -> Result<Self> {
        let coin = Self {
            denom: denom.into(),
            amount: amount.into(),
        };
        coin.validate_basic()?;
        Ok(coin)
    }

    /// Perform Cosmos SDK stateless coin validation.
    pub fn validate_basic(&self) -> Result<()> {
        validate_denom(&self.denom)?;
        validate_amount(&self.amount)?;
        Ok(())
    }

    /// Convert protobuf into a domain coin.
    pub fn from_proto(proto: proto::base::Coin) -> Result<Self> {
        Self::new(proto.denom, proto.amount)
    }

    /// Convert this coin to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::base::Coin {
        proto::base::Coin {
            denom: self.denom.clone(),
            amount: self.amount.clone(),
        }
    }
}

fn validate_denom(denom: &str) -> Result<()> {
    let mut chars = denom.chars();
    let Some(first) = chars.next() else {
        return Err(Error::InvalidDenom);
    };
    if denom.len() < 3
        || denom.len() > 128
        || !first.is_ascii_alphabetic()
        || !chars.all(is_denom_char)
    {
        return Err(Error::InvalidDenom);
    }
    Ok(())
}

fn is_denom_char(value: char) -> bool {
    value.is_ascii_alphanumeric() || matches!(value, '/' | ':' | '.' | '_' | '-')
}

fn validate_amount(amount: &str) -> Result<()> {
    if amount.is_empty()
        || amount.starts_with('-')
        || amount.len() > MAX_AMOUNT_DIGITS
        || !amount.chars().all(|value| value.is_ascii_digit())
    {
        return Err(Error::InvalidAmount);
    }
    Ok(())
}
