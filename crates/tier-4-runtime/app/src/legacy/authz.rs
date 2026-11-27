//! cosmos-sdk v0.46 `x/authz` of celestia-app v1 (`v1.31.2-sdk-v0.46.16`
//! `x/authz/msgs.go`, `keeper/msg_server.go`, `keeper/keeper.go`):
//! `ValidateBasic` and the `MsgGrant`, `MsgRevoke` and `MsgExec` servers.
//!
//! The store layout is the one later releases kept ([`x_authz::grant_key`],
//! [`x_authz::grant_queue_key`]). v0.46 differs from them in its checks
//! (no expiration check in `ValidateBasic`, no blocked-address check for
//! a new grantee account), its errors and its events, and in that nested
//! `MsgExec` messages are validated by the router (`ValidateBasic`, with
//! celestia's zero-amount exception) rather than by authz.
//!
//! Authorizations: `GenericAuthorization`, staking `StakeAuthorization`
//! and bank `SendAuthorization` as v0.46 defines them, and ibc-go v6
//! `TransferAuthorization`. Any authorization type the v1 interface
//! registry does not know (a decode failure in Go) is not ported.

#![allow(clippy::similar_names)] // granter / grantee

use celestia_proto::Any;
use celestia_proto::cosmos::authz::v1beta1::{
    self as authz, EventGrant, EventRevoke, GenericAuthorization, Grant, GrantQueueItem,
};
use celestia_proto::cosmos::bank::v1beta1 as bank;
use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;
use celestia_proto::cosmos::distribution::v1beta1 as distr;
use celestia_proto::cosmos::feegrant::v1beta1 as feegrant;
use celestia_proto::cosmos::gov;
use celestia_proto::cosmos::staking::v1beta1 as staking;
use celestia_proto::ibc::applications::transfer::v1 as transfer;
use pbjson_types::Timestamp;
use prost::{Message, Name};
use sdk::error::sdk as errs;
use sdk::{Context, Result, SdkError};
use x_authz::errors;

use super::msgs;

/// Gas per grant-queue entry `removeFromGrantQueue` looks at.
const QUEUE_GAS_PER_ITERATION: u64 = 20;
/// Gas per validator a `StakeAuthorization` list check looks at.
const STAKE_GAS_PER_ITERATION: u64 = 10;

/// The authorizations of the v1 interface registry that are ported.
#[derive(Debug, Clone)]
enum Authorization {
    Generic(GenericAuthorization),
    Stake(staking::StakeAuthorization),
    Transfer(transfer::TransferAuthorization),
    Send(bank::SendAuthorization),
}

fn decode<M: Message + Default>(msg: &Any) -> Result<M> {
    M::decode(msg.value.as_slice()).map_err(|e| errs::tx_decode(e.to_string()))
}

impl Authorization {
    /// The cached value of a decoded authorization `Any`: `Ok(None)` for
    /// an empty type URL (nothing unpacked).
    fn from_any(any: &Any) -> Result<Option<Self>> {
        let not_ported = |what: &str| {
            super::not_ported(errs::tx_decode(format!(
                "the cosmos-sdk v0.46 decoding of the authorization {what} is not ported"
            )))
        };
        let v = any.value.as_slice();
        match any.type_url.as_str() {
            "" => Ok(None),
            u if u == GenericAuthorization::type_url() => GenericAuthorization::decode(v)
                .map(|a| Some(Self::Generic(a)))
                .map_err(|_| not_ported(u)),
            u if u == staking::StakeAuthorization::type_url() => {
                let a = staking::StakeAuthorization::decode(v).map_err(|_| not_ported(u))?;
                // gogoproto decodes the amount as a `math.Int` customtype.
                if let Some(m) = &a.max_tokens
                    && !m.amount.is_empty()
                    && x_staking::msgs::wire_int(&m.amount).is_err()
                {
                    return Err(not_ported(u));
                }
                Ok(Some(Self::Stake(a)))
            }
            u if u == transfer::TransferAuthorization::type_url() => {
                let mut a =
                    transfer::TransferAuthorization::decode(v).map_err(|_| not_ported(u))?;
                for al in &mut a.allocations {
                    // ibc-go v6 has no `allowed_packet_data` (field 5):
                    // gogoproto skips it.
                    al.allowed_packet_data.clear();
                    // gogoproto decodes each amount as a `math.Int`
                    // customtype.
                    if al.spend_limit.iter().any(|c| {
                        !c.amount.is_empty() && x_staking::msgs::wire_int(&c.amount).is_err()
                    }) {
                        return Err(not_ported(u));
                    }
                }
                Ok(Some(Self::Transfer(a)))
            }
            u if u == bank::SendAuthorization::type_url() => {
                let mut a = bank::SendAuthorization::decode(v).map_err(|_| not_ported(u))?;
                // cosmos-sdk v0.46 has no `allow_list` (field 2): gogoproto
                // skips it.
                a.allow_list.clear();
                if a.spend_limit
                    .iter()
                    .any(|c| !c.amount.is_empty() && x_staking::msgs::wire_int(&c.amount).is_err())
                {
                    return Err(not_ported(u));
                }
                Ok(Some(Self::Send(a)))
            }
            other => Err(not_ported(other)),
        }
    }

    /// `Authorization.ValidateBasic`.
    fn validate_basic(&self) -> Result<()> {
        match self {
            Self::Generic(_) => Ok(()),
            Self::Stake(a) => {
                if let Some(m) = &a.max_tokens
                    && sdk::coins::raw_is_negative(m)?
                {
                    let c = sdk::coins::Coin::from_proto(m)?;
                    return Err(
                        errors::negative_max_tokens().wrap(format!("negative coin amount: {c}"))
                    );
                }
                if a.authorization_type == 0 {
                    return Err(errors::unknown_authorization_type());
                }
                Ok(())
            }
            Self::Transfer(a) => validate_transfer_authorization(a),
            Self::Send(a) => {
                // gogoproto leaves an empty repeated field nil.
                if a.spend_limit.is_empty() {
                    return Err(errs::invalid_coins("").wrap("spend limit cannot be nil"));
                }
                for c in &a.spend_limit {
                    if !sdk::coins::raw_is_positive(c)? {
                        return Err(errs::invalid_coins("").wrap("spend limit must be positive"));
                    }
                }
                Ok(())
            }
        }
    }

    /// `Authorization.Accept` of a dispatched message.
    fn accept(&self, ctx: &mut Context<'_>, msg: &Any) -> Result<Accepted> {
        match self {
            Self::Generic(_) => Ok(Accepted {
                delete: false,
                updated: None,
            }),
            Self::Stake(a) => accept_stake(ctx, a, msg),
            Self::Transfer(a) => accept_transfer(ctx, a, msg),
            Self::Send(a) => accept_send(a, msg),
        }
    }

