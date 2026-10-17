//! `x/auth` (celestiaorg/cosmos-sdk v0.52.12).
//!
//! Store layout (collections):
//! - `0x00` params (`Item`),
//! - `0x01 || addr` accounts (`IndexedMap`, value = `Any`-packed account),
//! - `0x02` global account number (`Sequence`, big-endian u64),
//! - `"accountNumber" || u64` unique index → raw address.

use celestia_proto::Any;
use celestia_proto::cosmos::auth::v1beta1::{BaseAccount, GenesisState, ModuleAccount, Params};
use celestia_proto::cosmos::vesting::v1beta1 as vpb;
use prost::{Message, Name};
use sdk::error::sdk as errs;
use sdk::{Context, Result};

pub mod vesting;

/// Store key.
pub const STORE_KEY: &str = "acc";
/// Module name.
pub const MODULE_NAME: &str = "auth";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 5;
/// Bech32 account prefix.
pub const BECH32_PREFIX: &str = "celestia";

const PARAMS_KEY: &[u8] = &[0x00];
const ACCOUNTS_PREFIX: u8 = 0x01;
const ACCOUNT_NUMBER_KEY: &[u8] = &[0x02];
const ACCOUNT_BY_NUMBER_PREFIX: &[u8] = b"accountNumber";

/// Fee collector module account name.
pub const FEE_COLLECTOR: &str = "fee_collector";
/// Permission: may mint.
pub const MINTER: &str = "minter";
/// Permission: may burn.
pub const BURNER: &str = "burner";
/// Permission: may stake.
pub const STAKING: &str = "staking";

/// celestia-app `maccPerms`.
pub const MODULE_ACCOUNT_PERMS: &[(&str, &[&str])] = &[
    (FEE_COLLECTOR, &[]),
    ("distribution", &[]),
    ("gov", &[BURNER]),
    ("mint", &[MINTER]),
    ("bonded_tokens_pool", &[BURNER, STAKING]),
    ("not_bonded_tokens_pool", &[BURNER, STAKING]),
    ("transfer", &[MINTER, BURNER]),
    ("interchainaccounts", &[]),
    ("hyperlane", &[]),
    ("warp", &[MINTER, BURNER]),
    ("forwarding", &[]),
    ("fibre", &[]),
];

/// The module accounts (`maccPerms`) of the current app version:
/// celestia-app v9.0.8 (app version 9) has no `fibre` account (ADR 0004),
/// so its address is neither a module account nor blocked there.
pub fn module_account_perms()
-> impl Iterator<Item = &'static (&'static str, &'static [&'static str])> {
    let v9 = sdk::version::is_v9(sdk::version::app_version());
    MODULE_ACCOUNT_PERMS
        .iter()
        .filter(move |(n, _)| !(v9 && *n == "fibre"))
}

/// A 20-byte (or 32-byte for some module-derived) account address.
pub type Address = Vec<u8>;

/// `authtypes.NewModuleAddress(name)` = `sha256(name)[:20]`.
#[must_use]
pub fn module_address(name: &str) -> Address {
    hash::sha256(name.as_bytes())[..20].to_vec()
}

/// Bech32 (`celestia1…`) form of an address.
pub fn bech32(addr: &[u8]) -> Result<String> {
    encoding::bech32::encode(BECH32_PREFIX, addr).map_err(|e| errs::invalid_address(e.to_string()))
}

/// Parses a bech32 account address.
pub fn parse_bech32(s: &str) -> Result<Address> {
    let (hrp, bytes) =
        encoding::bech32::decode(s).map_err(|e| errs::invalid_address(format!("{s}: {e}")))?;
    if hrp != BECH32_PREFIX {
        return Err(errs::invalid_address(format!(
            "invalid Bech32 prefix; expected {BECH32_PREFIX}, got {hrp}"
        )));
    }
    if bytes.is_empty() {
        return Err(errs::invalid_address("addresses cannot be empty"));
    }
    // `sdk.VerifyAddressFormat`: `address.MaxAddrLen`.
    if bytes.len() > 255 {
        return Err(errs::invalid_address(format!(
            "address max length is 255, got {}: unknown address",
            bytes.len()
        )));
    }
    Ok(bytes)
}

