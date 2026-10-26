//! Authorization types: `GenericAuthorization`, bank `SendAuthorization`,
//! staking `StakeAuthorization`.

use celestia_proto::Any;
use celestia_proto::cosmos::authz::v1beta1::{GenericAuthorization, Grant};
use celestia_proto::cosmos::bank::v1beta1::{MsgSend, SendAuthorization};
use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;
use celestia_proto::cosmos::staking::v1beta1 as staking;
use celestia_proto::cosmos::staking::v1beta1::stake_authorization::Policy;
use celestia_proto::ibc::applications::transfer::v1 as ibc_transfer;
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
    /// ibc-go `TransferAuthorization`.
    Transfer(ibc_transfer::TransferAuthorization),
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
    sdk::coins::new_coins_go(cs)
}

/// `Coins.SafeSub`: `(a - b, any negative)`.
fn safe_sub(a: &Coins, b: &Coins) -> Result<(Coins, bool)> {
    let neg: Vec<Coin> = b
        .iter()
        .map(|c| Coin::with_amount(c.denom.clone(), c.amount.neg()))
        .collect();
    let diff = safe_add(a, &neg)?;
    let negative = diff.iter().any(|c| c.amount.is_negative());
    Ok((diff, negative))
}

/// `Coins.safeAdd`: panics unless both sides are sorted (equal denoms
/// allowed: a stored spend limit is not re-validated), sums every coin of
/// a denom, drops zeros and sorts.
fn safe_add(a: &Coins, b: &[Coin]) -> Result<Coins> {
    let sorted = |v: &[Coin]| v.windows(2).all(|w| w[0].denom <= w[1].denom);
    if !sorted(&a.0) {
        return Err(sdk::error::go_panic("Coins (self) must be sorted"));
    }
    if !sorted(b) {
        return Err(sdk::error::go_panic("Wrong argument: coins must be sorted"));
    }
    let mut sums: std::collections::BTreeMap<&str, math::Int> = std::collections::BTreeMap::new();
    for c in a.iter().chain(b) {
        let sum = sums.entry(&c.denom).or_insert_with(math::Int::zero);
        *sum = sum.checked_add(&c.amount).map_err(sdk::coins::add_panic)?;
    }
    Ok(Coins(
        sums.into_iter()
            .filter(|(_, amount)| !amount.is_zero())
            .map(|(denom, amount)| Coin::with_amount(denom, amount))
            .collect(),
    ))
}

