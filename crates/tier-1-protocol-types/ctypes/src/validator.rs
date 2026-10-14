//! `Validator` (`types/validator.go`).

use crate::error::{Error, Result};
use crate::gofmt::hex_upper;
use crate::proto::types as pb;
use crate::pubkey::PubKey;
use prost::Message;

/// A validator with its (volatile) proposer priority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validator {
    /// Address (normally derived from the public key).
    pub address: Vec<u8>,
    /// Public key.
    pub pub_key: PubKey,
    /// Voting power.
    pub voting_power: i64,
    /// Proposer priority.
    pub proposer_priority: i64,
}

impl Validator {
    /// `NewValidator`.
    #[must_use]
    pub fn new(pub_key: PubKey, voting_power: i64) -> Self {
        Self {
            address: pub_key.address(),
            pub_key,
            voting_power,
            proposer_priority: 0,
        }
    }

    /// `Validator.ValidateBasic`.
    pub fn validate_basic(&self) -> Result<()> {
        if self.voting_power < 0 {
            return Err(Error::msg("validator has negative voting power"));
        }
        let addr = self.pub_key.address();
        if self.address != addr {
            return Err(Error::msg(format!(
                "validator address is incorrectly derived from pubkey. Exp: {}, got {}",
                hex_upper(&addr),
                hex_upper(&self.address)
            )));
        }
        Ok(())
    }

    /// `Validator.CompareProposerPriority`: true when `self` wins over
    /// `other` (higher priority, then lower address). Go panics when the
    /// addresses are identical; this then prefers `self`.
    #[must_use]
    pub fn wins_over(&self, other: &Self) -> bool {
        match self.proposer_priority.cmp(&other.proposer_priority) {
            core::cmp::Ordering::Greater => true,
            core::cmp::Ordering::Less => false,
            core::cmp::Ordering::Equal => self.address <= other.address,
        }
    }

    /// `Validator.Bytes`: the `SimpleValidator` encoding hashed into the
    /// validators hash.
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        pb::SimpleValidator {
            pub_key: Some(self.pub_key.to_proto()),
            voting_power: self.voting_power,
        }
        .encode_to_vec()
    }

    /// `Validator.ToProto`.
    #[must_use]
    pub fn to_proto(&self) -> pb::Validator {
        pb::Validator {
            address: self.address.clone(),
            pub_key: Some(self.pub_key.to_proto()),
            voting_power: self.voting_power,
            proposer_priority: self.proposer_priority,
        }
    }

    /// `ValidatorFromProto` (the address is taken as given).
    pub fn from_proto(vp: &pb::Validator) -> Result<Self> {
        let pub_key = PubKey::from_proto(&vp.pub_key.clone().unwrap_or_default())?;
        Ok(Self {
            address: vp.address.clone(),
            pub_key,
            voting_power: vp.voting_power,
            proposer_priority: vp.proposer_priority,
        })
    }
}

impl core::fmt::Display for Validator {
    /// `Validator.String`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "Validator{{{} {} VP:{} A:{}}}",
            hex_upper(&self.address),
            self.pub_key,
            self.voting_power,
            self.proposer_priority
        )
    }
}

/// Go's `%v` of a `[]*Validator`.
#[must_use]
pub fn validator_list_go(vals: &[Validator]) -> String {
    let items: Vec<String> = vals.iter().map(ToString::to_string).collect();
    format!("[{}]", items.join(" "))
}

/// `ValidatorListString`.
#[must_use]
pub fn validator_list_string(vals: &[Validator]) -> String {
    vals.iter()
        .map(|v| format!("{}:{}", hex_upper(&v.address), v.voting_power))
        .collect::<Vec<_>>()
        .join(",")
}
