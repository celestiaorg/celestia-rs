//! v11 patch-release ante decorator, after NestedMsg and before FibreLegacyEscrow.

use celestia_proto::Any;
use celestia_proto::celestia::fibre::v1::{MsgPayForFibre, MsgPaymentPromiseTimeout};
use celestia_proto::cosmos::authz::v1beta1::MsgExec;
use celestia_proto::cosmos::distribution::v1beta1::{
    MsgSetWithdrawAddress, MsgWithdrawDelegatorReward,
};
use sdk::frozen::{self, Network};
use sdk::{Context, tx::Tx};

use crate::{R, celestia::decode, txview};

/// Go v11 `Rules.AnteHandle`, activated by the consensus version.
pub fn ante(ctx: &mut Context<'_>, tx: &Tx) -> R<()> {
    if !frozen::active(ctx) {
        return Ok(());
    }
    let Some(n) = frozen::network(&ctx.header.chain_id) else {
        return Ok(());
    };
    if txview::fee_granter(tx)?.is_some_and(|a| n.has(&a)) {
        return Err(frozen::error("listed fee granter").into());
    }
    let listed = txview::signers(tx)?.iter().any(|s| n.has(s));
    if listed {
        let fees = sdk::coins::Coins::from_proto(&txview::fee(tx)?.amount)?;
        let max = sdk::coins::Coin::new("utia", frozen::MAX_FEE).amount;
        if fees.0.iter().any(|c| c.denom != "utia" || c.amount > max) {
            return Err(frozen::error(&format!("fee {fees} exceeds 1000000utia")).into());
        }
    }
    for msg in tx.gogo_msgs() {
        if listed && !allowed(n, msg)? {
            return Err(frozen::error(&format!("{} not allowed", msg.type_url)).into());
        }
        check_msg(ctx, n, msg)?;
    }
    Ok(())
}

fn allowed(n: &Network, msg: &Any) -> R<bool> {
    Ok(match msg.type_url.as_str() {
        "/cosmos.bank.v1beta1.MsgSend"
        | "/cosmos.staking.v1beta1.MsgUndelegate"
        | "/cosmos.distribution.v1beta1.MsgWithdrawDelegatorReward" => true,
        "/cosmos.distribution.v1beta1.MsgSetWithdrawAddress" => {
            decode::<MsgSetWithdrawAddress>(msg)?.withdraw_address == n.whitehat_text
        }
        _ => false,
    })
}

fn check_msg(ctx: &mut Context<'_>, n: &Network, msg: &Any) -> R<()> {
    match msg.type_url.as_str() {
        "/cosmos.distribution.v1beta1.MsgWithdrawDelegatorReward" => {
            let m = decode::<MsgWithdrawDelegatorReward>(msg)?;
            if let Ok(del) = txview::acc_address_from_bech32(&m.delegator_address)
                && n.has(&del)
            {
                let to = x_distribution::store::get_withdraw_addr(ctx, &del)?;
                if to != del && to != n.whitehat {
                    return Err(frozen::error(
                        "withdraw address is neither the delegator nor the whitehat",
                    )
                    .into());
                }
            }
        }
        "/celestia.fibre.v1.MsgPayForFibre" => {
            let m = decode::<MsgPayForFibre>(msg)?;
            check_promise(ctx, &m.payment_promise.unwrap_or_default())?;
        }
        "/celestia.fibre.v1.MsgPaymentPromiseTimeout" => {
            let m = decode::<MsgPaymentPromiseTimeout>(msg)?;
            check_promise(ctx, &m.payment_promise.unwrap_or_default())?;
        }
        "/cosmos.authz.v1beta1.MsgExec" => {
            for inner in decode::<MsgExec>(msg)?.msgs {
                // GetMsgV1Signers sees the re-packed gogoproto message.
                let inner = txdecode::gogo_marshaled(&inner);
                let signers = txview::msg_signers(&inner)?;
                if !allowed(n, &inner)? && signers.iter().any(|s| n.has(s)) {
                    return Err(frozen::error(&format!(
                        "{} not allowed in MsgExec",
                        inner.type_url
                    ))
                    .into());
                }
                check_msg(ctx, n, &inner)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn check_promise(
    ctx: &Context<'_>,
    p: &celestia_proto::celestia::fibre::v1::PaymentPromise,
) -> R<()> {
    if x_fibre::promise::signer_is_frozen(&ctx.header.chain_id, p)? {
        return Err(frozen::error("payment promise from a listed escrow").into());
    }
    Ok(())
}
