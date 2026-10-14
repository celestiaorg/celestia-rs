//! Authorization types: `GenericAuthorization`, bank `SendAuthorization`,
//! staking `StakeAuthorization`.

use celestia_proto::Any;
use celestia_proto::cosmos::authz::v1beta1::{GenericAuthorization, Grant};
use celestia_proto::cosmos::bank::v1beta1::{MsgSend, SendAuthorization};
use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;
use celestia_proto::cosmos::staking::v1beta1 as staking;
use celestia_proto::cosmos::staking::v1beta1::stake_authorization::Policy;
use prost::{Message, Name};
use sdk::address::undefined;
use sdk::coins::{Coin, Coins};
use sdk::error::sdk as sdkerr;
use sdk::{Context, Result, SdkError};

use crate::errors;

/// Gas per allow/deny-list iteration of the bank and staking authorizations.
pub const LIST_GAS_PER_ITERATION: u64 = 10;

/// A decoded authorization.
#[derive(Debug, Clone, PartialEq)]
pub enum Authorization {
    /// `GenericAuthorization`.
    Generic(GenericAuthorization),
    /// bank `SendAuthorization`.
    Send(SendAuthorization),
    /// staking `StakeAuthorization`.
    Stake(staking::StakeAuthorization),
}

/// `authz.AcceptResponse`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AcceptResponse {
    /// The message is authorized.
    pub accept: bool,
    /// The grant must be deleted.
    pub delete: bool,
    /// The grant's new authorization.
    pub updated: Option<Authorization>,
}

fn decode<M: Message + Default>(bz: &[u8]) -> Result<M> {
    M::decode(bz).map_err(|e| sdkerr::unpack_any(e.to_string()))
}

/// `sdk.NewCoins`: sanitizes and validates (panics on invalid coins).
fn new_coins(cs: &[PbCoin]) -> Result<Coins> {
    let c = Coins::from_proto(cs).map_err(|e| sdk::error::go_panic(e.log))?;
    let mut v = c.0;
    v.retain(|c| !c.is_zero());
    v.sort_by(|a, b| a.denom.cmp(&b.denom));
    if v.windows(2).any(|w| w[0].denom == w[1].denom) || !Coins(v.clone()).is_valid() {
        return Err(sdk::error::go_panic("invalid coins"));
    }
    Ok(Coins(v))
}

/// `Coins.SafeSub`: `(a - b, any negative)`.
fn safe_sub(a: &Coins, b: &Coins) -> Result<(Coins, bool)> {
    let mut neg = Vec::new();
    for c in b {
        neg.push(Coin::with_amount(c.denom.clone(), c.amount.neg()));
    }
    let diff = a.add(&Coins(neg))?;
    let negative = diff.iter().any(|c| c.amount.is_negative());
    Ok((diff, negative))
}

impl Authorization {
    /// Decodes a packed authorization.
    pub fn from_any(any: &Any) -> Result<Self> {
        let v = any.value.as_slice();
        Ok(match any.type_url.as_str() {
            u if u == GenericAuthorization::type_url() => Self::Generic(decode(v)?),
            u if u == SendAuthorization::type_url() => Self::Send(decode(v)?),
            u if u == staking::StakeAuthorization::type_url() => Self::Stake(decode(v)?),
            other => {
                return Err(sdkerr::invalid_type(format!(
                    "expected authz.Authorization, got {other}"
                )));
            }
        })
    }

    /// `Grant.GetAuthorization`.
    pub fn from_grant(g: &Grant) -> Result<Self> {
        let any = g
            .authorization
            .as_ref()
            .ok_or_else(|| sdkerr::invalid_type("authorization is nil"))?;
        Self::from_any(any)
    }

    /// Packs into `Any`.
    #[must_use]
    pub fn to_any(&self) -> Any {
        match self {
            Self::Generic(a) => Any::pack(a),
            Self::Send(a) => Any::pack(a),
            Self::Stake(a) => Any::pack(a),
        }
    }

    /// `MsgTypeURL`.
    pub fn msg_type_url(&self) -> Result<String> {
        Ok(match self {
            Self::Generic(a) => a.msg.clone(),
            Self::Send(_) => MsgSend::type_url(),
            Self::Stake(a) => stake_msg_type(a.authorization_type)?,
        })
    }

    /// `ValidateBasic`.
    pub fn validate_basic(&self) -> Result<()> {
        match self {
            Self::Generic(a) => {
                if a.msg.is_empty() {
                    return Err(undefined("msg type cannot be empty"));
                }
                Ok(())
            }
            Self::Send(a) => {
                if a.spend_limit.is_empty() {
                    return Err(sdkerr::invalid_coins("spend limit cannot be nil"));
                }
                let lim = Coins::from_proto(&a.spend_limit)?;
                if !lim.iter().all(|c| c.amount.is_positive()) {
                    return Err(sdkerr::invalid_coins("spend limit must be positive"));
                }
                let mut seen = std::collections::HashSet::new();
                for addr in &a.allow_list {
                    if !seen.insert(addr) {
                        return Err(SdkError::new("bank", 8, "duplicate entry"));
                    }
                }
                Ok(())
            }
            Self::Stake(a) => {
                if let Some(m) = &a.max_tokens {
                    let c = Coin::from_proto(m)?;
                    if c.amount.is_negative() {
                        return Err(errors::negative_max_tokens()
                            .wrap(format!("negative coin amount: {c}")));
                    }
                }
                if a.authorization_type == 0 {
                    return Err(errors::unknown_authorization_type());
                }
                Ok(())
            }
        }
    }

