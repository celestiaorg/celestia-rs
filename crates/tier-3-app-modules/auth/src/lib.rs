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
/// Protobuf type URL used for module accounts in the account interface store.
pub const MODULE_ACCOUNT_TYPE_URL: &str = "/cosmos.auth.v1beta1.ModuleAccount";

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

/// A Cosmos SDK module account and its embedded base account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleAccount {
    /// Base account fields shared by every auth account variant.
    pub base_account: BaseAccount,
    /// Registered module name used to derive the account address.
    pub name: String,
    /// Module permissions such as `staking`, `minter`, or `burner`.
    pub permissions: Vec<String>,
}

impl ModuleAccount {
    /// Construct a module account with an address derived from `name`.
    #[must_use]
    pub fn new(name: impl Into<String>, permissions: Vec<String>, account_number: u64) -> Self {
        let name = name.into();
        Self {
            base_account: BaseAccount::new(cosmos_types::module_address(&name), account_number),
            name,
            permissions,
        }
    }

    /// Return whether this account has a named module permission.
    #[must_use]
    pub fn has_permission(&self, permission: &str) -> bool {
        self.permissions.iter().any(|value| value == permission)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StoredAccount {
    Base(BaseAccount),
    Module(ModuleAccount),
}

impl StoredAccount {
    fn base(&self) -> &BaseAccount {
        match self {
            Self::Base(account) => account,
            Self::Module(account) => &account.base_account,
        }
    }

    fn base_mut(&mut self) -> &mut BaseAccount {
        match self {
            Self::Base(account) => account,
            Self::Module(account) => &mut account.base_account,
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
        decode_stored_account(address, &bytes).map(|account| Some(account.base().clone()))
    }

    /// Return a module account by address, rejecting other account variants.
    pub fn get_module_account(&self, address: &AccountAddress) -> Result<Option<ModuleAccount>> {
        let Some(bytes) = self.store.get(&account_store_key(address))? else {
            return Ok(None);
        };
        match decode_stored_account(address, &bytes)? {
            StoredAccount::Module(account) => Ok(Some(account)),
            StoredAccount::Base(_) => Err(Error::NotModuleAccount),
        }
    }

    /// Return whether an account exists at `address`.
    pub fn has_account(&self, address: &AccountAddress) -> Result<bool> {
        self.store
            .has(&account_store_key(address))
            .map_err(Error::from)
    }

    /// Persist a base account in the SDK account interface encoding.
    pub fn set_account(&mut self, account: &BaseAccount) -> Result<()> {
        self.set_stored_account(&StoredAccount::Base(account.clone()))
    }

    /// Persist a module account in the SDK account interface encoding.
    pub fn set_module_account(&mut self, account: &ModuleAccount) -> Result<()> {
        validate_module_account(account)?;
        self.set_stored_account(&StoredAccount::Module(account.clone()))
    }

    fn set_stored_account(&mut self, account: &StoredAccount) -> Result<()> {
        let base = account.base();
        let previous = self
            .store
            .get(&account_store_key(&base.address))?
            .map(|bytes| decode_stored_account(&base.address, &bytes))
            .transpose()?;
        let index_key = account_number_index_key(base.account_number);
        if let Some(indexed_address) = self.store.get(&index_key)?
            && indexed_address != base.address
        {
            return Err(Error::AccountNumberAlreadyExists);
        }
        if let Some(previous) = previous {
            self.store
                .delete(&account_number_index_key(previous.base().account_number))?;
        }
        self.store.set(index_key, base.address.to_vec())?;
        self.store.set(
            account_store_key(&base.address),
            encode_stored_account(account),
        )?;
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
                decode_stored_account(&address, &entry.value).map(|account| account.base().clone())
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
        let bytes = self
            .store
            .get(&account_store_key(address))?
            .ok_or(Error::AccountNotFound)?;
        let mut account = decode_stored_account(address, &bytes)?;
        update(account.base_mut());
        self.set_stored_account(&account)
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
        let bytes = self
            .store
            .get(&account_store_key(address))
            .map_err(|error| ante_state_error(&Error::from(error)))?
            .ok_or_else(|| ante_state_error(&Error::AccountNotFound))?;
        if matches!(
            decode_stored_account(address, &bytes).map_err(|error| ante_state_error(&error))?,
            StoredAccount::Module(_)
        ) {
            return Err(ante_state_error(&Error::ModulePublicKeyUnsupported));
        }
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
    /// A base account was requested as a module account.
    #[error("account is not a module account")]
    NotModuleAccount,
    /// Module accounts cannot be assigned public keys.
    #[error("module accounts do not support public keys")]
    ModulePublicKeyUnsupported,
    /// Module account name, address, or permissions were invalid.
    #[error("invalid module account")]
    InvalidModuleAccount,
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

fn encode_base_account(account: &BaseAccount) -> cosmos_types::proto::auth::BaseAccount {
    cosmos_types::proto::auth::BaseAccount {
        address: cosmos_types::account_address_to_bech32(&account.address),
        pub_key: account.public_key.as_ref().map(Any::to_proto),
        account_number: account.account_number,
        sequence: account.sequence,
    }
}

fn encode_stored_account(account: &StoredAccount) -> Vec<u8> {
    let (type_url, value) = match account {
        StoredAccount::Base(account) => (
            BASE_ACCOUNT_TYPE_URL,
            encode_base_account(account).encode_to_vec(),
        ),
        StoredAccount::Module(account) => (
            MODULE_ACCOUNT_TYPE_URL,
            cosmos_types::proto::auth::ModuleAccount {
                base_account: Some(encode_base_account(&account.base_account)),
                name: account.name.clone(),
                permissions: account.permissions.clone(),
            }
            .encode_to_vec(),
        ),
    };
    cosmos_types::proto::google::Any {
        type_url: type_url.to_owned(),
        value,
    }
    .encode_to_vec()
}

fn decode_base_account(
    address: &AccountAddress,
    proto: cosmos_types::proto::auth::BaseAccount,
) -> Result<BaseAccount> {
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

fn decode_stored_account(address: &AccountAddress, bytes: &[u8]) -> Result<StoredAccount> {
    let outer =
        cosmos_types::proto::google::Any::decode(bytes).map_err(|_| Error::MalformedAccount)?;
    match outer.type_url.as_str() {
        BASE_ACCOUNT_TYPE_URL => {
            let proto = cosmos_types::proto::auth::BaseAccount::decode(outer.value.as_slice())
                .map_err(|_| Error::MalformedAccount)?;
            decode_base_account(address, proto).map(StoredAccount::Base)
        }
        MODULE_ACCOUNT_TYPE_URL => {
            let proto = cosmos_types::proto::auth::ModuleAccount::decode(outer.value.as_slice())
                .map_err(|_| Error::MalformedAccount)?;
            let base = proto.base_account.ok_or(Error::MalformedAccount)?;
            let account = ModuleAccount {
                base_account: decode_base_account(address, base)?,
                name: proto.name,
                permissions: proto.permissions,
            };
            validate_module_account(&account)?;
            Ok(StoredAccount::Module(account))
        }
        _ => Err(Error::UnsupportedAccountType(outer.type_url)),
    }
}

fn validate_module_account(account: &ModuleAccount) -> Result<()> {
    if account.name.trim().is_empty()
        || account.base_account.address != cosmos_types::module_address(&account.name)
        || account.base_account.public_key.is_some()
        || account
            .permissions
            .iter()
            .any(|permission| permission.trim().is_empty())
    {
        return Err(Error::InvalidModuleAccount);
    }
    Ok(())
}

fn ante_state_error(error: &Error) -> ante::Error {
    ante::Error::State(error.to_string())
}
