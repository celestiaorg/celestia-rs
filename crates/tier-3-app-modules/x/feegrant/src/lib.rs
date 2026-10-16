//! `x/feegrant` (cosmossdk.io/x/feegrant v0.1.1).
//!
//! Store layout (raw KV):
//! - `0x00 || len grantee || len granter` → `Grant`
//! - `0x01 || time(exp) || len grantee || len granter` → empty (expiry queue)
//!
//! Go quirk kept: `revokeAllowance` deletes the queue entry under the key
//! with granter and grantee swapped, so a revoked grant's queue entry stays
//! until it is pruned.

#![allow(clippy::similar_names)] // granter / grantee

pub mod allowance;

use celestia_proto::Any;
use celestia_proto::cosmos::feegrant::v1beta1::{
    GenesisState, Grant, MsgGrantAllowance, MsgGrantAllowanceResponse, MsgPruneAllowances,
    MsgPruneAllowancesResponse, MsgRevokeAllowance, MsgRevokeAllowanceResponse,
};
use pbjson_types::Timestamp;
use prost::Message;
use sdk::address::acc_from_bech32;
use sdk::coins::Coins;
use sdk::error::sdk as sdkerr;
use sdk::event::event;
use sdk::time::format_time_bytes;
use sdk::{Context, Result, SdkError};

pub use allowance::Allowance;

/// Store key.
pub const STORE_KEY: &str = "feegrant";
/// Module name.
pub const MODULE_NAME: &str = "feegrant";

/// `x/feegrant` errors (codespace `feegrant`).
pub mod errors {
    use sdk::SdkError;

    macro_rules! feegrant_errors {
        ($($name:ident = $code:expr, $desc:expr;)*) => {
            $(
                #[doc = $desc]
                #[must_use]
                pub fn $name() -> SdkError {
                    SdkError::new("feegrant", $code, $desc)
                }
            )*
        };
    }

    feegrant_errors! {
        fee_limit_exceeded = 2, "fee limit exceeded";
        fee_limit_expired = 3, "fee allowance expired";
        invalid_duration = 4, "invalid duration";
        no_allowance = 5, "no allowance";
        no_messages = 6, "allowed messages are empty";
        message_not_allowed = 7, "message not allowed";
    }
}

fn lp(b: &[u8]) -> Vec<u8> {
    sdk::codec::length_prefixed(b)
}

/// `FeeAllowanceKey(granter, grantee)`.
#[must_use]
pub fn allowance_key(granter: &[u8], grantee: &[u8]) -> Vec<u8> {
    let mut k = vec![0x00];
    k.extend_from_slice(&lp(grantee));
    k.extend_from_slice(&lp(granter));
    k
}

/// `FeeAllowancePrefixQueue(exp, key)`.
#[must_use]
pub fn queue_key(exp: &Timestamp, key_suffix: &[u8]) -> Vec<u8> {
    let mut k = vec![0x01];
    k.extend_from_slice(&format_time_bytes(exp));
    k.extend_from_slice(key_suffix);
    k
}

/// `ParseAddressesFromFeeAllowanceQueueKey`: `(granter, grantee)`.
fn parse_queue_key(key: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut i = 1 + sdk::time::SORTABLE_TIME_LEN;
    let mut take = || -> Vec<u8> {
        let n = key.get(i).copied().unwrap_or(0) as usize;
        let v = key.get(i + 1..i + 1 + n).unwrap_or_default().to_vec();
        i += 1 + n;
        v
    };
    let grantee = take();
    let granter = take();
    (granter, grantee)
}

fn bech32(addr: &[u8]) -> String {
    sdk::address::to_bech32(sdk::address::ACC_PREFIX, addr)
}

/// `getGrant` (`ErrNotFound` when absent).
pub fn get_grant(ctx: &mut Context<'_>, granter: &[u8], grantee: &[u8]) -> Result<Grant> {
    let bz = ctx
        .get(STORE_KEY, &allowance_key(granter, grantee))?
        .unwrap_or_default();
    if bz.is_empty() {
        return Err(sdkerr::not_found("fee-grant not found"));
    }
    Grant::decode(bz.as_slice()).map_err(|e| sdk::address::undefined(e.to_string()))
}

/// `GetAllowance`.
pub fn get_allowance(ctx: &mut Context<'_>, granter: &[u8], grantee: &[u8]) -> Result<Allowance> {
    let g = get_grant(ctx, granter, grantee)?;
    Allowance::from_grant(&g)
}