    /// `MsgTypeURL` (a `StakeAuthorization` of an unknown type panics).
    fn msg_type_url(&self) -> Result<String> {
        match self {
            Self::Generic(a) => Ok(a.msg.clone()),
            Self::Stake(a) => match a.authorization_type {
                1 => Ok(staking::MsgDelegate::type_url()),
                2 => Ok(staking::MsgUndelegate::type_url()),
                3 => Ok(staking::MsgBeginRedelegate::type_url()),
                _ => Err(sdk::error::go_panic(format!(
                    "cannot normalize authz type with types.AuthorizationType: {}",
                    errors::unknown_authorization_type().log
                ))),
            },
            Self::Transfer(_) => Ok(transfer::MsgTransfer::type_url()),
            Self::Send(_) => Ok(bank::MsgSend::type_url()),
        }
    }

    /// `NewAnyWithValue` of the authorization as Go holds it.
    fn to_any(&self) -> Any {
        let any = match self {
            Self::Generic(a) => Any::pack(a),
            Self::Stake(a) => Any::pack(a),
            Self::Transfer(a) => Any::pack(a),
            Self::Send(a) => Any::pack(a),
        };
        txdecode::gogo_repack(&any)
    }
}

/// `Grant.ValidateBasic` of a decoded grant.
fn validate_grant(g: &Grant) -> Result<()> {
    // `g.Authorization.GetCachedValue()` on a nil `Any`.
    let any = g
        .authorization
        .as_ref()
        .ok_or_else(x_staking::msgs::nil_deref)?;
    match Authorization::from_any(any)? {
        Some(a) => a.validate_basic(),
        None => Err(errs::invalid_type("expected <nil>, got <nil>")),
    }
}

fn granter_grantee(granter: &str, grantee: &str) -> Result<()> {
    let granter = msgs::acc_address(granter, "granter")?;
    let grantee = msgs::acc_address(grantee, "grantee")?;
    if granter == grantee {
        return Err(errors::grantee_is_granter());
    }
    Ok(())
}

/// `ValidateBasic` of the authz messages; `None` for any other message.
#[must_use]
pub fn validate_basic(msg: &Any) -> Option<Result<()>> {
    Some(if msg.type_url == authz::MsgGrant::type_url() {
        decode::<authz::MsgGrant>(msg).and_then(|m| {
            granter_grantee(&m.granter, &m.grantee)?;
            validate_grant(&m.grant.unwrap_or_default())
        })
    } else if msg.type_url == authz::MsgRevoke::type_url() {
        decode::<authz::MsgRevoke>(msg).and_then(|m| {
            granter_grantee(&m.granter, &m.grantee)?;
            if m.msg_type_url.is_empty() {
                return Err(errs::invalid_request("missing method name"));
            }
            Ok(())
        })
    } else if msg.type_url == authz::MsgExec::type_url() {
        decode::<authz::MsgExec>(msg).and_then(|m| {
            msgs::acc_address(&m.grantee, "grantee")?;
            if m.msgs.is_empty() {
                return Err(errs::invalid_request("messages cannot be empty"));
            }
            Ok(())
        })
    } else {
        return None;
    })
}

/// Executes an authz message; `None` for any other message.
pub fn route(ctx: &mut Context<'_>, msg: &Any) -> Option<Result<Any>> {
    Some(if msg.type_url == authz::MsgGrant::type_url() {
        decode(msg).and_then(|m| grant(ctx, &m).map(|r| Any::pack(&r)))
    } else if msg.type_url == authz::MsgRevoke::type_url() {
        decode(msg).and_then(|m| revoke(ctx, &m).map(|r| Any::pack(&r)))
    } else if msg.type_url == authz::MsgExec::type_url() {
        decode(msg).and_then(|m| exec(ctx, &m).map(|r| Any::pack(&r)))
    } else {
        return None;
    })
}

/// `sdk.AccAddressFromBech32` in a msg server: a plain error (code 1).
fn server_acc(s: &str) -> Result<Vec<u8>> {
    sdk::address::acc_from_bech32_go(s).map_err(sdk::error::internal)
}

fn bech32(addr: &[u8]) -> Result<String> {
    x_auth::bech32(addr)
}

fn emit_typed(
    ctx: &mut Context<'_>,
    e: std::result::Result<celestia_proto::celestia_core_proto::abci::Event, serde_json::Error>,
) -> Result<()> {
    ctx.events
        .emit(e.map_err(|e| errs::json_marshal(e.to_string()))?);
    Ok(())
}

fn get_grant(ctx: &mut Context<'_>, key: &[u8]) -> Result<Option<Grant>> {
    ctx.get(x_authz::STORE_KEY, key)?
        .map(|b| Grant::decode(b.as_slice()).map_err(|e| sdk::error::go_panic(e.to_string())))
        .transpose()
}

/// authz `msgServer.Grant` (v0.46).
fn grant(ctx: &mut Context<'_>, msg: &authz::MsgGrant) -> Result<authz::MsgGrantResponse> {
    let grantee = server_acc(&msg.grantee)?;
    if x_auth::get_account(ctx, &grantee)?.is_none() {
        let acc = x_auth::new_account_with_address(ctx, &grantee)?;
        x_auth::set_account(ctx, &acc)?;
    }
    let granter = server_acc(&msg.granter)?;
    let g = msg.grant.clone().unwrap_or_default();
    let any = g
        .authorization
        .as_ref()
        .ok_or_else(|| errs::invalid_type("authorization is nil"))?;
    let auth = Authorization::from_any(any)?
        .ok_or_else(|| errs::invalid_type("expected <nil>, got <nil>"))?;
    let t = auth.msg_type_url()?;
    if !has_handler(&t) {
        return Err(errs::invalid_type(format!("{t} doesn't exist.")));
    }
    save_grant(ctx, &grantee, &granter, &auth, g.expiration.as_ref())?;
    Ok(authz::MsgGrantResponse {})
}

/// `MsgServiceRouter.HandlerByTypeURL(t) != nil`: the messages of every
/// module celestia-app v3.13.0 registers, whatever their app versions.
fn has_handler(type_url: &str) -> bool {
    (1..=3).any(|v| super::tx::gatekeeper_accepts(v, type_url) == Some(true))
}

