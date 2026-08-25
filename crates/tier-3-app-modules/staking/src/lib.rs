//! Cosmos SDK `x/staking` validator and delegation storage.
//!
//! See `docs/superpowers/specs/m4/staking.md` for the API contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use cosmos_types::{AccountAddress, ValoperAddress};
use prost::Message;

/// Cosmos SDK mounted store name for `x/staking`.
pub const STORE_KEY: &str = "staking";

const VALIDATORS_PREFIX: u8 = 0x21;
const DELEGATIONS_PREFIX: u8 = 0x31;
const DELEGATIONS_BY_VALIDATOR_PREFIX: u8 = 0x71;
const ADDRESS_LEN: u8 = 20;

/// Store-backed `x/staking` keeper.
#[derive(Debug)]
pub struct Keeper<'a> {
    store: &'a mut kvstore::Store,
}

impl<'a> Keeper<'a> {
    /// Create a keeper over the mounted `staking` KV store.
    #[must_use]
    pub const fn new(store: &'a mut kvstore::Store) -> Self {
        Self { store }
    }

    /// Return a validator primary record when present.
    pub fn validator(
        &self,
        address: &ValoperAddress,
    ) -> Result<Option<cosmos_types::proto::staking::Validator>> {
        self.store
            .get(&validator_key(address))?
            .map(|bytes| decode_validator(&bytes, address))
            .transpose()
    }

    /// Set a validator primary record under its operator address.
    pub fn set_validator(
        &mut self,
        validator: &cosmos_types::proto::staking::Validator,
    ) -> Result<()> {
        let address = cosmos_types::valoper_address_from_bech32(&validator.operator_address)
            .map_err(|_| Error::InvalidAddress)?;
        validate_validator(validator)?;
        self.store
            .set(validator_key(&address), validator.encode_to_vec())?;
        Ok(())
    }

    /// Delete a validator primary record.
    pub fn remove_validator(&mut self, address: &ValoperAddress) -> Result<()> {
        self.store.delete(&validator_key(address))?;
        Ok(())
    }

    /// Return a delegation when present.
    pub fn delegation(
        &self,
        delegator: &AccountAddress,
        validator: &ValoperAddress,
    ) -> Result<Option<cosmos_types::proto::staking::Delegation>> {
        self.store
            .get(&delegation_key(delegator, validator))?
            .map(|bytes| decode_delegation(&bytes, delegator, validator))
            .transpose()
    }

    /// Set a delegation primary record and validator reverse index.
    pub fn set_delegation(
        &mut self,
        delegation: &cosmos_types::proto::staking::Delegation,
    ) -> Result<()> {
        let delegator = cosmos_types::account_address_from_bech32(&delegation.delegator_address)
            .map_err(|_| Error::InvalidAddress)?;
        let validator = cosmos_types::valoper_address_from_bech32(&delegation.validator_address)
            .map_err(|_| Error::InvalidAddress)?;
        validate_delegation_shares(&delegation.shares)?;
        self.store.set(
            delegation_by_validator_key(&validator, &delegator),
            Vec::new(),
        )?;
        self.store.set(
            delegation_key(&delegator, &validator),
            delegation.encode_to_vec(),
        )?;
        Ok(())
    }

    /// Delete a delegation primary record and validator reverse index.
    pub fn remove_delegation(
        &mut self,
        delegator: &AccountAddress,
        validator: &ValoperAddress,
    ) -> Result<()> {
        self.store
            .delete(&delegation_by_validator_key(validator, delegator))?;
        self.store.delete(&delegation_key(delegator, validator))?;
        Ok(())
    }

    /// Return delegations for a delegator in validator-address byte order.
    pub fn delegator_delegations(
        &self,
        delegator: &AccountAddress,
    ) -> Result<Vec<cosmos_types::proto::staking::Delegation>> {
        let prefix = delegator_delegations_prefix(delegator);
        let end = kvstore::prefix_end_bytes(&prefix).ok_or(Error::InvalidStoreKey)?;
        self.store
            .iter(Some(&prefix), Some(&end))?
            .map(|entry| {
                let suffix = entry
                    .key
                    .strip_prefix(prefix.as_slice())
                    .ok_or(Error::InvalidStoreKey)?;
                let (&length, bytes) = suffix.split_first().ok_or(Error::InvalidStoreKey)?;
                if length != ADDRESS_LEN {
                    return Err(Error::InvalidStoreKey);
                }
                let validator: ValoperAddress =
                    bytes.try_into().map_err(|_| Error::InvalidStoreKey)?;
                decode_delegation(&entry.value, delegator, &validator)
            })
            .collect()
    }

    /// Return delegations for a validator in delegator-address byte order.
    pub fn validator_delegations(
        &self,
        validator: &ValoperAddress,
    ) -> Result<Vec<cosmos_types::proto::staking::Delegation>> {
        let prefix = validator_delegations_prefix(validator);
        let end = kvstore::prefix_end_bytes(&prefix).ok_or(Error::InvalidStoreKey)?;
        self.store
            .iter(Some(&prefix), Some(&end))?
            .map(|entry| {
                let delegator: AccountAddress = entry
                    .key
                    .strip_prefix(prefix.as_slice())
                    .ok_or(Error::InvalidStoreKey)?
                    .try_into()
                    .map_err(|_| Error::InvalidStoreKey)?;
                self.delegation(&delegator, validator)?
                    .ok_or(Error::MissingDelegation)
            })
            .collect()
    }
}

