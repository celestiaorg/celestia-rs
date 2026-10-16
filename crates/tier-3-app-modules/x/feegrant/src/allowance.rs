//! Fee allowances: `BasicAllowance`, `PeriodicAllowance`,
//! `AllowedMsgAllowance`.

use celestia_proto::Any;
use celestia_proto::cosmos::feegrant::v1beta1::{
    AllowedMsgAllowance, BasicAllowance, Grant, PeriodicAllowance,
};
use pbjson_types::Timestamp;
use prost::{Message, Name};
use sdk::coins::{Coin, Coins};
use sdk::error::sdk as sdkerr;
use sdk::{Context, Result};

use crate::errors;

/// Gas per allowed-message check (`gasCostPerIteration`).
pub const MSG_CHECK_GAS: u64 = 10;

/// A decoded fee allowance.
#[derive(Debug, Clone, PartialEq)]
pub enum Allowance {
    /// `BasicAllowance`.
    Basic(BasicAllowance),
    /// `PeriodicAllowance`.
    Periodic(PeriodicAllowance),
    /// `AllowedMsgAllowance` with its decoded inner allowance.
    AllowedMsg(Box<Allowance>, Vec<String>),
}

fn decode<M: Message + Default>(bz: &[u8]) -> Result<M> {
    M::decode(bz).map_err(|e| sdkerr::unpack_any(e.to_string()))
}

/// Coins as stored (no sanitizing).
fn coins(cs: &[celestia_proto::cosmos::base::v1beta1::Coin]) -> Result<Coins> {
    Coins::from_proto(cs)
}

/// `Coins.SafeSub(fee...)`: `(a - fee, any negative)` with zeros removed.
fn safe_sub(a: &Coins, fee: &Coins) -> Result<(Coins, bool)> {
    let neg: Vec<Coin> = fee
        .iter()
        .map(|c| Coin::with_amount(c.denom.clone(), c.amount.neg()))
        .collect();
    let d = a.add(&Coins(neg))?;
    let n = d.iter().any(|c| c.amount.is_negative());
    Ok((d, n))
}

/// `Coins.IsValid()` on the coins as Go decoded them (`Err`: the panic of
/// a nil amount it reaches).
fn raw_valid(cs: &[celestia_proto::cosmos::base::v1beta1::Coin]) -> Result<bool> {
    Ok(sdk::coins::raw_coins_validate(cs)?.is_ok())
}

fn basic_validate(a: &BasicAllowance) -> Result<()> {
    if !a.spend_limit.is_empty() {
        // Coins.IsValid also rejects non-positive amounts, so Go's separate
        // IsAllPositive check is unreachable.
        if !raw_valid(&a.spend_limit)? {
            return Err(sdkerr::invalid_coins(format!(
                "send amount is invalid: {}",
                sdk::coins::raw_coins_string(&a.spend_limit)
            )));
        }
    }
    if let Some(e) = &a.expiration
        && e.seconds < 0
    {
        return Err(errors::invalid_duration().wrap("expiration time cannot be negative"));
    }
    Ok(())
}

impl Allowance {
    /// Decodes a packed allowance.
    pub fn from_any(any: &Any) -> Result<Self> {
        let v = any.value.as_slice();
        Ok(match any.type_url.as_str() {
            u if u == BasicAllowance::type_url() => Self::Basic(decode(v)?),
            u if u == PeriodicAllowance::type_url() => Self::Periodic(decode(v)?),
            u if u == AllowedMsgAllowance::type_url() => {
                let a: AllowedMsgAllowance = decode(v)?;
                let inner = a
                    .allowance
                    .as_ref()
                    .ok_or_else(|| errors::no_allowance().wrap("failed to get allowance"))
                    .and_then(Self::from_any)?;
                Self::AllowedMsg(Box::new(inner), a.allowed_messages)
            }
            _ => return Err(errors::no_allowance().wrap("failed to get allowance")),
        })
    }

    /// The errors of Go's `ValidateBasic` that depend on the allowance
    /// tree's shape and come before its inner allowances are resolved
    /// (`AllowedMsgAllowance`: a nil inner allowance, then an empty
    /// message list, then `GetAllowance`), in Go's order. [`Self::from_any`]
    /// resolves the whole tree first, so the grant handler runs this
    /// before it.
    pub fn check_shape(any: &Any) -> Result<()> {
        if any.type_url == AllowedMsgAllowance::type_url() {
            let a: AllowedMsgAllowance = decode(any.value.as_slice())?;
            let Some(inner) = a.allowance.as_ref() else {
                return Err(errors::no_allowance().wrap("allowance should not be empty"));
            };
            if a.allowed_messages.is_empty() {
                return Err(errors::no_messages().wrap("allowed messages shouldn't be empty"));
            }
            return Self::check_shape(inner);
        }
        Ok(())
    }

