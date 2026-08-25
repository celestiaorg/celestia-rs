//! Cosmos SDK `x/bank` balance storage and keeper operations.
//!
//! See `docs/superpowers/specs/m4/bank.md` for the API contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use cosmos_types::{AccountAddress, Coin};
use prost::Message;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

/// Cosmos SDK mounted store name for `x/bank`.
pub const STORE_KEY: &str = "bank";
/// Cosmos SDK fee collector module account name.
pub const FEE_COLLECTOR_NAME: &str = "fee_collector";
/// Protobuf type URL handled by [`Keeper::execute_message`] for bank sends.
pub const MSG_SEND_TYPE_URL: &str = "/cosmos.bank.v1beta1.MsgSend";
/// Protobuf type URL handled by [`Keeper::execute_message`] for bank multi-sends.
pub const MSG_MULTI_SEND_TYPE_URL: &str = "/cosmos.bank.v1beta1.MsgMultiSend";

const SUPPLY_PREFIX: u8 = 0;
const BALANCES_PREFIX: u8 = 2;
const DENOM_ADDRESS_PREFIX: u8 = 3;
const ACCOUNT_ADDRESS_LEN: u8 = 20;
const MAX_SDK_INT: &str =
    "115792089237316195423570985008687907853269984665640564039457584007913129639935";

/// One recipient and its positive, sorted bank transfer amounts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendOutput {
    /// Recipient account address.
    pub address: AccountAddress,
    /// Coins credited to the recipient.
    pub coins: Vec<Coin>,
}

/// Result of executing one routed bank message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageResult {
    /// Ordered Cosmos SDK-compatible events emitted by the message.
    pub events: Vec<abci_types::Event>,
}

/// Store-backed `x/bank` keeper for base-account balances.
#[derive(Debug)]
pub struct Keeper<'a> {
    store: &'a mut kvstore::Store,
}

impl<'a> Keeper<'a> {
    /// Create a keeper over the mounted `bank` KV store.
    #[must_use]
    pub const fn new(store: &'a mut kvstore::Store) -> Self {
        Self { store }
    }

    /// Return the fee collector module address used by ante fee deduction.
    #[must_use]
    pub fn fee_collector_address() -> AccountAddress {
        cosmos_types::module_address(FEE_COLLECTOR_NAME)
    }

    /// Return one balance, or a canonical zero coin when absent.
    pub fn balance(&self, address: &AccountAddress, denom: &str) -> Result<Coin> {
        validate_denom(denom)?;
        let amount = self
            .store
            .get(&balance_key(address, denom))?
            .map_or_else(|| Ok("0".to_owned()), |bytes| decode_amount(&bytes))?;
        Coin::new(denom, amount).map_err(|_| Error::InvalidCoin)
    }

    /// Return all non-zero balances for an account in denomination order.
    pub fn all_balances(&self, address: &AccountAddress) -> Result<Vec<Coin>> {
        let prefix = account_balance_prefix(address);
        let end = kvstore::prefix_end_bytes(&prefix).ok_or(Error::InvalidStoreKey)?;
        self.store
            .iter(Some(&prefix), Some(&end))?
            .map(|entry| {
                let denom = entry
                    .key
                    .strip_prefix(prefix.as_slice())
                    .and_then(|bytes| core::str::from_utf8(bytes).ok())
                    .ok_or(Error::InvalidStoreKey)?;
                let amount = decode_amount(&entry.value)?;
                Coin::new(denom, amount).map_err(|_| Error::InvalidCoin)
            })
            .collect()
    }

    /// Set one balance, deleting the primary value and reverse index when zero.
    pub fn set_balance(&mut self, address: &AccountAddress, coin: &Coin) -> Result<()> {
        coin.validate_basic().map_err(|_| Error::InvalidCoin)?;
        let amount = normalize_amount(&coin.amount)?;
        let primary_key = balance_key(address, &coin.denom);
        let index_key = denom_address_index_key(&coin.denom, address);
        if amount == "0" {
            self.store.delete(&index_key)?;
            self.store.delete(&primary_key)?;
        } else {
            self.store.set(index_key, Vec::new())?;
            self.store.set(primary_key, amount.into_bytes())?;
        }
        Ok(())
    }