    /// `Accept`.
    pub fn accept(&self, ctx: &mut Context<'_>, msg: &Any) -> Result<AcceptResponse> {
        match self {
            Self::Generic(_) => Ok(AcceptResponse {
                accept: true,
                ..AcceptResponse::default()
            }),
            Self::Send(a) => accept_send(ctx, a, msg),
            Self::Stake(a) => accept_stake(ctx, a, msg),
        }
    }
}

fn consume(ctx: &mut Context<'_>, desc: &'static str) -> Result<()> {
    ctx.gas
        .consume(LIST_GAS_PER_ITERATION, desc)
        .map_err(|_| sdkerr::out_of_gas(""))
}

fn accept_send(ctx: &mut Context<'_>, a: &SendAuthorization, msg: &Any) -> Result<AcceptResponse> {
    if msg.type_url != MsgSend::type_url() {
        return Err(sdkerr::invalid_type("type mismatch"));
    }
    let m: MsgSend = decode(&msg.value)?;
    let limit = Coins::from_proto(&a.spend_limit)?;
    let (left, negative) = safe_sub(&limit, &new_coins(&m.amount)?)?;
    if negative {
        return Err(sdkerr::insufficient_funds(
            "requested amount is more than spend limit",
        ));
    }
    let mut found = false;
    for addr in &a.allow_list {
        consume(ctx, "send authorization")?;
        if *addr == m.to_address {
            found = true;
            break;
        }
    }
    if !a.allow_list.is_empty() && !found {
        return Err(sdkerr::unauthorized(format!(
            "cannot send to {} address",
            m.to_address
        )));
    }
    if left.is_empty() {
        return Ok(AcceptResponse {
            accept: true,
            delete: true,
            updated: None,
        });
    }
    Ok(AcceptResponse {
        accept: true,
        delete: false,
        updated: Some(Authorization::Send(SendAuthorization {
            spend_limit: left.to_proto(),
            allow_list: a.allow_list.clone(),
        })),
    })
}

fn stake_msg_type(t: i32) -> Result<String> {
    Ok(match t {
        1 => staking::MsgDelegate::type_url(),
        2 => staking::MsgUndelegate::type_url(),
        3 => staking::MsgBeginRedelegate::type_url(),
        4 => staking::MsgCancelUnbondingDelegation::type_url(),
        _ => {
            return Err(sdk::error::go_panic(
                "cannot normalize authz type: unknown authorization type",
            ));
        }
    })
}

fn accept_stake(
    ctx: &mut Context<'_>,
    a: &staking::StakeAuthorization,
    msg: &Any,
) -> Result<AcceptResponse> {
    let (val, amount) = match msg.type_url.as_str() {
        u if u == staking::MsgDelegate::type_url() => {
            let m: staking::MsgDelegate = decode(&msg.value)?;
            (m.validator_address, m.amount)
        }
        u if u == staking::MsgUndelegate::type_url() => {
            let m: staking::MsgUndelegate = decode(&msg.value)?;
            (m.validator_address, m.amount)
        }
        u if u == staking::MsgBeginRedelegate::type_url() => {
            let m: staking::MsgBeginRedelegate = decode(&msg.value)?;
            (m.validator_dst_address, m.amount)
        }
        u if u == staking::MsgCancelUnbondingDelegation::type_url() => {
            let m: staking::MsgCancelUnbondingDelegation = decode(&msg.value)?;
            (m.validator_address, m.amount)
        }
        _ => return Err(sdkerr::invalid_request("unknown msg type")),
    };
    let amount = Coin::from_proto(&amount.unwrap_or_default())?;
    let (allow, deny): (&[String], &[String]) = match &a.policy {
        Some(Policy::AllowList(v)) => (&v.address, &[]),
        Some(Policy::DenyList(v)) => (&[], &v.address),
        None => (&[], &[]),
    };
    let mut exists = false;
    for v in allow {
        consume(ctx, "stake authorization")?;
        if *v == val {
            exists = true;
            break;
        }
    }
    for v in deny {
        consume(ctx, "stake authorization")?;
        if *v == val {
            return Err(sdkerr::unauthorized(format!(
                "cannot delegate/undelegate to {v} validator"
            )));
        }
    }
    if !allow.is_empty() && !exists {
        return Err(sdkerr::unauthorized(format!(
            "cannot delegate/undelegate to {val} validator"
        )));
    }
    let Some(max) = &a.max_tokens else {
        return Ok(AcceptResponse {
            accept: true,
            delete: false,
            updated: Some(Authorization::Stake(staking::StakeAuthorization {
                max_tokens: None,
                authorization_type: a.authorization_type,
                policy: a.policy.clone(),
            })),
        });
    };
    let max = Coin::from_proto(max)?;
    if max.denom != amount.denom {
        return Err(undefined(format!(
            "invalid coin denoms: {}, {}",
            max.denom, amount.denom
        )));
    }
    let left = max
        .amount
        .checked_sub(&amount.amount)
        .map_err(|e| sdk::error::go_panic(e.to_string()))?;
    if left.is_negative() {
        return Err(undefined(format!(
            "negative coin amount: {left}{}",
            max.denom
        )));
    }
    if left.is_zero() {
        return Ok(AcceptResponse {
            accept: true,
            delete: true,
            updated: None,
        });
    }
    Ok(AcceptResponse {
        accept: true,
        delete: false,
        updated: Some(Authorization::Stake(staking::StakeAuthorization {
            max_tokens: Some(Coin::with_amount(max.denom, left).to_proto()),
            authorization_type: a.authorization_type,
            policy: a.policy.clone(),
        })),
    })
}
