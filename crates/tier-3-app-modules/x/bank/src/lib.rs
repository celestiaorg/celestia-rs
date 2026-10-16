//! `x/bank` (celestiaorg/cosmos-sdk v0.52.12).
//!
//! Store layout (collections):
//! - `0x00 || denom` supply (`math.Int` decimal string),
//! - `0x01 || denom` denom metadata,
//! - `0x02 || len(addr) addr || denom` balances (`math.Int` decimal string),
//! - `0x03 || denom 0x00 || len(addr) addr` reverse index (empty value),
//! - `0x04 || denom` send-enabled (`BoolValue`),
//! - `0x05` params.

pub mod msgs;
pub mod vesting_msgs;

use celestia_proto::cosmos::bank::v1beta1::{GenesisState, Metadata, Params};
use prost::Message;
use sdk::coins::{Amount, Coin, Coins, math_err};
use sdk::error::sdk as errs;
use sdk::event::event;
use sdk::{Context, Result};
use x_auth::{Account, Address};

/// Store key.
pub const STORE_KEY: &str = "bank";
/// Module name.
pub const MODULE_NAME: &str = "bank";
/// Consensus version.
pub const CONSENSUS_VERSION: u64 = 4;

const SUPPLY: u8 = 0x00;
const DENOM_METADATA: u8 = 0x01;
const BALANCES: u8 = 0x02;
const DENOM_ADDRESS: u8 = 0x03;
const SEND_ENABLED: u8 = 0x04;
const PARAMS: &[u8] = &[0x05];

fn balance_key(addr: &[u8], denom: &str) -> Vec<u8> {
    sdk::codec::key(&[
        &[BALANCES],
        &sdk::codec::length_prefixed(addr),
        &gostr::to_go_bytes(denom),
    ])
}

fn reverse_key(addr: &[u8], denom: &str) -> Vec<u8> {
    sdk::codec::key(&[
        &[DENOM_ADDRESS],
        &sdk::codec::string_non_terminal(denom),
        &sdk::codec::length_prefixed(addr),
    ])
}

fn denom_key(prefix: u8, denom: &str) -> Vec<u8> {
    sdk::codec::key(&[&[prefix], &gostr::to_go_bytes(denom)])
}

fn parse_int(b: &[u8]) -> Result<Amount> {
    let s = std::str::from_utf8(b).map_err(|e| errs::logic(e.to_string()))?;
    Amount::parse(s).map_err(math_err)
}

/// Params.
pub fn get_params(ctx: &mut Context<'_>) -> Result<Params> {
    ctx.get(STORE_KEY, PARAMS)?
        .map(|b| Params::decode(b.as_slice()).map_err(|e| errs::unpack_any(e.to_string())))
        .transpose()
        .map(Option::unwrap_or_default)
}

/// Sets params. Like Go's `SetParams`, the deprecated `send_enabled` list is
/// moved into the dedicated send-enabled entries and cleared.
pub fn set_params(ctx: &mut Context<'_>, mut p: Params) -> Result<()> {
    #[allow(deprecated)]
    let legacy = std::mem::take(&mut p.send_enabled);
    for se in legacy {
        set_send_enabled(ctx, &se.denom, se.enabled)?;
    }
    ctx.set(STORE_KEY, PARAMS.to_vec(), p.encode_to_vec())
}

/// `SetSendEnabled` (`BoolValue` proto).
pub fn set_send_enabled(ctx: &mut Context<'_>, denom: &str, enabled: bool) -> Result<()> {
    let v = pbjson_bool(enabled);
    ctx.set(STORE_KEY, denom_key(SEND_ENABLED, denom), v)
}

fn pbjson_bool(b: bool) -> Vec<u8> {
    // google.protobuf.BoolValue { value: b }
    if b { vec![0x08, 0x01] } else { Vec::new() }
}

/// `GetBalance`.
pub fn get_balance(ctx: &mut Context<'_>, addr: &[u8], denom: &str) -> Result<Coin> {
    let amount = ctx
        .get(STORE_KEY, &balance_key(addr, denom))?
        .map(|b| parse_int(&b))
        .transpose()?
        .unwrap_or_else(Amount::zero);
    Ok(Coin::with_amount(denom, amount))
}

