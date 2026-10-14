//! In-memory [`BankKeeper`] for tests (balances outside the store).

use std::cell::RefCell;
use std::collections::BTreeMap;

use math::Int;
use sdk::{Context, Result};

use crate::coins::Coins;
use crate::module_address;
use crate::router::BankKeeper;

/// Fake bank: balances and supply in memory; module accounts use
/// [`module_address`].
#[derive(Debug, Default)]
pub struct FakeBank {
    balances: RefCell<BTreeMap<(Vec<u8>, String), Int>>,
    supply: RefCell<BTreeMap<String, Int>>,
}

impl FakeBank {
    /// Credits `amount` of `denom` to `addr` (and to the supply).
    pub fn fund(&self, addr: &[u8], denom: &str, amount: i64) {
        self.add(addr, denom, &Int::from(amount));
        let mut s = self.supply.borrow_mut();
        let e = s.entry(denom.to_string()).or_default();
        *e = e.checked_add(&Int::from(amount)).unwrap_or_default();
    }

    /// Balance of `addr` in `denom`.
    #[must_use]
    pub fn balance(&self, addr: &[u8], denom: &str) -> Int {
        self.balances
            .borrow()
            .get(&(addr.to_vec(), denom.to_string()))
            .cloned()
            .unwrap_or_default()
    }

    /// Total supply of `denom`.
    #[must_use]
    pub fn supply(&self, denom: &str) -> Int {
        self.supply.borrow().get(denom).cloned().unwrap_or_default()
    }

    fn add(&self, addr: &[u8], denom: &str, amount: &Int) {
        let mut b = self.balances.borrow_mut();
        let e = b.entry((addr.to_vec(), denom.to_string())).or_default();
        *e = e.checked_add(amount).unwrap_or_default();
    }

    fn sub(&self, addr: &[u8], amt: &Coins) -> Result<()> {
        for c in &amt.0 {
            if self.balance(addr, &c.denom) < c.amount {
                return Err(sdk::error::sdk::insufficient_funds(format!(
                    "spendable balance {}{} is smaller than {c}",
                    self.balance(addr, &c.denom),
                    c.denom
                )));
            }
        }
        for c in &amt.0 {
            self.add(addr, &c.denom, &c.amount.neg());
        }
        Ok(())
    }

    fn send(&self, from: &[u8], to: &[u8], amt: &Coins) -> Result<()> {
        self.sub(from, amt)?;
        for c in &amt.0 {
            self.add(to, &c.denom, &c.amount);
        }
        Ok(())
    }
}

impl BankKeeper for FakeBank {
    fn send_coins_from_account_to_module(
        &self,
        _: &mut Context<'_>,
        sender: &[u8],
        module: &str,
        amt: &Coins,
    ) -> Result<()> {
        self.send(sender, &module_address(module), amt)
    }

    fn send_coins_from_module_to_account(
        &self,
        _: &mut Context<'_>,
        module: &str,
        recipient: &[u8],
        amt: &Coins,
    ) -> Result<()> {
        self.send(&module_address(module), recipient, amt)
    }

    fn mint_coins(&self, _: &mut Context<'_>, module: &str, amt: &Coins) -> Result<()> {
        for c in &amt.0 {
            self.add(&module_address(module), &c.denom, &c.amount);
            let mut s = self.supply.borrow_mut();
            let e = s.entry(c.denom.clone()).or_default();
            *e = e.checked_add(&c.amount).unwrap_or_default();
        }
        Ok(())
    }

    fn burn_coins(&self, _: &mut Context<'_>, module: &str, amt: &Coins) -> Result<()> {
        self.sub(&module_address(module), amt)?;
        for c in &amt.0 {
            let mut s = self.supply.borrow_mut();
            let e = s.entry(c.denom.clone()).or_default();
            *e = e.checked_sub(&c.amount).unwrap_or_default();
        }
        Ok(())
    }

    fn has_supply(&self, _: &mut Context<'_>, denom: &str) -> Result<bool> {
        Ok(!self.supply(denom).is_zero())
    }
}
