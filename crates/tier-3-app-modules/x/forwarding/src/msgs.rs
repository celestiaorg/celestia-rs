//! x/forwarding Msg server (`keeper/msg_server.go`, `keeper/keeper.go`),
//! `ValidateBasic` and forwarding-address derivation (`types/address.go`).
//!
//! Errors: Go returns most failures as `fmt.Errorf("%w: ...")` wrappers,
//! which carry no ABCI code (`undefined`/1) even when they wrap a
//! registered error; only `ErrNoBalance` is returned unwrapped.

use celestia_proto::celestia::forwarding::v1::{
    EventTokenForwarded, MsgForward, MsgForwardResponse,
};
use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;
use math::Int;
use sdk::address::{ACC_PREFIX, acc_from_bech32, to_bech32};
use sdk::coins::{Coin, Coins};
use sdk::error::internal;
use sdk::{Context, Result, SdkError};
use x_hyperlane::{Deps, HexAddress, HyperlaneMessage, StandardHookMetadata};
use x_warp::{TOKEN_TYPE_COLLATERAL, TOKEN_TYPE_SYNTHETIC, TransferArgs};

use crate::MODULE_NAME;

/// `ForwardVersion`.
pub const FORWARD_VERSION: u8 = 1;
/// `appconsts.BondDenom`.
pub const BOND_DENOM: &str = "utia";

/// `ErrNoBalance`.
#[must_use]
pub fn err_no_balance() -> SdkError {
    SdkError::new(MODULE_NAME, 3, "no balance at forwarding address")
}

/// cosmos-sdk `address.Module(name, key)` (one derivation key).
#[must_use]
pub fn module_address_derived(name: &str, key: &[u8]) -> [u8; 32] {
    let th = hash::sha256(b"module");
    let mut h = hash::Sha256Hasher::new();
    h.update(&th);
    h.update(name.as_bytes());
    h.update(&[0]);
    h.update(key);
    h.finalize()
}

/// `DeriveForwardingAddress` (20-byte account address).
pub fn derive_forwarding_address(
    dest_domain: u32,
    dest_recipient: &[u8],
    token_id: &[u8],
    hook_id: &[u8],
    hook_metadata: &[u8],
) -> core::result::Result<Vec<u8>, String> {
    if dest_recipient.len() != 32 {
        return Err(format!(
            "invalid recipient length: expected 32 bytes, got {}",
            dest_recipient.len()
        ));
    }
    if token_id.len() != 32 {
        return Err(format!(
            "invalid token identifier: expected 32 bytes, got {}",
            token_id.len()
        ));
    }
    if !hook_id.is_empty() && hook_id.len() != 32 {
        return Err(format!(
            "invalid post-dispatch hook identifier: expected 32 bytes, got {}",
            hook_id.len()
        ));
    }
    let hook_set = !hook_id.is_empty() && hook_id.iter().any(|b| *b != 0);
    let bound = hook_set || !hook_metadata.is_empty();
    let mut domain = [0u8; 32];
    domain[28..].copy_from_slice(&dest_domain.to_be_bytes());
    let mut h = hash::Sha256Hasher::new();
    h.update(&domain);
    h.update(dest_recipient);
    h.update(token_id);
    if bound {
        h.update(if hook_set { hook_id } else { &[0u8; 32] });
        h.update(hook_metadata);
    }
    let call_digest = h.finalize();
    let mut h = hash::Sha256Hasher::new();
    h.update(&[FORWARD_VERSION]);
    h.update(&call_digest);
    let salt = h.finalize();
    Ok(module_address_derived(MODULE_NAME, &salt)[..20].to_vec())
}

fn decode_hex_address(s: &str) -> core::result::Result<HexAddress, String> {
    HexAddress::decode(s)
}