/// `Keeper.SaveGrant` (v0.46).
fn save_grant(
    ctx: &mut Context<'_>,
    grantee: &[u8],
    granter: &[u8],
    auth: &Authorization,
    expiration: Option<&Timestamp>,
) -> Result<()> {
    let msg_type = auth.msg_type_url()?;
    let key = x_authz::grant_key(grantee, granter, &msg_type);
    // `authz.NewGrant`.
    let now = ctx.header.time;
    if let Some(exp) = expiration
        && !sdk::time::after(exp, &now)
    {
        return Err(errors::invalid_expiration_time().wrap(format!(
            "expiration must be after the current block time ({}), got {}",
            sdk::time::format_rfc3339(&now),
            sdk::time::format_rfc3339(exp)
        )));
    }
    let grant = Grant {
        authorization: Some(auth.to_any()),
        expiration: expiration.copied(),
    };
    let old_exp = get_grant(ctx, &key)?.and_then(|g| g.expiration);
    if let Some(old) = &old_exp
        && expiration.is_none_or(|e| sdk::time::cmp(e, old) != std::cmp::Ordering::Equal)
    {
        remove_from_grant_queue(ctx, &key, granter, grantee, old)?;
    }
    if let Some(exp) = expiration
        && old_exp
            .as_ref()
            .is_none_or(|o| sdk::time::cmp(o, exp) != std::cmp::Ordering::Equal)
    {
        insert_into_grant_queue(ctx, granter, grantee, &msg_type, exp)?;
    }
    ctx.set(x_authz::STORE_KEY, key, grant.encode_to_vec())?;
    let e = sdk::event::typed_event(&EventGrant {
        msg_type_url: msg_type,
        granter: bech32(granter)?,
        grantee: bech32(grantee)?,
    });
    emit_typed(ctx, e)
}

fn queue_item(ctx: &mut Context<'_>, key: &[u8]) -> Result<Option<GrantQueueItem>> {
    ctx.get(x_authz::STORE_KEY, key)?
        .map(|b| {
            GrantQueueItem::decode(b.as_slice()).map_err(|e| errs::json_unmarshal(e.to_string()))
        })
        .transpose()
}

/// `insertIntoGrantQueue`.
fn insert_into_grant_queue(
    ctx: &mut Context<'_>,
    granter: &[u8],
    grantee: &[u8],
    msg_type: &str,
    exp: &Timestamp,
) -> Result<()> {
    let key = x_authz::grant_queue_key(exp, granter, grantee);
    let mut item = queue_item(ctx, &key)?.unwrap_or_default();
    item.msg_type_urls.push(msg_type.to_owned());
    ctx.set(x_authz::STORE_KEY, key, item.encode_to_vec())
}

/// `removeFromGrantQueue`.
fn remove_from_grant_queue(
    ctx: &mut Context<'_>,
    grant_key: &[u8],
    granter: &[u8],
    grantee: &[u8],
    exp: &Timestamp,
) -> Result<()> {
    let key = x_authz::grant_queue_key(exp, granter, grantee);
    let Some(item) = queue_item(ctx, &key)? else {
        return Err(errors::no_grant_key_found()
            .wrap("can't remove grant from the expire queue, grant key not found"));
    };
    let (_, _, msg_type) = x_authz::parse_grant_key(grant_key);
    let mut urls = item.msg_type_urls;
    for i in 0..urls.len() {
        consume(ctx, QUEUE_GAS_PER_ITERATION, "grant queue")?;
        if urls[i] == msg_type {
            urls.swap_remove(i);
            let v = GrantQueueItem {
                msg_type_urls: urls,
            }
            .encode_to_vec();
            ctx.set(x_authz::STORE_KEY, key, v)?;
            break;
        }
    }
    Ok(())
}

fn consume(ctx: &mut Context<'_>, gas: u64, what: &'static str) -> Result<()> {
    ctx.gas
        .consume(gas, what)
        .map_err(|_| errs::out_of_gas(format!("out of gas in location: {what}")))
}

/// `Keeper.DeleteGrant` (v0.46).
fn delete_grant(
    ctx: &mut Context<'_>,
    grantee: &[u8],
    granter: &[u8],
    msg_type: &str,
) -> Result<()> {
    let key = x_authz::grant_key(grantee, granter, msg_type);
    let Some(grant) = get_grant(ctx, &key)? else {
        return Err(errors::no_authorization_found().wrap(format!(
            "failed to delete grant with key {}",
            gostr::from_go_bytes(&key)
        )));
    };
    if let Some(exp) = &grant.expiration {
        remove_from_grant_queue(ctx, &key, granter, grantee, exp)?;
    }
    ctx.delete(x_authz::STORE_KEY, &key)?;
    let e = sdk::event::typed_event(&EventRevoke {
        msg_type_url: msg_type.to_owned(),
        granter: bech32(granter)?,
        grantee: bech32(grantee)?,
    });
    emit_typed(ctx, e)
}

/// authz `msgServer.Revoke` (v0.46).
fn revoke(ctx: &mut Context<'_>, msg: &authz::MsgRevoke) -> Result<authz::MsgRevokeResponse> {
    let grantee = server_acc(&msg.grantee)?;
    let granter = server_acc(&msg.granter)?;
    delete_grant(ctx, &grantee, &granter, &msg.msg_type_url)?;
    Ok(authz::MsgRevokeResponse {})
}

/// `Keeper.update` (v0.46).
fn update(
    ctx: &mut Context<'_>,
    grantee: &[u8],
    granter: &[u8],
    updated: &Authorization,
) -> Result<()> {
    let key = x_authz::grant_key(grantee, granter, &updated.msg_type_url()?);
    let Some(mut grant) = get_grant(ctx, &key)? else {
        return Err(errors::no_authorization_found());
    };
    grant.authorization = Some(updated.to_any());
    ctx.set(x_authz::STORE_KEY, key, grant.encode_to_vec())
}

/// `KVGasConfig().IterNextCostFlat`, which `TransferAuthorization`
/// charges per allow-list entry it looks at.
const ITER_NEXT_COST_FLAT: u64 = 30;

/// ibc-go v6 transfer `ErrInvalidAuthorization`.
fn invalid_transfer_authorization(msg: &str) -> SdkError {
    SdkError::new("transfer", 10, "invalid transfer authorization").wrap(msg)
}

