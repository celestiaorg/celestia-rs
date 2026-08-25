//! Cosmos SDK `x/staking` validator and delegation storage.
//!
//! See `docs/superpowers/specs/m4/staking.md` for the API contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use cosmos_types::{AccountAddress, ValoperAddress};
use num_bigint::BigUint;
use prost::Message;

/// Cosmos SDK mounted store name for `x/staking`.
pub const STORE_KEY: &str = "staking";
/// Celestia's staking denomination.
pub const BOND_DENOM: &str = "utia";
/// Protobuf type URL handled by [`Keeper::execute_message`] for delegations.
pub const MSG_DELEGATE_TYPE_URL: &str = "/cosmos.staking.v1beta1.MsgDelegate";
/// Cosmos SDK module account receiving tokens delegated to bonded validators.
pub const BONDED_POOL_NAME: &str = "bonded_tokens_pool";
/// Cosmos SDK module account receiving tokens delegated to non-bonded validators.
pub const NOT_BONDED_POOL_NAME: &str = "not_bonded_tokens_pool";

const VALIDATORS_PREFIX: u8 = 0x21;
const VALIDATORS_BY_POWER_PREFIX: u8 = 0x23;
const DELEGATIONS_PREFIX: u8 = 0x31;
const DELEGATIONS_BY_VALIDATOR_PREFIX: u8 = 0x71;
const ADDRESS_LEN: u8 = 20;
const DECIMAL_PRECISION: u64 = 1_000_000_000_000_000_000;
const POWER_REDUCTION: u64 = 1_000_000;
const BONDED: i32 = 3;
const UNBONDED: i32 = 1;
const UNBONDING: i32 = 2;
const MAX_SDK_INT: &str =
    "115792089237316195423570985008687907853269984665640564039457584007913129639935";

