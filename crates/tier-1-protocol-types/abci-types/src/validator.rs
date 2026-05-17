//! ABCI validator and misbehavior types.

use crate::validation::address_from_bytes;
use crate::{Error, Result, proto};
use tendermint_types::PublicKey;
use tendermint_types::hash::Address;

/// CometBFT misbehavior variants reported through ABCI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum MisbehaviorType {
    /// Unknown misbehavior type.
    Unknown = 0,
    /// Duplicate vote evidence.
    DuplicateVote = 1,
    /// Light-client attack evidence.
    LightClientAttack = 2,
}

impl TryFrom<i32> for MisbehaviorType {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self> {
        match value {
            0 => Ok(Self::Unknown),
            1 => Ok(Self::DuplicateVote),
            2 => Ok(Self::LightClientAttack),
            _ => Err(Error::InvalidMisbehavior),
        }
    }
}

impl From<MisbehaviorType> for i32 {
    fn from(value: MisbehaviorType) -> Self {
        value as Self
    }
}

/// ABCI validator identity and voting power.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validator {
    /// Validator address.
    pub address: Address,
    /// Validator voting power.
    pub power: i64,
}

impl Validator {
    /// Perform stateless validator validation.
    pub fn validate_basic(&self) -> Result<()> {
        if self.power < 0 {
            return Err(Error::InvalidValidator);
        }
        Ok(())
    }

    /// Convert protobuf into a domain validator.
    pub fn from_proto(proto: proto::abci::Validator) -> Result<Self> {
        let validator = Self {
            address: address_from_bytes(&proto.address)?,
            power: proto.power,
        };
        validator.validate_basic()?;
        Ok(validator)
    }

    /// Convert this validator to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::abci::Validator {
        proto::abci::Validator {
            address: self.address.to_vec().into(),
            power: self.power,
        }
    }
}

/// ABCI validator-set update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorUpdate {
    /// Validator public key.
    pub public_key: PublicKey,
    /// Validator voting power.
    pub power: i64,
}

impl ValidatorUpdate {
    /// Perform stateless validator-update validation.
    pub fn validate_basic(&self) -> Result<()> {
        if self.power < 0 {
            return Err(Error::InvalidValidatorUpdate);
        }
        Ok(())
    }

    /// Convert protobuf into a domain validator update.
    pub fn from_proto(proto: proto::abci::ValidatorUpdate) -> Result<Self> {
        let public_key = PublicKey::from_proto(proto.pub_key.ok_or(Error::InvalidPublicKey)?)
            .map_err(|_| Error::InvalidPublicKey)?;
        let update = Self {
            public_key,
            power: proto.power,
        };
        update.validate_basic()?;
        Ok(update)
    }

    /// Convert this validator update to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::abci::ValidatorUpdate {
        proto::abci::ValidatorUpdate {
            pub_key: Some(self.public_key.to_proto()),
            power: self.power,
        }
    }
}

/// ABCI misbehavior descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Misbehavior {
    /// Misbehavior type.
    pub ty: MisbehaviorType,
    /// Offending validator.
    pub validator: Validator,
    /// Height when the offense occurred.
    pub height: i64,
    /// Time when the offense occurred.
    pub time: proto::google::Timestamp,
    /// Total voting power at the evidence height.
    pub total_voting_power: i64,
}

impl Misbehavior {
    /// Perform stateless misbehavior validation.
    pub fn validate_basic(&self) -> Result<()> {
        if self.height <= 0 || self.total_voting_power < 0 {
            return Err(Error::InvalidMisbehavior);
        }
        self.validator.validate_basic()?;
        Ok(())
    }

    /// Convert protobuf into a domain misbehavior descriptor.
    pub fn from_proto(proto: proto::abci::Misbehavior) -> Result<Self> {
        let misbehavior = Self {
            ty: MisbehaviorType::try_from(proto.r#type)?,
            validator: proto
                .validator
                .map(Validator::from_proto)
                .transpose()?
                .ok_or(Error::InvalidMisbehavior)?,
            height: proto.height,
            time: proto.time.ok_or(Error::InvalidMisbehavior)?,
            total_voting_power: proto.total_voting_power,
        };
        misbehavior.validate_basic()?;
        Ok(misbehavior)
    }

    /// Convert this misbehavior descriptor to protobuf.
    #[must_use]
    pub fn to_proto(&self) -> proto::abci::Misbehavior {
        proto::abci::Misbehavior {
            r#type: self.ty.into(),
            validator: Some(self.validator.to_proto()),
            height: self.height,
            time: Some(self.time),
            total_voting_power: self.total_voting_power,
        }
    }
}