/// ibc-go v6 `TransferAuthorization.ValidateBasic`.
fn validate_transfer_authorization(a: &transfer::TransferAuthorization) -> Result<()> {
    if a.allocations.is_empty() {
        return Err(invalid_transfer_authorization(
            "allocations cannot be empty",
        ));
    }
    let mut channels = std::collections::HashSet::new();
    for al in &a.allocations {
        if !channels.insert(al.source_channel.as_str()) {
            return Err(x_ibc::errors::channel::invalid_channel(format!(
                "duplicate source channel ID: {}",
                al.source_channel
            )));
        }
        // gogoproto leaves an empty repeated field nil.
        if al.spend_limit.is_empty() {
            return Err(errs::invalid_coins("").wrap("spend limit cannot be nil"));
        }
        if let Err(e) = sdk::coins::raw_coins_validate(&al.spend_limit)? {
            return Err(errs::invalid_coins("").wrap(e));
        }
        x_ibc::host::port_identifier_validator(&al.source_port)
            .map_err(|e| e.wrap("invalid source port ID"))?;
        x_ibc::host::channel_identifier_validator(&al.source_channel)
            .map_err(|e| e.wrap("invalid source channel ID"))?;
        let mut seen = std::collections::HashSet::new();
        for addr in &al.allow_list {
            if !seen.insert(addr.as_str()) {
                // Go's `Wrapf` lacks the argument.
                return Err(invalid_transfer_authorization(
                    "duplicate entry in allow list %!s(MISSING)",
                ));
            }
        }
    }
    Ok(())
}

/// `math.Int` of a gogoproto coin amount, panicking on nil as Go does.
fn coin_int(c: &celestia_proto::cosmos::base::v1beta1::Coin) -> Result<math::Int> {
    wire_int_or_nil(&c.amount)
}

/// v0.46 `Coins.SafeSub(coinsB...)` of a valid coin set: `NewCoins(coinsB)`
/// (zeros dropped, sorted; an invalid set panics), then the sorted
/// difference without zero coins, and whether any coin is negative.
fn safe_sub_coins(limit: &[PbCoin], sub: &[PbCoin]) -> Result<(Vec<PbCoin>, bool)> {
    let mut b: Vec<(String, math::Int)> = Vec::with_capacity(sub.len());
    for c in sub {
        let v = coin_int(c)?;
        if !v.is_zero() {
            b.push((c.denom.clone(), v));
        }
    }
    b.sort_by(|x, y| x.0.cmp(&y.0));
    let invalid = b
        .iter()
        .any(|(d, v)| !sdk::coins::is_valid_denom(d) || !v.is_positive())
        || b.windows(2).any(|w| w[0].0 == w[1].0);
    if invalid {
        let text: Vec<String> = b.iter().map(|(d, v)| format!("{v}{d}")).collect();
        return Err(sdk::error::go_panic(format!(
            "invalid coin set {}",
            text.join(",")
        )));
    }
    let mut merged: std::collections::BTreeMap<String, math::Int> =
        std::collections::BTreeMap::new();
    for c in limit {
        let v = coin_int(c)?;
        let e = merged
            .entry(c.denom.clone())
            .or_insert_with(math::Int::zero);
        *e = e
            .checked_add(&v)
            .map_err(|e| sdk::error::go_panic(e.to_string()))?;
    }
    for (d, v) in b {
        let e = merged.entry(d).or_insert_with(math::Int::zero);
        *e = e
            .checked_sub(&v)
            .map_err(|e| sdk::error::go_panic(e.to_string()))?;
    }
    let mut negative = false;
    let left = merged
        .into_iter()
        .filter(|(_, v)| !v.is_zero())
        .map(|(denom, v)| {
            negative |= v.is_negative();
            PbCoin {
                denom,
                amount: v.to_string(),
            }
        })
        .collect();
    Ok((left, negative))
}

/// cosmos-sdk v0.46 bank `SendAuthorization.Accept` (no allow list).
fn accept_send(a: &bank::SendAuthorization, msg: &Any) -> Result<Accepted> {
    if msg.type_url != bank::MsgSend::type_url() {
        return Err(errs::invalid_type("type mismatch"));
    }
    let m: bank::MsgSend = decode(msg)?;
    let (left, negative) = safe_sub_coins(&a.spend_limit, &m.amount)?;
    if negative {
        return Err(errs::insufficient_funds(
            "requested amount is more than spend limit",
        ));
    }
    if left.is_empty() {
        return Ok(Accepted {
            delete: true,
            updated: None,
        });
    }
    Ok(Accepted {
        delete: false,
        updated: Some(Authorization::Send(bank::SendAuthorization {
            spend_limit: left,
            allow_list: vec![],
        })),
    })
}

/// ibc-go v6 `isAllowedAddress`: `IterNextCostFlat` per entry looked at.
fn is_allowed_address(ctx: &mut Context<'_>, receiver: &str, allowed: &[String]) -> Result<bool> {
    if allowed.is_empty() {
        return Ok(true);
    }
    for addr in allowed {
        consume(ctx, ITER_NEXT_COST_FLAT, "transfer authorization")?;
        if addr == receiver {
            return Ok(true);
        }
    }
    Ok(false)
}

/// ibc-go v6 `TransferAuthorization.Accept` (`Coins.SafeSub` of v0.46:
/// the token's coin set is sanitized, zero results are dropped).
fn accept_transfer(
    ctx: &mut Context<'_>,
    a: &transfer::TransferAuthorization,
    msg: &Any,
) -> Result<Accepted> {
    if msg.type_url != transfer::MsgTransfer::type_url() {
        return Err(errs::invalid_type("").wrap("type mismatch"));
    }
    let m: transfer::MsgTransfer = decode(msg)?;
    let token = m.token.clone().unwrap_or_default();
    let mut allocations = a.allocations.clone();
    let Some(index) = allocations
        .iter()
        .position(|al| al.source_channel == m.source_channel && al.source_port == m.source_port)
    else {
        return Err(
            errs::not_found("").wrap("requested port and channel allocation does not exist")
        );
    };
    let al = allocations[index].clone();
    if !is_allowed_address(ctx, &m.receiver, &al.allow_list)? {
        return Err(errs::invalid_address("").wrap("not allowed receiver address for transfer"));
    }
    // `SpendLimit.AmountOf(denom)` validates the denom (a panic).
    if !sdk::coins::is_valid_denom(&token.denom) {
        return Err(sdk::error::go_panic(format!(
            "invalid denom: {}",
            token.denom
        )));
    }
    let unbounded = math::Int::parse(
        "115792089237316195423570985008687907853269984665640564039457584007913129639935",
    )
    .map_err(|e| sdk::error::go_panic(e.to_string()))?;
    let mut limit_of_denom = math::Int::zero();
    for c in &al.spend_limit {
        if c.denom == token.denom {
            limit_of_denom = coin_int(c)?;
        }
    }
    if limit_of_denom == unbounded {
        return Ok(Accepted {
            delete: false,
            updated: None,
        });
    }
    let (left, negative) = safe_sub_coins(&al.spend_limit, std::slice::from_ref(&token))?;
    if negative {
        return Err(errs::insufficient_funds("").wrap("requested amount is more than spend limit"));
    }
    if left.is_empty() {
        allocations.remove(index);
        if allocations.is_empty() {
            return Ok(Accepted {
                delete: true,
                updated: None,
            });
        }
    } else {
        allocations[index].spend_limit = left;
    }
    Ok(Accepted {
        delete: false,
        updated: Some(Authorization::Transfer(transfer::TransferAuthorization {
            allocations,
        })),
    })
}