/// `GetAllBalances` (denom order).
pub fn get_all_balances(ctx: &mut Context<'_>, addr: &[u8]) -> Result<Coins> {
    let prefix = sdk::codec::key(&[&[BALANCES], &sdk::codec::length_prefixed(addr)]);
    let mut it = ctx.prefix_iter(STORE_KEY, &prefix)?;
    let mut out = Vec::new();
    while let Some((k, v)) = it.next(ctx)? {
        let denom = gostr::from_go_bytes(&k[prefix.len()..]).into_owned();
        out.push(Coin::with_amount(denom, parse_int(&v)?));
    }
    Ok(Coins(out))
}

/// `setBalance`: zero balances are removed (with their index entry).
pub fn set_balance(ctx: &mut Context<'_>, addr: &[u8], coin: &Coin) -> Result<()> {
    if !coin.is_valid() {
        return Err(errs::invalid_coins("").wrap(coin.to_string()));
    }
    if coin.is_zero() {
        // IndexedMap.Remove: unreference then remove.
        ctx.delete(STORE_KEY, &reverse_key(addr, &coin.denom))?;
        return ctx.delete(STORE_KEY, &balance_key(addr, &coin.denom));
    }
    // IndexedMap.Set: ReversePair.Reference then primary Set.
    ctx.set(STORE_KEY, reverse_key(addr, &coin.denom), Vec::new())?;
    ctx.set(
        STORE_KEY,
        balance_key(addr, &coin.denom),
        coin.amount.to_string().into_bytes(),
    )
}

/// `GetSupply`.
pub fn get_supply(ctx: &mut Context<'_>, denom: &str) -> Result<Coin> {
    let amount = ctx
        .get(STORE_KEY, &denom_key(SUPPLY, denom))?
        .map(|b| parse_int(&b))
        .transpose()?
        .unwrap_or_else(Amount::zero);
    Ok(Coin::with_amount(denom, amount))
}

/// `setSupply` (zero supply is removed).
pub fn set_supply(ctx: &mut Context<'_>, coin: &Coin) -> Result<()> {
    let k = denom_key(SUPPLY, &coin.denom);
    if coin.is_zero() {
        ctx.delete(STORE_KEY, &k)
    } else {
        ctx.set(STORE_KEY, k, coin.amount.to_string().into_bytes())
    }
}

/// `SetDenomMetaData`.
pub fn set_denom_metadata(ctx: &mut Context<'_>, m: &Metadata) -> Result<()> {
    ctx.set(
        STORE_KEY,
        denom_key(DENOM_METADATA, &m.base),
        m.encode_to_vec(),
    )
}

/// `LockedCoins`: the vesting coins not delegated, for vesting accounts.
pub fn locked_coins(ctx: &mut Context<'_>, addr: &[u8]) -> Result<Coins> {
    let now = ctx.header.time;
    match x_auth::get_account(ctx, addr)? {
        Some(acc) => Ok(x_auth::vesting::locked_coins(&acc, &now)?.unwrap_or_default()),
        None => Ok(Coins::new()),
    }
}

/// `SpendableCoins`: balances minus locked coins (empty if negative).
pub fn spendable_coins(ctx: &mut Context<'_>, addr: &[u8]) -> Result<Coins> {
    let total = get_all_balances(ctx, addr)?;
    let locked = locked_coins(ctx, addr)?;
    let (spendable, neg) = x_auth::vesting::safe_sub(&total, &locked)?;
    Ok(if neg { Coins::new() } else { spendable })
}