    /// Return the total supply for a denomination, or zero when absent.
    pub fn supply(&self, denom: &str) -> Result<Coin> {
        validate_denom(denom)?;
        let amount = self
            .store
            .get(&supply_key(denom))?
            .map_or_else(|| Ok("0".to_owned()), |bytes| decode_amount(&bytes))?;
        Coin::new(denom, amount).map_err(|_| Error::InvalidCoin)
    }

    /// Set total supply, deleting zero values as required by bank invariants.
    pub fn set_supply(&mut self, coin: &Coin) -> Result<()> {
        coin.validate_basic().map_err(|_| Error::InvalidCoin)?;
        let amount = normalize_amount(&coin.amount)?;
        if amount == "0" {
            self.store.delete(&supply_key(&coin.denom))?;
        } else {
            self.store
                .set(supply_key(&coin.denom), amount.into_bytes())?;
        }
        Ok(())
    }

    /// Transfer sorted, unique positive coins without changing supply.
    pub fn send_coins(
        &mut self,
        from: &AccountAddress,
        to: &AccountAddress,
        coins: &[Coin],
    ) -> Result<()> {
        validate_coins(coins)?;
        if from == to {
            for coin in coins {
                let balance = self.balance(from, &coin.denom)?;
                let transfer = normalize_amount(&coin.amount)?;
                if compare_amounts(&balance.amount, &transfer) == Ordering::Less {
                    return Err(Error::InsufficientFunds);
                }
            }
            return Ok(());
        }
        let mut updates = Vec::with_capacity(coins.len());
        for coin in coins {
            let from_amount = self.balance(from, &coin.denom)?.amount;
            let to_amount = self.balance(to, &coin.denom)?.amount;
            let transfer = normalize_amount(&coin.amount)?;
            if compare_amounts(&from_amount, &transfer) == Ordering::Less {
                return Err(Error::InsufficientFunds);
            }
            updates.push((
                Coin::new(&coin.denom, subtract_amounts(&from_amount, &transfer)?)
                    .map_err(|_| Error::InvalidCoin)?,
                Coin::new(&coin.denom, add_amounts(&to_amount, &transfer)?)
                    .map_err(|_| Error::InvalidCoin)?,
            ));
        }
        for (debit, credit) in updates {
            self.set_balance(from, &debit)?;
            self.set_balance(to, &credit)?;
        }
        Ok(())
    }