/// Result of executing one routed staking message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageResult {
    /// Ordered Cosmos SDK-compatible events emitted by the message.
    pub events: Vec<abci_types::Event>,
}

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

    /// Return the operator address stored at a validator's current power key.
    pub fn validator_by_power_index(
        &self,
        validator: &cosmos_types::proto::staking::Validator,
    ) -> Result<Option<ValoperAddress>> {
        let address = cosmos_types::valoper_address_from_bech32(&validator.operator_address)
            .map_err(|_| Error::InvalidAddress)?;
        let Some(bytes) = self.store.get(&validator_power_key(validator, &address)?)? else {
            return Ok(None);
        };
        bytes
            .try_into()
            .map(Some)
            .map_err(|_| Error::InvalidStoreKey)
    }

    /// Set a validator's power-index entry unless it is jailed.
    pub fn set_validator_by_power_index(
        &mut self,
        validator: &cosmos_types::proto::staking::Validator,
    ) -> Result<()> {
        validate_validator(validator)?;
        if validator.jailed {
            return Ok(());
        }
        let address = cosmos_types::valoper_address_from_bech32(&validator.operator_address)
            .map_err(|_| Error::InvalidAddress)?;
        self.store
            .set(validator_power_key(validator, &address)?, address.to_vec())?;
        Ok(())
    }

    /// Delete a validator's power-index entry.
    pub fn remove_validator_by_power_index(
        &mut self,
        validator: &cosmos_types::proto::staking::Validator,
    ) -> Result<()> {
        let address = cosmos_types::valoper_address_from_bech32(&validator.operator_address)
            .map_err(|_| Error::InvalidAddress)?;
        self.store
            .delete(&validator_power_key(validator, &address)?)?;
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

    /// Decode and execute a supported protobuf staking message.
    pub fn execute_message(
        &mut self,
        bank: &mut bank::Keeper<'_>,
        message: &cosmos_types::Any,
    ) -> Result<MessageResult> {
        match message.type_url.as_str() {
            MSG_DELEGATE_TYPE_URL => self.execute_delegate(bank, &message.value),
            other => Err(Error::UnsupportedMessage(other.to_owned())),
        }
    }

    fn execute_delegate(
        &mut self,
        bank: &mut bank::Keeper<'_>,
        bytes: &[u8],
    ) -> Result<MessageResult> {
        let message = cosmos_types::proto::staking::MsgDelegate::decode(bytes)
            .map_err(|_| Error::MalformedMessage)?;
        let delegator = cosmos_types::account_address_from_bech32(&message.delegator_address)
            .map_err(|_| Error::InvalidAddress)?;
        let validator_address =
            cosmos_types::valoper_address_from_bech32(&message.validator_address)
                .map_err(|_| Error::InvalidAddress)?;
        let coin = message.amount.ok_or(Error::InvalidCoin)?;
        if coin.denom != BOND_DENOM {
            return Err(Error::InvalidBondDenom);
        }
        let amount = parse_positive_uint(&coin.amount).map_err(|_| Error::InvalidCoin)?;
        let mut validator = self
            .validator(&validator_address)?
            .ok_or(Error::ValidatorNotFound)?;
        let tokens = parse_uint(&validator.tokens).map_err(|_| Error::InvalidValidator)?;
        let validator_shares =
            parse_uint(&validator.delegator_shares).map_err(|_| Error::InvalidValidator)?;
        if tokens == BigUint::from(0_u8) && validator_shares != BigUint::from(0_u8) {
            return Err(Error::InvalidExchangeRate);
        }
        let issued_shares = if validator_shares == BigUint::from(0_u8) {
            &amount * BigUint::from(DECIMAL_PRECISION)
        } else {
            let product = &validator_shares * &amount;
            if product > max_sdk_uint()? {
                return Err(Error::InvalidShares);
            }
            product / &tokens
        };
        if issued_shares == BigUint::from(0_u8) {
            return Err(Error::InvalidShares);
        }

        let old_validator = validator.clone();
        let mut delegation = self
            .delegation(&delegator, &validator_address)?
            .unwrap_or_else(|| cosmos_types::proto::staking::Delegation {
                delegator_address: message.delegator_address.clone(),
                validator_address: message.validator_address.clone(),
                shares: "0".to_owned(),
            });
        let old_shares = parse_uint(&delegation.shares).map_err(|_| Error::InvalidDelegation)?;
        validator.tokens = (&tokens + &amount).to_string();
        validator.delegator_shares = (&validator_shares + &issued_shares).to_string();
        delegation.shares = (old_shares + &issued_shares).to_string();
        validate_validator(&validator)?;
        validate_delegation_shares(&delegation.shares)?;
        let _ = validator_power_key(&validator, &validator_address)?;

        let pool_name = match validator.status {
            BONDED => BONDED_POOL_NAME,
            UNBONDED | UNBONDING => NOT_BONDED_POOL_NAME,
            _ => return Err(Error::InvalidValidatorStatus),
        };
        let pool = cosmos_types::module_address(pool_name);
        let transfer = cosmos_types::Coin::new(BOND_DENOM, coin.amount.clone())
            .map_err(|_| Error::InvalidCoin)?;
        bank.send_coins(&delegator, &pool, core::slice::from_ref(&transfer))?;

        self.remove_validator_by_power_index(&old_validator)?;
        self.set_validator(&validator)?;
        self.set_validator_by_power_index(&validator)?;
        self.set_delegation(&delegation)?;

        Ok(MessageResult {
            events: delegate_events(
                &delegator,
                &pool,
                &message.delegator_address,
                &message.validator_address,
                &coin.amount,
                &issued_shares,
            ),
        })
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
    /// A routed protobuf message was not supported by this keeper.
    #[error("unsupported staking message: {0}")]
    UnsupportedMessage(String),
    /// A supported protobuf message could not be decoded.
    #[error("malformed staking message")]
    MalformedMessage,
    /// A delegation amount was missing or invalid.
    #[error("invalid delegation coin")]
    InvalidCoin,
    /// A delegation did not use Celestia's bond denomination.
    #[error("invalid staking bond denomination")]
    InvalidBondDenom,
    /// The requested validator does not exist.
    #[error("validator not found")]
    ValidatorNotFound,
    /// The validator has an invalid bond status.
    #[error("invalid validator bond status")]
    InvalidValidatorStatus,
    /// Validator tokens and shares encode an invalid exchange rate.
    #[error("invalid validator exchange rate")]
    InvalidExchangeRate,
    /// The bank transfer for a staking transition failed.
    #[error("staking bank transfer: {0}")]
    Bank(#[from] bank::Error),
}

/// Result alias for staking keeper operations.
pub type Result<T> = core::result::Result<T, Error>;

fn validate_validator(validator: &cosmos_types::proto::staking::Validator) -> Result<()> {
    if validator.operator_address.is_empty()
        || !is_sdk_uint(&validator.tokens)
        || !is_sdk_uint(&validator.min_self_delegation)
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

fn is_sdk_uint(value: &str) -> bool {
    is_canonical_integer(value)
        && (value.len() < MAX_SDK_INT.len()
            || (value.len() == MAX_SDK_INT.len() && value <= MAX_SDK_INT))
}

fn parse_uint(value: &str) -> Result<BigUint> {
    if !is_canonical_integer(value) {
        return Err(Error::InvalidValidator);
    }
    let number = value
        .parse::<BigUint>()
        .map_err(|_| Error::InvalidValidator)?;
    if number > max_sdk_uint()? {
        return Err(Error::InvalidValidator);
    }
    Ok(number)
}

fn max_sdk_uint() -> Result<BigUint> {
    MAX_SDK_INT
        .parse::<BigUint>()
        .map_err(|_| Error::InvalidValidator)
}

fn parse_positive_uint(value: &str) -> Result<BigUint> {
    let number = parse_uint(value)?;
    if number == BigUint::from(0_u8) {
        return Err(Error::InvalidCoin);
    }
    Ok(number)
}

fn validate_shares(shares: &str) -> Result<()> {
    if !is_sdk_uint(shares) {
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

fn validator_power_key(
    validator: &cosmos_types::proto::staking::Validator,
    address: &ValoperAddress,
) -> Result<Vec<u8>> {
    let tokens = parse_uint(&validator.tokens)?;
    let power = tokens / BigUint::from(POWER_REDUCTION);
    let power = power.to_u64_digits().first().copied().unwrap_or(0);
    let mut key = Vec::with_capacity(10 + address.len());
    key.push(VALIDATORS_BY_POWER_PREFIX);
    key.extend_from_slice(&power.to_be_bytes());
    key.push(ADDRESS_LEN);
    key.extend(address.iter().map(|byte| !byte));
    Ok(key)
}

fn delegate_events(
    delegator: &AccountAddress,
    pool: &AccountAddress,
    delegator_bech32: &str,
    validator_bech32: &str,
    amount: &str,
    issued_shares: &BigUint,
) -> Vec<abci_types::Event> {
    let coin = format!("{amount}{BOND_DENOM}");
    let delegator_account = cosmos_types::account_address_to_bech32(delegator);
    let pool_account = cosmos_types::account_address_to_bech32(pool);
    vec![
        event(
            "coin_spent",
            [("spender", &delegator_account), ("amount", &coin)],
        ),
        event(
            "coin_received",
            [("receiver", &pool_account), ("amount", &coin)],
        ),
        event(
            "delegate",
            [
                ("validator", validator_bech32),
                ("delegator", delegator_bech32),
                ("amount", &coin),
                ("new_shares", &format_decimal(issued_shares)),
            ],
        ),
    ]
}

fn event<const N: usize>(ty: &str, attributes: [(&str, &str); N]) -> abci_types::Event {
    abci_types::Event {
        ty: ty.to_owned(),
        attributes: attributes
            .into_iter()
            .map(|(key, value)| abci_types::EventAttribute {
                key: key.to_owned(),
                value: value.to_owned(),
                index: false,
            })
            .collect(),
    }
}

fn format_decimal(atomics: &BigUint) -> String {
    let digits = atomics.to_string();
    if digits.len() <= 18 {
        return format!("0.{digits:0>18}");
    }
    let split = digits.len() - 18;
    format!("{}.{}", &digits[..split], &digits[split..])
}
