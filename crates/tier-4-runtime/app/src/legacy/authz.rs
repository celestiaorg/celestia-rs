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
//! Authorizations: `GenericAuthorization` and staking `StakeAuthorization`
//! as v0.46 defines them. bank `SendAuthorization` and ibc-go's
//! `TransferAuthorization`, and any authorization type the v1 interface
//! registry does not know (a decode failure in Go), are not ported.

#![allow(clippy::similar_names)] // granter / grantee

use celestia_proto::Any;
use celestia_proto::cosmos::authz::v1beta1::{
    self as authz, EventGrant, EventRevoke, GenericAuthorization, Grant, GrantQueueItem,
};
use celestia_proto::cosmos::distribution::v1beta1 as distr;
use celestia_proto::cosmos::staking::v1beta1 as staking;
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
        }
    }

    /// `NewAnyWithValue` of the authorization as Go holds it.
    fn to_any(&self) -> Any {
        let any = match self {
            Self::Generic(a) => Any::pack(a),
            Self::Stake(a) => Any::pack(a),
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
fn exec_signer(msg: &Any) -> Result<Option<Vec<u8>>> {
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
        } else {
            return Ok(None);
        },
    ))
}

/// The router's `ValidateBasic` of a dispatched message: a failure is
/// returned unless the message has a zero `GetAmount()` (celestia's
/// exception), in which case its handler runs on the invalid message,
/// which is not ported.
fn router_validate_basic(msg: &Any) -> Result<()> {
    let Err(e) = msgs::validate_basic(msg) else {
        return Ok(());
    };
    let amount = if msg.type_url == staking::MsgDelegate::type_url() {
        Some(decode::<staking::MsgDelegate>(msg)?.amount)
    } else if msg.type_url == staking::MsgUndelegate::type_url() {
        Some(decode::<staking::MsgUndelegate>(msg)?.amount)
    } else if msg.type_url == staking::MsgBeginRedelegate::type_url() {
        Some(decode::<staking::MsgBeginRedelegate>(msg)?.amount)
    } else {
        None
    };
    match amount {
        Some(c)
            if c.as_ref().is_none_or(|c| {
                c.amount.is_empty()
                    || x_staking::msgs::wire_int(&c.amount).is_ok_and(|a| a.is_zero())
            }) =>
        {
            Err(super::not_ported(errs::invalid_request(format!(
                "the cosmos-sdk v0.46 handler of {} on a zero amount its ValidateBasic rejects is not ported",
                msg.type_url
            ))))
        }
        _ => Err(e),
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
        let Some(granter) = exec_signer(m)? else {
            return Err(super::not_ported(errs::unknown_request(format!(
                "the cosmos-sdk v0.46 dispatch of {} by MsgExec is not ported",
                m.type_url
            ))));
        };
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
            let resp = match &auth {
                Authorization::Generic(_) => Accepted {
                    delete: false,
                    updated: None,
                },
                Authorization::Stake(a) => accept_stake(ctx, a, m)?,
            };
            if resp.delete {
                delete_grant(ctx, &grantee, &granter, &m.type_url)?;
            } else if let Some(u) = &resp.updated {
                update(ctx, &grantee, &granter, u)?;
            }
        }
        // The router's handler: a fresh event manager, `ValidateBasic`,
        // the circuit breaker (the gatekeeper already accepted every
        // nested message), then the msg server.
        router_validate_basic(m).map_err(|e| wrap_failed(e, m))?;
        let outer = std::mem::take(&mut ctx.events);
        let resp = msgs::route(ctx, m);
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