/// Any account type stored by `x/auth`.
#[derive(Debug, Clone, PartialEq)]
pub enum Account {
    /// `BaseAccount`.
    Base(BaseAccount),
    /// `ModuleAccount`.
    Module(ModuleAccount),
    /// `ContinuousVestingAccount`.
    ContinuousVesting(vpb::ContinuousVestingAccount),
    /// `DelayedVestingAccount`.
    DelayedVesting(vpb::DelayedVestingAccount),
    /// `PeriodicVestingAccount`.
    PeriodicVesting(vpb::PeriodicVestingAccount),
    /// `PermanentLockedAccount`.
    PermanentLocked(vpb::PermanentLockedAccount),
    /// ICS-27 `InterchainAccount` (created by the ICA host).
    Interchain(celestia_proto::ibc::applications::interchain_accounts::v1::InterchainAccount),
}

fn decode<M: Message + Default>(b: &[u8]) -> Result<M> {
    M::decode(b).map_err(|e| errs::unpack_any(e.to_string()))
}

impl Account {
    /// Decodes an `Any`-packed account.
    pub fn from_any(any: &Any) -> Result<Self> {
        let v = any.value.as_slice();
        Ok(match any.type_url.as_str() {
            u if u == BaseAccount::type_url() => Self::Base(decode(v)?),
            u if u == ModuleAccount::type_url() => Self::Module(decode(v)?),
            u if u == vpb::ContinuousVestingAccount::type_url() => {
                Self::ContinuousVesting(decode(v)?)
            }
            u if u == vpb::DelayedVestingAccount::type_url() => Self::DelayedVesting(decode(v)?),
            u if u == vpb::PeriodicVestingAccount::type_url() => Self::PeriodicVesting(decode(v)?),
            u if u == vpb::PermanentLockedAccount::type_url() => Self::PermanentLocked(decode(v)?),
            u if u
                == celestia_proto::ibc::applications::interchain_accounts::v1::InterchainAccount::type_url() =>
            {
                Self::Interchain(decode(v)?)
            }
            other => return Err(errs::unpack_any(format!("unknown account type {other}"))),
        })
    }

    /// Packs into `Any` (`codec.CollInterfaceValue`).
    #[must_use]
    pub fn to_any(&self) -> Any {
        match self {
            Self::Base(a) => Any::pack(a),
            Self::Module(a) => Any::pack(a),
            Self::ContinuousVesting(a) => Any::pack(a),
            Self::DelayedVesting(a) => Any::pack(a),
            Self::PeriodicVesting(a) => Any::pack(a),
            Self::PermanentLocked(a) => Any::pack(a),
            Self::Interchain(a) => Any::pack(a),
        }
    }

    /// The embedded base account.
    #[must_use]
    pub fn base(&self) -> Option<&BaseAccount> {
        match self {
            Self::Base(a) => Some(a),
            Self::Module(a) => a.base_account.as_ref(),
            Self::ContinuousVesting(a) => a.base_vesting_account.as_ref()?.base_account.as_ref(),
            Self::DelayedVesting(a) => a.base_vesting_account.as_ref()?.base_account.as_ref(),
            Self::PeriodicVesting(a) => a.base_vesting_account.as_ref()?.base_account.as_ref(),
            Self::PermanentLocked(a) => a.base_vesting_account.as_ref()?.base_account.as_ref(),
            Self::Interchain(a) => a.base_account.as_ref(),
        }
    }