impl Authorization {
    /// Decodes a packed authorization.
    pub fn from_any(any: &Any) -> Result<Self> {
        let v = any.value.as_slice();
        Ok(match any.type_url.as_str() {
            u if u == GenericAuthorization::type_url() => Self::Generic(decode(v)?),
            u if u == SendAuthorization::type_url() => Self::Send(decode(v)?),
            u if u == staking::StakeAuthorization::type_url() => Self::Stake(decode(v)?),
            u if u == ibc_transfer::TransferAuthorization::type_url() => Self::Transfer(decode(v)?),
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
            Self::Transfer(a) => Any::pack(a),
        }
    }

    /// `MsgTypeURL`.
    pub fn msg_type_url(&self) -> Result<String> {
        Ok(match self {
            Self::Generic(a) => a.msg.clone(),
            Self::Send(_) => MsgSend::type_url(),
            Self::Stake(a) => stake_msg_type(a.authorization_type)?,
            Self::Transfer(_) => ibc_transfer::MsgTransfer::type_url(),
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
                // IsAllPositive: a nil amount panics once reached.
                for c in &a.spend_limit {
                    if !sdk::coins::raw_is_positive(c)? {
                        return Err(sdkerr::invalid_coins("spend limit must be positive"));
                    }
                }
                let mut seen = std::collections::HashSet::new();
                for addr in &a.allow_list {
                    if !seen.insert(addr) {
                        return Err(SdkError::new("bank", 8, "duplicate entry"));
                    }
                }
                Ok(())
            }
            Self::Transfer(a) => validate_transfer(a),
            Self::Stake(a) => {
                if let Some(m) = &a.max_tokens
                    && sdk::coins::raw_is_negative(m)?
                {
                    let c = Coin::from_proto(m)?;
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

    /// `Accept`.
    pub fn accept(&self, ctx: &mut Context<'_>, msg: &Any) -> Result<AcceptResponse> {
        match self {
            Self::Generic(_) => Ok(AcceptResponse {
                accept: true,
                ..AcceptResponse::default()
            }),
            Self::Send(a) => accept_send(ctx, a, msg),
            Self::Stake(a) => accept_stake(ctx, a, msg),
            Self::Transfer(a) => accept_transfer(ctx, a, msg),
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

/// ibc-go `ErrInvalidAuthorization` (transfer/10).
fn invalid_transfer_authorization(log: &str) -> SdkError {
    SdkError::new(
        "transfer",
        10,
        if log.is_empty() {
            "invalid transfer authorization".to_owned()
        } else {
            format!("{log}: invalid transfer authorization")
        },
    )
}

/// An ibc-go `ibcerrors` error (codespace `ibc`).
fn ibc_err(code: u32, desc: &str, log: &str) -> SdkError {
    SdkError::new("ibc", code, format!("{log}: {desc}"))
}

/// `TransferAuthorization.ValidateBasic` (ibc-go v8).
fn validate_transfer(a: &ibc_transfer::TransferAuthorization) -> Result<()> {
    if a.allocations.is_empty() {
        return Err(invalid_transfer_authorization(
            "allocations cannot be empty",
        ));
    }
    let mut channels = std::collections::HashSet::new();
    for al in &a.allocations {
        if !channels.insert(al.source_channel.as_str()) {
            return Err(SdkError::new(
                "channel",
                4,
                format!(
                    "duplicate source channel ID: {}: invalid channel",
                    al.source_channel
                ),
            ));
        }
        if al.spend_limit.is_empty() {
            return Err(ibc_err(6, "invalid coins", "spend limit cannot be nil"));
        }
        if let Err(e) = sdk::coins::raw_coins_validate(&al.spend_limit)? {
            return Err(ibc_err(6, "invalid coins", &e));
        }
        x_ibc::host::port_identifier_validator(&al.source_port)
            .map_err(|e| e.wrap("invalid source port ID"))?;
        x_ibc::host::channel_identifier_validator(&al.source_channel)
            .map_err(|e| e.wrap("invalid source channel ID"))?;
        let mut seen = std::collections::HashSet::new();
        for addr in &al.allow_list {
            if !seen.insert(addr) {
                return Err(invalid_transfer_authorization(&format!(
                    "duplicate entry in allow list {addr}"
                )));
            }
        }
    }
    Ok(())
}

/// `KVGasConfig().IterNextCostFlat`.
const ITER_NEXT_COST_FLAT: u64 = 30;

/// `TransferAuthorization.Accept` (ibc-go v8).
fn accept_transfer(
    ctx: &mut Context<'_>,
    a: &ibc_transfer::TransferAuthorization,
    msg: &Any,
) -> Result<AcceptResponse> {
    if msg.type_url != ibc_transfer::MsgTransfer::type_url() {
        return Err(ibc_err(12, "invalid type", "type mismatch"));
    }
    let m: ibc_transfer::MsgTransfer = decode(&msg.value)?;
    let gas = |ctx: &mut Context<'_>| {
        ctx.gas
            .consume(ITER_NEXT_COST_FLAT, "transfer authorization")
            .map_err(|_| sdkerr::out_of_gas(""))
    };
    for (index, al) in a.allocations.iter().enumerate() {
        if !(al.source_channel == m.source_channel && al.source_port == m.source_port) {
            continue;
        }
        // isAllowedAddress
        if !al.allow_list.is_empty() {
            let mut allowed = false;
            for addr in &al.allow_list {
                gas(ctx)?;
                if *addr == m.receiver {
                    allowed = true;
                    break;
                }
            }
            if !allowed {
                return Err(ibc_err(
                    5,
                    "invalid address",
                    "not allowed receiver address for transfer",
                ));
            }
        }
        // validateMemo
        if al.allowed_packet_data.is_empty() {
            if !m.memo.trim().is_empty() {
                return Err(invalid_transfer_authorization(
                    "memo must be empty because allowed packet data in allocation is empty",
                ));
            }
        } else if !(al.allowed_packet_data.len() == 1 && al.allowed_packet_data[0] == "*") {
            let mut ok = false;
            for allowed in &al.allowed_packet_data {
                gas(ctx)?;
                if m.memo.trim() == allowed.trim() {
                    ok = true;
                    break;
                }
            }
            if !ok {
                return Err(invalid_transfer_authorization(&format!(
                    "not allowed memo: {}",
                    m.memo
                )));
            }
        }
        let token = Coin::from_proto(&m.token.clone().unwrap_or_default())?;
        let limit = Coins::from_proto(&al.spend_limit)?;
        let unbounded = math::Int::parse(
            "115792089237316195423570985008687907853269984665640564039457584007913129639935",
        );
        if unbounded.is_ok_and(|u| limit.amount_of(&token.denom) == u) {
            return Ok(AcceptResponse {
                accept: true,
                delete: false,
                updated: None,
            });
        }
        let (left, negative) = safe_sub(&limit, &Coins(vec![token]))?;
        if negative {
            return Err(ibc_err(
                3,
                "insufficient funds",
                "requested amount is more than spend limit",
            ));
        }
        let mut allocations = a.allocations.clone();
        if left.is_empty() {
            allocations.remove(index);
            if allocations.is_empty() {
                return Ok(AcceptResponse {
                    accept: true,
                    delete: true,
                    updated: None,
                });
            }
        } else {
            allocations[index].spend_limit = left.to_proto();
        }
        return Ok(AcceptResponse {
            accept: true,
            delete: false,
            updated: Some(Authorization::Transfer(
                ibc_transfer::TransferAuthorization { allocations },
            )),
        });
    }
    Err(ibc_err(
        16,
        "not found",
        "requested port and channel allocation does not exist",
    ))
}

fn stake_msg_type(t: i32) -> Result<String> {
    Ok(match t {
        1 => staking::MsgDelegate::type_url(),
        2 => staking::MsgUndelegate::type_url(),
        3 => staking::MsgBeginRedelegate::type_url(),
        4 => staking::MsgCancelUnbondingDelegation::type_url(),
        _ => {
            // `MsgTypeURL` panics with `normalizeAuthzType`'s
            // `errorsmod.Wrapf` error; `%v` of it (gov's `PANICKED: %v`,
            // a proposal's `failed_reason`) carries the wrap's frame.
            return Err(sdk::error::go_panic(format!(
                "cannot normalize authz type with types.AuthorizationType: unknown authorization type{}",
                sdk::version::cosmos_sdk_frame("x/staking/types/authz.go:193")
            )));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn c(denom: &str, amount: &str) -> PbCoin {
        PbCoin {
            denom: denom.into(),
            amount: amount.into(),
        }
    }

    #[test]
    fn nil_amounts_panic_like_go() {
        // SendAuthorization: IsAllPositive reaches the nil amount.
        let a = Authorization::Send(SendAuthorization {
            spend_limit: vec![c("u", "")],
            allow_list: Vec::new(),
        });
        assert!(a.validate_basic().unwrap_err().is_panic());
        // ... unless an earlier coin is not positive.
        let a = Authorization::Send(SendAuthorization {
            spend_limit: vec![c("utia", "0"), c("uxyz", "")],
            allow_list: Vec::new(),
        });
        assert!(!a.validate_basic().unwrap_err().is_panic());
        // StakeAuthorization: MaxTokens.IsNegative.
        let a = Authorization::Stake(staking::StakeAuthorization {
            max_tokens: Some(c("utia", "")),
            authorization_type: 1,
            policy: None,
        });
        assert!(a.validate_basic().unwrap_err().is_panic());
        // TransferAuthorization: SpendLimit.Validate's own error text.
        let alloc = |spend_limit| ibc_transfer::Allocation {
            source_port: "transfer".into(),
            source_channel: "channel-0".into(),
            spend_limit,
            allow_list: Vec::new(),
            allowed_packet_data: Vec::new(),
        };
        let a = Authorization::Transfer(ibc_transfer::TransferAuthorization {
            allocations: vec![alloc(vec![c("u", "")])],
        });
        let e = a.validate_basic().unwrap_err();
        assert_eq!(e.log, "invalid denom: u: invalid coins");
        let a = Authorization::Transfer(ibc_transfer::TransferAuthorization {
            allocations: vec![alloc(vec![c("utia", "0")])],
        });
        let e = a.validate_basic().unwrap_err();
        assert_eq!(e.log, "coin 0utia amount is not positive: invalid coins");
        let a = Authorization::Transfer(ibc_transfer::TransferAuthorization {
            allocations: vec![alloc(vec![c("utia", "")])],
        });
        assert!(a.validate_basic().unwrap_err().is_panic());
    }

    /// `SpendLimit.SafeSub` (`safeAdd`) of a stored spend limit, which
    /// `ValidateBasic` does not check for order: an unsorted one panics
    /// before the allow list is read; equal denoms are summed (deliver-diff
    /// `v9-upgrade` seed 10498).
    #[test]
    fn spend_limit_safe_sub_like_go() {
        let coins = |v: &[(&str, i64)]| {
            Coins(
                v.iter()
                    .map(|(d, a)| Coin::with_amount(*d, math::Int::from(*a)))
                    .collect(),
            )
        };
        let e = safe_sub(&coins(&[("utib", 5), ("utia", 5)]), &coins(&[("utia", 1)])).unwrap_err();
        assert!(e.is_panic());
        assert_eq!(e.log, "Coins (self) must be sorted");
        let (left, neg) =
            safe_sub(&coins(&[("utia", 5), ("utia", 3)]), &coins(&[("utia", 1)])).unwrap();
        assert!(!neg);
        assert_eq!(left, coins(&[("utia", 7)]));
        let (left, neg) =
            safe_sub(&coins(&[("utia", 1)]), &coins(&[("utia", 1), ("utib", 2)])).unwrap();
        assert!(neg);
        assert_eq!(left, coins(&[("utib", -2)]));
    }

    /// `StakeAuthorization.MsgTypeURL` panics with `normalizeAuthzType`'s
    /// wrapped error; gov's `PANICKED: %v` prints its frame in the
    /// executing release's cosmos-sdk (deliver-diff `v9-upgrade` seed 8513).
    #[test]
    fn unknown_stake_type_panics_with_go_s_frame() {
        let a = Authorization::Stake(staking::StakeAuthorization {
            authorization_type: 5,
            ..Default::default()
        });
        for (v, sdk) in [(9, "v0.52.8"), (10, "v0.52.12")] {
            sdk::version::set_app_version(v);
            let e = a.msg_type_url().unwrap_err();
            assert!(e.is_panic());
            assert_eq!(
                e.log,
                format!(
                    "cannot normalize authz type with types.AuthorizationType: unknown authorization type [celestiaorg/cosmos-sdk@{sdk}/x/staking/types/authz.go:193]"
                )
            );
        }
        sdk::version::set_app_version(0);
    }
}