    /// `Grant.GetGrant`.
    pub fn from_grant(g: &Grant) -> Result<Self> {
        let any = g
            .allowance
            .as_ref()
            .ok_or_else(|| errors::no_allowance().wrap("failed to get allowance"))?;
        Self::from_any(any)
    }

    /// Packs into `Any` (gogoproto layout: non-nullable fields present).
    #[must_use]
    pub fn to_any(&self) -> Any {
        match self {
            Self::Basic(a) => Any::pack(a),
            Self::Periodic(a) => {
                let mut a = a.clone();
                a.basic.get_or_insert_with(BasicAllowance::default);
                a.period.get_or_insert_with(Default::default);
                a.period_reset.get_or_insert_with(sdk::time::go_zero);
                Any::pack(&a)
            }
            Self::AllowedMsg(inner, msgs) => Any::pack(&AllowedMsgAllowance {
                allowance: Some(inner.to_any()),
                allowed_messages: msgs.clone(),
            }),
        }
    }

    /// `NewAnyWithValue` of the allowance Go holds after changing it
    /// (`SetAllowance` re-packs the inner allowance too): canonical
    /// gogoproto encoding at every level.
    #[must_use]
    pub fn to_canonical_any(&self) -> Any {
        match self {
            Self::AllowedMsg(inner, msgs) => {
                txdecode::gogo_repack(&Any::pack(&AllowedMsgAllowance {
                    allowance: Some(inner.to_canonical_any()),
                    allowed_messages: msgs.clone(),
                }))
            }
            other => txdecode::gogo_repack(&other.to_any()),
        }
    }

    /// `ExpiresAt`.
    pub fn expires_at(&self) -> Result<Option<Timestamp>> {
        Ok(match self {
            Self::Basic(a) => a.expiration,
            Self::Periodic(a) => a.basic.as_ref().and_then(|b| b.expiration),
            Self::AllowedMsg(inner, _) => inner.expires_at()?,
        })
    }

    /// `ValidateBasic`.
    pub fn validate_basic(&self) -> Result<()> {
        match self {
            Self::Basic(a) => basic_validate(a),
            Self::Periodic(a) => {
                Self::Basic(a.basic.clone().unwrap_or_default()).validate_basic()?;
                if !raw_valid(&a.period_spend_limit)? {
                    return Err(sdkerr::invalid_coins(format!(
                        "spend amount is invalid: {}",
                        sdk::coins::raw_coins_string(&a.period_spend_limit)
                    )));
                }
                if a.period_spend_limit.is_empty() {
                    return Err(sdkerr::invalid_coins("spend limit must be positive"));
                }
                if !raw_valid(&a.period_can_spend)? {
                    return Err(sdkerr::invalid_coins(format!(
                        "can spend amount is invalid: {}",
                        sdk::coins::raw_coins_string(&a.period_can_spend)
                    )));
                }
                let psl = coins(&a.period_spend_limit)?;
                let basic = a.basic.clone().unwrap_or_default();
                if !basic.spend_limit.is_empty() {
                    let bsl = coins(&basic.spend_limit)?;
                    let subset = psl.0.len() <= bsl.0.len()
                        && psl.iter().all(|c| !bsl.amount_of(&c.denom).is_zero());
                    if !subset {
                        return Err(sdkerr::invalid_coins(
                            "period spend limit has different currency than basic spend limit",
                        ));
                    }
                }
                if a.period
                    .as_ref()
                    .is_some_and(|p| p.seconds < 0 || p.nanos < 0)
                {
                    return Err(errors::invalid_duration().wrap("negative clock step"));
                }
                Ok(())
            }
            Self::AllowedMsg(inner, msgs) => {
                if msgs.is_empty() {
                    return Err(errors::no_messages().wrap("allowed messages shouldn't be empty"));
                }
                inner.validate_basic()
            }
        }
    }

    /// `Accept`: returns `(remove, result)`; the allowance is updated in
    /// place like Go's pointer receivers.
    pub fn accept(
        &mut self,
        ctx: &mut Context<'_>,
        fee: &Coins,
        msgs: &[Any],
    ) -> (bool, Result<()>) {
        match self.accept_inner(ctx, fee, msgs) {
            Ok(remove) => (remove, Ok(())),
            Err((remove, e)) => (remove, Err(e)),
        }
    }