/// `subUnlockedCoins`.
pub fn sub_unlocked_coins(ctx: &mut Context<'_>, addr: &[u8], amt: &Coins) -> Result<()> {
    if !amt.is_valid() {
        return Err(errs::invalid_coins("").wrap(amt.to_string()));
    }
    let locked = locked_coins(ctx, addr)?;
    for coin in amt {
        let balance = get_balance(ctx, addr, &coin.denom)?;
        let locked_amt = locked.amount_of(&coin.denom);
        let spendable = balance.amount.checked_sub(&locked_amt).map_err(math_err)?;
        if spendable.is_negative() {
            return Err(errs::insufficient_funds(format!(
                "locked amount exceeds account balance funds: {locked_amt}{} > {balance}",
                coin.denom
            )));
        }
        let new_amt = spendable.checked_sub(&coin.amount).map_err(math_err)?;
        if new_amt.is_negative() {
            return Err(errs::insufficient_funds(format!(
                "spendable balance {spendable}{} is smaller than {coin}",
                coin.denom
            )));
        }
        let new_balance = balance.amount.checked_sub(&coin.amount).map_err(math_err)?;
        set_balance(
            ctx,
            addr,
            &Coin::with_amount(coin.denom.clone(), new_balance),
        )?;
    }
    ctx.events.emit(event(
        "coin_spent",
        [
            ("spender", x_auth::bech32(addr)?),
            ("amount", amt.to_string()),
        ],
    ));
    Ok(())
}

/// `addCoins`.
pub fn add_coins(ctx: &mut Context<'_>, addr: &[u8], amt: &Coins) -> Result<()> {
    if !amt.is_valid() {
        return Err(errs::invalid_coins("").wrap(amt.to_string()));
    }
    for coin in amt {
        let balance = get_balance(ctx, addr, &coin.denom)?;
        let new = balance.amount.checked_add(&coin.amount).map_err(math_err)?;
        set_balance(ctx, addr, &Coin::with_amount(coin.denom.clone(), new))?;
    }
    ctx.events.emit(event(
        "coin_received",
        [
            ("receiver", x_auth::bech32(addr)?),
            ("amount", amt.to_string()),
        ],
    ));
    Ok(())
}

/// `SendCoins`.
pub fn send_coins(ctx: &mut Context<'_>, from: &[u8], to: &[u8], amt: &Coins) -> Result<()> {
    sub_unlocked_coins(ctx, from, amt)?;
    add_coins(ctx, to, amt)?;
    if !x_auth::has_account(ctx, to)? {
        let acc = x_auth::new_account_with_address(ctx, to)?;
        x_auth::set_account(ctx, &acc)?;
    }
    let from_s = x_auth::bech32(from)?;
    ctx.events.emit(event(
        "transfer",
        [
            ("recipient", x_auth::bech32(to)?),
            ("sender", from_s.clone()),
            ("amount", amt.to_string()),
        ],
    ));
    ctx.events.emit(event("message", [("sender", from_s)]));
    Ok(())
}

fn module_addr(name: &str) -> Result<Address> {
    if x_auth::module_permissions(name).is_none() {
        return Err(errs::unknown_address(format!(
            "module account {name} does not exist"
        )));
    }
    Ok(x_auth::module_address(name))
}

/// `BlockedAddr`: all module accounts except gov.
#[must_use]
pub fn is_blocked(addr: &[u8]) -> bool {
    x_auth::MODULE_ACCOUNT_PERMS
        .iter()
        .any(|(name, _)| *name != "gov" && x_auth::module_address(name) == addr)
}

/// `SendCoinsFromModuleToAccount`.
pub fn send_from_module_to_account(
    ctx: &mut Context<'_>,
    module: &str,
    to: &[u8],
    amt: &Coins,
) -> Result<()> {
    let from = module_addr(module)?;
    if is_blocked(to) {
        return Err(errs::unauthorized(format!(
            "{} is not allowed to receive funds",
            x_auth::bech32(to)?
        )));
    }
    send_coins(ctx, &from, to, amt)
}

/// `GetModuleAccount(..).GetAddress()`.
pub fn module_account_addr(ctx: &mut Context<'_>, module: &str) -> Result<Address> {
    let acc = x_auth::get_module_account(ctx, module)?
        .ok_or_else(|| errs::unknown_address(format!("module account {module} does not exist")))?;
    x_auth::Account::Module(acc).address()
}

/// `SendCoinsFromModuleToModule`.
pub fn send_from_module_to_module(
    ctx: &mut Context<'_>,
    from: &str,
    to: &str,
    amt: &Coins,
) -> Result<()> {
    let from = module_addr(from)?;
    let to = module_account_addr(ctx, to)?;
    send_coins(ctx, &from, &to, amt)
}