/// What `Authorization.Accept` returns.
struct Accepted {
    delete: bool,
    updated: Option<Authorization>,
}

/// `StakeAuthorization.Accept` (v0.46) of a nested staking message.
/// A gogoproto `math.Int` customtype from its wire text: empty is a nil
/// `Int`, whose arithmetic dereferences nil (a Go panic).
fn wire_int_or_nil(s: &str) -> Result<math::Int> {
    if s.is_empty() {
        return Err(sdk::error::go_panic(
            "runtime error: invalid memory address or nil pointer dereference",
        ));
    }
    x_staking::msgs::wire_int(s)
}

/// v0.46 `Coin.SafeSub` of the stake authorization's `MaxTokens` by the
/// message amount: plain errors for a denom mismatch and a negative
/// result (`Int.Sub` panics past 256 bits).
fn safe_sub(
    max: &celestia_proto::cosmos::base::v1beta1::Coin,
    amount: Option<&celestia_proto::cosmos::base::v1beta1::Coin>,
) -> Result<celestia_proto::cosmos::base::v1beta1::Coin> {
    let amount = amount.cloned().unwrap_or_default();
    if max.denom != amount.denom {
        return Err(sdk::error::internal(format!(
            "invalid coin denoms: {}, {}",
            max.denom, amount.denom
        )));
    }
    let left = wire_int_or_nil(&max.amount)?
        .checked_sub(&wire_int_or_nil(&amount.amount)?)
        .map_err(|e| sdk::error::go_panic(e.to_string()))?;
    if left.is_negative() {
        return Err(sdk::error::internal("negative coin amount"));
    }
    Ok(celestia_proto::cosmos::base::v1beta1::Coin {
        denom: max.denom.clone(),
        amount: left.to_string(),
    })
}

fn accept_stake(
    ctx: &mut Context<'_>,
    a: &staking::StakeAuthorization,
    msg: &Any,
) -> Result<Accepted> {
    use staking::stake_authorization::Policy;
    let (validator, amount) = if msg.type_url == staking::MsgDelegate::type_url() {
        let m: staking::MsgDelegate = decode(msg)?;
        (m.validator_address, m.amount)
    } else if msg.type_url == staking::MsgUndelegate::type_url() {
        let m: staking::MsgUndelegate = decode(msg)?;
        (m.validator_address, m.amount)
    } else if msg.type_url == staking::MsgBeginRedelegate::type_url() {
        let m: staking::MsgBeginRedelegate = decode(msg)?;
        (m.validator_dst_address, m.amount)
    } else {
        return Err(errs::invalid_request("unknown msg type"));
    };
    let (allow, deny): (&[String], &[String]) = match &a.policy {
        Some(Policy::AllowList(v)) => (&v.address, &[]),
        Some(Policy::DenyList(v)) => (&[], &v.address),
        None => (&[], &[]),
    };
    let mut exists = false;
    for v in allow {
        consume(ctx, STAKE_GAS_PER_ITERATION, "stake authorization")?;
        if *v == validator {
            exists = true;
            break;
        }
    }
    for v in deny {
        consume(ctx, STAKE_GAS_PER_ITERATION, "stake authorization")?;
        if *v == validator {
            return Err(errs::unauthorized(format!(
                "cannot delegate/undelegate to {v} validator"
            )));
        }
    }
    if !allow.is_empty() && !exists {
        return Err(errs::unauthorized(format!(
            "cannot delegate/undelegate to {validator} validator"
        )));
    }
    if let Some(max) = &a.max_tokens {
        let left = safe_sub(max, amount.as_ref())?;
        if left.amount == "0" {
            return Ok(Accepted {
                delete: true,
                updated: None,
            });
        }
        return Ok(Accepted {
            delete: false,
            updated: Some(Authorization::Stake(staking::StakeAuthorization {
                max_tokens: Some(left),
                policy: a.policy.clone(),
                authorization_type: a.authorization_type,
            })),
        });
    }
    Ok(Accepted {
        delete: false,
        updated: Some(Authorization::Stake(staking::StakeAuthorization {
            max_tokens: None,
            policy: a.policy.clone(),
            authorization_type: a.authorization_type,
        })),
    })
}

/// The single signer `msg.GetSigners()` returns in v0.46 for a message
/// `MsgExec` can dispatch (an address that does not parse is empty).
/// `None` for a message whose dispatch is not ported.
pub(super) fn exec_signer(msg: &Any) -> Result<Option<Vec<u8>>> {
    let acc = |s: &str| sdk::address::acc_from_bech32_go(s).unwrap_or_default();
    let u = msg.type_url.as_str();
    Ok(Some(
        if u == distr::MsgWithdrawDelegatorReward::type_url() {
            acc(&decode::<distr::MsgWithdrawDelegatorReward>(msg)?.delegator_address)
        } else if u == distr::MsgSetWithdrawAddress::type_url() {
            acc(&decode::<distr::MsgSetWithdrawAddress>(msg)?.delegator_address)
        } else if u == distr::MsgWithdrawValidatorCommission::type_url() {
            let m: distr::MsgWithdrawValidatorCommission = decode(msg)?;
            x_staking::addr::val_from_string(&m.validator_address).unwrap_or_default()
        } else if u == staking::MsgDelegate::type_url() {
            acc(&decode::<staking::MsgDelegate>(msg)?.delegator_address)
        } else if u == staking::MsgUndelegate::type_url() {
            acc(&decode::<staking::MsgUndelegate>(msg)?.delegator_address)
        } else if u == staking::MsgBeginRedelegate::type_url() {
            acc(&decode::<staking::MsgBeginRedelegate>(msg)?.delegator_address)
        } else if u == staking::MsgCancelUnbondingDelegation::type_url() {
            acc(&decode::<staking::MsgCancelUnbondingDelegation>(msg)?.delegator_address)
        } else if u == distr::MsgFundCommunityPool::type_url() {
            acc(&decode::<distr::MsgFundCommunityPool>(msg)?.depositor)
        } else if u == bank::MsgSend::type_url() {
            acc(&decode::<bank::MsgSend>(msg)?.from_address)
        } else if u == gov::v1beta1::MsgVote::type_url() {
            acc(&decode::<gov::v1beta1::MsgVote>(msg)?.voter)
        } else if u == gov::v1::MsgVote::type_url() {
            acc(&decode::<gov::v1::MsgVote>(msg)?.voter)
        } else if u == gov::v1beta1::MsgVoteWeighted::type_url() {
            acc(&decode::<gov::v1beta1::MsgVoteWeighted>(msg)?.voter)
        } else if u == gov::v1::MsgVoteWeighted::type_url() {
            acc(&decode::<gov::v1::MsgVoteWeighted>(msg)?.voter)
        } else if u == gov::v1beta1::MsgDeposit::type_url() {
            acc(&decode::<gov::v1beta1::MsgDeposit>(msg)?.depositor)
        } else if u == gov::v1::MsgDeposit::type_url() {
            acc(&decode::<gov::v1::MsgDeposit>(msg)?.depositor)
        } else if u == gov::v1beta1::MsgSubmitProposal::type_url() {
            acc(&decode::<gov::v1beta1::MsgSubmitProposal>(msg)?.proposer)
        } else if u == gov::v1::MsgSubmitProposal::type_url() {
            acc(&decode::<gov::v1::MsgSubmitProposal>(msg)?.proposer)
        } else if u == authz::MsgGrant::type_url() {
            acc(&decode::<authz::MsgGrant>(msg)?.granter)
        } else if u == authz::MsgRevoke::type_url() {
            acc(&decode::<authz::MsgRevoke>(msg)?.granter)
        } else if u == feegrant::MsgGrantAllowance::type_url() {
            acc(&decode::<feegrant::MsgGrantAllowance>(msg)?.granter)
        } else if u == feegrant::MsgRevokeAllowance::type_url() {
            acc(&decode::<feegrant::MsgRevokeAllowance>(msg)?.granter)
        } else if u == transfer::MsgTransfer::type_url() {
            // ibc-go v6 `MsgTransfer.GetSigners` panics on a sender that
            // does not parse.
            let m: transfer::MsgTransfer = decode(msg)?;
            sdk::address::acc_from_bech32_go(&m.sender).map_err(sdk::error::go_panic)?
        } else if u == celestia_proto::celestia::blob::v1::MsgPayForBlobs::type_url() {
            // celestia-app `MsgPayForBlobs.GetSigners`
            // (`sdk.MustAccAddressFromBech32`).
            let m: celestia_proto::celestia::blob::v1::MsgPayForBlobs = decode(msg)?;
            sdk::address::acc_from_bech32_go(&m.signer).map_err(sdk::error::go_panic)?
        } else {
            return Ok(None);
        },
    ))
}

