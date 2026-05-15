//! CometBFT validator and validator-set types.

use crate::error::{Error, Result};
use crate::hash::{Address, address_from_bytes};
use crate::proto;
use crypto::{ed25519, secp256k1};
use prost::Message;

/// CometBFT public key variants used by validator and consensus types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicKey {
    /// Ed25519 consensus public key.
    Ed25519(ed25519::PublicKeyBytes),
    /// secp256k1 account public key.
    Secp256k1(secp256k1::PublicKeyBytes),
}

impl PublicKey {
    /// Return the CometBFT address for this public key.
    #[must_use]
    pub fn address(&self) -> Address {
        match self {
            Self::Ed25519(public_key) => ed25519::address(public_key),
            Self::Secp256k1(public_key) => secp256k1::address(public_key),
        }
    }

    /// Verify a signature with this public key.
    #[must_use]
    pub fn verify_signature(&self, msg: &[u8], signature: &[u8]) -> bool {
        match self {
            Self::Ed25519(public_key) => signature
                .try_into()
                .is_ok_and(|signature| ed25519::verify(public_key, msg, &signature)),
            Self::Secp256k1(public_key) => signature
                .try_into()
                .is_ok_and(|signature| secp256k1::verify(public_key, msg, &signature)),
        }
    }

    /// Convert a protobuf public key into a domain public key.
    pub fn from_proto(proto: proto::crypto::PublicKey) -> Result<Self> {
        let Some(sum) = proto.sum else {
            return Err(Error::InvalidPublicKey);
        };
        match sum {
            proto::crypto::public_key::Sum::Ed25519(bytes) => {
                let public_key = bytes.try_into().map_err(|_| Error::InvalidPublicKey)?;
                Ok(Self::Ed25519(public_key))
            }
            proto::crypto::public_key::Sum::Secp256k1(bytes) => {
                let public_key = bytes.try_into().map_err(|_| Error::InvalidPublicKey)?;
                Ok(Self::Secp256k1(public_key))
            }
        }
    }

    /// Convert this public key to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::crypto::PublicKey {
        let sum = match self {
            Self::Ed25519(public_key) => {
                proto::crypto::public_key::Sum::Ed25519(public_key.to_vec())
            }
            Self::Secp256k1(public_key) => {
                proto::crypto::public_key::Sum::Secp256k1(public_key.to_vec())
            }
        };
        proto::crypto::PublicKey { sum: Some(sum) }
    }
}

/// A CometBFT validator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validator {
    /// Validator address.
    pub address: Address,
    /// Validator public key.
    pub public_key: PublicKey,
    /// Validator voting power.
    pub voting_power: i64,
    /// CometBFT proposer priority.
    pub proposer_priority: i64,
}

impl Validator {
    /// Create a new validator from a public key and voting power.
    #[must_use]
    pub fn new(public_key: PublicKey, voting_power: i64) -> Self {
        Self {
            address: public_key.address(),
            public_key,
            voting_power,
            proposer_priority: 0,
        }
    }

    /// Perform CometBFT stateless validator validation.
    pub fn validate_basic(&self) -> Result<()> {
        if self.voting_power < 0 {
            return Err(Error::InvalidValidator);
        }
        if self.address != self.public_key.address() {
            return Err(Error::InvalidValidator);
        }
        Ok(())
    }

    /// Return the consensus bytes CometBFT hashes for validator sets.
    pub fn bytes(&self) -> Result<Vec<u8>> {
        let proto = proto::types::SimpleValidator {
            pub_key: Some(self.public_key.to_proto()),
            voting_power: self.voting_power,
        };
        Ok(proto.encode_to_vec())
    }

    /// Convert a protobuf validator into a domain validator.
    pub fn from_proto(proto: proto::types::Validator) -> Result<Self> {
        let public_key = PublicKey::from_proto(proto.pub_key.ok_or(Error::InvalidPublicKey)?)?;
        let validator = Self {
            address: address_from_bytes(&proto.address)?,
            public_key,
            voting_power: proto.voting_power,
            proposer_priority: proto.proposer_priority,
        };
        validator.validate_basic()?;
        Ok(validator)
    }

    /// Convert this validator to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::types::Validator {
        proto::types::Validator {
            address: self.address.to_vec(),
            pub_key: Some(self.public_key.to_proto()),
            voting_power: self.voting_power,
            proposer_priority: self.proposer_priority,
        }
    }
}

/// A CometBFT validator set sorted by address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorSet {
    /// Validators sorted by address.
    pub validators: Vec<Validator>,
    /// Current proposer, when known.
    pub proposer: Option<Validator>,
}

impl ValidatorSet {
    /// Create a validator set sorted by validator address.
    pub fn new(mut validators: Vec<Validator>) -> Result<Self> {
        validators.sort_by(|left, right| left.address.cmp(&right.address));
        for validator in &validators {
            validator.validate_basic()?;
        }
        if validators
            .windows(2)
            .any(|window| window[0].address == window[1].address)
        {
            return Err(Error::InvalidValidatorSet);
        }
        let proposer = validators.first().cloned();
        Ok(Self {
            validators,
            proposer,
        })
    }

    /// Perform CometBFT stateless validator-set validation.
    pub fn validate_basic(&self) -> Result<()> {
        Self::new(self.validators.clone()).map(|_| ())
    }

    /// Return CometBFT's validator-set hash.
    pub fn hash(&self) -> Result<crate::Hash> {
        let bytes = self
            .validators
            .iter()
            .map(Validator::bytes)
            .collect::<Result<Vec<_>>>()?;
        Ok(merkle::root_from_byte_slices(&bytes))
    }

    /// Return the total voting power.
    pub fn total_voting_power(&self) -> Result<i64> {
        self.validators.iter().try_fold(0i64, |total, validator| {
            total
                .checked_add(validator.voting_power)
                .ok_or(Error::InvalidValidatorSet)
        })
    }

    /// Convert a protobuf validator set into a domain validator set.
    pub fn from_proto(proto: proto::types::ValidatorSet) -> Result<Self> {
        let validators = proto
            .validators
            .into_iter()
            .map(Validator::from_proto)
            .collect::<Result<Vec<_>>>()?;
        let mut set = Self::new(validators)?;
        set.proposer = proto.proposer.map(Validator::from_proto).transpose()?;
        Ok(set)
    }

    /// Convert this validator set to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::types::ValidatorSet {
        proto::types::ValidatorSet {
            validators: self.validators.iter().map(Validator::to_proto).collect(),
            proposer: self.proposer.as_ref().map(Validator::to_proto),
            total_voting_power: self.total_voting_power().unwrap_or_default(),
        }
    }
}