/// `Coin.Validate` on the proto coin (gogoproto `math.Int`).
fn validate_coin(c: Option<&PbCoin>) -> core::result::Result<Coin, String> {
    let c = c.cloned().unwrap_or_default();
    if !sdk::coins::is_valid_denom(&c.denom) {
        return Err(format!("invalid denom: {}", c.denom));
    }
    // `Coin.Validate`: an empty amount is a nil `math.Int`.
    if c.amount.is_empty() {
        return Err("amount is nil".into());
    }
    let amount = Int::unmarshal(c.amount.as_bytes()).map_err(|e| e.to_string())?;
    if amount.is_negative() {
        return Err(format!("negative coin amount: {amount}"));
    }
    Ok(Coin::with_amount(c.denom, amount))
}

/// `MsgForward.ValidateBasic` (every failure is unregistered).
pub fn validate_basic(m: &MsgForward) -> Result<()> {
    acc_from_bech32(&m.signer)
        .map_err(|e| internal(format!("invalid signer address: {}", e.log)))?;
    acc_from_bech32(&m.forward_addr)
        .map_err(|e| internal(format!("invalid forward address: {}", e.log)))?;
    decode_hex_address(&m.dest_recipient)
        .map_err(|e| internal(format!("invalid dest_recipient hex format: {e}")))?;
    decode_hex_address(&m.token_id)
        .map_err(|e| internal(format!("invalid token_id hex format: {e}")))?;
    validate_coin(m.max_igp_fee.as_ref())
        .map_err(|e| internal(format!("invalid max_igp_fee: {e}")))?;
    Ok(())
}

fn hl_coin(c: &Coin) -> Result<x_hyperlane::Coin> {
    Ok(x_hyperlane::Coin {
        denom: c.denom.clone(),
        amount: c.amount.clone(),
    })
}

fn coins(c: &Coin) -> Result<Coins> {
    Coins::sanitized(vec![c.clone()])
}

/// `QuoteIgpFeeForToken` (also used by the `QuoteForwardingFee` query).
pub fn quote_igp_fee(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    token: &x_warp::HypToken,
    dest_domain: u32,
    hook: Option<HexAddress>,
    hook_metadata: &[u8],
) -> core::result::Result<Coin, String> {
    let router = x_warp::get_enrolled_router(ctx, token.id.internal_id(), dest_domain)
        .map_err(|e| e.log)?
        .ok_or_else(|| {
            format!(
                "no router for domain {dest_domain}: collections: not found: key '({}, {dest_domain})'",
                token.id.internal_id()
            )
        })?;
    let metadata = StandardHookMetadata {
        address: Vec::new(),
        gas_limit: router.gas,
        custom_hook_metadata: hook_metadata.to_vec(),
    };
    let message = HyperlaneMessage {
        destination: dest_domain,
        ..HyperlaneMessage::default()
    };
    let quoted = x_hyperlane::core::quote_dispatch(
        ctx,
        deps,
        &token.origin_mailbox,
        &hook.unwrap_or_default(),
        &metadata,
        &message,
    )
    .map_err(|e| format!("failed to quote dispatch: {}", e.log))?;
    if quoted.0.len() > 1 {
        return Err("multi-denom IGP fees not supported".into());
    }
    Ok(quoted.0.first().map_or_else(
        || Coin::new(BOND_DENOM, 0),
        |c| Coin::with_amount(c.denom.clone(), c.amount.clone()),
    ))
}