/// The router's `ValidateBasic` of a dispatched message (celestia's
/// cosmos-sdk fork, `baseapp/msg_service_router.go`): a failure is
/// returned unless the message has a `GetAmount() sdk.Coin` whose amount
/// is zero, in which case its handler runs on the invalid message
/// (`Ok(true)`). A nil amount panics in `IsZero`.
fn router_validate_basic(msg: &Any) -> Result<bool> {
    let Err(e) = msgs::validate_basic(msg) else {
        return Ok(false);
    };
    let amount = if msg.type_url == staking::MsgDelegate::type_url() {
        Some(decode::<staking::MsgDelegate>(msg)?.amount)
    } else if msg.type_url == staking::MsgUndelegate::type_url() {
        Some(decode::<staking::MsgUndelegate>(msg)?.amount)
    } else if msg.type_url == staking::MsgBeginRedelegate::type_url() {
        Some(decode::<staking::MsgBeginRedelegate>(msg)?.amount)
    } else if msg.type_url == staking::MsgCancelUnbondingDelegation::type_url() {
        Some(decode::<staking::MsgCancelUnbondingDelegation>(msg)?.amount)
    } else {
        None
    };
    let Some(coin) = amount else { return Err(e) };
    let raw = coin.map(|c| c.amount).unwrap_or_default();
    if raw.is_empty() {
        return Err(sdk::error::go_panic(
            "runtime error: invalid memory address or nil pointer dereference",
        ));
    }
    if !x_staking::msgs::wire_int(&raw).is_ok_and(|a| a.is_zero()) {
        return Err(e);
    }
    Ok(true)
}

/// The msg service router's handler of `msg` (celestia's cosmos-sdk
/// fork, `baseapp/msg_service_router.go`): `ValidateBasic` (a zero
/// `GetAmount` skips its error), the circuit breaker, then the msg
/// server. Events go to `ctx.events`.
pub(super) fn router_handle(ctx: &mut Context<'_>, m: &Any) -> Result<Any> {
    let zero_amount = router_validate_basic(m)?;
    super::tx::circuit_breaker(ctx, &m.type_url)?;
    if zero_amount {
        msgs::zero_amount(ctx, m)
    } else {
        msgs::route(ctx, m)
    }
}

/// `fmt.Sprintf("%v", msg)` of a dispatched message.
fn msg_text(msg: &Any) -> String {
    x_ante::gogotext::compact_text(&msg.type_url, &msg.value, 1)
        .unwrap_or_else(|| msg.type_url.clone())
}

/// authz `msgServer.Exec` and `Keeper.DispatchActions` (v0.46).
fn exec(ctx: &mut Context<'_>, msg: &authz::MsgExec) -> Result<authz::MsgExecResponse> {
    let grantee = server_acc(&msg.grantee)?;
    let now = ctx.header.time;
    let mut results = Vec::with_capacity(msg.msgs.len());
    for (i, m) in msg.msgs.iter().enumerate() {
        // The nested message as Go holds it (the `MsgExec` keeps them as
        // received).
        let m = &msgs::gogo_view(m)?;
        // `msg.GetSigners()` (every type `decode_v1` accepts has them):
        // exactly one.
        let signers = super::signers::signers(m)?.ok_or_else(|| {
            super::not_ported(errs::unknown_request(format!(
                "the cosmos-sdk v0.46 signers of {}",
                m.type_url
            )))
        })?;
        let [granter] = <[Vec<u8>; 1]>::try_from(signers).map_err(|_| {
            SdkError::new(
                "authz",
                9,
                "authorization can be given to msg with only one signer",
            )
        })?;
        if granter != grantee {
            let key = x_authz::grant_key(&grantee, &granter, &m.type_url);
            let Some(grant) = get_grant(ctx, &key)? else {
                return Err(errors::no_authorization_found().wrap(format!(
                    "failed to update grant with key {}",
                    gostr::from_go_bytes(&key)
                )));
            };
            if grant
                .expiration
                .as_ref()
                .is_some_and(|e| sdk::time::before(e, &now))
            {
                return Err(errors::authorization_expired());
            }
            let auth = match &grant.authorization {
                None => return Err(errs::invalid_type("authorization is nil")),
                Some(any) => Authorization::from_any(any)?
                    .ok_or_else(|| errs::invalid_type("expected <nil>, got <nil>"))?,
            };
            let resp = auth.accept(ctx, m)?;
            if resp.delete {
                delete_grant(ctx, &grantee, &granter, &m.type_url)?;
            } else if let Some(u) = &resp.updated {
                update(ctx, &grantee, &granter, u)?;
            }
        }
        // The router's handler: a fresh event manager, `ValidateBasic`,
        // the circuit breaker (the gatekeeper already accepted every
        // nested message), then the msg server.
        let outer = std::mem::take(&mut ctx.events);
        let resp = router_handle(ctx, m);
        let nested = std::mem::replace(&mut ctx.events, outer).take();
        let resp = resp.map_err(|e| wrap_failed(e, m))?;
        let before = ctx.events.len();
        for e in nested {
            ctx.events.emit(e);
        }
        ctx.events
            .append_attribute_since(before, "authz_msg_index", &i.to_string());
        results.push(resp.value);
    }
    Ok(authz::MsgExecResponse { results })
}