fn new_grant(granter: &[u8], grantee: &[u8], a: &Allowance) -> Grant {
    Grant {
        granter: bech32(granter),
        grantee: bech32(grantee),
        allowance: Some(a.to_any()),
    }
}

/// `Keeper.GrantAllowance`.
pub fn grant_allowance(
    ctx: &mut Context<'_>,
    granter: &[u8],
    grantee: &[u8],
    a: &Allowance,
) -> Result<()> {
    if get_allowance(ctx, granter, grantee).is_ok() {
        return Err(sdkerr::invalid_request("fee allowance already exists"));
    }
    if x_auth::get_account(ctx, grantee)?.is_none() {
        if x_bank::is_blocked(grantee) {
            return Err(sdkerr::unauthorized(format!(
                "{} is not allowed to receive funds",
                bech32(grantee)
            )));
        }
        let acc = x_auth::new_account_with_address(ctx, grantee)?;
        x_auth::set_account(ctx, &acc)?;
    }
    let key = allowance_key(granter, grantee);
    let exp = a.expires_at()?;
    if let Some(exp) = &exp {
        if sdk::time::before(exp, &ctx.header.time) {
            return Err(sdkerr::invalid_request(
                "expiration is before current block time",
            ));
        }
        ctx.set(STORE_KEY, queue_key(exp, &key[1..]), Vec::new())?;
    }
    let grant = new_grant(granter, grantee, a);
    ctx.set(STORE_KEY, key, grant.encode_to_vec())?;
    ctx.events.emit(event(
        "set_feegrant",
        [("granter", grant.granter), ("grantee", grant.grantee)],
    ));
    Ok(())
}

fn update_allowance(
    ctx: &mut Context<'_>,
    granter: &[u8],
    grantee: &[u8],
    a: &Allowance,
) -> Result<()> {
    get_grant(ctx, granter, grantee)?;
    let grant = new_grant(granter, grantee, a);
    ctx.set(
        STORE_KEY,
        allowance_key(granter, grantee),
        grant.encode_to_vec(),
    )?;
    ctx.events.emit(event(
        "update_feegrant",
        [("granter", grant.granter), ("grantee", grant.grantee)],
    ));
    Ok(())
}

fn revoke_allowance(ctx: &mut Context<'_>, granter: &[u8], grantee: &[u8]) -> Result<()> {
    let grant = get_allowance(ctx, granter, grantee)?;
    ctx.delete(STORE_KEY, &allowance_key(granter, grantee))?;
    if let Some(exp) = grant.expires_at()? {
        // Go passes FeeAllowanceKey(grantee, granter): swapped.
        let swapped = allowance_key(grantee, granter);
        ctx.delete(STORE_KEY, &queue_key(&exp, &swapped[1..]))?;
    }
    ctx.events.emit(event(
        "revoke_feegrant",
        [("granter", bech32(granter)), ("grantee", bech32(grantee))],
    ));
    Ok(())
}

/// `Keeper.UseGrantedFees` (called by the ante handler's fee decorator).
pub fn use_granted_fees(
    ctx: &mut Context<'_>,
    granter: &[u8],
    grantee: &[u8],
    fee: &Coins,
    msgs: &[Any],
) -> Result<()> {
    let mut grant = get_allowance(ctx, granter, grantee)?;
    let (remove, res) = grant.accept(ctx, fee, msgs);
    let use_event = |ctx: &mut Context<'_>| {
        ctx.events.emit(event(
            "use_feegrant",
            [("granter", bech32(granter)), ("grantee", bech32(grantee))],
        ));
    };
    if remove {
        let _ = revoke_allowance(ctx, granter, grantee);
        res?;
        use_event(ctx);
        return Ok(());
    }
    res?;
    use_event(ctx);
    update_allowance(ctx, granter, grantee, &grant)
}

/// `RemoveExpiredAllowances`.
pub fn remove_expired_allowances(ctx: &mut Context<'_>, limit: usize) -> Result<()> {
    let mut end = vec![0x01];
    end.extend_from_slice(&format_time_bytes(&ctx.header.time));
    end.push(0); // InclusiveEndBytes
    let mut it = ctx.iterator(STORE_KEY, Some(&[0x01]), Some(&end))?;
    let mut count = 0;
    while let Some((k, _)) = it.next(ctx)? {
        ctx.delete(STORE_KEY, &k)?;
        let (granter, grantee) = parse_queue_key(&k);
        ctx.delete(STORE_KEY, &allowance_key(&granter, &grantee))?;
        count += 1;
        if count == limit {
            return Ok(());
        }
    }
    Ok(())
}