/// `msgServer.Forward`.
#[allow(clippy::too_many_lines)]
pub fn forward(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    m: &MsgForward,
) -> Result<MsgForwardResponse> {
    let forward_addr = acc_from_bech32(&m.forward_addr).map_err(|e| {
        internal(format!(
            "invalid forward_addr {:?}: {}",
            m.forward_addr, e.log
        ))
    })?;
    let dest_recipient = decode_hex_address(&m.dest_recipient)
        .map_err(|e| internal(format!("invalid dest_recipient hex: {e}")))?;
    let token_id = decode_hex_address(&m.token_id)
        .map_err(|e| internal(format!("invalid token_id hex: {e}")))?;
    let mut hook = None;
    if !m.custom_hook_id.is_empty() {
        let h = decode_hex_address(&m.custom_hook_id)
            .map_err(|e| internal(format!("invalid custom_hook_id hex: {e}")))?;
        if !h.is_zero() {
            hook = Some(h);
        }
    }
    let mut hook_metadata = Vec::new();
    if !m.custom_hook_metadata.is_empty() {
        hook_metadata = x_hyperlane::util::decode_eth_hex(&m.custom_hook_metadata)
            .map_err(|e| internal(format!("invalid custom_hook_metadata hex: {e}")))?;
    }
    let hook_bytes = hook.map(|h| h.0.to_vec()).unwrap_or_default();
    let expected = derive_forwarding_address(
        m.dest_domain,
        &dest_recipient.0,
        &token_id.0,
        &hook_bytes,
        &hook_metadata,
    )
    .map_err(|e| internal(format!("failed to derive forwarding address: {e}")))?;
    if forward_addr != expected {
        return Err(internal(format!(
            "derived address does not match provided address: provided={} derived={}",
            to_bech32(ACC_PREFIX, &forward_addr),
            to_bech32(ACC_PREFIX, &expected)
        )));
    }
    let signer = acc_from_bech32(&m.signer)
        .map_err(|e| internal(format!("invalid signer address: {}", e.log)))?;
    let Some(token) = x_warp::get_token(ctx, token_id.internal_id())? else {
        return Err(internal(format!(
            "token {} not found: collections: not found: key '{}' of type uint64",
            m.token_id,
            token_id.internal_id()
        )));
    };
    let denom = match token.token_type {
        TOKEN_TYPE_COLLATERAL => token.origin_denom.clone(),
        TOKEN_TYPE_SYNTHETIC => format!("hyperlane/{}", token.id),
        _ => {
            return Err(internal(
                "failed to resolve token denom: unsupported token denom",
            ));
        }
    };
    // SpendableCoin: GetBalance, then LockedCoins. App version 9
    // (celestia-app v9.0.8) forwards GetBalance and never reads the
    // account's locked coins.
    let balance = x_bank::get_balance(ctx, &forward_addr, &denom)?;
    let app_version = ctx.consensus_params.version.as_ref().map_or(0, |v| v.app);
    let locked = if app_version != 0 && app_version < 10 {
        math::Int::zero()
    } else {
        x_bank::locked_coins(ctx, &forward_addr)?.amount_of(&denom)
    };
    let spendable = balance
        .amount
        .checked_sub(&locked)
        .map_err(|e| sdk::error::go_panic(e.to_string()))?;
    if spendable.is_negative() {
        return Err(sdk::error::go_panic(format!(
            "negative coin amount: {spendable}"
        )));
    }
    let balance = Coin::with_amount(denom, spendable);
    if balance.amount.is_zero() {
        return Err(err_no_balance());
    }
    let max_igp_fee = validate_coin(m.max_igp_fee.as_ref()).map_err(internal)?;
    let fail = |e: String| {
        internal(format!(
            "token forward failed: {}:{} ({e})",
            balance.denom, balance.amount
        ))
    };
    let message_id = forward_token(
        ctx,
        deps,
        &forward_addr,
        &signer,
        token,
        &balance,
        m.dest_domain,
        dest_recipient,
        &max_igp_fee,
        hook,
        &hook_metadata,
    )
    .map_err(fail)?;
    if let Ok(ev) = sdk::event::typed_event(&EventTokenForwarded {
        forward_addr: m.forward_addr.clone(),
        token_id: m.token_id.clone(),
        denom: balance.denom.clone(),
        amount: balance.amount.to_string(),
        message_id: message_id.to_string(),
    }) {
        ctx.events.emit(ev);
    }
    Ok(MsgForwardResponse {
        denom: balance.denom.clone(),
        amount: balance.amount.to_string(),
        message_id: message_id.to_string(),
    })
}

