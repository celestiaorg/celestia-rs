//! Wiring of `hyperlane` and `warp` (`app.go`: `HyperlaneKeeper`,
//! `WarpKeeper` with both token types enabled and the gov authority), the
//! x/zkism ISM (router type 42) and the Msg servers that need the hyperlane
//! routers (x/zkism create, x/forwarding).

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
static ZKISM: x_zkism::msgs::ZkIsm = x_zkism::msgs::ZkIsm;

/// Builds the hyperlane routers with warp, and the zkism ISM where the
/// release has x/zkism (celestia-app v7 on: before it, a ZK-typed id misses
/// the ISM router, as any unregistered type does).
#[must_use]
pub fn deps(warp: &Warp, zkism: bool) -> Deps<'_> {
    let mut d = Deps::new(&BANK);
    // Static, distinct ids: registration cannot fail.
    let _ = warp.register(&mut d);
    if zkism {
        let _ = d.isms.register(x_zkism::msgs::MODULE_TYPE_ZK_ISM, &ZKISM);
    }
    d
}

/// x/zkism `CreateInterchainSecurityModule` (allocates an id on the
/// hyperlane ISM router).
pub fn zkism_create(
    ctx: &mut Context<'_>,
    msg: &celestia_proto::celestia::zkism::v1::MsgCreateInterchainSecurityModule,
) -> Result<celestia_proto::celestia::zkism::v1::MsgCreateInterchainSecurityModuleResponse> {
    let warp = Warp::celestia();
    let zkism = ctx.release().has_zkism();
    x_zkism::msgs::create_ism(ctx, &deps(&warp, zkism), msg)
}

/// x/forwarding `Forward`.
pub fn forwarding_forward(
    ctx: &mut Context<'_>,
    msg: &celestia_proto::celestia::forwarding::v1::MsgForward,
) -> Result<celestia_proto::celestia::forwarding::v1::MsgForwardResponse> {
    let warp = Warp::celestia();
    let zkism = ctx.release().has_zkism();
    x_forwarding::msgs::forward(ctx, &deps(&warp, zkism), msg)
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
    let deps = deps(&warp, ctx.release().has_zkism());
    x_hyperlane::handle_msg(ctx, &deps, msg).or_else(|| warp.handle_msg(ctx, &deps, msg))
}

#[cfg(test)]
mod tests {
    use celestia_proto::hyperlane::core::interchain_security::v1 as ismpb;
    use prost::Message;
    use sdk::{ExecMode, Header, MultiStore};

    use super::*;

    /// celestia-app v4.1.0 and v5.0.12 enable only collateral warp tokens
    /// (`warpkeeper.NewKeeper(..., []int32{HYP_TOKEN_TYPE_COLLATERAL})`);
    /// v6.4.x on enable synthetic tokens too.
    #[test]
    fn warp_synthetic_tokens_from_v6() {
        let prev = sdk::version::app_version();
        for (version, synthetic) in [(4, false), (5, false), (6, true), (9, true), (10, true)] {
            sdk::version::set_app_version(version);
            let mut want = vec![x_warp::TOKEN_TYPE_COLLATERAL];
            if synthetic {
                want.push(x_warp::TOKEN_TYPE_SYNTHETIC);
            }
            assert_eq!(Warp::celestia().enabled_tokens, want, "v{version}");
        }
        sdk::version::set_app_version(prev);
    }

    /// celestia-app v4 to v6 register no ZK ISM type in the hyperlane ISM
    /// router (x/zkism arrives in v7): a route to a type-42 id misses the
    /// router, with no gas, exactly as an unregistered type does. From v7
    /// the zkism module answers (and reads its store).
    #[test]
    fn zk_ism_type_is_routed_only_from_v7() -> Result<()> {
        let attempt = |version: u64, module_type: u8| -> (String, u64) {
            let mut ms = MultiStore::new();
            for (name, kind) in crate::stores::all() {
                ms.mount(name, kind).unwrap();
            }
            let mut ctx = Context::new(&mut ms, Header::default(), ExecMode::Finalize);
            ctx.consensus_params.version = Some(
                celestia_proto::celestia_core_proto::tendermint::types::VersionParams {
                    app: version,
                },
            );
            ctx.gas = kvstore::GasMeter::new(10_000_000);
            let ism = format!("0x{}{:08x}{:016x}", "00".repeat(20), module_type, 1);
            let msg = Any::pack(&ismpb::MsgCreateRoutingIsm {
                creator: "celestia1x".into(),
                routes: vec![ismpb::Route { ism, domain: 1 }],
            });
            let err = crate::router::route(&mut ctx, &msg).unwrap_err();
            (err.log, ctx.gas.consumed())
        };
        for version in [4, 5, 6] {
            let (zk_log, zk_gas) = attempt(version, x_zkism::msgs::MODULE_TYPE_ZK_ISM);
            let (other_log, other_gas) = attempt(version, 99);
            assert_eq!(zk_gas, other_gas, "v{version}: gas");
            // Compared without formatting the logs into the message.
            assert!(
                zk_log.replace("0000002a", "XX") == other_log.replace("00000063", "XX"),
                "v{version}: the logs differ beyond the module type"
            );
        }
        let (_, zk_gas) = attempt(8, x_zkism::msgs::MODULE_TYPE_ZK_ISM);
        let (_, other_gas) = attempt(8, 99);
        assert!(zk_gas > other_gas, "v8: the zkism module reads its store");
        Ok(())
    }

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