    /// Transfer one input to one or more outputs without changing supply.
    pub fn multi_send(
        &mut self,
        from: &AccountAddress,
        input: &[Coin],
        outputs: &[SendOutput],
    ) -> Result<()> {
        validate_coins(input)?;
        if input.is_empty() || outputs.is_empty() {
            return Err(Error::InvalidInputOutput);
        }

        let mut output_totals = BTreeMap::<&str, String>::new();
        for output in outputs {
            validate_coins(&output.coins)?;
            if output.coins.is_empty() {
                return Err(Error::InvalidInputOutput);
            }
            for coin in &output.coins {
                let total = output_totals
                    .get(coin.denom.as_str())
                    .map_or("0", String::as_str);
                output_totals.insert(&coin.denom, add_amounts(total, &coin.amount)?);
            }
        }
        let input_totals = input
            .iter()
            .map(|coin| Ok((coin.denom.as_str(), normalize_amount(&coin.amount)?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        if output_totals != input_totals {
            return Err(Error::InvalidInputOutput);
        }

        let mut updates = BTreeMap::<(AccountAddress, String), String>::new();
        for coin in input {
            let amount = normalize_amount(&coin.amount)?;
            let balance = self.balance(from, &coin.denom)?.amount;
            if compare_amounts(&balance, &amount) == Ordering::Less {
                return Err(Error::InsufficientFunds);
            }
            updates.insert(
                (*from, coin.denom.clone()),
                subtract_amounts(&balance, &amount)?,
            );
        }
        for output in outputs {
            for coin in &output.coins {
                let key = (output.address, coin.denom.clone());
                let balance = match updates.get(&key) {
                    Some(amount) => amount.clone(),
                    None => self.balance(&output.address, &coin.denom)?.amount,
                };
                updates.insert(key, add_amounts(&balance, &coin.amount)?);
            }
        }
        for ((address, denom), amount) in updates {
            self.set_balance(
                &address,
                &Coin::new(denom, amount).map_err(|_| Error::InvalidCoin)?,
            )?;
        }
        Ok(())
    }

    /// Decode and execute a supported protobuf bank message.
    pub fn execute_message(&mut self, message: &cosmos_types::Any) -> Result<MessageResult> {
        match message.type_url.as_str() {
            MSG_SEND_TYPE_URL => self.execute_send(&message.value),
            MSG_MULTI_SEND_TYPE_URL => self.execute_multi_send(&message.value),
            other => Err(Error::UnsupportedMessage(other.to_owned())),
        }
    }

    /// Mint sorted, unique positive coins to an address and increase supply.
    pub fn mint_coins(&mut self, address: &AccountAddress, coins: &[Coin]) -> Result<()> {
        validate_coins(coins)?;
        let mut updates = Vec::with_capacity(coins.len());
        for coin in coins {
            let balance = self.balance(address, &coin.denom)?;
            let supply = self.supply(&coin.denom)?;
            updates.push((
                Coin::new(&coin.denom, add_amounts(&balance.amount, &coin.amount)?)
                    .map_err(|_| Error::InvalidCoin)?,
                Coin::new(&coin.denom, add_amounts(&supply.amount, &coin.amount)?)
                    .map_err(|_| Error::InvalidCoin)?,
            ));
        }
        for (balance, supply) in updates {
            self.set_balance(address, &balance)?;
            self.set_supply(&supply)?;
        }
        Ok(())
    }

    /// Burn sorted, unique positive coins from an address and decrease supply.
    pub fn burn_coins(&mut self, address: &AccountAddress, coins: &[Coin]) -> Result<()> {
        validate_coins(coins)?;
        let mut updates = Vec::with_capacity(coins.len());
        for coin in coins {
            let amount = normalize_amount(&coin.amount)?;
            let balance = self.balance(address, &coin.denom)?;
            let supply = self.supply(&coin.denom)?;
            if compare_amounts(&balance.amount, &amount) == Ordering::Less
                || compare_amounts(&supply.amount, &amount) == Ordering::Less
            {
                return Err(Error::InsufficientFunds);
            }
            updates.push((
                Coin::new(&coin.denom, subtract_amounts(&balance.amount, &amount)?)
                    .map_err(|_| Error::InvalidCoin)?,
                Coin::new(&coin.denom, subtract_amounts(&supply.amount, &amount)?)
                    .map_err(|_| Error::InvalidCoin)?,
            ));
        }
        for (balance, supply) in updates {
            self.set_balance(address, &balance)?;
            self.set_supply(&supply)?;
        }
        Ok(())
    }

    fn execute_send(&mut self, bytes: &[u8]) -> Result<MessageResult> {
        let message = cosmos_types::proto::bank::MsgSend::decode(bytes)
            .map_err(|_| Error::MalformedMessage)?;
        let from = decode_account_address(&message.from_address)?;
        let to = decode_account_address(&message.to_address)?;
        let coins = decode_positive_coins(message.amount)?;
        self.send_coins(&from, &to, &coins)?;
        Ok(MessageResult {
            events: send_events(&from, &to, &coins),
        })
    }

    fn execute_multi_send(&mut self, bytes: &[u8]) -> Result<MessageResult> {
        let message = cosmos_types::proto::bank::MsgMultiSend::decode(bytes)
            .map_err(|_| Error::MalformedMessage)?;
        let [input] = message.inputs.as_slice() else {
            return Err(Error::InvalidInputOutput);
        };
        if message.outputs.is_empty() {
            return Err(Error::InvalidInputOutput);
        }
        let from = decode_account_address(&input.address)?;
        let input_coins = decode_positive_coins(input.coins.clone())?;
        let outputs = message
            .outputs
            .into_iter()
            .map(|output| {
                Ok(SendOutput {
                    address: decode_account_address(&output.address)?,
                    coins: decode_positive_coins(output.coins)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        self.multi_send(&from, &input_coins, &outputs)?;
        Ok(MessageResult {
            events: multi_send_events(&from, &input_coins, &outputs),
        })
    }
}

impl ante::FeeKeeper for Keeper<'_> {
    fn spendable_balance(
        &self,
        address: &AccountAddress,
        denom: &str,
    ) -> ante::Result<Option<Coin>> {
        self.balance(address, denom)
            .map(Some)
            .map_err(|error| ante::Error::State(error.to_string()))
    }

    fn deduct_fees(&mut self, payer: &AccountAddress, fees: &[Coin]) -> ante::Result<()> {
        self.send_coins(payer, &Self::fee_collector_address(), fees)
            .map_err(|error| match error {
                Error::InsufficientFunds => ante::Error::InsufficientFunds,
                other => ante::Error::State(other.to_string()),
            })
    }
}

/// Errors returned by the bank keeper.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Underlying key/value store operation failed.
    #[error("bank store: {0}")]
    Store(#[from] kvstore::Error),
    /// A coin or denomination was invalid.
    #[error("invalid coin")]
    InvalidCoin,
    /// A stored or supplied amount was not a canonical SDK integer.
    #[error("invalid amount")]
    InvalidAmount,
    /// Arithmetic exceeded the Cosmos SDK 256-bit integer range.
    #[error("amount overflow")]
    AmountOverflow,
    /// A debit exceeded the available balance or supply.
    #[error("insufficient funds")]
    InsufficientFunds,
    /// Stored balance key bytes were malformed.
    #[error("invalid bank store key")]
    InvalidStoreKey,
    /// A protobuf message could not be decoded.
    #[error("malformed bank message")]
    MalformedMessage,
    /// A routed protobuf message type is not supported by this keeper.
    #[error("unsupported bank message: {0}")]
    UnsupportedMessage(String),
    /// A bank message contained an invalid account address.
    #[error("invalid bank account address")]
    InvalidAddress,
    /// Multi-send inputs and outputs were missing, invalid, or unbalanced.
    #[error("invalid bank input/output")]
    InvalidInputOutput,
}

/// Result alias for bank keeper operations.
pub type Result<T> = core::result::Result<T, Error>;

fn validate_denom(denom: &str) -> Result<()> {
    Coin::new(denom, "0")
        .map(|_| ())
        .map_err(|_| Error::InvalidCoin)
}

fn validate_coins(coins: &[Coin]) -> Result<()> {
    let mut previous = None;
    let mut denoms = BTreeSet::new();
    for coin in coins {
        coin.validate_basic().map_err(|_| Error::InvalidCoin)?;
        let amount = normalize_amount(&coin.amount)?;
        if amount == "0" || !denoms.insert(coin.denom.as_str()) {
            return Err(Error::InvalidCoin);
        }
        if previous.is_some_and(|value: &str| value >= coin.denom.as_str()) {
            return Err(Error::InvalidCoin);
        }
        previous = Some(coin.denom.as_str());
    }
    Ok(())
}

fn decode_account_address(value: &str) -> Result<AccountAddress> {
    cosmos_types::account_address_from_bech32(value).map_err(|_| Error::InvalidAddress)
}

fn decode_positive_coins(coins: Vec<cosmos_types::proto::base::Coin>) -> Result<Vec<Coin>> {
    let coins = coins
        .into_iter()
        .map(Coin::from_proto)
        .collect::<core::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::InvalidCoin)?;
    if coins.is_empty() {
        return Err(Error::InvalidCoin);
    }
    validate_coins(&coins)?;
    Ok(coins)
}

fn send_events(
    from: &AccountAddress,
    to: &AccountAddress,
    coins: &[Coin],
) -> Vec<abci_types::Event> {
    let from = cosmos_types::account_address_to_bech32(from);
    let to = cosmos_types::account_address_to_bech32(to);
    let amount = coins_string(coins);
    vec![
        event("coin_spent", [("spender", &from), ("amount", &amount)]),
        event("coin_received", [("receiver", &to), ("amount", &amount)]),
        event(
            "transfer",
            [("recipient", &to), ("sender", &from), ("amount", &amount)],
        ),
        event("message", [("sender", &from)]),
    ]
}

fn multi_send_events(
    from: &AccountAddress,
    input: &[Coin],
    outputs: &[SendOutput],
) -> Vec<abci_types::Event> {
    let from = cosmos_types::account_address_to_bech32(from);
    let input_amount = coins_string(input);
    let mut events = vec![
        event(
            "coin_spent",
            [("spender", &from), ("amount", &input_amount)],
        ),
        event("message", [("sender", &from)]),
    ];
    for output in outputs {
        let recipient = cosmos_types::account_address_to_bech32(&output.address);
        let amount = coins_string(&output.coins);
        events.extend([
            event(
                "coin_received",
                [("receiver", &recipient), ("amount", &amount)],
            ),
            event(
                "transfer",
                [
                    ("recipient", &recipient),
                    ("sender", &from),
                    ("amount", &amount),
                ],
            ),
        ]);
    }
    events
}

fn coins_string(coins: &[Coin]) -> String {
    coins
        .iter()
        .map(|coin| {
            let amount = coin.amount.trim_start_matches('0');
            let amount = if amount.is_empty() { "0" } else { amount };
            format!("{amount}{}", coin.denom)
        })
        .collect::<Vec<_>>()
        .join(",")
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

fn normalize_amount(amount: &str) -> Result<String> {
    if amount.is_empty() || !amount.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Error::InvalidAmount);
    }
    let normalized = amount.trim_start_matches('0');
    let normalized = if normalized.is_empty() {
        "0"
    } else {
        normalized
    };
    if compare_amounts(normalized, MAX_SDK_INT) == Ordering::Greater {
        return Err(Error::AmountOverflow);
    }
    Ok(normalized.to_owned())
}

fn decode_amount(bytes: &[u8]) -> Result<String> {
    let amount = core::str::from_utf8(bytes).map_err(|_| Error::InvalidAmount)?;
    let normalized = normalize_amount(amount)?;
    if normalized != amount {
        return Err(Error::InvalidAmount);
    }
    Ok(normalized)
}

fn compare_amounts(left: &str, right: &str) -> Ordering {
    left.len()
        .cmp(&right.len())
        .then_with(|| left.as_bytes().cmp(right.as_bytes()))
}

fn add_amounts(left: &str, right: &str) -> Result<String> {
    let left = normalize_amount(left)?;
    let right = normalize_amount(right)?;
    let mut carry = 0_u8;
    let mut digits = Vec::with_capacity(left.len().max(right.len()) + 1);
    let mut left = left.bytes().rev().map(|byte| byte - b'0');
    let mut right = right.bytes().rev().map(|byte| byte - b'0');
    loop {
        let lhs = left.next();
        let rhs = right.next();
        if lhs.is_none() && rhs.is_none() && carry == 0 {
            break;
        }
        let sum = lhs.unwrap_or(0) + rhs.unwrap_or(0) + carry;
        digits.push(b'0' + sum % 10);
        carry = sum / 10;
    }
    digits.reverse();
    let amount = String::from_utf8(digits).map_err(|_| Error::InvalidAmount)?;
    normalize_amount(&amount)
}

fn subtract_amounts(left: &str, right: &str) -> Result<String> {
    let left = normalize_amount(left)?;
    let right = normalize_amount(right)?;
    if compare_amounts(&left, &right) == Ordering::Less {
        return Err(Error::InsufficientFunds);
    }
    let mut borrow = 0_i16;
    let mut digits = Vec::with_capacity(left.len());
    let mut right = right.bytes().rev().map(|byte| i16::from(byte - b'0'));
    for lhs in left.bytes().rev().map(|byte| i16::from(byte - b'0')) {
        let mut value = lhs - borrow - right.next().unwrap_or(0);
        if value < 0 {
            value += 10;
            borrow = 1;
        } else {
            borrow = 0;
        }
        digits.push(b'0' + u8::try_from(value).map_err(|_| Error::InvalidAmount)?);
    }
    while digits.len() > 1 && digits.last() == Some(&b'0') {
        digits.pop();
    }
    digits.reverse();
    String::from_utf8(digits).map_err(|_| Error::InvalidAmount)
}

fn account_balance_prefix(address: &AccountAddress) -> Vec<u8> {
    let mut key = Vec::with_capacity(2 + address.len());
    key.extend_from_slice(&[BALANCES_PREFIX, ACCOUNT_ADDRESS_LEN]);
    key.extend_from_slice(address);
    key
}

fn balance_key(address: &AccountAddress, denom: &str) -> Vec<u8> {
    let mut key = account_balance_prefix(address);
    key.extend_from_slice(denom.as_bytes());
    key
}

fn denom_address_index_key(denom: &str, address: &AccountAddress) -> Vec<u8> {
    let mut key = Vec::with_capacity(3 + denom.len() + address.len());
    key.push(DENOM_ADDRESS_PREFIX);
    key.extend_from_slice(denom.as_bytes());
    key.push(0);
    key.push(ACCOUNT_ADDRESS_LEN);
    key.extend_from_slice(address);
    key
}

fn supply_key(denom: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(1 + denom.len());
    key.push(SUPPLY_PREFIX);
    key.extend_from_slice(denom.as_bytes());
    key
}