/// `SendCoinsFromAccountToModule`.
pub fn send_from_account_to_module(
    ctx: &mut Context<'_>,
    from: &[u8],
    module: &str,
    amt: &Coins,
) -> Result<()> {
    let to = module_account_addr(ctx, module)?;
    send_coins(ctx, from, &to, amt)
}

fn require_perm(ctx: &mut Context<'_>, module: &str, perm: &str) -> Result<Address> {
    let acc = x_auth::get_module_account(ctx, module)?
        .ok_or_else(|| errs::unknown_address(format!("module account {module} does not exist")))?;
    if !acc.permissions.iter().any(|p| p == perm) {
        return Err(errs::unauthorized(format!(
            "module account {module} does not have permissions to {} tokens",
            if perm == x_auth::MINTER {
                "mint"
            } else {
                "burn"
            }
        )));
    }
    x_auth::Account::Module(acc).address()
}

/// `MintCoins`.
pub fn mint_coins(ctx: &mut Context<'_>, module: &str, amt: &Coins) -> Result<()> {
    let addr = require_perm(ctx, module, x_auth::MINTER)?;
    add_coins(ctx, &addr, amt)?;
    for c in amt {
        let s = get_supply(ctx, &c.denom)?;
        let n = s.amount.checked_add(&c.amount).map_err(math_err)?;
        set_supply(ctx, &Coin::with_amount(c.denom.clone(), n))?;
    }
    ctx.events.emit(event(
        "coinbase",
        [
            ("minter", x_auth::bech32(&addr)?),
            ("amount", amt.to_string()),
        ],
    ));
    Ok(())
}

/// `BurnCoins`.
pub fn burn_coins(ctx: &mut Context<'_>, module: &str, amt: &Coins) -> Result<()> {
    let addr = require_perm(ctx, module, x_auth::BURNER)?;
    sub_unlocked_coins(ctx, &addr, amt)?;
    for c in amt {
        let s = get_supply(ctx, &c.denom)?;
        let n = s.amount.checked_sub(&c.amount).map_err(math_err)?;
        set_supply(ctx, &Coin::with_amount(c.denom.clone(), n))?;
    }
    ctx.events.emit(event(
        "burn",
        [
            ("burner", x_auth::bech32(&addr)?),
            ("amount", amt.to_string()),
        ],
    ));
    Ok(())
}

fn tracked_account(ctx: &mut Context<'_>, addr: &[u8]) -> Result<Account> {
    x_auth::get_account(ctx, addr)?.ok_or_else(|| {
        errs::unknown_address(format!(
            "account {} does not exist",
            x_auth::bech32(addr).unwrap_or_default()
        ))
    })
}

/// `trackDelegation`: vesting accounts record the delegated vesting and
/// free amounts.
fn track_delegation(
    ctx: &mut Context<'_>,
    addr: &[u8],
    balance: &Coins,
    amt: &Coins,
) -> Result<()> {
    let mut acc = tracked_account(ctx, addr)?;
    if x_auth::vesting::is_vesting(&acc) {
        let now = ctx.header.time;
        x_auth::vesting::track_delegation(&mut acc, &now, balance, amt)?;
        x_auth::set_account(ctx, &acc)?;
    }
    Ok(())
}

/// `trackUndelegation`.
fn track_undelegation(ctx: &mut Context<'_>, addr: &[u8], amt: &Coins) -> Result<()> {
    let mut acc = tracked_account(ctx, addr)?;
    if x_auth::vesting::is_vesting(&acc) {
        x_auth::vesting::track_undelegation(&mut acc, amt)?;
        x_auth::set_account(ctx, &acc)?;
    }
    Ok(())
}

