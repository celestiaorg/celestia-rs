//! Resolve Go transaction options within the operator's explicit DA signing accounts.
use super::Options;
use anyhow::{Context as _, Result, ensure};
use serde::Deserialize as _;
use serde_json::Value;
use std::sync::Arc;

struct Account<W> {
    name: String,
    address: String,
    wallet: Arc<W>,
}

pub(crate) struct Accounts<W> {
    first: Account<W>,
    others: Vec<Account<W>>,
}

impl<W> Accounts<W> {
    pub(crate) fn open<'a>(
        names: impl Iterator<Item = &'a String>,
        mut load: impl FnMut(&str) -> Result<(String, Arc<W>)>,
    ) -> Result<Option<Self>> {
        let mut accounts = Vec::<Account<W>>::new();
        for name in names {
            let (address, wallet) = load(name)?;
            ensure!(
                !accounts.iter().any(|a| a.address == address),
                "duplicate DA signing account"
            );
            accounts.push(Account {
                name: name.clone(),
                address,
                wallet,
            });
        }
        let mut accounts = accounts.into_iter();
        Ok(accounts.next().map(|first| Self {
            first,
            others: accounts.collect(),
        }))
    }

    pub(crate) fn default_wallet(&self) -> &Arc<W> {
        &self.first.wallet
    }

    pub(crate) fn select(&self, args: &[Value]) -> Result<&Arc<W>> {
        let options = args.last().context("expected transaction options")?;
        let options = Option::<Options>::deserialize(options)?.unwrap_or_default();
        let mut accounts = std::iter::once(&self.first).chain(&self.others);
        // Go's signer_address overrides key_name when the client supplies both.
        let selected = if !options.signer_address.is_empty() {
            accounts.find(|a| a.address == options.signer_address)
        } else if !options.key_name.is_empty() {
            accounts.find(|a| a.name == options.key_name)
        } else {
            Some(&self.first)
        };
        Ok(&selected
            .context("requested account is not configured for DA signing")?
            .wallet)
    }
}