    /// Mutable embedded base account (created empty if absent).
    pub fn base_mut(&mut self) -> &mut BaseAccount {
        match self {
            Self::Base(a) => a,
            Self::Module(a) => a.base_account.get_or_insert_with(Default::default),
            Self::ContinuousVesting(a) => a
                .base_vesting_account
                .get_or_insert_with(Default::default)
                .base_account
                .get_or_insert_with(Default::default),
            Self::DelayedVesting(a) => a
                .base_vesting_account
                .get_or_insert_with(Default::default)
                .base_account
                .get_or_insert_with(Default::default),
            Self::PeriodicVesting(a) => a
                .base_vesting_account
                .get_or_insert_with(Default::default)
                .base_account
                .get_or_insert_with(Default::default),
            Self::PermanentLocked(a) => a
                .base_vesting_account
                .get_or_insert_with(Default::default)
                .base_account
                .get_or_insert_with(Default::default),
            Self::Interchain(a) => a.base_account.get_or_insert_with(Default::default),
        }
    }

    /// Bech32 address string.
    #[must_use]
    pub fn address_str(&self) -> &str {
        self.base().map_or("", |b| b.address.as_str())
    }

    /// Raw address bytes.
    pub fn address(&self) -> Result<Address> {
        parse_bech32(self.address_str())
    }

    /// Account number.
    #[must_use]
    pub fn account_number(&self) -> u64 {
        self.base().map_or(0, |b| b.account_number)
    }

    /// Sequence.
    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.base().map_or(0, |b| b.sequence)
    }

    /// Public key (`Any`), if set.
    #[must_use]
    pub fn pub_key(&self) -> Option<&Any> {
        self.base().and_then(|b| b.pub_key.as_ref())
    }

    /// Module account view.
    #[must_use]
    pub fn as_module(&self) -> Option<&ModuleAccount> {
        match self {
            Self::Module(m) => Some(m),
            _ => None,
        }
    }
}

fn account_key(addr: &[u8]) -> Vec<u8> {
    let mut k = Vec::with_capacity(1 + addr.len());
    k.push(ACCOUNTS_PREFIX);
    k.extend_from_slice(addr);
    k
}

fn number_key(n: u64) -> Vec<u8> {
    let mut k = ACCOUNT_BY_NUMBER_PREFIX.to_vec();
    k.extend_from_slice(&n.to_be_bytes());
    k
}

/// Params.
pub fn get_params(ctx: &mut Context<'_>) -> Result<Params> {
    Ok(ctx
        .get(STORE_KEY, PARAMS_KEY)?
        .map(|b| decode(&b))
        .transpose()?
        .unwrap_or_default())
}

/// Sets params.
pub fn set_params(ctx: &mut Context<'_>, p: &Params) -> Result<()> {
    ctx.set(STORE_KEY, PARAMS_KEY.to_vec(), p.encode_to_vec())
}

/// `GetAccount`.
pub fn get_account(ctx: &mut Context<'_>, addr: &[u8]) -> Result<Option<Account>> {
    ctx.get(STORE_KEY, &account_key(addr))?
        .map(|b| Account::from_any(&decode::<Any>(&b)?))
        .transpose()
}

/// `HasAccount`.
pub fn has_account(ctx: &mut Context<'_>, addr: &[u8]) -> Result<bool> {
    ctx.has(STORE_KEY, &account_key(addr))
}

/// `SetAccount` (`IndexedMap.Set` with the unique account-number index).
pub fn set_account(ctx: &mut Context<'_>, acc: &Account) -> Result<()> {
    let addr = acc.address()?;
    let pk = account_key(&addr);
    // Unique.Reference: lazily load the old value and drop its index entry.
    if let Some(old) = ctx.get(STORE_KEY, &pk)? {
        let old = Account::from_any(&decode::<Any>(&old)?)?;
        ctx.delete(STORE_KEY, &number_key(old.account_number()))?;
    }
    let nk = number_key(acc.account_number());
    if ctx.has(STORE_KEY, &nk)? {
        return Err(errs::conflict(format!(
            "index uniqueness constrain violation: {}",
            acc.account_number()
        )));
    }
    ctx.set(STORE_KEY, nk, addr)?;
    ctx.set(STORE_KEY, pk, acc.to_any().encode_to_vec())
}

