//! Cosmos SDK `x/auth` account storage and keeper operations.
//!
//! See `docs/superpowers/specs/m4/auth.md` for the API contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use ante::{AccountReader, AccountWriter};
use cosmos_types::{AccountAddress, Any};
use prost::Message;

/// Cosmos SDK mounted store name for `x/auth`.
pub const STORE_KEY: &str = "acc";
/// Protobuf type URL used for base accounts in the account interface store.
pub const BASE_ACCOUNT_TYPE_URL: &str = "/cosmos.auth.v1beta1.BaseAccount";

const ACCOUNT_KEY_PREFIX: u8 = 1;
const GLOBAL_ACCOUNT_NUMBER_KEY: [u8; 1] = [2];
const ACCOUNT_NUMBER_INDEX_PREFIX: &[u8] = b"accountNumber";

/// A Cosmos SDK base account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseAccount {
    /// Account address.
    pub address: AccountAddress,
    /// Optional account public key.
    pub public_key: Option<Any>,
    /// Stable account number assigned at account creation.
    pub account_number: u64,
    /// Transaction sequence.
    pub sequence: u64,
}

impl BaseAccount {
    /// Construct a base account without a public key.
    #[must_use]
    pub const fn new(address: AccountAddress, account_number: u64) -> Self {
        Self {
            address,
            public_key: None,
            account_number,
            sequence: 0,
        }
    }

    fn to_ante_account(&self) -> ante::Account {
        ante::Account {
            address: self.address,
            account_number: self.account_number,
            sequence: self.sequence,
            public_key: self.public_key.clone(),
        }
    }
}

/// Store-backed `x/auth` account keeper.
#[derive(Debug)]
pub struct Keeper<'a> {
    store: &'a mut kvstore::Store,
}

impl<'a> Keeper<'a> {
    /// Create a keeper over the mounted `acc` KV store.
    #[must_use]
    pub const fn new(store: &'a mut kvstore::Store) -> Self {
        Self { store }
    }

    /// Return an account by address.
    pub fn get_account(&self, address: &AccountAddress) -> Result<Option<BaseAccount>> {
        let Some(bytes) = self.store.get(&account_store_key(address))? else {
            return Ok(None);
        };
        decode_account(address, &bytes).map(Some)
    }

    /// Return whether an account exists at `address`.
    pub fn has_account(&self, address: &AccountAddress) -> Result<bool> {
        self.store
            .has(&account_store_key(address))
            .map_err(Error::from)
    }

    /// Persist a base account in the SDK account interface encoding.
    pub fn set_account(&mut self, account: &BaseAccount) -> Result<()> {
        let previous = self.get_account(&account.address)?;
        let index_key = account_number_index_key(account.account_number);
        if let Some(indexed_address) = self.store.get(&index_key)?
            && indexed_address != account.address
        {
            return Err(Error::AccountNumberAlreadyExists);
        }
        if let Some(previous) = previous {
            self.store
                .delete(&account_number_index_key(previous.account_number))?;
        }
        self.store.set(index_key, account.address.to_vec())?;
        let value = encode_account(account);
        self.store.set(account_store_key(&account.address), value)?;
        Ok(())
    }

    /// Remove an account, returning whether it existed.
    pub fn remove_account(&mut self, address: &AccountAddress) -> Result<bool> {
        let Some(account) = self.get_account(address)? else {
            return Ok(false);
        };
        self.store
            .delete(&account_number_index_key(account.account_number))?;
        self.store.delete(&account_store_key(address))?;
        Ok(true)
    }

    /// Return an account address by its unique account number.
    pub fn address_by_account_number(&self, account_number: u64) -> Result<Option<AccountAddress>> {
        self.store
            .get(&account_number_index_key(account_number))?
            .map(|bytes| {
                bytes
                    .try_into()
                    .map_err(|_| Error::MalformedAccountNumberIndex)
            })
            .transpose()
    }

    /// Return all base accounts in lexicographic address order.
    pub fn all_accounts(&self) -> Result<Vec<BaseAccount>> {
        let end = [ACCOUNT_KEY_PREFIX + 1];
        self.store
            .iter(Some(&[ACCOUNT_KEY_PREFIX]), Some(&end))?
            .map(|entry| {
                let address = address_from_store_key(&entry.key)?;
                decode_account(&address, &entry.value)
            })
            .collect()
    }

    /// Return the next account number without incrementing it.
    pub fn peek_next_account_number(&self) -> Result<u64> {
        self.store.get(&GLOBAL_ACCOUNT_NUMBER_KEY)?.map_or_else(
            || Ok(0),
            |bytes| {
                bytes
                    .as_slice()
                    .try_into()
                    .map(u64::from_be_bytes)
                    .map_err(|_| Error::MalformedAccountNumber)
            },
        )
    }

    /// Return and increment the global account number sequence.
    pub fn next_account_number(&mut self) -> Result<u64> {
        let current = self.peek_next_account_number()?;
        let next = current.checked_add(1).ok_or(Error::AccountNumberOverflow)?;
        self.store.set(
            GLOBAL_ACCOUNT_NUMBER_KEY.to_vec(),
            next.to_be_bytes().to_vec(),
        )?;
        Ok(current)
    }