    fn accept_inner(
        &mut self,
        ctx: &mut Context<'_>,
        fee: &Coins,
        msgs: &[Any],
    ) -> std::result::Result<bool, (bool, sdk::SdkError)> {
        let now = ctx.header.time;
        let err = |e: sdk::SdkError| (false, e);
        match self {
            Self::Basic(a) => {
                if a.expiration
                    .as_ref()
                    .is_some_and(|e| sdk::time::before(e, &now))
                {
                    return Err((true, errors::fee_limit_expired().wrap("basic allowance")));
                }
                if a.spend_limit.is_empty() {
                    return Ok(false);
                }
                let lim = coins(&a.spend_limit).map_err(err)?;
                let (left, neg) = safe_sub(&lim, fee).map_err(err)?;
                if neg {
                    return Err((false, errors::fee_limit_exceeded().wrap("basic allowance")));
                }
                a.spend_limit = left.to_proto();
                Ok(left.is_empty())
            }
            Self::Periodic(a) => {
                let basic = a.basic.get_or_insert_with(BasicAllowance::default);
                if basic
                    .expiration
                    .as_ref()
                    .is_some_and(|e| sdk::time::after(&now, e))
                {
                    return Err((true, errors::fee_limit_expired().wrap("absolute limit")));
                }
                try_reset_period(a, &now).map_err(err)?;
                let pcs = coins(&a.period_can_spend).map_err(err)?;
                let (left, neg) = safe_sub(&pcs, fee).map_err(err)?;
                a.period_can_spend = left.to_proto();
                if neg {
                    return Err((false, errors::fee_limit_exceeded().wrap("period limit")));
                }
                let basic = a.basic.get_or_insert_with(BasicAllowance::default);
                if basic.spend_limit.is_empty() {
                    return Ok(false);
                }
                let bsl = coins(&basic.spend_limit).map_err(err)?;
                let (left, neg) = safe_sub(&bsl, fee).map_err(err)?;
                basic.spend_limit = left.to_proto();
                if neg {
                    return Err((false, errors::fee_limit_exceeded().wrap("absolute limit")));
                }
                Ok(left.is_empty())
            }
            Self::AllowedMsg(inner, allowed) => {
                for _ in allowed.iter() {
                    consume(ctx).map_err(err)?;
                }
                for m in msgs {
                    consume(ctx).map_err(err)?;
                    if !allowed.contains(&m.type_url) {
                        return Err((
                            false,
                            errors::message_not_allowed()
                                .wrap("message does not exist in allowed messages"),
                        ));
                    }
                }
                let mut copy = (**inner).clone();
                let r = copy.accept_inner(ctx, fee, msgs);
                if matches!(r, Ok(false)) {
                    **inner = copy;
                }
                r
            }
        }
    }
}

fn consume(ctx: &mut Context<'_>) -> Result<()> {
    ctx.gas
        .consume(MSG_CHECK_GAS, "check msg")
        .map_err(|_| sdkerr::out_of_gas(""))
}

/// `PeriodicAllowance.tryResetPeriod`.
fn try_reset_period(a: &mut PeriodicAllowance, now: &Timestamp) -> Result<()> {
    let reset = a.period_reset.unwrap_or_else(sdk::time::go_zero);
    if sdk::time::before(now, &reset) {
        return Ok(());
    }
    let basic = a.basic.clone().unwrap_or_default();
    let bsl = coins(&basic.spend_limit)?;
    let (_, neg) = safe_sub(&bsl, &coins(&a.period_spend_limit)?)?;
    if neg && !bsl.is_empty() {
        a.period_can_spend.clone_from(&basic.spend_limit);
    } else {
        a.period_can_spend.clone_from(&a.period_spend_limit);
    }
    let period = a.period.unwrap_or_default();
    let mut next = sdk::time::add_duration(&reset, &period);
    if sdk::time::after(now, &next) {
        next = sdk::time::add_duration(now, &period);
    }
    a.period_reset = Some(next);
    Ok(())
}

#[cfg(test)]
mod tests {
    use celestia_proto::cosmos::base::v1beta1::Coin as PbCoin;

    use super::*;

    fn c(denom: &str, amount: &str) -> PbCoin {
        PbCoin {
            denom: denom.into(),
            amount: amount.into(),
        }
    }

    #[test]
    fn nil_spend_limit_amount_panics_like_go() {
        // Coins.IsValid reaches IsPositive on the nil amount: panic.
        let a = Allowance::Basic(BasicAllowance {
            spend_limit: vec![c("utia", "")],
            expiration: None,
        });
        assert!(a.validate_basic().unwrap_err().is_panic());
        // An invalid denom fails first.
        let a = Allowance::Basic(BasicAllowance {
            spend_limit: vec![c("u", "")],
            expiration: None,
        });
        let e = a.validate_basic().unwrap_err();
        assert!(!e.is_panic());
        assert_eq!(e.log, "send amount is invalid: <nil>u: invalid coins");
        let a = Allowance::Periodic(PeriodicAllowance {
            basic: None,
            period: None,
            period_spend_limit: vec![c("utia", "5")],
            period_can_spend: vec![c("utia", "")],
            period_reset: None,
        });
        assert!(a.validate_basic().unwrap_err().is_panic());
    }
}