#[allow(clippy::too_many_arguments)]
fn forward_token(
    ctx: &mut Context<'_>,
    deps: &Deps<'_>,
    forward_addr: &[u8],
    signer: &[u8],
    token: x_warp::HypToken,
    balance: &Coin,
    dest_domain: u32,
    dest_recipient: HexAddress,
    max_igp_fee: &Coin,
    hook: Option<HexAddress>,
    hook_metadata: &[u8],
) -> core::result::Result<HexAddress, String> {
    let has_route = x_warp::has_enrolled_router(ctx, token.id.internal_id(), dest_domain)
        .map_err(|e| format!("error checking warp route: {}", e.log))?;
    if !has_route {
        return Err("no warp route to destination domain".into());
    }
    let quoted = quote_igp_fee(ctx, deps, &token, dest_domain, hook, hook_metadata)
        .map_err(|e| format!("failed to quote IGP fee: {e}"))?;
    let positive = !quoted.amount.is_zero() && !quoted.amount.is_negative();
    if positive {
        if max_igp_fee.denom != quoted.denom {
            return Err(format!(
                "max_igp_fee denom mismatch: provided {}, required {}",
                max_igp_fee.denom, quoted.denom
            ));
        }
        if max_igp_fee.amount < quoted.amount {
            return Err(format!(
                "IGP fee provided is less than required: provided {max_igp_fee}, required {quoted}"
            ));
        }
    }
    let fee_before = x_bank::get_balance(ctx, forward_addr, &quoted.denom).map_err(|e| e.log)?;
    if positive {
        x_bank::send_coins(
            ctx,
            signer,
            forward_addr,
            &coins(&quoted).map_err(|e| e.log)?,
        )
        .map_err(|e| format!("failed to collect IGP fee from relayer: {}", e.log))?;
    }
    let router = x_warp::get_enrolled_router(ctx, token.id.internal_id(), dest_domain)
        .map_err(|e| e.log)?
        .ok_or_else(|| format!("no router for domain {dest_domain}"))?;
    let args = TransferArgs {
        sender: to_bech32(ACC_PREFIX, forward_addr),
        destination_domain: dest_domain,
        recipient: dest_recipient,
        amount: balance.amount.clone(),
        custom_hook_id: hook,
        gas_limit: router.gas,
        max_fee: hl_coin(&quoted).map_err(|e| e.log)?,
        custom_hook_metadata: hook_metadata.to_vec(),
        nil: x_warp::TransferNils::default(),
    };
    let message_id = x_warp::remote_transfer(ctx, deps, token, args)
        .map_err(|e| format!("warp transfer failed: {}", e.log))?;
    if positive {
        let fee_after = x_bank::get_balance(ctx, forward_addr, &quoted.denom).map_err(|e| e.log)?;
        let math_err = |e: math::MathError| e.to_string();
        let mut used = fee_before
            .amount
            .checked_add(&quoted.amount)
            .and_then(|v| v.checked_sub(&fee_after.amount))
            .map_err(math_err)?;
        if balance.denom == quoted.denom {
            used = used.checked_sub(&balance.amount).map_err(math_err)?;
        }
        let excess = quoted.amount.checked_sub(&used).map_err(math_err)?;
        if !excess.is_zero() && !excess.is_negative() {
            let c = Coin::with_amount(quoted.denom.clone(), excess);
            x_bank::send_coins(ctx, forward_addr, signer, &coins(&c).map_err(|e| e.log)?)
                .map_err(|e| format!("failed to refund excess IGP fee to relayer: {}", e.log))?;
        }
    }
    Ok(message_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_address_matches_sdk() {
        // address.Module("forwarding", []byte{1}) from cosmos-sdk.
        let a = module_address_derived("forwarding", &[1]);
        assert_eq!(a.len(), 32);
        let r = derive_forwarding_address(1, &[1; 32], &[2; 32], &[], &[]);
        assert_eq!(r.map(|a| a.len()), Ok(20));
        // A zero hook id without metadata does not bind.
        assert_eq!(
            derive_forwarding_address(1, &[1; 32], &[2; 32], &[0; 32], &[]),
            derive_forwarding_address(1, &[1; 32], &[2; 32], &[], &[])
        );
        assert_ne!(
            derive_forwarding_address(1, &[1; 32], &[2; 32], &[], &[1]),
            derive_forwarding_address(1, &[1; 32], &[2; 32], &[], &[])
        );
        assert!(derive_forwarding_address(1, &[1; 31], &[2; 32], &[], &[]).is_err());
    }
}