/// `EndBlocker`.
pub fn end_block(ctx: &mut Context<'_>) -> Result<()> {
    remove_expired_allowances(ctx, 200)
}

/// `MsgGrantAllowance`.
pub fn handle_grant_allowance(
    ctx: &mut Context<'_>,
    msg: &MsgGrantAllowance,
) -> Result<MsgGrantAllowanceResponse> {
    if gostr::equal_fold(&msg.grantee, &msg.granter) {
        return Err(sdkerr::invalid_address(
            "cannot self-grant fee authorization",
        ));
    }
    let grantee = sdk::address::acc_codec_to_bytes(&msg.grantee)?;
    let granter = sdk::address::acc_codec_to_bytes(&msg.granter)?;
    if get_allowance(ctx, &granter, &grantee).is_ok() {
        return Err(sdkerr::invalid_request("fee allowance already exists"));
    }
    // `msg.GetFeeAllowanceI()`: `msg.Allowance.GetCachedValue()` on a
    // nil `*Any` (absent allowance) dereferences it.
    let a = msg
        .allowance
        .as_ref()
        .ok_or_else(|| {
            sdk::error::go_panic("runtime error: invalid memory address or nil pointer dereference")
        })
        .and_then(|any| {
            Allowance::check_shape(any)?;
            Allowance::from_any(any)
        })?;
    a.validate_basic()?;
    grant_allowance(ctx, &granter, &grantee, &a)?;
    Ok(MsgGrantAllowanceResponse {})
}

/// `MsgRevokeAllowance`.
pub fn handle_revoke_allowance(
    ctx: &mut Context<'_>,
    msg: &MsgRevokeAllowance,
) -> Result<MsgRevokeAllowanceResponse> {
    if msg.grantee == msg.granter {
        return Err(sdkerr::invalid_address("addresses must be different"));
    }
    let grantee = sdk::address::acc_codec_to_bytes(&msg.grantee)?;
    let granter = sdk::address::acc_codec_to_bytes(&msg.granter)?;
    revoke_allowance(ctx, &granter, &grantee)?;
    Ok(MsgRevokeAllowanceResponse {})
}

/// `MsgPruneAllowances`.
pub fn handle_prune_allowances(
    ctx: &mut Context<'_>,
    msg: &MsgPruneAllowances,
) -> Result<MsgPruneAllowancesResponse> {
    remove_expired_allowances(ctx, 75)?;
    ctx.events
        .emit(event("prune_feegrant", [("pruner", msg.pruner.clone())]));
    Ok(MsgPruneAllowancesResponse {})
}

/// `InitGenesis`.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &GenesisState) -> Result<()> {
    for g in &gs.allowances {
        let granter = acc_from_bech32(&g.granter)?;
        let grantee = acc_from_bech32(&g.grantee)?;
        let a = Allowance::from_grant(g)?;
        grant_allowance(ctx, &granter, &grantee, &a)?;
    }
    Ok(())
}

/// `ExportGenesis`: every allowance (`Grant`) in store order.
pub fn export_genesis(ctx: &mut Context<'_>) -> Result<GenesisState> {
    let mut it = ctx.prefix_iter(STORE_KEY, &[0x00])?;
    let mut allowances = Vec::new();
    while let Some((_, v)) = it.next(ctx)? {
        allowances.push(
            <celestia_proto::cosmos::feegrant::v1beta1::Grant as prost::Message>::decode(
                v.as_slice(),
            )
            .map_err(|e| sdkerr::json_unmarshal(e.to_string()))?,
        );
    }
    Ok(GenesisState { allowances })
}

/// `InitGenesis` from JSON.
pub fn init_genesis_json(ctx: &mut Context<'_>, genesis: &serde_json::Value) -> Result<()> {
    let gs: GenesisState = serde_json::from_value(genesis.clone())
        .map_err(|e| sdkerr::json_unmarshal(e.to_string()))?;
    init_genesis(ctx, &gs)
}

/// Keeps `SdkError` reachable for downstream signatures.
pub type FeegrantError = SdkError;