    /// Allocate and persist a new base account.
    pub fn create_account(&mut self, address: AccountAddress) -> Result<BaseAccount> {
        if self.has_account(&address)? {
            return Err(Error::AccountAlreadyExists);
        }
        let account = BaseAccount::new(address, self.next_account_number()?);
        self.set_account(&account)?;
        Ok(account)
    }

    fn update_account(
        &mut self,
        address: &AccountAddress,
        update: impl FnOnce(&mut BaseAccount),
    ) -> Result<()> {
        let mut account = self.get_account(address)?.ok_or(Error::AccountNotFound)?;
        update(&mut account);
        self.set_account(&account)
    }
}

impl AccountReader for Keeper<'_> {
    fn account(&self, address: &AccountAddress) -> ante::Result<Option<ante::Account>> {
        self.get_account(address)
            .map(|account| account.map(|value| value.to_ante_account()))
            .map_err(|error| ante_state_error(&error))
    }
}

impl AccountWriter for Keeper<'_> {
    fn set_sequence(&mut self, address: &AccountAddress, sequence: u64) -> ante::Result<()> {
        self.update_account(address, |account| account.sequence = sequence)
            .map_err(|error| ante_state_error(&error))
    }

    fn set_public_key(&mut self, address: &AccountAddress, public_key: Any) -> ante::Result<()> {
        self.update_account(address, |account| account.public_key = Some(public_key))
            .map_err(|error| ante_state_error(&error))
    }
}

/// Errors returned by the auth keeper.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Underlying key/value store operation failed.
    #[error("auth store: {0}")]
    Store(#[from] kvstore::Error),
    /// Stored account bytes were not valid protobuf.
    #[error("malformed account encoding")]
    MalformedAccount,
    /// Stored account used an unsupported interface type.
    #[error("unsupported account type: {0}")]
    UnsupportedAccountType(String),
    /// Stored account address did not match its storage key.
    #[error("account address does not match storage key")]
    AddressMismatch,
    /// Stored account-number sequence was not an eight-byte big-endian integer.
    #[error("malformed global account number")]
    MalformedAccountNumber,
    /// Global account-number increment overflowed.
    #[error("global account number overflow")]
    AccountNumberOverflow,
    /// An account update targeted a missing account.
    #[error("account not found")]
    AccountNotFound,
    /// Account creation targeted an existing account.
    #[error("account already exists")]
    AccountAlreadyExists,
    /// A different account already used the requested account number.
    #[error("account number already exists")]
    AccountNumberAlreadyExists,
    /// An account-number index contained an invalid address.
    #[error("malformed account number index")]
    MalformedAccountNumberIndex,
}

/// Result alias for auth keeper operations.
pub type Result<T> = core::result::Result<T, Error>;

fn account_store_key(address: &AccountAddress) -> Vec<u8> {
    let mut key = Vec::with_capacity(1 + address.len());
    key.push(ACCOUNT_KEY_PREFIX);
    key.extend_from_slice(address);
    key
}

fn account_number_index_key(account_number: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(ACCOUNT_NUMBER_INDEX_PREFIX.len() + 8);
    key.extend_from_slice(ACCOUNT_NUMBER_INDEX_PREFIX);
    key.extend_from_slice(&account_number.to_be_bytes());
    key
}

fn address_from_store_key(key: &[u8]) -> Result<AccountAddress> {
    key.strip_prefix(&[ACCOUNT_KEY_PREFIX])
        .and_then(|address| address.try_into().ok())
        .ok_or(Error::MalformedAccount)
}

fn encode_account(account: &BaseAccount) -> Vec<u8> {
    let base_account = cosmos_types::proto::auth::BaseAccount {
        address: cosmos_types::account_address_to_bech32(&account.address),
        pub_key: account.public_key.as_ref().map(Any::to_proto),
        account_number: account.account_number,
        sequence: account.sequence,
    };
    cosmos_types::proto::google::Any {
        type_url: BASE_ACCOUNT_TYPE_URL.to_owned(),
        value: base_account.encode_to_vec(),
    }
    .encode_to_vec()
}

fn decode_account(address: &AccountAddress, bytes: &[u8]) -> Result<BaseAccount> {
    let outer =
        cosmos_types::proto::google::Any::decode(bytes).map_err(|_| Error::MalformedAccount)?;
    if outer.type_url != BASE_ACCOUNT_TYPE_URL {
        return Err(Error::UnsupportedAccountType(outer.type_url));
    }
    let proto = cosmos_types::proto::auth::BaseAccount::decode(outer.value.as_slice())
        .map_err(|_| Error::MalformedAccount)?;
    let stored_address = cosmos_types::account_address_from_bech32(&proto.address)
        .map_err(|_| Error::MalformedAccount)?;
    if stored_address != *address {
        return Err(Error::AddressMismatch);
    }
    let public_key = proto
        .pub_key
        .map(Any::from_proto)
        .transpose()
        .map_err(|_| Error::MalformedAccount)?;
    Ok(BaseAccount {
        address: stored_address,
        public_key,
        account_number: proto.account_number,
        sequence: proto.sequence,
    })
}

fn ante_state_error(error: &Error) -> ante::Error {
    ante::Error::State(error.to_string())
}