fn wrap_failed(e: SdkError, msg: &Any) -> SdkError {
    e.wrap(format!(
        "failed to execute message; message {}",
        msg_text(msg)
    ))
}

#[cfg(test)]
mod stake_max_tokens_tests {
    use super::*;
    use celestia_proto::cosmos::base::v1beta1::Coin;

    fn coin(denom: &str, amount: &str) -> Coin {
        Coin {
            denom: denom.into(),
            amount: amount.into(),
        }
    }

    fn accept(max: Coin, amount: Coin) -> Result<Accepted> {
        let mut ms = sdk::MultiStore::new();
        let mut ctx = Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
        let a = staking::StakeAuthorization {
            max_tokens: Some(max),
            policy: None,
            authorization_type: 1,
        };
        let msg = Any::pack(&staking::MsgDelegate {
            delegator_address: String::new(),
            validator_address: "v".into(),
            amount: Some(amount),
        });
        accept_stake(&mut ctx, &a, &msg)
    }

    /// v0.46 `StakeAuthorization.Accept` with `MaxTokens`: the limit
    /// shrinks by the amount (`SafeSub`); exhausting it deletes the grant;
    /// exceeding it or another denom is a plain error; a nil limit amount
    /// panics.
    #[test]
    fn max_tokens_shrink_and_delete() {
        let r = accept(coin("utia", "100"), coin("utia", "40")).unwrap();
        assert!(!r.delete);
        let Some(Authorization::Stake(u)) = r.updated else {
            panic!("no update")
        };
        assert_eq!(u.max_tokens, Some(coin("utia", "60")));
        assert_eq!(u.authorization_type, 1);
        let r = accept(coin("utia", "100"), coin("utia", "100")).unwrap();
        assert!(r.delete && r.updated.is_none());
        for (max, amount) in [
            (coin("utia", "100"), coin("utia", "101")),
            (coin("uatom", "100"), coin("utia", "1")),
        ] {
            let e = accept(max, amount).err().unwrap();
            assert_eq!((e.codespace, e.code), ("undefined", 1), "{}", e.log);
        }
        let e = sdk::error::recover(|| accept(coin("utia", ""), coin("utia", "1")))
            .err()
            .unwrap();
        assert!(e.is_panic(), "{}", e.log);
    }
}

#[cfg(test)]
mod transfer_authorization_tests {
    use super::*;
    use celestia_proto::cosmos::base::v1beta1::Coin;

    fn coin(denom: &str, amount: &str) -> Coin {
        Coin {
            denom: denom.into(),
            amount: amount.into(),
        }
    }

    fn alloc(channel: &str, limit: Vec<Coin>, allow: &[&str]) -> transfer::Allocation {
        transfer::Allocation {
            source_port: "transfer".into(),
            source_channel: channel.into(),
            spend_limit: limit,
            allow_list: allow.iter().map(|s| (*s).to_owned()).collect(),
            allowed_packet_data: vec![],
        }
    }

    fn msg(channel: &str, token: Coin, receiver: &str) -> Any {
        Any::pack(&transfer::MsgTransfer {
            source_port: "transfer".into(),
            source_channel: channel.into(),
            token: Some(token),
            sender: String::new(),
            receiver: receiver.into(),
            ..Default::default()
        })
    }

    fn accept(a: Vec<transfer::Allocation>, m: &Any) -> (Result<Accepted>, u64) {
        let mut ms = sdk::MultiStore::new();
        let mut ctx = Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
        let r = Authorization::Transfer(transfer::TransferAuthorization { allocations: a })
            .accept(&mut ctx, m);
        (r, ctx.gas.consumed())
    }

    fn updated(r: Accepted) -> Vec<transfer::Allocation> {
        assert!(!r.delete);
        match r.updated {
            Some(Authorization::Transfer(t)) => t.allocations,
            _ => panic!("no transfer update"),
        }
    }

    /// ibc-go v6 `TransferAuthorization.Accept`: the matching allocation's
    /// spend limit shrinks; an exhausted allocation is removed and the
    /// grant deleted when none is left; the allow list costs
    /// `IterNextCostFlat` (30) per entry looked at; the max-uint256
    /// sentinel is never reduced.
    #[test]
    #[allow(clippy::many_single_char_names)]
    fn accept_spends_removes_and_deletes() {
        let a = vec![
            alloc("channel-0", vec![coin("utia", "100")], &[]),
            alloc(
                "channel-1",
                vec![coin("uatom", "5"), coin("utia", "10")],
                &[],
            ),
        ];
        let (r, _) = accept(a.clone(), &msg("channel-0", coin("utia", "40"), "r"));
        let u = updated(r.unwrap());
        assert_eq!(u[0].spend_limit, vec![coin("utia", "60")]);
        assert_eq!(u[1], a[1]);
        // Exhausting channel-0 removes it; channel-1 remains.
        let (r, _) = accept(a.clone(), &msg("channel-0", coin("utia", "100"), "r"));
        assert_eq!(updated(r.unwrap()), vec![a[1].clone()]);
        // Exhausting one denom of channel-1 keeps the other.
        let (r, _) = accept(a.clone(), &msg("channel-1", coin("utia", "10"), "r"));
        assert_eq!(u[0].source_channel, "channel-0");
        assert_eq!(updated(r.unwrap())[1].spend_limit, vec![coin("uatom", "5")]);
        // The last allocation exhausted: delete.
        let one = vec![alloc("channel-0", vec![coin("utia", "7")], &[])];
        let (r, _) = accept(one, &msg("channel-0", coin("utia", "7"), "r"));
        let r = r.unwrap();
        assert!(r.delete && r.updated.is_none());
        // Sentinel: accepted without an update.
        let max = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
        let s = vec![alloc("channel-0", vec![coin("utia", max)], &[])];
        let (r, _) = accept(s, &msg("channel-0", coin("utia", "7"), "r"));
        let r = r.unwrap();
        assert!(!r.delete && r.updated.is_none());
        // Allow list: gas per entry looked at.
        let l = vec![alloc(
            "channel-0",
            vec![coin("utia", "9")],
            &["a", "b", "c"],
        )];
        let (r, gas) = accept(l.clone(), &msg("channel-0", coin("utia", "1"), "b"));
        assert!(r.is_ok());
        assert_eq!(gas, 60);
        let (r, gas) = accept(l, &msg("channel-0", coin("utia", "1"), "z"));
        let e = r.err().unwrap();
        assert_eq!((e.codespace, e.code), ("sdk", 7), "{}", e.log);
        assert_eq!(
            e.log,
            "not allowed receiver address for transfer: invalid address"
        );
        assert_eq!(gas, 90);
    }