/// `RemoveAccount`.
pub fn remove_account(ctx: &mut Context<'_>, acc: &Account) -> Result<()> {
    let addr = acc.address()?;
    let pk = account_key(&addr);
    if let Some(old) = ctx.get(STORE_KEY, &pk)? {
        let old = Account::from_any(&decode::<Any>(&old)?)?;
        ctx.delete(STORE_KEY, &number_key(old.account_number()))?;
    }
    ctx.delete(STORE_KEY, &pk)
}

/// `NextAccountNumber` (`Sequence.Next` via `Item.Get` + `Set`).
pub fn next_account_number(ctx: &mut Context<'_>) -> Result<u64> {
    let n = ctx
        .get(STORE_KEY, ACCOUNT_NUMBER_KEY)?
        .map(|b| sdk::codec::decode_u64(&b).ok_or_else(|| errs::logic("bad account number")))
        .transpose()?
        .unwrap_or(0);
    ctx.set(
        STORE_KEY,
        ACCOUNT_NUMBER_KEY.to_vec(),
        (n + 1).to_be_bytes().to_vec(),
    )?;
    Ok(n)
}

/// `NewAccountWithAddress`: a `BaseAccount` with the next account number
/// (not stored).
pub fn new_account_with_address(ctx: &mut Context<'_>, addr: &[u8]) -> Result<Account> {
    let number = next_account_number(ctx)?;
    Ok(Account::Base(BaseAccount {
        address: bech32(addr)?,
        pub_key: None,
        account_number: number,
        sequence: 0,
    }))
}

/// Permissions of a known module account.
#[must_use]
pub fn module_permissions(name: &str) -> Option<&'static [&'static str]> {
    module_account_perms()
        .find(|(n, _)| *n == name)
        .map(|(_, p)| *p)
}

/// `GetModuleAccount`: loads or creates (and stores) the module account.
pub fn get_module_account(ctx: &mut Context<'_>, name: &str) -> Result<Option<ModuleAccount>> {
    let Some(perms) = module_permissions(name) else {
        return Ok(None);
    };
    let addr = module_address(name);
    if let Some(acc) = get_account(ctx, &addr)? {
        return match acc {
            Account::Module(m) => Ok(Some(m)),
            // `panic("account is not a module account")`: reachable for
            // the fibre address between the v10 switch and its upgrade
            // handler (v9 does not block it, security.md 72).
            _ => Err(sdk::error::go_panic("account is not a module account")),
        };
    }
    let number = next_account_number(ctx)?;
    let macc = ModuleAccount {
        base_account: Some(BaseAccount {
            address: bech32(&addr)?,
            pub_key: None,
            account_number: number,
            sequence: 0,
        }),
        name: name.to_owned(),
        permissions: perms.iter().map(|p| (*p).to_owned()).collect(),
    };
    set_account(ctx, &Account::Module(macc.clone()))?;
    Ok(Some(macc))
}

/// All accounts in address order.
pub fn all_accounts(ctx: &mut Context<'_>) -> Result<Vec<Account>> {
    let mut it = ctx.prefix_iter(STORE_KEY, &[ACCOUNTS_PREFIX])?;
    let mut out = Vec::new();
    while let Some((_, v)) = it.next(ctx)? {
        out.push(Account::from_any(&decode::<Any>(&v)?)?);
    }
    Ok(out)
}

/// `ExportGenesis`: params and every account (address order).
pub fn export_genesis(ctx: &mut Context<'_>) -> Result<GenesisState> {
    Ok(GenesisState {
        params: Some(get_params(ctx)?),
        accounts: all_accounts(ctx)?.iter().map(Account::to_any).collect(),
    })
}