/// `DelegateCoins`.
pub fn delegate_coins(
    ctx: &mut Context<'_>,
    del: &[u8],
    module_addr: &[u8],
    amt: &Coins,
) -> Result<()> {
    if x_auth::get_account(ctx, module_addr)?.is_none() {
        return Err(errs::unknown_address(format!(
            "module account {} does not exist",
            x_auth::bech32(module_addr)?
        )));
    }
    if !amt.is_valid() {
        return Err(errs::invalid_coins("").wrap(amt.to_string()));
    }
    let mut balances = Coins::new();
    for coin in amt {
        let balance = get_balance(ctx, del, &coin.denom)?;
        if balance.amount < coin.amount {
            return Err(errs::insufficient_funds(format!(
                "failed to delegate; {balance} is smaller than {amt}"
            )));
        }
        balances = balances.add(&Coins(vec![balance.clone()]))?;
        let rest = balance.amount.checked_sub(&coin.amount).map_err(math_err)?;
        set_balance(ctx, del, &Coin::with_amount(coin.denom.clone(), rest))?;
    }
    track_delegation(ctx, del, &balances, amt).map_err(|e| e.wrap("failed to track delegation"))?;
    ctx.events.emit(event(
        "coin_spent",
        [
            ("spender", x_auth::bech32(del)?),
            ("amount", amt.to_string()),
        ],
    ));
    add_coins(ctx, module_addr, amt)
}

/// `UndelegateCoins`.
pub fn undelegate_coins(
    ctx: &mut Context<'_>,
    module_addr: &[u8],
    del: &[u8],
    amt: &Coins,
) -> Result<()> {
    if x_auth::get_account(ctx, module_addr)?.is_none() {
        return Err(errs::unknown_address(format!(
            "module account {} does not exist",
            x_auth::bech32(module_addr)?
        )));
    }
    if !amt.is_valid() {
        return Err(errs::invalid_coins("").wrap(amt.to_string()));
    }
    sub_unlocked_coins(ctx, module_addr, amt)?;
    track_undelegation(ctx, del, amt).map_err(|e| e.wrap("failed to track undelegation"))?;
    add_coins(ctx, del, amt)
}

fn staking_module_addr(ctx: &mut Context<'_>, module: &str) -> Result<Address> {
    let acc = x_auth::get_module_account(ctx, module)?
        .ok_or_else(|| errs::unknown_address(format!("module account {module} does not exist")))?;
    if !acc.permissions.iter().any(|p| p == x_auth::STAKING) {
        return Err(errs::unauthorized(format!(
            "module account {module} does not have permissions to receive delegated coins"
        )));
    }
    x_auth::Account::Module(acc).address()
}

/// `DelegateCoinsFromAccountToModule`.
pub fn delegate_coins_from_account_to_module(
    ctx: &mut Context<'_>,
    from: &[u8],
    module: &str,
    amt: &Coins,
) -> Result<()> {
    let to = staking_module_addr(ctx, module)?;
    delegate_coins(ctx, from, &to, amt)
}

/// `UndelegateCoinsFromModuleToAccount`.
pub fn undelegate_coins_from_module_to_account(
    ctx: &mut Context<'_>,
    module: &str,
    to: &[u8],
    amt: &Coins,
) -> Result<()> {
    let from = staking_module_addr(ctx, module)?;
    undelegate_coins(ctx, &from, to, amt)
}

/// `InitGenesis`.
pub fn init_genesis(ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    let gs: GenesisState =
        serde_json::from_value(genesis.clone()).map_err(|e| errs::json_unmarshal(e.to_string()))?;
    set_params(ctx, gs.params.clone().unwrap_or_default())?;
    for se in &gs.send_enabled {
        set_send_enabled(ctx, &se.denom, se.enabled)?;
    }
    let mut balances = gs
        .balances
        .iter()
        .map(|b| Ok((x_auth::parse_bech32(&b.address).unwrap_or_default(), b)))
        .collect::<Result<Vec<_>>>()?;
    balances.sort_by(|a, b| a.0.cmp(&b.0));
    let mut total = Coins::new();
    for (_, b) in balances {
        let addr = x_auth::parse_bech32(&b.address)?;
        let coins = Coins::from_proto(&b.coins)?;
        for c in &coins {
            ctx.set(STORE_KEY, reverse_key(&addr, &c.denom), Vec::new())?;
            ctx.set(
                STORE_KEY,
                balance_key(&addr, &c.denom),
                c.amount.to_string().into_bytes(),
            )?;
        }
        total = total.add(&coins)?;
    }
    let supply = Coins::from_proto(&gs.supply)?;
    if !supply.is_empty() && supply != total {
        return Err(errs::logic(format!(
            "genesis supply is incorrect, expected {supply}, got {total}"
        )));
    }
    for c in &total {
        set_supply(ctx, c)?;
    }
    for m in &gs.denom_metadata {
        set_denom_metadata(ctx, m)?;
    }
    Ok(())
}