    #[test]
    fn accept_failures() {
        let a = vec![alloc("channel-0", vec![coin("utia", "10")], &[])];
        let e = accept(a.clone(), &msg("channel-9", coin("utia", "1"), "r"))
            .0
            .err()
            .unwrap();
        assert_eq!((e.codespace, e.code), ("sdk", 38), "{}", e.log);
        assert_eq!(
            e.log,
            "requested port and channel allocation does not exist: not found"
        );
        for token in [coin("utia", "11"), coin("uatom", "1")] {
            let e = accept(a.clone(), &msg("channel-0", token, "r"))
                .0
                .err()
                .unwrap();
            assert_eq!((e.codespace, e.code), ("sdk", 5), "{}", e.log);
            assert_eq!(
                e.log,
                "requested amount is more than spend limit: insufficient funds"
            );
        }
        let other = Any::pack(&bank::MsgSend::default());
        let e = accept(a, &other).0.err().unwrap();
        assert_eq!(e.log, "type mismatch: invalid type");
    }

    /// `TransferAuthorization.ValidateBasic` (v6).
    #[test]
    fn validate_basic_errors() {
        let v = |a: Vec<transfer::Allocation>| {
            Authorization::Transfer(transfer::TransferAuthorization { allocations: a })
                .validate_basic()
                .err()
                .map(|e| (e.codespace, e.code, e.log))
        };
        let ok = alloc("channel-0", vec![coin("utia", "1")], &["x"]);
        assert_eq!(v(vec![ok.clone()]), None);
        assert_eq!(
            v(vec![]),
            Some((
                "transfer",
                10,
                "allocations cannot be empty: invalid transfer authorization".into()
            ))
        );
        assert_eq!(
            v(vec![ok.clone(), ok.clone()]),
            Some((
                "channel",
                4,
                "duplicate source channel ID: channel-0: invalid channel".into()
            ))
        );
        assert_eq!(
            v(vec![alloc("channel-0", vec![], &[])]),
            Some(("sdk", 10, "spend limit cannot be nil: invalid coins".into()))
        );
        assert_eq!(
            v(vec![alloc("channel-0", vec![coin("utia", "0")], &[])]),
            Some((
                "sdk",
                10,
                "coin 0utia amount is not positive: invalid coins".into()
            ))
        );
        assert_eq!(
            v(vec![alloc(
                "channel-0",
                vec![coin("utia", "1")],
                &["x", "x"]
            )]),
            Some((
                "transfer",
                10,
                "duplicate entry in allow list %!s(MISSING): invalid transfer authorization".into()
            ))
        );
        let mut bad_port = ok;
        bad_port.source_port = "a".into();
        let e = v(vec![bad_port]).unwrap();
        assert!(
            e.2.starts_with("invalid source port ID: identifier a has invalid length"),
            "{}",
            e.2
        );
    }
}

#[cfg(test)]
mod send_authorization_tests {
    use super::*;
    use celestia_proto::cosmos::base::v1beta1::Coin;

    fn coin(denom: &str, amount: &str) -> Coin {
        Coin {
            denom: denom.into(),
            amount: amount.into(),
        }
    }

    fn send(amount: Vec<Coin>) -> Any {
        Any::pack(&bank::MsgSend {
            from_address: String::new(),
            to_address: String::new(),
            amount,
        })
    }

    fn accept(limit: Vec<Coin>, m: &Any) -> Result<Accepted> {
        let mut ms = sdk::MultiStore::new();
        let mut ctx = Context::new(&mut ms, sdk::Header::default(), sdk::ExecMode::Finalize);
        Authorization::Send(bank::SendAuthorization {
            spend_limit: limit,
            allow_list: vec![],
        })
        .accept(&mut ctx, m)
    }

    /// v0.46 `SendAuthorization.Accept`: `SpendLimit.SafeSub(amount...)`;
    /// an exhausted limit deletes the grant; more than the limit is
    /// `ErrInsufficientFunds`.
    #[test]
    fn accept_spends_and_deletes() {
        let limit = vec![coin("uatom", "3"), coin("utia", "10")];
        let r = accept(limit.clone(), &send(vec![coin("utia", "4")])).unwrap();
        assert!(!r.delete);
        let Some(Authorization::Send(left)) = r.updated else {
            panic!("no update")
        };
        assert_eq!(
            left.spend_limit,
            vec![coin("uatom", "3"), coin("utia", "6")]
        );
        let r = accept(
            limit.clone(),
            &send(vec![coin("uatom", "3"), coin("utia", "10")]),
        )
        .unwrap();
        assert!(r.delete && r.updated.is_none());
        for amount in [vec![coin("utia", "11")], vec![coin("ufoo", "1")]] {
            let e = accept(limit.clone(), &send(amount)).err().unwrap();
            assert_eq!((e.codespace, e.code), ("sdk", 5), "{}", e.log);
        }
        // `NewCoins` sorts the amount, then panics on a duplicate denom.
        let r = accept(
            limit.clone(),
            &send(vec![coin("utia", "1"), coin("uatom", "1")]),
        )
        .unwrap();
        assert!(!r.delete);
        let e = accept(limit, &send(vec![coin("utia", "1"), coin("utia", "1")]))
            .err()
            .unwrap();
        assert!(e.is_panic(), "{}", e.log);
    }

    /// v0.46 `SendAuthorization.ValidateBasic`.
    #[test]
    fn validate_basic() {
        let v = |limit: Vec<Coin>| {
            Authorization::Send(bank::SendAuthorization {
                spend_limit: limit,
                allow_list: vec![],
            })
            .validate_basic()
            .err()
            .map(|e| (e.code, e.log))
        };
        assert_eq!(v(vec![coin("utia", "1")]), None);
        assert_eq!(
            v(vec![]),
            Some((10, "spend limit cannot be nil: invalid coins".into()))
        );
        assert_eq!(
            v(vec![coin("utia", "0")]),
            Some((10, "spend limit must be positive: invalid coins".into()))
        );
    }
}