/// `SanitizeGenesisAccounts`: fix duplicate account numbers with the lowest
/// unused values, then sort by account number (stable).
#[must_use]
pub fn sanitize_genesis_accounts(mut accs: Vec<Account>) -> Vec<Account> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut seen = BTreeSet::new();
    let mut dups: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for (i, a) in accs.iter().enumerate() {
        if !seen.insert(a.account_number()) {
            dups.entry(a.account_number()).or_default().push(i);
        }
    }
    let mut global = 0u64;
    for idxs in dups.values() {
        for &i in idxs {
            while seen.contains(&global) {
                global += 1;
            }
            accs[i].base_mut().account_number = global;
            seen.insert(global);
        }
    }
    accs.sort_by_key(Account::account_number);
    accs
}

/// `InitGenesis`.
pub fn init_genesis(ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    let gs: GenesisState =
        serde_json::from_value(genesis.clone()).map_err(|e| errs::json_unmarshal(e.to_string()))?;
    set_params(ctx, &gs.params.unwrap_or_default())?;
    let accounts = gs
        .accounts
        .iter()
        .map(Account::from_any)
        .collect::<Result<Vec<_>>>()?;
    let mut last: Option<u64> = None;
    for acc in sanitize_genesis_accounts(accounts) {
        let n = acc.account_number();
        while last.is_none_or(|l| l < n) {
            last = Some(next_account_number(ctx)?);
        }
        set_account(ctx, &acc)?;
    }
    get_module_account(ctx, FEE_COLLECTOR)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::{ExecMode, Header, Kind, MultiStore};

    fn with_ctx(f: impl FnOnce(&mut Context<'_>) -> Result<()>) -> Result<()> {
        let mut ms = MultiStore::new();
        ms.mount(STORE_KEY, Kind::Iavl)?;
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        f(&mut ctx)
    }

    // Found by `fuzz-diff deliver` across the v9 -> v10 upgrade
    // (security.md 72): a base account at the fibre address (v9 does not
    // block it) makes `GetModuleAccount` panic until the v10 handler
    // replaces it (queries between the switch and that handler).
    #[test]
    fn module_address_holding_a_base_account_panics() -> Result<()> {
        with_ctx(|ctx| {
            let addr = module_address("fibre");
            let acc = new_account_with_address(ctx, &addr)?;
            set_account(ctx, &acc)?;
            let e = get_module_account(ctx, "fibre").unwrap_err();
            assert_eq!(
                (e.codespace, e.code, &*e.log),
                ("undefined", 111_222, "account is not a module account")
            );
            Ok(())
        })
    }

    #[test]
    fn module_address_is_truncated_sha256() {
        // authtypes.NewModuleAddress("fee_collector")
        assert_eq!(
            bech32(&module_address(FEE_COLLECTOR)).unwrap(),
            "celestia17xpfvakm2amg962yls6f84z3kell8c5lpnjs3s"
        );
    }

    #[test]
    fn module_account_created_once_with_index() -> Result<()> {
        with_ctx(|ctx| {
            let m = get_module_account(ctx, FEE_COLLECTOR)?.unwrap();
            assert_eq!(m.base_account.as_ref().unwrap().account_number, 0);
            let again = get_module_account(ctx, FEE_COLLECTOR)?.unwrap();
            assert_eq!(m, again);
            assert_eq!(
                ctx.get(STORE_KEY, &number_key(0))?,
                Some(module_address(FEE_COLLECTOR))
            );
            assert_eq!(next_account_number(ctx)?, 1);
            assert!(get_module_account(ctx, "nope")?.is_none());
            Ok(())
        })
    }

    #[test]
    fn duplicate_numbers_are_sanitized() {
        let mk = |a: &str, n| {
            Account::Base(BaseAccount {
                address: a.into(),
                pub_key: None,
                account_number: n,
                sequence: 0,
            })
        };
        let out = sanitize_genesis_accounts(vec![mk("a", 0), mk("b", 0), mk("c", 0)]);
        let nums: Vec<_> = out.iter().map(Account::account_number).collect();
        assert_eq!(nums, vec![0, 1, 2]);
        assert_eq!(out[1].address_str(), "b");
    }
}