/// `ExportGenesis`: params, balances grouped by address (address order),
/// total supply (denom order), denom metadata and send-enabled entries.
pub fn export_genesis(ctx: &mut Context<'_>) -> Result<GenesisState> {
    use celestia_proto::cosmos::bank::v1beta1::{Balance, SendEnabled};
    use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;
    let params = get_params(ctx)?;
    let mut balances: Vec<Balance> = Vec::new();
    let mut last: Option<Vec<u8>> = None;
    let mut it = ctx.prefix_iter(STORE_KEY, &[BALANCES])?;
    while let Some((k, v)) = it.next(ctx)? {
        let n = usize::from(*k.get(1).ok_or_else(|| errs::logic("balance key"))?);
        let addr = k
            .get(2..2 + n)
            .ok_or_else(|| errs::logic("balance key"))?
            .to_vec();
        let denom = gostr::from_go_bytes(&k[2 + n..]).into_owned();
        let coin = PbCoin {
            denom,
            amount: parse_int(&v)?.to_string(),
        };
        if last.as_deref() == Some(addr.as_slice()) {
            if let Some(b) = balances.last_mut() {
                b.coins.push(coin);
            }
        } else {
            balances.push(Balance {
                address: x_auth::bech32(&addr)?,
                coins: vec![coin],
            });
            last = Some(addr);
        }
    }
    let mut supply = Vec::new();
    let mut it = ctx.prefix_iter(STORE_KEY, &[SUPPLY])?;
    while let Some((k, v)) = it.next(ctx)? {
        supply.push(PbCoin {
            denom: gostr::from_go_bytes(&k[1..]).into_owned(),
            amount: parse_int(&v)?.to_string(),
        });
    }
    let mut denom_metadata = Vec::new();
    let mut it = ctx.prefix_iter(STORE_KEY, &[DENOM_METADATA])?;
    while let Some((_, v)) = it.next(ctx)? {
        denom_metadata
            .push(Metadata::decode(v.as_slice()).map_err(|e| errs::unpack_any(e.to_string()))?);
    }
    let mut send_enabled = Vec::new();
    let mut it = ctx.prefix_iter(STORE_KEY, &[SEND_ENABLED])?;
    while let Some((k, v)) = it.next(ctx)? {
        send_enabled.push(SendEnabled {
            denom: gostr::from_go_bytes(&k[1..]).into_owned(),
            enabled: v == pbjson_bool(true),
        });
    }
    Ok(GenesisState {
        params: Some(params),
        balances,
        supply,
        denom_metadata,
        send_enabled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdk::{ExecMode, Header, Kind, MultiStore};

    #[test]
    fn send_creates_account_and_moves_funds() -> Result<()> {
        let mut ms = MultiStore::new();
        ms.mount(STORE_KEY, Kind::Iavl)?;
        ms.mount(x_auth::STORE_KEY, Kind::Iavl)?;
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let (a, b) = (vec![1u8; 20], vec![2u8; 20]);
        set_balance(&mut ctx, &a, &Coin::new("utia", 10))?;
        send_coins(&mut ctx, &a, &b, &Coins(vec![Coin::new("utia", 10)]))?;
        assert!(get_balance(&mut ctx, &a, "utia")?.is_zero());
        assert_eq!(ctx.get(STORE_KEY, &balance_key(&a, "utia"))?, None);
        assert_eq!(
            get_balance(&mut ctx, &b, "utia")?.amount,
            Amount::from_u64(10)
        );
        assert!(x_auth::has_account(&mut ctx, &b)?);
        let err = send_coins(&mut ctx, &a, &b, &Coins(vec![Coin::new("utia", 1)])).unwrap_err();
        assert_eq!(err.code, 5);
        Ok(())
    }

    #[test]
    fn key_layout_matches_go() {
        let addr = [0x4fu8; 20];
        let k = reverse_key(&addr, "utia");
        assert_eq!(&k[..6], b"\x03utia\x00");
        assert_eq!(k[6], 20);
        assert_eq!(&balance_key(&addr, "utia")[..2], &[0x02, 20]);
    }
}