/// Errors returned by the staking keeper.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Underlying key/value store operation failed.
    #[error("staking store: {0}")]
    Store(#[from] kvstore::Error),
    /// An account or validator address was invalid.
    #[error("invalid staking address")]
    InvalidAddress,
    /// A stored validator protobuf or its contents were invalid.
    #[error("invalid validator")]
    InvalidValidator,
    /// A stored delegation protobuf or its contents were invalid.
    #[error("invalid delegation")]
    InvalidDelegation,
    /// Delegation shares were not canonical positive SDK decimal atomics.
    #[error("invalid delegation share atomics")]
    InvalidShares,
    /// Stored staking key bytes were malformed.
    #[error("invalid staking store key")]
    InvalidStoreKey,
    /// A reverse index referred to an absent delegation.
    #[error("missing indexed delegation")]
    MissingDelegation,
}

/// Result alias for staking keeper operations.
pub type Result<T> = core::result::Result<T, Error>;

fn validate_validator(validator: &cosmos_types::proto::staking::Validator) -> Result<()> {
    if validator.operator_address.is_empty()
        || !is_canonical_integer(&validator.tokens)
        || !is_canonical_integer(&validator.min_self_delegation)
        || validator.min_self_delegation == "0"
    {
        return Err(Error::InvalidValidator);
    }
    validate_shares(&validator.delegator_shares).map_err(|_| Error::InvalidValidator)
}

fn is_canonical_integer(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && (value.len() == 1 || !value.starts_with('0'))
}

fn validate_shares(shares: &str) -> Result<()> {
    if !is_canonical_integer(shares) {
        return Err(Error::InvalidShares);
    }
    Ok(())
}

fn validate_delegation_shares(shares: &str) -> Result<()> {
    validate_shares(shares)?;
    if shares == "0" {
        return Err(Error::InvalidShares);
    }
    Ok(())
}

fn decode_validator(
    bytes: &[u8],
    expected: &ValoperAddress,
) -> Result<cosmos_types::proto::staking::Validator> {
    let validator = cosmos_types::proto::staking::Validator::decode(bytes)
        .map_err(|_| Error::InvalidValidator)?;
    let actual = cosmos_types::valoper_address_from_bech32(&validator.operator_address)
        .map_err(|_| Error::InvalidValidator)?;
    if &actual != expected {
        return Err(Error::InvalidValidator);
    }
    validate_validator(&validator)?;
    Ok(validator)
}

fn decode_delegation(
    bytes: &[u8],
    expected_delegator: &AccountAddress,
    expected_validator: &ValoperAddress,
) -> Result<cosmos_types::proto::staking::Delegation> {
    let delegation = cosmos_types::proto::staking::Delegation::decode(bytes)
        .map_err(|_| Error::InvalidDelegation)?;
    let delegator = cosmos_types::account_address_from_bech32(&delegation.delegator_address)
        .map_err(|_| Error::InvalidDelegation)?;
    let validator = cosmos_types::valoper_address_from_bech32(&delegation.validator_address)
        .map_err(|_| Error::InvalidDelegation)?;
    if &delegator != expected_delegator || &validator != expected_validator {
        return Err(Error::InvalidDelegation);
    }
    validate_delegation_shares(&delegation.shares).map_err(|_| Error::InvalidDelegation)?;
    Ok(delegation)
}

fn validator_key(address: &ValoperAddress) -> Vec<u8> {
    let mut key = Vec::with_capacity(2 + address.len());
    key.extend_from_slice(&[VALIDATORS_PREFIX, ADDRESS_LEN]);
    key.extend_from_slice(address);
    key
}

fn delegator_delegations_prefix(delegator: &AccountAddress) -> Vec<u8> {
    let mut key = Vec::with_capacity(2 + delegator.len());
    key.extend_from_slice(&[DELEGATIONS_PREFIX, ADDRESS_LEN]);
    key.extend_from_slice(delegator);
    key
}

fn delegation_key(delegator: &AccountAddress, validator: &ValoperAddress) -> Vec<u8> {
    let mut key = delegator_delegations_prefix(delegator);
    key.push(ADDRESS_LEN);
    key.extend_from_slice(validator);
    key
}

fn validator_delegations_prefix(validator: &ValoperAddress) -> Vec<u8> {
    let mut key = Vec::with_capacity(2 + validator.len());
    key.extend_from_slice(&[DELEGATIONS_BY_VALIDATOR_PREFIX, ADDRESS_LEN]);
    key.extend_from_slice(validator);
    key
}

fn delegation_by_validator_key(validator: &ValoperAddress, delegator: &AccountAddress) -> Vec<u8> {
    let mut key = validator_delegations_prefix(validator);
    key.extend_from_slice(delegator);
    key
}
