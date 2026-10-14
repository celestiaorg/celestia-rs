//! Wiring of `hyperlane` and `warp` (`app.go`: `HyperlaneKeeper`,
//! `WarpKeeper` with both token types enabled and the gov authority).
//!
//! x/zkism (ISM type 42) and x/forwarding are not ported yet; zkism's ISM
//! registration belongs in [`deps`] once it is.

use celestia_proto::Any;
use sdk::{Context, Result};
use x_hyperlane::{BankKeeper, Coins, Deps};
use x_warp::Warp;

/// The x/bank keeper as seen by hyperlane and warp.
#[derive(Debug, Clone, Copy, Default)]
pub struct Bank;

fn coins(c: &Coins) -> sdk::coins::Coins {
    sdk::coins::Coins(
        c.0.iter()
            .map(|c| sdk::coins::Coin::with_amount(c.denom.clone(), c.amount.clone()))
            .collect(),
    )
}

impl BankKeeper for Bank {
    fn send_coins_from_account_to_module(
        &self,
        ctx: &mut Context<'_>,
        sender: &[u8],
        recipient_module: &str,
        amt: &Coins,
    ) -> Result<()> {
        x_bank::send_from_account_to_module(ctx, sender, recipient_module, &coins(amt))
    }

    fn send_coins_from_module_to_account(
        &self,
        ctx: &mut Context<'_>,
        sender_module: &str,
        recipient: &[u8],
        amt: &Coins,
    ) -> Result<()> {
        x_bank::send_from_module_to_account(ctx, sender_module, recipient, &coins(amt))
    }

    fn mint_coins(&self, ctx: &mut Context<'_>, module: &str, amt: &Coins) -> Result<()> {
        x_bank::mint_coins(ctx, module, &coins(amt))
    }

    fn burn_coins(&self, ctx: &mut Context<'_>, module: &str, amt: &Coins) -> Result<()> {
        x_bank::burn_coins(ctx, module, &coins(amt))
    }

    fn has_supply(&self, ctx: &mut Context<'_>, denom: &str) -> Result<bool> {
        // `Supply.Has`: the supply collection is `0x00 || denom`.
        ctx.has(
            x_bank::STORE_KEY,
            &sdk::codec::key(&[&[0x00], denom.as_bytes()]),
        )
    }
}

static BANK: Bank = Bank;

/// Builds the hyperlane routers with warp registered.
#[must_use]
pub fn deps(warp: &Warp) -> Deps<'_> {
    let mut d = Deps::new(&BANK);
    // Static, distinct ids: registration cannot fail.
    let _ = warp.register(&mut d);
    d
}

/// Whether `type_url` is a hyperlane or warp message.
#[must_use]
pub fn has_handler(type_url: &str) -> bool {
    x_hyperlane::msg::MSG_TYPE_URLS.contains(&type_url) || x_warp::MSG_TYPE_URLS.contains(&type_url)
}

/// Executes a hyperlane or warp message (`None` for other messages).
pub fn route(ctx: &mut Context<'_>, msg: &Any) -> Option<Result<Any>> {
    if !has_handler(&msg.type_url) {
        return None;
    }
    let warp = Warp::celestia();
    let deps = deps(&warp);
    x_hyperlane::handle_msg(ctx, &deps, msg).or_else(|| warp.handle_msg(ctx, &deps, msg))
}

#[cfg(test)]
mod tests {
    use celestia_proto::hyperlane::core::interchain_security::v1 as ismpb;
    use prost::Message;
    use sdk::{ExecMode, Header, MultiStore};

    use super::*;

    #[test]
    fn routes_hyperlane_messages() -> Result<()> {
        let mut ms = MultiStore::new();
        for (name, kind) in crate::stores::all() {
            ms.mount(name, kind)?;
        }
        let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
        let msg = Any::pack(&ismpb::MsgCreateNoopIsm {
            creator: "celestia1x".into(),
        });
        assert!(crate::router::has_handler(&msg.type_url));
        let resp = crate::router::route(&mut ctx, &msg)?;
        let id = ismpb::MsgCreateNoopIsmResponse::decode(resp.value.as_slice())
            .map_err(|e| sdk::error::sdk::tx_decode(e.to_string()))?
            .id;
        assert_eq!(
            id,
            "0x726f757465725f69736d00000000000000000000000000000000000000000000"
        );
        assert!(!has_handler("/cosmos.bank.v1beta1.MsgSend"));
        Ok(())
    }
}
